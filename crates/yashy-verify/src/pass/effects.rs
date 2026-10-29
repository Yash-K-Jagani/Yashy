//! The effect pass: what a function does to the world.
//!
//! Effects are inferred bottom-up over the call graph to a fixpoint. A
//! function's effect starts at `PURE` and joins each callee's effect plus
//! its own body's direct effects. The fixpoint terminates because the
//! lattice has height 3 (three booleans).

use std::collections::BTreeMap;

use yashy_common::{codes, DeclaredEffect, Diagnostic, Effect, FileId, Severity, Span};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm};

/// Run the effect pass over `program`, pushing diagnostics into `out`.
/// Returns the inferred effect map.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) -> BTreeMap<String, Effect> {
    let mut effects = infer_effects(program);

    // Check declarations against inferred effects.
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            if let Some(declared) = function.declared.to_effect() {
                let inferred = effects.get(&function.name).copied().unwrap_or(Effect::PURE);
                // A declaration that is stricter than the body is an error.
                let violated = match function.declared {
                    DeclaredEffect::Pure => !inferred.is_pure(),
                    DeclaredEffect::Io => inferred.unsafe_,
                    DeclaredEffect::Unsafe => false, // allows everything
                    DeclaredEffect::Async => false,  // async allows everything for now
                    DeclaredEffect::Inferred => false,
                };
                if violated {
                    let (missing, remedy) = match function.declared {
                        DeclaredEffect::Pure => ("io", "remove `pure`, or remove the IO call"),
                        DeclaredEffect::Io => ("unsafe", "declare the function `unsafe`"),
                        _ => ("", ""),
                    };
                    if !missing.is_empty() {
                        out.push(Diagnostic::new(
                            Severity::Error,
                            function.file,
                            function.span,
                            format!(
                                "declared `{}`, but the body performs `{}`",
                                declared_name(function.declared),
                                missing,
                            ),
                        ).with_code(codes::EFFECT));
                    }
                }
                let _ = declared;
            }
        }
    }

    // E1301: unsafe call outside an unsafe fn.
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_unsafe_calls(function, &effects, out);
        }
    }

    effects
}

fn declared_name(d: DeclaredEffect) -> &'static str {
    match d {
        DeclaredEffect::Inferred => "inferred",
        DeclaredEffect::Pure => "pure",
        DeclaredEffect::Io => "io",
        DeclaredEffect::Unsafe => "unsafe",
        DeclaredEffect::Async => "async",
    }
}

/// Check that unsafe calls only happen inside unsafe functions.
fn check_unsafe_calls(function: &HirFn, _effects: &BTreeMap<String, Effect>, out: &mut Vec<Diagnostic>) {
    // An unsafe fn may call unsafe things. A non-unsafe fn may not.
    let is_unsafe_fn = function.declared == DeclaredEffect::Unsafe;
    if is_unsafe_fn {
        return;
    }
    // Walk the body looking for unsafe calls.
    let mut walker = UnsafeWalker { out, file: function.file, is_unsafe: is_unsafe_fn };
    walker.block(&function.body);
}

struct UnsafeWalker<'a> {
    out: &'a mut Vec<Diagnostic>,
    file: FileId,
    is_unsafe: bool,
}

impl UnsafeWalker<'_> {
    fn block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }
    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Expr(expr, ..) => self.expr(expr),
            HirStmt::Const { init, .. } | HirStmt::Var { init, .. } | HirStmt::Destructure { init, .. } => self.expr(init),
            HirStmt::Assign { value, .. } => self.expr(value),
            HirStmt::IndexAssign { base, index, value, .. } => {
                self.expr(base); self.expr(index); self.expr(value);
            }
            HirStmt::FieldAssign { value, .. } => self.expr(value),
            HirStmt::FieldPtrAssign { pointer, index, value, .. } => {
                self.expr(pointer); self.expr(index); self.expr(value);
            }
            HirStmt::Return(Some(expr), _) => self.expr(expr),
            HirStmt::If { init, cond, then_block, else_block, .. } => {
                if let Some(init) = init { self.stmt(init); }
                self.expr(cond);
                self.block(then_block);
                if let Some(else_block) = else_block { self.block(else_block); }
            }
            HirStmt::ForRange { start, end, body, .. } => {
                self.expr(start); self.expr(end); self.block(body);
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.expr(iterable); self.block(body);
            }
            HirStmt::ForC { init, cond, update, body, .. } => {
                if let Some(init) = init { self.stmt(init); }
                if let Some(cond) = cond { self.expr(cond); }
                if let Some(update) = update { self.expr(update); }
                self.block(body);
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard { self.expr(guard); }
                    self.expr(&arm.body);
                }
            }
            HirStmt::Defer { body, .. } => self.block(body),
            HirStmt::LoopInvariant(c) => self.expr(&c.expr),
            _ => {}
        }
    }
    fn expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Binary { lhs, rhs, .. } => { self.expr(lhs); self.expr(rhs); }
            HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => self.expr(operand),
            HirExpr::Call { callee, args, unsafe_call, span, .. } => {
                if *unsafe_call && !self.is_unsafe {
                    let name = callee.join(".");
                    self.out.push(Diagnostic::new(
                        Severity::Error,
                        self.file,
                        *span,
                        format!("`{name}` is an unsafe call outside an `unsafe fn`"),
                    ).with_code(codes::UNSAFE_CALL));
                }
                for arg in args { self.expr(arg); }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, v, _) in fields { self.expr(v); }
            }
            HirExpr::SliceLit { elements, .. } | HirExpr::FixedArrayLit { elements, .. } | HirExpr::TupleLit { elements, .. } => {
                for e in elements { self.expr(e); }
            }
            HirExpr::FieldAccess { base, .. } => self.expr(base),
            HirExpr::Index { base, index, .. } => { self.expr(base); self.expr(index); }
            HirExpr::View { base, start, end, .. } => { self.expr(base); self.expr(start); self.expr(end); }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.expr(expr),
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(g) = &arm.guard { self.expr(g); }
                    self.expr(&arm.body);
                }
            }
            HirExpr::Lambda { f, .. } => {
                // Nested function: check it too.
                let saved = self.file;
                self.file = f.file;
                let was = self.is_unsafe;
                // A lambda inherits... actually lambdas don't have declared effects.
                // Just walk the body.
                self.block(&f.body);
                self.file = saved;
                self.is_unsafe = was;
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args { self.expr(arg); }
            }
            HirExpr::HigherOrder { receiver, func, init, .. } => {
                self.expr(receiver);
                self.expr(func);
                if let Some(init) = init { self.expr(init); }
            }
            _ => {}
        }
    }
}

/// Infer effects for all functions in a program.
pub fn infer_effects(program: &HirProgram) -> BTreeMap<String, Effect> {
    // Build the call graph and compute effects to a fixpoint.
    // Each function starts as PURE. We iterate: for each function, join
    // its own direct effects with its callees' effects. Repeat until
    // no change.
    let mut effects: BTreeMap<String, Effect> = BTreeMap::new();

    // Initialize all functions to PURE.
    for item in &program.items {
        if let HirItem::Fn(f) = item {
            effects.insert(f.name.clone(), Effect::PURE);
        }
    }

    // Also register extern functions as unsafe.
    for item in &program.items {
        if let HirItem::Extern(block) = item {
            for decl in &block.decls {
                effects.insert(decl.name.clone(), Effect { io: false, unsafe_: true, async_: false });
            }
        }
    }

    // Fixpoint iteration. The lattice height is 3 (three booleans), so
    // at most 3 updates per function.
    let mut changed = true;
    while changed {
        changed = false;
        for item in &program.items {
            if let HirItem::Fn(f) = item {
                let mut effect = direct_effects(&f.body);
                // Join effects of callees.
                for callee in collect_callees(&f.body) {
                    if let Some(ce) = effects.get(&callee) {
                        effect = effect.join(*ce);
                    }
                }
                // Also check contract expressions.
                for clause in f.requires.iter().chain(f.ensures.iter()) {
                    effect = effect.join(direct_effects_expr(&clause.expr));
                    let mut contract_callees = Vec::new();
                    collect_callees_expr(&clause.expr, &mut contract_callees);
                    for callee in contract_callees {
                        if let Some(ce) = effects.get(&callee) {
                            effect = effect.join(*ce);
                        }
                    }
                }
                let old = effects.get(&f.name).copied().unwrap_or(Effect::PURE);
                if effect != old {
                    effects.insert(f.name.clone(), effect);
                    changed = true;
                }
            }
        }
    }

    effects
}

/// Collect direct effects from a block.
fn direct_effects(block: &HirBlock) -> Effect {
    let mut effect = Effect::PURE;
    for stmt in &block.stmts {
        effect = effect.join(direct_effects_stmt(stmt));
    }
    effect
}

fn direct_effects_stmt(stmt: &HirStmt) -> Effect {
    match stmt {
        HirStmt::Expr(expr, ..) => direct_effects_expr(expr),
        HirStmt::Const { init, .. } | HirStmt::Var { init, .. } | HirStmt::Destructure { init, .. } => {
            direct_effects_expr(init)
        }
        HirStmt::Assign { value, .. } => direct_effects_expr(value),
        HirStmt::IndexAssign { base, index, value, .. } => {
            direct_effects_expr(base).join(direct_effects_expr(index)).join(direct_effects_expr(value))
        }
        HirStmt::FieldAssign { value, .. } => direct_effects_expr(value),
        HirStmt::FieldPtrAssign { pointer, index, value, .. } => {
            direct_effects_expr(pointer).join(direct_effects_expr(index)).join(direct_effects_expr(value))
        }
        HirStmt::Return(Some(expr), _) => direct_effects_expr(expr),
        HirStmt::If { cond, then_block, else_block, .. } => {
            let mut e = direct_effects_expr(cond);
            e = e.join(direct_effects(then_block));
            if let Some(eb) = else_block {
                e = e.join(direct_effects(eb));
            }
            e
        }
        HirStmt::ForRange { start, end, body, .. } => {
            direct_effects_expr(start).join(direct_effects_expr(end)).join(direct_effects(body))
        }
        HirStmt::ForIn { iterable, body, .. } => {
            direct_effects_expr(iterable).join(direct_effects(body))
        }
        HirStmt::ForC { init, cond, update, body, .. } => {
            let mut e = Effect::PURE;
            if let Some(init) = init {
                e = e.join(direct_effects_stmt(init));
            }
            if let Some(cond) = cond {
                e = e.join(direct_effects_expr(cond));
            }
            if let Some(update) = update {
                e = e.join(direct_effects_expr(update));
            }
            e = e.join(direct_effects(body));
            e
        }
        HirStmt::Switch { subject, arms, .. } => {
            let mut e = direct_effects_expr(subject);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    e = e.join(direct_effects_expr(guard));
                }
                e = e.join(direct_effects_expr(&arm.body));
            }
            e
        }
        HirStmt::Defer { body, .. } => direct_effects(body),
        HirStmt::LoopInvariant(c) => direct_effects_expr(&c.expr),
        _ => Effect::PURE,
    }
}

fn direct_effects_expr(expr: &HirExpr) -> Effect {
    match expr {
        HirExpr::Call { callee, args, unsafe_call, .. } => {
            let mut effect = if *unsafe_call {
                Effect { io: false, unsafe_: true, async_: false }
            } else {
                // Check if it's an IO builtin.
                let name = callee.last().map(|s| s.as_str()).unwrap_or("");
                if is_io_builtin(name) {
                    Effect { io: true, unsafe_: false, async_: false }
                } else {
                    Effect::PURE
                }
            };
            for arg in args {
                effect = effect.join(direct_effects_expr(arg));
            }
            effect
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            direct_effects_expr(lhs).join(direct_effects_expr(rhs))
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            direct_effects_expr(operand)
        }
        HirExpr::StructLit { fields, .. } => {
            let mut e = Effect::PURE;
            for (_, v, _) in fields {
                e = e.join(direct_effects_expr(v));
            }
            e
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            let mut e = Effect::PURE;
            for elem in elements {
                e = e.join(direct_effects_expr(elem));
            }
            e
        }
        HirExpr::FieldAccess { base, .. } => direct_effects_expr(base),
        HirExpr::Index { base, index, .. } => {
            direct_effects_expr(base).join(direct_effects_expr(index))
        }
        HirExpr::View { base, start, end, .. } => {
            direct_effects_expr(base).join(direct_effects_expr(start)).join(direct_effects_expr(end))
        }
        HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => direct_effects_expr(expr),
        HirExpr::SwitchExpr { subject, arms, .. } => {
            let mut e = direct_effects_expr(subject);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    e = e.join(direct_effects_expr(guard));
                }
                e = e.join(direct_effects_expr(&arm.body));
            }
            e
        }
        HirExpr::Lambda { f, .. } => {
            // A lambda's effect is its body's effect.
            direct_effects(&f.body)
        }
        HirExpr::CallCapture { args, .. } => {
            let mut e = Effect::PURE;
            for arg in args {
                e = e.join(direct_effects_expr(arg));
            }
            e
        }
        HirExpr::HigherOrder { receiver, func, init, .. } => {
            let mut e = direct_effects_expr(receiver).join(direct_effects_expr(func));
            if let Some(init) = init {
                e = e.join(direct_effects_expr(init));
            }
            e
        }
        _ => Effect::PURE,
    }
}

fn is_io_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "println"
            | "print_str"
            | "print_line_str"
            | "print_i64"
            | "println_i64"
            | "print_f64"
            | "println_f64"
            | "read_line"
            | "read_file"
            | "write_file"
            | "getenv"
            | "getcwd"
            | "now"
            | "sleep"
            | "rand"
            | "time"
            | "process_exit"
            | "process_args"
            | "str_from_i64"
            | "str_from_f64"
    )
}

fn collect_callees(block: &HirBlock) -> Vec<String> {
    let mut callees = Vec::new();
    for stmt in &block.stmts {
        collect_callees_stmt(stmt, &mut callees);
    }
    callees
}

fn collect_callees_stmt(stmt: &HirStmt, callees: &mut Vec<String>) {
    match stmt {
        HirStmt::Expr(expr, ..) => collect_callees_expr(expr, callees),
        HirStmt::Const { init, .. } | HirStmt::Var { init, .. } | HirStmt::Destructure { init, .. } => {
            collect_callees_expr(init, callees);
        }
        HirStmt::Assign { value, .. } => collect_callees_expr(value, callees),
        HirStmt::IndexAssign { base, index, value, .. } => {
            collect_callees_expr(base, callees);
            collect_callees_expr(index, callees);
            collect_callees_expr(value, callees);
        }
        HirStmt::FieldAssign { value, .. } => collect_callees_expr(value, callees),
        HirStmt::FieldPtrAssign { pointer, index, value, .. } => {
            collect_callees_expr(pointer, callees);
            collect_callees_expr(index, callees);
            collect_callees_expr(value, callees);
        }
        HirStmt::Return(Some(expr), _) => collect_callees_expr(expr, callees),
        HirStmt::If { init, cond, then_block, else_block, .. } => {
            if let Some(init) = init {
                collect_callees_stmt(init, callees);
            }
            collect_callees_expr(cond, callees);
            for c in collect_callees(then_block) {
                callees.push(c);
            }
            if let Some(eb) = else_block {
                for c in collect_callees(eb) {
                    callees.push(c);
                }
            }
        }
        HirStmt::ForRange { start, end, body, .. } => {
            collect_callees_expr(start, callees);
            collect_callees_expr(end, callees);
            for c in collect_callees(body) {
                callees.push(c);
            }
        }
        HirStmt::ForIn { iterable, body, .. } => {
            collect_callees_expr(iterable, callees);
            for c in collect_callees(body) {
                callees.push(c);
            }
        }
        HirStmt::ForC { init, cond, update, body, .. } => {
            if let Some(init) = init {
                collect_callees_stmt(init, callees);
            }
            if let Some(cond) = cond {
                collect_callees_expr(cond, callees);
            }
            if let Some(update) = update {
                collect_callees_expr(update, callees);
            }
            for c in collect_callees(body) {
                callees.push(c);
            }
        }
        HirStmt::Switch { subject, arms, .. } => {
            collect_callees_expr(subject, callees);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_callees_expr(guard, callees);
                }
                collect_callees_expr(&arm.body, callees);
            }
        }
        HirStmt::Defer { body, .. } => {
            for c in collect_callees(body) {
                callees.push(c);
            }
        }
        HirStmt::LoopInvariant(c) => collect_callees_expr(&c.expr, callees),
        _ => {}
    }
}

fn collect_callees_expr(expr: &HirExpr, callees: &mut Vec<String>) {
    match expr {
        HirExpr::Call { callee, args, .. } => {
            if callee.len() == 1 {
                callees.push(callee[0].clone());
            }
            for arg in args {
                collect_callees_expr(arg, callees);
            }
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            collect_callees_expr(lhs, callees);
            collect_callees_expr(rhs, callees);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            collect_callees_expr(operand, callees);
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, v, _) in fields {
                collect_callees_expr(v, callees);
            }
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            for e in elements {
                collect_callees_expr(e, callees);
            }
        }
        HirExpr::FieldAccess { base, .. } => collect_callees_expr(base, callees),
        HirExpr::Index { base, index, .. } => {
            collect_callees_expr(base, callees);
            collect_callees_expr(index, callees);
        }
        HirExpr::View { base, start, end, .. } => {
            collect_callees_expr(base, callees);
            collect_callees_expr(start, callees);
            collect_callees_expr(end, callees);
        }
        HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => {
            collect_callees_expr(expr, callees);
        }
        HirExpr::SwitchExpr { subject, arms, .. } => {
            collect_callees_expr(subject, callees);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_callees_expr(g, callees);
                }
                collect_callees_expr(&arm.body, callees);
            }
        }
        HirExpr::Lambda { f, .. } => {
            for c in collect_callees(&f.body) {
                callees.push(c);
            }
        }
        HirExpr::CallCapture { args, .. } => {
            for arg in args {
                collect_callees_expr(arg, callees);
            }
        }
        HirExpr::HigherOrder { receiver, func, init, .. } => {
            collect_callees_expr(receiver, callees);
            collect_callees_expr(func, callees);
            if let Some(init) = init {
                collect_callees_expr(init, callees);
            }
        }
        _ => {}
    }
}

