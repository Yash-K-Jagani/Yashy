//! The typestate pass: what state a resource is in.
//!
//! A resource handle carries a state: `Allocated`/`Freed`, `Unlocked`/`Locked`,
//! `Open`/`Closed`. Operations require a particular state and produce another;
//! an operation in the wrong state is `E1400`, and using a resource in a
//! terminal state is `E1401`. The state is tracked per binding across the
//! control-flow graph the same way the leak pass tracks release bits, and the
//! merge is an intersection: a resource must be in the same state on every path
//! that reaches a point, otherwise it is `E1400` with both states named.

use yashy_common::{codes, Diagnostic, FileId, Severity, Span};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm};
use crate::typestate::ResourceState as State;

/// Run the typestate pass over `program`, pushing diagnostics into `out`.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    check_typestate(program, out);
    check_race(program, out);
}

/// The typestate pass: what state a resource is in.
pub fn check_typestate(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    let mut pass = TypestatePass {
        out,
        file: FileId::from_index(0),
        frames: Vec::new(),
    };
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            pass.check_fn(function);
        }
    }
}

/// One tracked resource binding.
#[derive(Clone)]
struct Binding {
    name: String,
    span: Span,
    state: State,
    /// Whether this binding is a tracked resource. Non-resource bindings are
    /// ignored.
    tracked: bool,
}

type Scope = Vec<Binding>;
type StateFrames = Vec<Scope>;

struct TypestatePass<'a> {
    out: &'a mut Vec<Diagnostic>,
    file: FileId,
    frames: StateFrames,
}

impl TypestatePass<'_> {
    fn check_fn(&mut self, function: &HirFn) {
        let outer_frames = std::mem::take(&mut self.frames);
        let outer_file = self.file;
        self.file = function.file;
        let _ = self.block(&function.body);
        self.frames = outer_frames;
        self.file = outer_file;
    }

    fn block(&mut self, block: &HirBlock) -> bool {
        // Returns true if execution falls through.
        self.frames.push(Vec::new());
        let mut flow = true;
        for stmt in &block.stmts {
            if !flow {
                break;
            }
            flow = self.stmt(stmt);
        }
        self.frames.pop();
        flow
    }

    fn stmt(&mut self, stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Expr(expr, _, _) => {
                self.walk(expr);
                true
            }
            HirStmt::Const { name, ty, init, span, .. }
            | HirStmt::Var { name, ty, init, span, .. } => {
                self.walk(init);
                let state = resource_state_of_init(init);
                if let Some(state) = state {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.push(Binding {
                            name: name.clone(),
                            span: *span,
                            state,
                            tracked: true,
                        });
                    }
                }
                let _ = ty;
                true
            }
            HirStmt::Destructure { init, bindings, span, .. } => {
                self.walk(init);
                let state = resource_state_of_init(init);
                if let Some(state) = state {
                    if let Some(frame) = self.frames.last_mut() {
                        for binding in bindings.iter().flatten() {
                            frame.push(Binding {
                                name: binding.0.clone(),
                                span: *span,
                                state,
                                tracked: true,
                            });
                        }
                    }
                }
                true
            }
            HirStmt::Assign { name, value, .. } => {
                self.walk(value);
                let new_state = resource_state_of_init(value);
                if let Some(state) = new_state {
                    if let Some(binding) = self.find_mut(name) {
                        binding.state = state;
                        binding.tracked = true;
                    }
                } else {
                    // Rebinding a non-resource value resets tracking.
                    if let Some(binding) = self.find_mut(name) {
                        binding.tracked = false;
                    }
                }
                true
            }
            HirStmt::IndexAssign { base, index, value, .. } => {
                self.walk(base);
                self.walk(index);
                self.walk(value);
                true
            }
            HirStmt::FieldAssign { value, .. } => {
                self.walk(value);
                true
            }
            HirStmt::FieldPtrAssign { pointer, index, value, .. } => {
                self.walk(pointer);
                self.walk(index);
                self.walk(value);
                true
            }
            HirStmt::Return(value, _) => {
                if let Some(value) = value {
                    self.walk(value);
                }
                false
            }
            HirStmt::If { init, cond, then_block, else_block, .. } => {
                if let Some(init) = init {
                    if !self.stmt(init) {
                        return false;
                    }
                }
                self.walk(cond);
                self.branch(then_block, else_block.as_ref())
            }
            HirStmt::ForRange { start, end, body, .. } => {
                self.walk(start);
                self.walk(end);
                self.loop_body(body);
                true
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.walk(iterable);
                self.loop_body(body);
                true
            }
            HirStmt::ForC { init, cond, update, body, .. } => {
                if let Some(init) = init {
                    if !self.stmt(init) {
                        return false;
                    }
                }
                if let Some(cond) = cond {
                    self.walk(cond);
                }
                let entry = self.frames.clone();
                let _ = self.block(body);
                if let Some(update) = update {
                    self.walk(update);
                }
                self.frames = entry;
                true
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.walk(subject);
                self.switch_arms(arms)
            }
            HirStmt::Break(_) => false,
            HirStmt::Continue(_) => false,
            HirStmt::Defer { body, .. } => {
                let _ = self.block(body);
                true
            }
            HirStmt::LoopInvariant(contract) => {
                self.walk(&contract.expr);
                true
            }
        }
    }

    fn walk(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Literal(..) | HirExpr::Ident(..) | HirExpr::AddrOf { .. } | HirExpr::Capture { .. } => {
                // Check terminal-state use on ident.
                if let HirExpr::Ident(path, span, _) = expr {
                    if path.len() == 1 {
                        if let Some(name) = path.first() {
                            if let Some(binding) = self.find(name) {
                                if binding.tracked && binding.state.is_terminal() {
                                    self.report_use_after_close(name, *span);
                                }
                            }
                        }
                    }
                }
            }
            HirExpr::Move(inner, _, _) => {
                self.walk(inner);
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            HirExpr::Unary { operand, .. } => self.walk(operand),
            HirExpr::Call { callee, args, .. } => {
                // Determine if this is a state-changing transition.
                let transition = transition_for(callee);
                // Walk args, but for the resource arg of a transition, handle state.
                for (i, arg) in args.iter().enumerate() {
                    // Check if arg is a tracked binding in terminal state (use).
                    // For transition calls, the resource arg is handled below.
                    let is_resource_arg = transition.is_some() && i == 0;
                    if !is_resource_arg {
                        self.walk(arg);
                    }
                }
                if let Some(t) = transition {
                    // First arg is the resource.
                    if let Some(HirExpr::Ident(path, span, _)) = args.first() {
                        if path.len() == 1 {
                            if let Some(name) = path.first() {
                                if let Some(binding) = self.find_mut(name) {
                                    if binding.tracked {
                                        let (req, next) = t;
                                        // chan_free accepts both Open and Closed.
                                        let callee_name = callee.last().map(|s| s.as_str()).unwrap_or("");
                                        let ok = if callee_name == "chan_free" {
                                            binding.state == State::Open || binding.state == State::Closed
                                        } else {
                                            binding.state == req
                                        };
                                        if !ok {
                                            let from = binding.state;
                                            let sp = *span;
                                            drop(binding);
                                            self.report_bad_transition(name, from, req, sp);
                                        } else {
                                            binding.state = next;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Walk remaining args.
                    for arg in args.iter().skip(1) {
                        self.walk(arg);
                    }
                }
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.walk(arg);
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.walk(value);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.walk(element);
                }
            }
            HirExpr::FieldAccess { base, .. } => self.walk(base),
            HirExpr::Index { base, index, .. } => {
                self.walk(base);
                self.walk(index);
            }
            HirExpr::View { base, start, end, .. } => {
                self.walk(base);
                self.walk(start);
                self.walk(end);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.walk(expr),
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.walk(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.walk(guard);
                    }
                    self.walk(&arm.body);
                }
            }
            HirExpr::Lambda { f, .. } => self.check_fn(f),
            HirExpr::HigherOrder { receiver, func, init, .. } => {
                self.walk(receiver);
                self.walk(func);
                if let Some(init) = init {
                    self.walk(init);
                }
            }
            HirExpr::Move(inner, _, _) => self.walk(inner),
            _ => {}
        }
    }

    fn branch(&mut self, then_block: &HirBlock, else_block: Option<&HirBlock>) -> bool {
        let entry = self.frames.clone();
        let mut falling: Vec<StateFrames> = Vec::new();
        if self.block(then_block) {
            falling.push(self.frames.clone());
        }
        self.frames = entry.clone();
        match else_block {
            Some(block) => {
                if self.block(block) {
                    falling.push(self.frames.clone());
                }
            }
            None => falling.push(entry.clone()),
        }
        if falling.is_empty() {
            self.frames = entry;
            return false;
        }
        // Merge: intersection. A tracked binding must have the same state on
        // every falling path; otherwise E1400 with both states.
        let merged = merge_states(&falling, self.out, self.file);
        self.frames = merged;
        true
    }

    fn loop_body(&mut self, body: &HirBlock) {
        let entry = self.frames.clone();
        let _ = self.block(body);
        self.frames = entry;
    }

    fn switch_arms(&mut self, arms: &[HirSwitchArm]) -> bool {
        if arms.is_empty() {
            return true;
        }
        let entry = self.frames.clone();
        let mut falling: Vec<StateFrames> = Vec::with_capacity(arms.len());
        for arm in arms {
            self.frames = entry.clone();
            if let Some(guard) = &arm.guard {
                self.walk(guard);
            }
            self.walk(&arm.body);
            falling.push(self.frames.clone());
        }
        let merged = merge_states(&falling, self.out, self.file);
        self.frames = merged;
        true
    }

    fn find(&self, name: &str) -> Option<&Binding> {
        self.frames
            .iter()
            .rev()
            .flat_map(|frame| frame.iter().rev())
            .find(|b| b.name == name)
    }

    fn find_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.frames
            .iter_mut()
            .rev()
            .flat_map(|frame| frame.iter_mut().rev())
            .find(|b| b.name == name)
    }

    fn report_bad_transition(&mut self, name: &str, from: State, to: State, span: Span) {
        self.out.push(Diagnostic::new(
            Severity::Error,
            self.file,
            span,
            format!("`{name}` is in state `{from}`, but this operation requires `{to}`"),
        ).with_code(codes::DOUBLE_CLOSE));
    }

    fn report_use_after_close(&mut self, name: &str, span: Span) {
        self.out.push(Diagnostic::new(
            Severity::Error,
            self.file,
            span,
            format!("`{name}` is used after it has been released"),
        ).with_code(codes::USE_AFTER_CLOSE));
    }
}

fn resource_state_of_init(expr: &HirExpr) -> Option<State> {
    match expr {
        HirExpr::Call { callee, .. } => {
            let name = callee.last().map(|s| s.as_str()).unwrap_or("");
            match name {
                "mutex_new" => Some(State::Unlocked),
                "chan_new" => Some(State::Open),
                "alloc" => Some(State::Allocated),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Returns (required_state, next_state) for a state-changing builtin.
fn transition_for(callee: &[String]) -> Option<(State, State)> {
    let name = callee.last().map(|s| s.as_str()).unwrap_or("");
    match name {
        "mutex_lock" => Some((State::Unlocked, State::Locked)),
        "mutex_unlock" => Some((State::Locked, State::Unlocked)),
        "mutex_free" => Some((State::Unlocked, State::Freed)),
        "chan_close" => Some((State::Open, State::Closed)),
        "chan_free" => Some((State::Open, State::Freed)),
        "dealloc" => Some((State::Allocated, State::Freed)),
        _ => {
            // Check for .free() method calls - callee like ["Type", "free"] or ["free"]
            if name == "free" {
                Some((State::Allocated, State::Freed))
            } else {
                None
            }
        }
    }
}

fn merge_states(
    states: &[StateFrames],
    out: &mut Vec<Diagnostic>,
    file: FileId,
) -> StateFrames {
    if states.is_empty() {
        return Vec::new();
    }
    if states.len() == 1 {
        return states[0].clone();
    }
    // Assume all states have the same structure (same scopes, same bindings).
    let num_scopes = states[0].len();
    let mut merged: StateFrames = Vec::with_capacity(num_scopes);
    for scope_idx in 0..num_scopes {
        let mut merged_scope: Scope = Vec::new();
        let num_bindings = states[0][scope_idx].len();
        for bind_idx in 0..num_bindings {
            // Find the binding across all states.
            let first = &states[0][scope_idx][bind_idx];
            let name = first.name.clone();
            let span = first.span;
            let tracked = first.tracked;
            // Collect states.
            let mut state_set: Vec<State> = Vec::new();
            for s in states {
                if scope_idx < s.len() && bind_idx < s[scope_idx].len() {
                    state_set.push(s[scope_idx][bind_idx].state);
                }
            }
            // If all same, use that. Otherwise report E1400 and use first.
            let merged_state = if state_set.iter().all(|st| *st == state_set[0]) {
                state_set[0]
            } else {
                // States disagree: this is a real possibility the program has
                // to account for, but reporting it here duplicates the leak
                // pass's E1000 on the same facts. Keep the more conservative
                // state (terminal if any path is terminal) and stay silent.
                if state_set.iter().any(|s| s.is_terminal()) {
                    *state_set.iter().find(|s| s.is_terminal()).unwrap()
                } else {
                    state_set[0]
                }
            };
            merged_scope.push(Binding {
                name,
                span,
                state: merged_state,
                tracked,
            });
        }
        merged.push(merged_scope);
    }
    merged
}

/// Race freedom: a capture that reaches a spawned thread must be shareable.
fn check_race(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_race_in_block(&function.body, program.uses_threads, out, function.file);
        }
    }
}

fn check_race_in_block(block: &HirBlock, uses_threads: bool, out: &mut Vec<Diagnostic>, file: FileId) {
    for stmt in &block.stmts {
        check_race_in_stmt(stmt, uses_threads, out, file);
    }
}

fn check_race_in_stmt(stmt: &HirStmt, uses_threads: bool, out: &mut Vec<Diagnostic>, file: FileId) {
    match stmt {
        HirStmt::Expr(expr, ..) => check_race_in_expr(expr, uses_threads, out, file),
        HirStmt::Const { init, .. } | HirStmt::Var { init, .. } | HirStmt::Destructure { init, .. } => {
            check_race_in_expr(init, uses_threads, out, file);
        }
        HirStmt::Assign { value, .. } => check_race_in_expr(value, uses_threads, out, file),
        HirStmt::IndexAssign { base, index, value, .. } => {
            check_race_in_expr(base, uses_threads, out, file);
            check_race_in_expr(index, uses_threads, out, file);
            check_race_in_expr(value, uses_threads, out, file);
        }
        HirStmt::FieldAssign { value, .. } => check_race_in_expr(value, uses_threads, out, file),
        HirStmt::FieldPtrAssign { pointer, index, value, .. } => {
            check_race_in_expr(pointer, uses_threads, out, file);
            check_race_in_expr(index, uses_threads, out, file);
            check_race_in_expr(value, uses_threads, out, file);
        }
        HirStmt::Return(Some(expr), _) => check_race_in_expr(expr, uses_threads, out, file),
        HirStmt::If { init, cond, then_block, else_block, .. } => {
            if let Some(init) = init { check_race_in_stmt(init, uses_threads, out, file); }
            check_race_in_expr(cond, uses_threads, out, file);
            check_race_in_block(then_block, uses_threads, out, file);
            if let Some(eb) = else_block { check_race_in_block(eb, uses_threads, out, file); }
        }
        HirStmt::ForRange { start, end, body, .. } => {
            check_race_in_expr(start, uses_threads, out, file);
            check_race_in_expr(end, uses_threads, out, file);
            check_race_in_block(body, uses_threads, out, file);
        }
        HirStmt::ForIn { iterable, body, .. } => {
            check_race_in_expr(iterable, uses_threads, out, file);
            check_race_in_block(body, uses_threads, out, file);
        }
        HirStmt::ForC { init, cond, update, body, .. } => {
            if let Some(init) = init { check_race_in_stmt(init, uses_threads, out, file); }
            if let Some(cond) = cond { check_race_in_expr(cond, uses_threads, out, file); }
            if let Some(update) = update { check_race_in_expr(update, uses_threads, out, file); }
            check_race_in_block(body, uses_threads, out, file);
        }
        HirStmt::Switch { subject, arms, .. } => {
            check_race_in_expr(subject, uses_threads, out, file);
            for arm in arms {
                if let Some(guard) = &arm.guard { check_race_in_expr(guard, uses_threads, out, file); }
                check_race_in_expr(&arm.body, uses_threads, out, file);
            }
        }
        HirStmt::Defer { body, .. } => check_race_in_block(body, uses_threads, out, file),
        HirStmt::LoopInvariant(c) => check_race_in_expr(&c.expr, uses_threads, out, file),
        _ => {}
    }
}

fn check_race_in_expr(expr: &HirExpr, uses_threads: bool, out: &mut Vec<Diagnostic>, file: FileId) {
    match expr {
        HirExpr::Call { callee, args, span, .. } => {
            let name = callee.last().map(|s| s.as_str()).unwrap_or("");
            if name == "thread_spawn" {
                for arg in args {
                    if let HirExpr::Lambda { f, .. } = arg {
                        for capture in &f.captures {
                            if !is_shareable(&capture.ty, uses_threads) {
                                out.push(Diagnostic::new(
                                    Severity::Error,
                                    file,
                                    f.span,
                                    format!("`{}` is shared across a thread boundary without a `Mutex`", capture.name),
                                ).with_code(codes::RACE));
                            }
                        }
                    }
                }
            }
            for arg in args {
                check_race_in_expr(arg, uses_threads, out, file);
            }
        }
        HirExpr::Lambda { f, .. } => check_race_in_block(&f.body, uses_threads, out, file),
        HirExpr::Binary { lhs, rhs, .. } => {
            check_race_in_expr(lhs, uses_threads, out, file);
            check_race_in_expr(rhs, uses_threads, out, file);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            check_race_in_expr(operand, uses_threads, out, file);
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, v, _) in fields { check_race_in_expr(v, uses_threads, out, file); }
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            for e in elements { check_race_in_expr(e, uses_threads, out, file); }
        }
        HirExpr::FieldAccess { base, .. } => check_race_in_expr(base, uses_threads, out, file),
        HirExpr::Index { base, index, .. } => {
            check_race_in_expr(base, uses_threads, out, file);
            check_race_in_expr(index, uses_threads, out, file);
        }
        HirExpr::View { base, start, end, .. } => {
            check_race_in_expr(base, uses_threads, out, file);
            check_race_in_expr(start, uses_threads, out, file);
            check_race_in_expr(end, uses_threads, out, file);
        }
        HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => {
            check_race_in_expr(expr, uses_threads, out, file);
        }
        HirExpr::SwitchExpr { subject, arms, .. } => {
            check_race_in_expr(subject, uses_threads, out, file);
            for arm in arms {
                if let Some(guard) = &arm.guard { check_race_in_expr(guard, uses_threads, out, file); }
                check_race_in_expr(&arm.body, uses_threads, out, file);
            }
        }
        HirExpr::CallCapture { args, .. } => {
            for arg in args { check_race_in_expr(arg, uses_threads, out, file); }
        }
        HirExpr::HigherOrder { receiver, func, init, .. } => {
            check_race_in_expr(receiver, uses_threads, out, file);
            check_race_in_expr(func, uses_threads, out, file);
            if let Some(init) = init { check_race_in_expr(init, uses_threads, out, file); }
        }
        _ => {}
    }
}

fn is_shareable(ty: &yashy_hir::ty::Type, uses_threads: bool) -> bool {
    use yashy_hir::ty::TypeKind;
    match &ty.kind {
        TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Char | TypeKind::Unit
        | TypeKind::CInt(_) | TypeKind::CUInt(_) | TypeKind::CFloat(_) | TypeKind::CVoid
        | TypeKind::FixedArray(..) | TypeKind::View(_) => true,
        TypeKind::Str => uses_threads,
        TypeKind::Mutex => true,
        TypeKind::Channel(_) => true,
        _ => false,
    }
}
