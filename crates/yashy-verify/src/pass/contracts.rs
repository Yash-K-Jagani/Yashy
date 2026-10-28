//! Discharging `requires`, `ensures` and loop `invariant`.
//!
//! The pass walks each function with an [`Env`] of facts and asks the domain
//! whether each clause is true. A clause the domain cannot prove is a
//! diagnostic, never an assumption: the whole point of a contract is that a
//! program which does not establish it is wrong, and a verifier that quietly
//! believes what it could not check is worse than no verifier at all.
//!
//! What the walk knows:
//!
//! - a parameter's range, from its type;
//! - the `requires` clauses, which are the caller's promise and are therefore
//!   true inside the body;
//! - every binding's value, through assignment;
//! - a slice's length, so `len(xs)` is a number when the length is written
//!   down;
//! - the branch a comparison selects, so `if n > 0 { .. }` knows `n > 0`.
//!
//! What it deliberately does not know: division, floating point, strings,
//! fields, elements, and the value a call returns. Those are [`Value::top`],
//! and "top" is "not proved".
//!
//! A callee cannot change a local integer, because the language has no way to
//! take the address of one: [`HirExpr::AddrOf`] is written only by the checker,
//! for a `*mut self` receiver, and a receiver is never an `int`. So the facts
//! about locals survive a call, and only slice lengths -- which live in a
//! header the callee shares -- are dropped at one.

use std::collections::HashMap;

use yashy_common::{codes, Diagnostic, FileId, ProofTrace, Severity, Span};
use yashy_hir::hir::{
    expr_ty, BinOp, HirBlock, HirContract, HirExpr, HirFn, HirItem, HirProgram, HirStmt, Literal,
    UnOp,
};
use yashy_hir::ty::{Type, TypeKind};

use crate::domain::{self, Env, Interval, Tri, Value};
use crate::render;
use crate::ContractMode;

/// Run the contract pass over every function in `program`.
pub fn check(program: &HirProgram, mode: ContractMode, out: &mut Vec<Diagnostic>) {
    // Only a function that states a precondition has one to check at a call
    // site, so that is the whole table. Methods are named `Type.method` and
    // cannot declare a contract yet, so a name here is always a free function
    // or one of its instantiations.
    let mut functions: HashMap<&str, &HirFn> = HashMap::new();
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            if !function.requires.is_empty() {
                functions.insert(function.name.as_str(), function);
            }
        }
    }
    let mut pass = ContractPass {
        out,
        mode,
        functions,
        quiet: false,
        file: FileId::from_index(0),
    };
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            pass.check_fn(function);
        }
    }
}

struct ContractPass<'p, 'o> {
    out: &'o mut Vec<Diagnostic>,
    mode: ContractMode,
    functions: HashMap<&'p str, &'p HirFn>,
    /// True while a loop's fixpoint is iterating. The same body is walked more
    /// than once there, and only the last walk may report: a diagnostic
    /// repeated three times is noise, not evidence.
    quiet: bool,
    /// The file of the function being walked. Spans index their own module's
    /// text, so a diagnostic needs this to point at the right source.
    file: FileId,
}

/// What a `return` promises.
#[derive(Clone, Copy)]
struct FnCtx<'a> {
    ensures: &'a [HirContract],
}

/// How control left a statement.
///
/// `fall` is the environment that reaches the next statement. `breaks` and
/// `continues` are the environments at every jump out of the current loop, kept
/// separately because each rejoins at a different place: a `break` after the
/// loop, a `continue` at the loop's update.
struct Flow {
    fall: Option<Env>,
    breaks: Vec<Env>,
    continues: Vec<Env>,
}

impl Flow {
    fn normal(env: Env) -> Flow {
        Flow {
            fall: Some(env),
            breaks: Vec::new(),
            continues: Vec::new(),
        }
    }

    fn stop() -> Flow {
        Flow {
            fall: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        }
    }

    fn absorb(&mut self, next: Flow) {
        self.fall = next.fall;
        self.breaks.extend(next.breaks);
        self.continues.extend(next.continues);
    }
}

/// One loop body walked once: the environments that reach the head again.
struct BackEdge {
    /// The body fell off its end, and the update ran.
    fall: Option<Env>,
    /// A `continue` reached, and the update ran.
    continues: Vec<Env>,
    /// A `break` left, without running the update.
    breaks: Vec<Env>,
}

impl<'p, 'o> ContractPass<'p, 'o> {
    // -- functions --------------------------------------------------------

    fn check_fn(&mut self, function: &HirFn) {
        let saved = self.file;
        self.file = function.file;
        let ctx = FnCtx {
            ensures: &function.ensures,
        };

        let mut env = Env::new();
        for param in &function.params {
            env.set(param.name.clone(), Value::of_type(&param.ty));
        }
        // A precondition is the caller's promise, so it is a fact in here. A
        // call site that cannot establish it is reported where it is written.
        for clause in &function.requires {
            self.assume(&clause.expr, &mut env);
        }

        self.walk_block(&function.body, env, ctx);
        self.file = saved;
    }

    // -- statements -------------------------------------------------------

    fn walk_block(&mut self, block: &HirBlock, env: Env, ctx: FnCtx) -> Flow {
        // A path that cannot happen proves nothing and must say nothing: every
        // property holds on an empty set of states, and a warning about dead
        // code is noise the user cannot act on.
        if env.is_impossible() {
            return Flow::stop();
        }
        let mut flow = Flow::normal(env);
        for stmt in &block.stmts {
            let Some(env) = flow.fall.take() else {
                // Everything after a `return` is unreachable, and a fact
                // proved about unreachable code is not worth a diagnostic.
                break;
            };
            let next = self.walk_stmt(stmt, env, ctx);
            flow.absorb(next);
        }
        flow
    }

    fn walk_stmt(&mut self, stmt: &HirStmt, env: Env, ctx: FnCtx) -> Flow {
        let mut env = env;
        match stmt {
            HirStmt::Expr(expr, ..) => {
                self.eval(expr, &mut env);
                Flow::normal(env)
            }
            HirStmt::Const { name, ty, init, .. } | HirStmt::Var { name, ty, init, .. } => {
                let mut value = self.eval(init, &mut env);
                if let Some(ty) = ty {
                    // The binding's declared type is a fact the initializer
                    // cannot contradict.
                    value = value.meet(Value::of_type(ty));
                }
                env.set(name.clone(), value);
                match self.length_of(init, &env) {
                    Some(length) => env.set_length(name.clone(), length),
                    None => env.forget_length(name),
                }
                Flow::normal(env)
            }
            HirStmt::Destructure { init, bindings, .. } => {
                self.eval(init, &mut env);
                // A tuple is not modelled, so each element is unknown. Setting
                // it is what keeps a stale binding from surviving the shadow.
                for (name, _) in bindings.iter().flatten() {
                    env.set(name.clone(), Value::top());
                }
                Flow::normal(env)
            }
            HirStmt::Assign { name, value, .. } => {
                let value = self.eval(value, &mut env);
                env.set(name.clone(), value);
                env.forget_length(name);
                Flow::normal(env)
            }
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.eval(base, &mut env);
                self.eval(index, &mut env);
                self.eval(value, &mut env);
                Flow::normal(env)
            }
            HirStmt::FieldAssign { root, value, .. } => {
                self.eval(value, &mut env);
                env.forget(root);
                Flow::normal(env)
            }
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.eval(pointer, &mut env);
                self.eval(index, &mut env);
                self.eval(value, &mut env);
                Flow::normal(env)
            }
            HirStmt::Return(expr, _) => {
                let result = match expr {
                    Some(expr) => self.eval(expr, &mut env),
                    None => Value::top(),
                };
                self.check_ensures(ctx, result, &mut env);
                Flow::stop()
            }
            HirStmt::If {
                init,
                cond,
                then_block,
                else_block,
                ..
            } => {
                if let Some(init) = init {
                    let mut init_flow = self.walk_stmt(init, env, ctx);
                    match init_flow.fall.take() {
                        Some(next) => env = next,
                        None => return init_flow,
                    }
                }
                // The condition is read for its call sites; its truth is
                // handled by `assume`, which only refines what it can.
                self.eval(cond, &mut env);
                let mut then_env = env.clone();
                self.assume(cond, &mut then_env);
                let mut else_env = env;
                self.assume_not(cond, &mut else_env);

                let then_flow = self.walk_block(then_block, then_env, ctx);
                let else_flow = match else_block {
                    Some(block) => self.walk_block(block, else_env, ctx),
                    None => Flow::normal(else_env),
                };

                let fall = match (then_flow.fall, else_flow.fall) {
                    (Some(a), Some(b)) => Some(a.join(&b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                let mut flow = Flow {
                    fall,
                    breaks: then_flow.breaks,
                    continues: then_flow.continues,
                };
                flow.breaks.extend(else_flow.breaks);
                flow.continues.extend(else_flow.continues);
                flow
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.eval(subject, &mut env);
                // An arm's body is one expression, so it cannot `return` or
                // `break`; it can only call, and a call can only change a
                // length.
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.eval(guard, &mut env);
                    }
                    self.eval(&arm.body, &mut env);
                }
                Flow::normal(env)
            }
            HirStmt::ForRange {
                name,
                start,
                end,
                body,
                ..
            } => {
                let start = self.eval(start, &mut env);
                let end = self.eval(end, &mut env);
                // A range whose start is at or past its end runs zero times.
                if start.interval.lo >= end.interval.hi {
                    return Flow::normal(env);
                }
                let mut body_env = env;
                body_env.set(
                    name.clone(),
                    Value::range(start.interval.lo, end.interval.hi - 1),
                );
                self.run_loop(body_env, None, None, body, ctx)
            }
            HirStmt::ForIn {
                name,
                iterable,
                body,
                ..
            } => {
                self.eval(iterable, &mut env);
                env.set(name.clone(), Value::top());
                self.run_loop(env, None, None, body, ctx)
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    let mut init_flow = self.walk_stmt(init, env, ctx);
                    match init_flow.fall.take() {
                        Some(next) => env = next,
                        None => return init_flow,
                    }
                }
                self.run_loop(env, cond.as_ref(), update.as_ref(), body, ctx)
            }
            HirStmt::Break(_) => Flow {
                fall: None,
                breaks: vec![env],
                continues: Vec::new(),
            },
            HirStmt::Continue(_) => Flow {
                fall: None,
                breaks: Vec::new(),
                continues: vec![env],
            },
            HirStmt::Defer { body, .. } => {
                // The body runs at scope exit, after the statements that
                // follow it. Walking it here finds a bad call site inside it,
                // and the environment it produces is thrown away because its
                // assignments have not happened yet.
                let scratch = env.clone();
                self.walk_block(body, env, ctx);
                Flow::normal(scratch)
            }
            HirStmt::LoopInvariant(_) => {
                // The loop's own walk reads the clause at the head, which is
                // the only place it means anything.
                Flow::normal(env)
            }
        }
    }

    // -- loops ------------------------------------------------------------

    fn run_loop(
        &mut self,
        entry: Env,
        cond: Option<&HirExpr>,
        update: Option<&HirExpr>,
        body: &HirBlock,
        ctx: FnCtx,
    ) -> Flow {
        let invariants: Vec<&HirContract> = body
            .stmts
            .iter()
            .take_while(|stmt| matches!(stmt, HirStmt::LoopInvariant(_)))
            .filter_map(|stmt| match stmt {
                HirStmt::LoopInvariant(clause) => Some(clause),
                _ => None,
            })
            .collect();

        let mut entry = entry;
        if let Some(cond) = cond {
            self.eval(cond, &mut entry);
        }

        // The invariant has to hold the first time the head is reached.
        for invariant in &invariants {
            if self.truth(&invariant.expr, &mut entry) != Tri::True {
                let text = render::expr(&invariant.expr);
                self.report(
                    "invariant",
                    invariant.span,
                    format!("cannot prove the loop invariant `{text}` at the first iteration"),
                    &text,
                    "establish the invariant before the loop, or simplify it",
                );
            }
        }

        let mut head = entry;
        for invariant in &invariants {
            self.assume(&invariant.expr, &mut head);
        }

        // Find a head environment the body maps back into itself. Widening a
        // binding the body changed to "unknown" is what makes the search end:
        // after the first change there is nothing left to learn about it, and
        // the invariant is the only fact that survives.
        let was_quiet = self.quiet;
        self.quiet = true;
        for _ in 0..3 {
            let back = self.walk_loop_body(head.clone(), update, body, ctx);
            let next = match joined_back(&back) {
                Some(back) => head.join(&back),
                None => head.clone(),
            };
            let mut next = next;
            for invariant in &invariants {
                self.assume(&invariant.expr, &mut next);
            }
            if next == head {
                break;
            }
            head = head.widen(&next);
            for invariant in &invariants {
                self.assume(&invariant.expr, &mut head);
            }
        }
        self.quiet = was_quiet;

        // The last walk is the one that reports: it starts from the fixpoint,
        // so it is both the most precise and the only one whose diagnostics
        // survive.
        let back = self.walk_loop_body(head.clone(), update, body, ctx);
        for env in &back.continues {
            self.check_invariants(&invariants, env);
        }
        if let Some(env) = &back.fall {
            self.check_invariants(&invariants, env);
        }

        // The invariant still holds after the loop: it held at every head, and
        // the head is where the test that failed was read. A `break` can leave
        // from the middle of a body, so its environment joins in.
        let mut exits = back.breaks;
        if let Some(cond) = cond {
            let mut after = head;
            self.assume_not(cond, &mut after);
            // A condition that is always true has no exit path to join.
            if !after.is_impossible() {
                exits.push(after);
            }
        }
        let fall = exits.into_iter().reduce(|a, b| a.join(&b));
        Flow {
            fall,
            breaks: Vec::new(),
            continues: Vec::new(),
        }
    }

    fn walk_loop_body(
        &mut self,
        head: Env,
        update: Option<&HirExpr>,
        body: &HirBlock,
        ctx: FnCtx,
    ) -> BackEdge {
        let flow = self.walk_block(body, head, ctx);
        let mut fall = flow.fall;
        let mut continues = flow.continues;
        if let Some(update) = update {
            if let Some(env) = fall.as_mut() {
                self.eval(update, env);
            }
            for env in &mut continues {
                self.eval(update, env);
            }
        }
        BackEdge {
            fall,
            continues,
            breaks: flow.breaks,
        }
    }

    fn check_invariants(&mut self, invariants: &[&HirContract], env: &Env) {
        for invariant in invariants {
            let mut scope = env.clone();
            if self.truth(&invariant.expr, &mut scope) != Tri::True {
                let text = render::expr(&invariant.expr);
                self.report(
                    "invariant",
                    invariant.span,
                    format!("cannot prove the loop invariant `{text}` at the end of an iteration"),
                    &text,
                    "the loop body must leave the invariant true",
                );
            }
        }
    }

    // -- clauses ----------------------------------------------------------

    fn check_ensures(&mut self, ctx: FnCtx, result: Value, env: &mut Env) {
        if ctx.ensures.is_empty() {
            return;
        }
        let mut scope = env.clone();
        scope.set("result", result);
        for clause in ctx.ensures {
            let mut clause_env = scope.clone();
            if self.truth(&clause.expr, &mut clause_env) != Tri::True {
                let text = render::expr(&clause.expr);
                self.report(
                    "ensures",
                    clause.span,
                    format!("cannot prove the postcondition `{text}`"),
                    &text,
                    "add a `requires` that implies this, or simplify the clause",
                );
            }
        }
    }

    fn check_call(
        &mut self,
        callee: &[String],
        span: Span,
        args: &[HirExpr],
        values: &[Value],
        env: &Env,
    ) {
        let [name] = callee else {
            return;
        };
        let Some(function) = self.functions.get(name.as_str()).copied() else {
            return;
        };
        if function.params.len() != values.len() {
            return;
        }
        let mut scope = Env::new();
        for (param, value) in function.params.iter().zip(values) {
            scope.set(param.name.clone(), *value);
        }
        // A length the caller knows is a fact about the argument, so the
        // callee's `len(xs)` can use it. The value side carries no length,
        // because a slice's value is opaque to the domain.
        for (param, arg) in function.params.iter().zip(args) {
            if let Some(length) = self.length_of(arg, env) {
                scope.set_length(param.name.clone(), length);
            }
        }
        for clause in &function.requires {
            let mut clause_env = scope.clone();
            if self.truth(&clause.expr, &mut clause_env) != Tri::True {
                let text = render::expr(&clause.expr);
                self.report(
                    "requires",
                    span,
                    format!(
                        "cannot prove the precondition `{text}` of `{}`",
                        function.name
                    ),
                    &text,
                    "strengthen the argument's range, or add a `requires` to this function",
                );
            }
        }
    }

    fn report(
        &mut self,
        obligation: &'static str,
        span: Span,
        message: String,
        missing_fact: &str,
        remedy: &str,
    ) {
        if self.quiet {
            return;
        }
        let severity = match self.mode {
            ContractMode::Warn => Severity::Warning,
            ContractMode::Enforce => Severity::Error,
        };
        self.out.push(
            Diagnostic::new(severity, self.file, span, message)
                .with_code(codes::CONTRACT)
                .with_label("this clause is not proved")
                .with_proof(ProofTrace {
                    obligation,
                    missing_fact: missing_fact.to_string(),
                    remedy: remedy.to_string(),
                }),
        );
    }

    // -- expressions ------------------------------------------------------

    /// The value an expression can take, as far as the domain can say.
    ///
    /// Every call site inside the expression is checked on the way, so a
    /// precondition is never skipped because the call was an argument to
    /// something else.
    fn eval(&mut self, expr: &HirExpr, env: &mut Env) -> Value {
        match expr {
            HirExpr::Literal(literal, ..) => match literal {
                Literal::I64(n) => Value::exact(*n),
                Literal::Char(b) => Value::exact(i64::from(*b)),
                _ => Value::top(),
            },
            HirExpr::Ident(path, ..) => match path.as_slice() {
                [name] => env.get(name),
                _ => Value::top(),
            },
            HirExpr::Binary { op, lhs, rhs, .. } => {
                let left = self.eval(lhs, env);
                let right = self.eval(rhs, env);
                match op {
                    BinOp::Add => left.add(right),
                    BinOp::Sub => left.sub(right),
                    BinOp::Mul => left.mul(right),
                    // Division is not modelled: rounding toward zero and the
                    // zero divisor would each need care the domain does not
                    // have yet.
                    _ => Value::top(),
                }
            }
            HirExpr::Unary { op, operand, .. } => match op {
                UnOp::Neg => self.eval(operand, env).neg(),
                UnOp::Not => {
                    self.eval(operand, env);
                    Value::top()
                }
            },
            HirExpr::Move(inner, ..) => self.eval(inner, env),
            HirExpr::Call {
                callee, args, span, ..
            } => {
                let values: Vec<Value> = args.iter().map(|arg| self.eval(arg, env)).collect();
                if is_len(callee) {
                    return match args.first() {
                        Some(arg) => self
                            .length_of(arg, env)
                            .map(Value::exact)
                            .unwrap_or_else(Value::top),
                        None => Value::top(),
                    };
                }
                self.check_call(callee, *span, args, &values, env);
                // A callee shares a slice's header, so a length it knew is no
                // longer trustworthy. The integers are safe: no local's
                // address can be taken, so no callee can write to one.
                env.forget_lengths();
                Value::top()
            }
            HirExpr::Cast { expr, ty, .. } => {
                let value = self.eval(expr, env);
                if is_int(ty) && is_int(&expr_ty(expr)) {
                    value
                } else {
                    Value::top()
                }
            }
            HirExpr::Try { expr, .. } => {
                self.eval(expr, env);
                Value::top()
            }
            HirExpr::AddrOf { .. } => Value::top(),
            HirExpr::FieldAccess { base, .. } => {
                self.eval(base, env);
                Value::top()
            }
            HirExpr::Index { base, index, .. } => {
                self.eval(base, env);
                self.eval(index, env);
                Value::top()
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.eval(value, env);
                }
                Value::top()
            }
            HirExpr::SliceLit { elements, .. } | HirExpr::FixedArrayLit { elements, .. } => {
                for element in elements {
                    self.eval(element, env);
                }
                Value::top()
            }
            HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.eval(element, env);
                }
                Value::top()
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.eval(base, env);
                self.eval(start, env);
                self.eval(end, env);
                Value::top()
            }
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.eval(subject, env);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.eval(guard, env);
                    }
                    self.eval(&arm.body, env);
                }
                Value::top()
            }
            HirExpr::Lambda { f, .. } => {
                // A closure has no contract of its own, but its body can call a
                // function that does, so it is walked as the function it is.
                self.check_fn(f);
                Value::top()
            }
            HirExpr::Capture { .. } => Value::top(),
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.eval(arg, env);
                }
                Value::top()
            }
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.eval(receiver, env);
                self.eval(func, env);
                if let Some(init) = init {
                    self.eval(init, env);
                }
                Value::top()
            }
        }
    }

    /// What a comparison is known to be, evaluating its operands once.
    fn truth(&mut self, expr: &HirExpr, env: &mut Env) -> Tri {
        match expr {
            HirExpr::Literal(Literal::Bool(value), ..) => {
                if *value {
                    Tri::True
                } else {
                    Tri::False
                }
            }
            HirExpr::Unary {
                op: UnOp::Not,
                operand,
                ..
            } => self.truth(operand, env).not(),
            HirExpr::Binary {
                op: BinOp::And,
                lhs,
                rhs,
                ..
            } => {
                let left = self.truth(lhs, env);
                if left == Tri::False {
                    return Tri::False;
                }
                left.and(self.truth(rhs, env))
            }
            HirExpr::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
                ..
            } => {
                let left = self.truth(lhs, env);
                if left == Tri::True {
                    return Tri::True;
                }
                left.or(self.truth(rhs, env))
            }
            HirExpr::Binary { op, lhs, rhs, .. } if is_comparison(*op) => {
                let left = self.eval(lhs, env);
                let right = self.eval(rhs, env);
                domain::compare(*op, left, right)
            }
            // A `bool` binding is not tracked; only comparisons are.
            HirExpr::Ident(..) => Tri::Unknown,
            _ => {
                self.eval(expr, env);
                Tri::Unknown
            }
        }
    }

    /// Add what `expr` being true says about the environment.
    fn assume(&mut self, expr: &HirExpr, env: &mut Env) {
        match expr {
            HirExpr::Literal(Literal::Bool(true), ..) => {}
            HirExpr::Literal(Literal::Bool(false), ..) => env.mark_impossible(),
            HirExpr::Binary {
                op: BinOp::And,
                lhs,
                rhs,
                ..
            } => {
                self.assume(lhs, env);
                self.assume(rhs, env);
            }
            HirExpr::Unary {
                op: UnOp::Not,
                operand,
                ..
            } => self.assume_not(operand, env),
            HirExpr::Binary { op, lhs, rhs, .. } if is_comparison(*op) => {
                self.refine(*op, lhs, rhs, env);
            }
            _ => {}
        }
    }

    /// Add what `expr` being false says about the environment.
    fn assume_not(&mut self, expr: &HirExpr, env: &mut Env) {
        match expr {
            HirExpr::Literal(Literal::Bool(true), ..) => env.mark_impossible(),
            HirExpr::Literal(Literal::Bool(false), ..) => {}
            HirExpr::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
                ..
            } => {
                self.assume_not(lhs, env);
                self.assume_not(rhs, env);
            }
            HirExpr::Unary {
                op: UnOp::Not,
                operand,
                ..
            } => self.assume(operand, env),
            HirExpr::Binary { op, lhs, rhs, .. } if is_comparison(*op) => {
                self.refine(negate(*op), lhs, rhs, env);
            }
            _ => {}
        }
    }

    /// Refine a binding from `lhs op rhs`, when one side is a name and the
    /// other is a constant.
    fn refine(&mut self, op: BinOp, lhs: &HirExpr, rhs: &HirExpr, env: &mut Env) {
        if let Some(name) = single_ident(lhs) {
            if let Some(constant) = self.constant(rhs, env) {
                refine_ident(env, name, op, constant);
                return;
            }
        }
        if let Some(name) = single_ident(rhs) {
            if let Some(constant) = self.constant(lhs, env) {
                refine_ident(env, name, flip(op), constant);
            }
        }
    }

    /// The integer an expression is, without evaluating calls or reporting
    /// anything: a refinement may only use a fact already in the environment.
    fn constant(&self, expr: &HirExpr, env: &Env) -> Option<i64> {
        match expr {
            HirExpr::Literal(Literal::I64(n), ..) => Some(*n),
            HirExpr::Ident(path, ..) => match path.as_slice() {
                [name] => env.get(name).exact_value(),
                _ => None,
            },
            HirExpr::Unary {
                op: UnOp::Neg,
                operand,
                ..
            } => self.constant(operand, env)?.checked_neg(),
            HirExpr::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
                ..
            } => self
                .constant(lhs, env)?
                .checked_add(self.constant(rhs, env)?),
            HirExpr::Binary {
                op: BinOp::Sub,
                lhs,
                rhs,
                ..
            } => self
                .constant(lhs, env)?
                .checked_sub(self.constant(rhs, env)?),
            _ => None,
        }
    }

    /// The length of a slice-shaped expression, when it is written down.
    fn length_of(&self, expr: &HirExpr, env: &Env) -> Option<i64> {
        match expr {
            HirExpr::SliceLit { elements, .. } | HirExpr::FixedArrayLit { elements, .. } => {
                i64::try_from(elements.len()).ok()
            }
            HirExpr::Ident(path, ..) => match path.as_slice() {
                [name] => env.length(name),
                _ => None,
            },
            HirExpr::View { start, end, .. } => {
                let start = self.constant(start, env)?;
                let end = self.constant(end, env)?;
                (end >= start).then_some(end - start)
            }
            _ => None,
        }
    }
}

/// The join of every environment that reaches the loop's head again.
fn joined_back(back: &BackEdge) -> Option<Env> {
    let mut joined: Option<Env> = None;
    let mut add = |env: &Env| {
        joined = Some(match joined.take() {
            Some(current) => current.join(env),
            None => env.clone(),
        });
    };
    if let Some(env) = &back.fall {
        add(env);
    }
    for env in &back.continues {
        add(env);
    }
    joined
}

fn refine_ident(env: &mut Env, name: &str, op: BinOp, constant: i64) {
    let current = env.get(name);
    let constant = constant as i128;
    let refined = match op {
        BinOp::Eq => current.meet(Value::exact(constant as i64)),
        BinOp::Ne => {
            // Excluding one value only narrows an interval that touches it at
            // an edge; anywhere else the interval is already the best answer.
            let interval = current.interval;
            let interval = if interval.lo == constant {
                Interval::new(interval.lo + 1, interval.hi)
            } else if interval.hi == constant {
                Interval::new(interval.lo, interval.hi - 1)
            } else {
                interval
            };
            current.meet(Value::range(interval.lo, interval.hi))
        }
        BinOp::Lt => current.meet(Value::range(i64::MIN as i128, constant - 1)),
        BinOp::Le => current.meet(Value::range(i64::MIN as i128, constant)),
        BinOp::Gt => current.meet(Value::range(constant + 1, i64::MAX as i128)),
        BinOp::Ge => current.meet(Value::range(constant, i64::MAX as i128)),
        _ => current,
    };
    if refined.is_empty() {
        // Nothing satisfies the condition, so the branch it guards is dead.
        env.mark_impossible();
    }
    env.set(name.to_string(), refined);
}

fn single_ident(expr: &HirExpr) -> Option<&str> {
    match expr {
        HirExpr::Ident(path, ..) => match path.as_slice() {
            [name] => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

fn negate(op: BinOp) -> BinOp {
    match op {
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        BinOp::Lt => BinOp::Ge,
        BinOp::Ge => BinOp::Lt,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        other => other,
    }
}

/// `constant < x` is `x > constant`.
fn flip(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Le => BinOp::Ge,
        BinOp::Gt => BinOp::Lt,
        BinOp::Ge => BinOp::Le,
        other => other,
    }
}

fn is_len(callee: &[String]) -> bool {
    matches!(callee, [name] if name.as_str() == "len" || name.as_str() == "view_len")
}

fn is_int(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::I64)
}
