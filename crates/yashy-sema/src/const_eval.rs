//! Compile-time evaluation of `const fn` bodies.
//!
//! A `const fn` is a function the compiler runs itself. It has no run-time
//! existence: a call to one is replaced by the value the compiler computed, so
//! the body is never compiled and never called.
//!
//! That is what makes the body a *restricted* language. The evaluator here
//! accepts only what it can compute without running the program:
//!
//! - literals, and the parameters the call supplied;
//! - arithmetic and comparisons on those;
//! - calls to other `const fn`s;
//! - `if`, with a `return` in each arm.
//!
//! Anything else is refused by name -- ``read_line`` is not allowed in a const
//! fn -- rather than evaluated, because an expression the evaluator cannot
//! compute is one whose value is not known until the program runs.
//!
//! # Two passes, one language
//!
//! [`check_body`] walks a `const fn` and reports the constructs that are not
//! allowed, at the span of the offending expression. It runs once, when the
//! function is declared, so a mistake is reported even if nothing ever calls
//! the function.
//!
//! [`evaluate`] runs the same language with values, and is what a call site
//! uses. A fault -- a division by zero, an integer overflow -- and a call that
//! would not terminate are reported at the call, because that is where the
//! value was wanted.
//!
//! The two share the shape rules, so a body that passes [`check_body`] cannot
//! surprise [`evaluate`]. Where they can still disagree -- a value the checker
//! cannot know, such as the result of a division by a zero that is only zero
//! for some arguments -- the evaluator's answer is the one that counts.

use std::collections::BTreeMap;

use yashy_common::{codes, FileId, Span};
use yashy_parser::ast::{BinOp, Block, Expr, Literal, Stmt, StringLit, UnOp};

use crate::error::TypeError;
use crate::hir::HirExpr;
use crate::ty::{Type, TypeKind};

/// A value a `const fn` can produce.
///
/// The four kinds are the ones the plan names: `int`, `float`, `bool` and
/// `string`. A `const fn` that returns anything else has no value the
/// compiler could put in a `const` binding.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    /// One byte, from a char literal or a folded char comparison.
    Char(u8),
}

impl ConstValue {
    /// The type of the value.
    pub fn kind(&self) -> TypeKind {
        match self {
            ConstValue::I64(_) => TypeKind::I64,
            ConstValue::F64(_) => TypeKind::F64,
            ConstValue::Bool(_) => TypeKind::Bool,
            ConstValue::Str(_) => TypeKind::Str,
            ConstValue::Char(_) => TypeKind::Char,
        }
    }

    /// The literal the value is, for the tree the checker builds.
    ///
    /// The value *is* a literal after evaluation: the compiler computed it,
    /// so the program carries the number rather than the call that produced
    /// it.
    pub fn to_expr(&self, span: Span) -> HirExpr {
        let (literal, kind) = match self {
            ConstValue::I64(value) => (Literal::I64(*value), TypeKind::I64),
            ConstValue::F64(value) => (Literal::F64(*value), TypeKind::F64),
            ConstValue::Bool(value) => (Literal::Bool(*value), TypeKind::Bool),
            ConstValue::Char(value) => (Literal::Char(*value), TypeKind::Char),
            ConstValue::Str(value) => {
                let text = StringLit {
                    value: value.clone(),
                    bytes: value.as_bytes().to_vec(),
                };
                (Literal::Str(text), TypeKind::Str)
            }
        };
        HirExpr::Literal(literal, span, Type { kind, span })
    }
}

/// A declared `const fn`.
///
/// The body is the *untyped* tree: the evaluator is what checks it, so there
/// is no typed form of it anywhere.
#[derive(Clone, Debug)]
pub struct ConstFn {
    /// The name the function is declared and called by.
    pub name: String,
    /// The parameters, in declaration order: a name and its type.
    pub params: Vec<(String, Type)>,
    /// What the function returns.
    pub ret: Type,
    /// The body, untyped.
    pub body: Block,
    /// The span of the whole declaration.
    pub span: Span,
    /// The file the declaration is in, so a diagnostic about the body is
    /// attributed to the right module.
    pub file: FileId,
}

/// The `const fn`s a program declares, by name.
///
/// A map rather than a list, because a call names the function; the order the
/// functions are *checked* in is the checker's business, not this module's.
pub type ConstFns = BTreeMap<String, ConstFn>;

/// How deep a `const fn` evaluation may nest before it is called
/// non-terminating.
///
/// A `const fn` may call itself, so a recursive one is legal -- but the
/// compiler is running it, and a program that recurses forever would run
/// forever here too. The limit is what turns that into a diagnostic.
///
/// The number is small on purpose: the evaluator's own recursion is on the
/// compiler's stack, and a deep evaluation would overflow it before the
/// limit was reached. A hundred and sixty-four nested calls is far more than
/// a constant expression needs.
const MAX_CONST_DEPTH: usize = 160;

/// How many calls one evaluation may make in total.
///
/// The depth limit alone does not bound the *work*: a function that calls
/// itself twice at each level is only as deep as its argument is large, but it
/// does twice the work at every level. This budget is the second half of the
/// bound, and it is what a runaway program hits first.
const MAX_CONST_CALLS: usize = 1_000_000;

/// The state that bounds one evaluation.
///
/// A `const fn` runs on the compiler's own stack, so an evaluation that never
/// ends would take the compiler down with it. These two counters are the
/// bound: the depth, which is what the stack would overflow on, and the number
/// of calls, which is what a wide recursion would exhaust first.
#[derive(Clone, Debug, Default)]
pub struct Work {
    /// How many calls deep the evaluation is.
    pub depth: usize,
    /// How many calls the evaluation has made in total.
    pub calls: usize,
    /// The `const fn`s the evaluation called, with the span of each call.
    ///
    /// A `const fn` belongs to the module that declares it, so a call to
    /// another module's private one is a mistake. The evaluation is what
    /// knows which functions it called.
    pub called: Vec<(String, Span)>,
}

/// What an unknown name means, which depends on where the lookup happened.
///
/// Inside a `const fn` body a name that is not a parameter is a mistake in the
/// body. In a `const` initializer it is a name the program computes at run
/// time, and the message has to say that rather than "no such value".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The name is not in scope at all.
    NotFound,
    /// The name is a run-time value, so the constant cannot be computed.
    NotConstant,
}

impl Missing {
    /// The message for a name this kind of lookup could not resolve.
    fn message(self, name: &str) -> String {
        match self {
            Missing::NotFound => format!("cannot find value `{name}`"),
            Missing::NotConstant => format!(
                "`{name}` is a run-time value: a `const fn` call can only use \
                 literals and other `const fn` calls"
            ),
        }
    }
}

/// A failure while evaluating a `const fn`.
///
/// The span is the *call* that wanted the value, not the expression inside
/// the body that could not produce one: the reader is at the call, and the
/// body's own errors are reported where the body is written.
#[derive(Clone, Debug)]
pub struct ConstError {
    pub span: Span,
    pub message: String,
}

impl ConstError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        ConstError {
            span,
            message: message.into(),
        }
    }
}

/// Check a `const fn` body against the restricted language.
///
/// Reports the first thing that is not allowed, at the span of the offending
/// expression. The message names the offending construct, so the reader knows
/// what to remove.
///
/// The `const fn`s the body calls are returned with the answer: a call to
/// another module's `const fn` is a mistake, and the body check is what knows
/// which functions the body called.
pub fn check_body(
    function: &ConstFn,
    consts: &ConstFns,
) -> Result<Vec<(String, Span)>, TypeError> {
    let env: Vec<(String, Type)> = function.params.clone();
    let mut work = Work::default();
    check_block(
        &function.body,
        &env,
        &function.ret,
        consts,
        "a const fn",
        &mut work,
    )?;
    // A `const fn` that returns a value must return one on every path: the
    // compiler folds the body, so a path that fell off the end would have no
    // value to give.
    if function.ret.kind != TypeKind::Unit && !returns_on_all_paths(&function.body) {
        return Err(TypeError::new(
            function.body.span,
            "a const fn that returns a value must return one on every path".to_string(),
        ));
    }
    Ok(work.called)
}

/// Whether every path through a block ends in a `return`.
///
/// A block returns on every path when its last statement returns, or is an
/// `if` whose two arms both return. Anything else -- a loop, a plain
/// expression -- leaves a path that falls off the end.
fn returns_on_all_paths(block: &Block) -> bool {
    let Some(last) = block.stmts.last() else {
        return false;
    };
    match last {
        Stmt::Return(_, _) => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            let Some(else_block) = else_block else {
                return false;
            };
            returns_on_all_paths(then_block) && returns_on_all_paths(else_block)
        }
        _ => false,
    }
}

/// Check the statements of a `const fn` body.
fn check_block(
    block: &Block,
    env: &[(String, Type)],
    ret: &Type,
    consts: &ConstFns,
    context: &str,
    work: &mut Work,
) -> Result<(), TypeError> {
    for statement in &block.stmts {
        match statement {
            Stmt::Return(value, span) => {
                let actual = match value {
                    Some(expr) => check_expr(expr, env, consts, context, work)?,
                    None => Type {
                        kind: TypeKind::Unit,
                        span: *span,
                    },
                };
                if ret.kind != TypeKind::Unit && actual.kind != ret.kind {
                    return Err(TypeError::new(
                        *span,
                        format!("expected `{ret}`, found `{actual}`"),
                    ));
                }
            }
            Stmt::If {
                init,
                cond,
                then_block,
                else_block,
                span,
            } => {
                if init.is_some() {
                    return Err(TypeError::new(
                        *span,
                        format!("a condition binding is not allowed in {context}"),
                    ));
                }
                let condition = check_expr(cond, env, consts, context, work)?;
                if condition.kind != TypeKind::Bool {
                    return Err(TypeError::new(
                        expr_span(cond),
                        format!("a condition must be `bool`, found `{condition}`"),
                    ));
                }
                check_block(then_block, env, ret, consts, context, work)?;
                if let Some(else_block) = else_block {
                    check_block(else_block, env, ret, consts, context, work)?;
                }
            }
            other => {
                return Err(TypeError::new(
                    other.span(),
                    format!("{} is not allowed in {context}", statement_kind(other)),
                ));
            }
        }
    }
    Ok(())
}

/// Check one expression and report the type it has.
///
/// The type is what the evaluator needs: it is the value's type, and for a
/// call it is the called function's return type.
fn check_expr(
    expr: &Expr,
    env: &[(String, Type)],
    consts: &ConstFns,
    context: &str,
    work: &mut Work,
) -> Result<Type, TypeError> {
    match expr {
        Expr::Literal(literal, span) => Ok(Type {
            kind: literal_kind(literal),
            span: *span,
        }),
        Expr::Ident(path, span) => {
            let name = path.last().cloned().unwrap_or_default();
            match env.iter().rev().find(|(param, _)| param == &name) {
                Some((_, ty)) => Ok(ty.clone()),
                None => Err(TypeError::new(
                    *span,
                    format!("cannot find value `{name}`"),
                )),
            }
        }
        Expr::Binary {
            op,
            lhs,
            rhs,
            span,
        } => {
            let left = check_expr(lhs, env, consts, context, work)?;
            let right = check_expr(rhs, env, consts, context, work)?;
            binary_kind(*op, &left, &right, *span)
        }
        Expr::Unary {
            op,
            operand,
            span,
        } => {
            let inner = check_expr(operand, env, consts, context, work)?;
            match op {
                UnOp::Neg if is_numeric(&inner.kind) => Ok(inner),
                UnOp::Neg => Err(TypeError::new(
                    *span,
                    format!("cannot apply `-` to `{}`", inner.kind),
                )),
                UnOp::Not if inner.kind == TypeKind::Bool => Ok(Type {
                    kind: TypeKind::Bool,
                    span: *span,
                }),
                UnOp::Not => Err(TypeError::new(
                    *span,
                    format!("cannot apply `!` to `{}`", inner.kind),
                )),
            }
        }
        Expr::Call {
            callee,
            args,
            span,
            ..
        } => {
            let name = match callee.as_slice() {
                [name] => name.clone(),
                _ => {
                    return Err(TypeError::new(
                        *span,
                        "a qualified call is not allowed in a const fn".to_string(),
                    ))
                }
            };
            let Some(function) = consts.get(&name) else {
                return Err(TypeError::new(
                    *span,
                    format!("`{name}` is not allowed in {context}"),
                ));
            };
            // The call is recorded, so a visibility check can see which
            // functions the body reached for.
            work.called.push((name.clone(), *span));
            if args.len() != function.params.len() {
                return Err(TypeError::new(
                    *span,
                    format!(
                        "expected {} arguments, found {}",
                        function.params.len(),
                        args.len()
                    ),
                )
                .with_code(codes::WRONG_ARGUMENT_COUNT));
            }
            for (argument, (_, param)) in args.iter().zip(function.params.iter()) {
                let actual = check_expr(argument, env, consts, context, work)?;
                if actual.kind != param.kind {
                    return Err(TypeError::new(
                        expr_span(argument),
                        format!("expected `{param}`, found `{actual}`"),
                    )
                    .with_code(codes::TYPE_MISMATCH));
                }
            }
            Ok(function.ret.clone())
        }
        other => Err(TypeError::new(
            expr_span(other),
            format!("{} is not allowed in {context}", expression_kind(other)),
        )),
    }
}

/// The type of a literal.
fn literal_kind(literal: &Literal) -> TypeKind {
    match literal {
        Literal::I64(_) => TypeKind::I64,
        Literal::F64(_) => TypeKind::F64,
        Literal::Bool(_) => TypeKind::Bool,
        Literal::Str(_) => TypeKind::Str,
        Literal::Char(_) => TypeKind::Char,
    }
}

/// The type a binary operation has, or the error it is.
fn binary_kind(op: BinOp, left: &Type, right: &Type, span: Span) -> Result<Type, TypeError> {
    let bool_type = || Type {
        kind: TypeKind::Bool,
        span,
    };
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
            let same = left.kind == right.kind;
            let allowed = is_numeric(&left.kind)
                || (matches!(op, BinOp::Add) && left.kind == TypeKind::Str);
            if same && allowed {
                return Ok(left.clone());
            }
            Err(TypeError::new(
                span,
                format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    op.symbol(),
                    left.kind,
                    right.kind
                ),
            ))
        }
        BinOp::Eq | BinOp::Ne => {
            if left.kind == right.kind {
                return Ok(bool_type());
            }
            Err(TypeError::new(
                span,
                format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    op.symbol(),
                    left.kind,
                    right.kind
                ),
            ))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            if left.kind == right.kind
                && (is_numeric(&left.kind) || left.kind == TypeKind::Char)
            {
                return Ok(bool_type());
            }
            Err(TypeError::new(
                span,
                format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    op.symbol(),
                    left.kind,
                    right.kind
                ),
            ))
        }
        BinOp::And | BinOp::Or => {
            if left.kind == TypeKind::Bool && right.kind == TypeKind::Bool {
                return Ok(bool_type());
            }
            Err(TypeError::new(
                span,
                format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    op.symbol(),
                    left.kind,
                    right.kind
                ),
            ))
        }
    }
}

/// Whether a type is one the evaluator can compute with.
fn is_numeric(kind: &TypeKind) -> bool {
    matches!(kind, TypeKind::I64 | TypeKind::F64)
}

/// The name of a statement, for the message that refuses it.
fn statement_kind(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::Expr(_, _) => "an expression statement",
        Stmt::Const { .. } => "a `const` binding",
        Stmt::Let { .. } => "a `let` binding",
        Stmt::Destructure { .. } => "a destructuring binding",
        Stmt::Assign { .. } => "an assignment",
        Stmt::Return(_, _) => "a `return`",
        Stmt::If { .. } => "an `if`",
        Stmt::ForRange { .. } => "a `for` loop",
        Stmt::ForIn { .. } => "a `for ... in` loop",
        Stmt::ForC { .. } => "a `for` loop",
        Stmt::Switch { .. } => "a `switch`",
        Stmt::Break(_) => "a `break`",
        Stmt::Continue(_) => "a `continue`",
        Stmt::Defer { .. } => "a `defer`",
    }
}

/// The name of an expression, for the message that refuses it.
fn expression_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Literal(_, _) => "a literal",
        Expr::Ident(_, _) => "a name",
        Expr::Binary { .. } => "an arithmetic expression",
        Expr::Unary { .. } => "a unary expression",
        Expr::Move(_, _) => "a `move`",
        Expr::Call { .. } => "a call",
        Expr::StructLit { .. } => "a struct literal",
        Expr::FieldAccess { .. } => "a field access",
        Expr::SliceLit { .. } => "an array literal",
        Expr::Index { .. } => "an index",
        Expr::FixedArrayLit { .. } => "a fixed-size array literal",
        Expr::TupleLit { .. } => "a tuple literal",
        Expr::IndexRange { .. } => "a range",
        Expr::Interp { .. } => "an interpolation",
        Expr::Cast { .. } => "a cast",
        Expr::Try { .. } => "a `?`",
        Expr::SwitchExpr { .. } => "a `switch`",
        Expr::MethodCall { .. } => "a method call",
        Expr::Lambda { .. } => "a lambda",
    }
}

/// The span of an expression.
fn expr_span(expr: &Expr) -> Span {
    match expr {
        Expr::Literal(_, span)
        | Expr::Ident(_, span)
        | Expr::Binary { span, .. }
        | Expr::Unary { span, .. }
        | Expr::Move(_, span)
        | Expr::Call { span, .. }
        | Expr::StructLit { span, .. }
        | Expr::FieldAccess { span, .. }
        | Expr::SliceLit { span, .. }
        | Expr::Index { span, .. }
        | Expr::FixedArrayLit { span, .. }
        | Expr::TupleLit { span, .. }
        | Expr::IndexRange { span, .. }
        | Expr::Interp { span, .. }
        | Expr::Cast { span, .. }
        | Expr::Try { span, .. }
        | Expr::SwitchExpr { span, .. }
        | Expr::MethodCall { span, .. }
        | Expr::Lambda { span, .. } => *span,
    }
}

/// Whether an expression mentions a `const fn`.
///
/// A `const` binding whose initializer mentions one is evaluated at compile
/// time; one that does not is an ordinary run-time binding, as it always was.
pub fn mentions_const_fn(expr: &Expr, consts: &ConstFns) -> bool {
    match expr {
        Expr::Literal(_, _) | Expr::Ident(_, _) => false,
        Expr::Binary { lhs, rhs, .. } => {
            mentions_const_fn(lhs, consts) || mentions_const_fn(rhs, consts)
        }
        Expr::Unary { operand, .. } => mentions_const_fn(operand, consts),
        Expr::Move(inner, _) => mentions_const_fn(inner, consts),
        Expr::Call { callee, args, .. } => {
            matches!(callee.as_slice(), [name] if consts.contains_key(name))
                || args.iter().any(|arg| mentions_const_fn(arg, consts))
        }
        Expr::StructLit { fields, .. } => fields
            .iter()
            .any(|(_, value, _)| mentions_const_fn(value, consts)),
        Expr::FieldAccess { base, .. } => mentions_const_fn(base, consts),
        Expr::SliceLit { elements, .. } => {
            elements.iter().any(|element| mentions_const_fn(element, consts))
        }
        Expr::Index { base, index, .. } => {
            mentions_const_fn(base, consts) || mentions_const_fn(index, consts)
        }
        Expr::FixedArrayLit { elements, .. } => {
            elements.iter().any(|element| mentions_const_fn(element, consts))
        }
        Expr::TupleLit { elements, .. } => {
            elements.iter().any(|element| mentions_const_fn(element, consts))
        }
        Expr::IndexRange {
            base, start, end, ..
        } => {
            mentions_const_fn(base, consts)
                || mentions_const_fn(start, consts)
                || mentions_const_fn(end, consts)
        }
        Expr::Interp { parts, .. } => parts.iter().any(|part| match part {
            yashy_parser::ast::InterpPart::Expr(expr, _) => mentions_const_fn(expr, consts),
            yashy_parser::ast::InterpPart::Text(_) => false,
        }),
        Expr::Cast { expr, .. } => mentions_const_fn(expr, consts),
        Expr::Try { expr, .. } => mentions_const_fn(expr, consts),
        Expr::SwitchExpr { subject, arms, .. } => {
            mentions_const_fn(subject, consts)
                || arms.iter().any(|arm| {
                    matches!(&arm.body, Expr::Call { args, .. } if args.iter().any(|arg| mentions_const_fn(arg, consts)))
                })
        }
        Expr::Lambda { .. } => false,
        Expr::MethodCall { .. } => false,
    }
}

/// Evaluate an expression at compile time.
///
/// `env` holds the values the enclosing `const fn` was called with, and
/// `work` the state that bounds the evaluation: how deep it may nest and how
/// many calls it may make. A call that would not terminate is reported rather
/// than followed.
pub fn evaluate(
    expr: &Expr,
    env: &[(String, ConstValue)],
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
) -> Result<ConstValue, ConstError> {
    match expr {
        Expr::Literal(literal, _) => Ok(match literal {
            Literal::I64(value) => ConstValue::I64(*value),
            Literal::F64(value) => ConstValue::F64(*value),
            Literal::Bool(value) => ConstValue::Bool(*value),
            Literal::Str(text) => ConstValue::Str(text.value.clone()),
            Literal::Char(byte) => ConstValue::Char(*byte),
        }),
        Expr::Ident(path, span) => {
            let name = path.last().cloned().unwrap_or_default();
            match env.iter().rev().find(|(param, _)| param == &name) {
                Some((_, value)) => Ok(value.clone()),
                None => Err(ConstError::new(*span, missing.message(&name))),
            }
        }
        Expr::Binary {
            op,
            lhs,
            rhs,
            span,
        } => {
            let left = evaluate(lhs, env, consts, work, missing)?;
            let right = evaluate(rhs, env, consts, work, missing)?;
            evaluate_binary(*op, &left, &right, *span)
        }
        Expr::Unary {
            op,
            operand,
            span,
        } => {
            let inner = evaluate(operand, env, consts, work, missing)?;
            match (op, &inner) {
                (UnOp::Neg, ConstValue::I64(value)) => {
                    Ok(ConstValue::I64(value.wrapping_neg()))
                }
                (UnOp::Neg, ConstValue::F64(value)) => Ok(ConstValue::F64(-value)),
                (UnOp::Neg, other) => Err(ConstError::new(
                    *span,
                    format!("cannot apply `-` to `{}`", other.kind()),
                )),
                (UnOp::Not, ConstValue::Bool(value)) => Ok(ConstValue::Bool(!value)),
                (UnOp::Not, other) => Err(ConstError::new(
                    *span,
                    format!("cannot apply `!` to `{}`", other.kind()),
                )),
            }
        }
        Expr::Call {
            callee,
            args,
            span,
            ..
        } => {
            let name = match callee.as_slice() {
                [name] => name.clone(),
                _ => {
                    return Err(ConstError::new(
                        *span,
                        "a qualified call is not allowed in a const fn",
                    ))
                }
            };
            let Some(function) = consts.get(&name) else {
                return Err(ConstError::new(
                    *span,
                    format!("`{name}` is not allowed in a const fn"),
                ));
            };
            work.calls = work.calls.saturating_add(1);
            // The call is recorded, so a visibility check can see which
            // functions the evaluation reached for.
            work.called.push((name.clone(), *span));
            if work.depth >= MAX_CONST_DEPTH || work.calls > MAX_CONST_CALLS {
                return Err(ConstError::new(
                    *span,
                    format!("`{name}` did not terminate: the evaluation is too deep"),
                ));
            }
            let mut values = Vec::with_capacity(args.len());
            for argument in args {
                values.push(evaluate(argument, env, consts, work, missing)?);
            }
            let mut bound = Vec::with_capacity(function.params.len());
            for ((param, _), value) in function.params.iter().zip(values.into_iter()) {
                bound.push((param.clone(), value));
            }
            work.depth = work.depth.saturating_add(1);
            let outcome = evaluate_block(&function.body, &bound, consts, work);
            work.depth = work.depth.saturating_sub(1);
            let Some(value) = outcome? else {
                return Err(ConstError::new(
                    *span,
                    format!("`{name}` does not return a value on every path"),
                ));
            };
            if value.kind() != function.ret.kind {
                return Err(ConstError::new(
                    *span,
                    format!(
                        "`{name}` returns `{}`, not `{}`",
                        value.kind(),
                        function.ret.kind
                    ),
                ));
            }
            Ok(value)
        }
        other => Err(ConstError::new(
            expr_span(other),
            format!(
                "{} is not allowed in a const fn",
                expression_kind(other)
            ),
        )),
    }
}

/// Evaluate the statements of a `const fn` body.
///
/// `Ok(None)` means the block *fell through*: no `return` was reached, so the
/// statement after it still runs. That is what makes `if c { return a; } return
/// b;` work: the `if` returns on one path and falls through on the other.
fn evaluate_block(
    block: &Block,
    env: &[(String, ConstValue)],
    consts: &ConstFns,
    work: &mut Work,
) -> Result<Option<ConstValue>, ConstError> {
    for statement in &block.stmts {
        match statement {
            Stmt::Return(Some(expr), _) => {
                let value = evaluate(expr, env, consts, work, Missing::NotFound)?;
                return Ok(Some(value));
            }
            Stmt::Return(None, span) => {
                return Err(ConstError::new(
                    *span,
                    "a const fn must return a value".to_string(),
                ))
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                let condition = evaluate(cond, env, consts, work, Missing::NotFound)?;
                let ConstValue::Bool(chosen) = condition else {
                    return Err(ConstError::new(
                        expr_span(cond),
                        "a condition must be `bool`".to_string(),
                    ));
                };
                let taken = if chosen {
                    Some(then_block)
                } else {
                    else_block.as_ref()
                };
                let Some(taken) = taken else {
                    // No `else`: the `if` did nothing, and the statements
                    // after it still run.
                    continue;
                };
                if let Some(value) = evaluate_block(taken, env, consts, work)? {
                    return Ok(Some(value));
                }
            }
            other => {
                return Err(ConstError::new(
                    other.span(),
                    format!(
                        "{} is not allowed in a const fn",
                        statement_kind(other)
                    ),
                ))
            }
        }
    }
    Ok(None)
}

/// The value of an arithmetic or comparison operation, or the error it is.
fn evaluate_binary(
    op: BinOp,
    left: &ConstValue,
    right: &ConstValue,
    span: Span,
) -> Result<ConstValue, ConstError> {
    match (op, left, right) {
        (BinOp::Add, ConstValue::I64(a), ConstValue::I64(b)) => a
            .checked_add(*b)
            .map(ConstValue::I64)
            .ok_or_else(|| overflow(span)),
        (BinOp::Sub, ConstValue::I64(a), ConstValue::I64(b)) => a
            .checked_sub(*b)
            .map(ConstValue::I64)
            .ok_or_else(|| overflow(span)),
        (BinOp::Mul, ConstValue::I64(a), ConstValue::I64(b)) => a
            .checked_mul(*b)
            .map(ConstValue::I64)
            .ok_or_else(|| overflow(span)),
        (BinOp::Div, ConstValue::I64(a), ConstValue::I64(b)) => {
            if *b == 0 {
                return Err(ConstError::new(span, "division by zero".to_string()));
            }
            a.checked_div(*b)
                .map(ConstValue::I64)
                .ok_or_else(|| overflow(span))
        }
        (BinOp::Rem, ConstValue::I64(a), ConstValue::I64(b)) => {
            if *b == 0 {
                return Err(ConstError::new(span, "division by zero".to_string()));
            }
            a.checked_rem(*b)
                .map(ConstValue::I64)
                .ok_or_else(|| overflow(span))
        }
        (BinOp::Add, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::F64(a + b)),
        (BinOp::Sub, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::F64(a - b)),
        (BinOp::Mul, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::F64(a * b)),
        (BinOp::Div, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::F64(a / b)),
        (BinOp::Rem, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::F64(a % b)),
        (BinOp::Add, ConstValue::Str(a), ConstValue::Str(b)) => {
            Ok(ConstValue::Str(format!("{a}{b}")))
        }
        (BinOp::Eq, a, b) => Ok(ConstValue::Bool(a == b)),
        (BinOp::Ne, a, b) => Ok(ConstValue::Bool(a != b)),
        (BinOp::Lt, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::Bool(a < b)),
        (BinOp::Le, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::Bool(a <= b)),
        (BinOp::Gt, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::Bool(a > b)),
        (BinOp::Ge, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::Bool(a >= b)),
        (BinOp::Lt, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::Bool(a < b)),
        (BinOp::Le, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::Bool(a <= b)),
        (BinOp::Gt, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::Bool(a > b)),
        (BinOp::Ge, ConstValue::F64(a), ConstValue::F64(b)) => Ok(ConstValue::Bool(a >= b)),
        (BinOp::Lt, ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a < b)),
        (BinOp::Le, ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a <= b)),
        (BinOp::Gt, ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a > b)),
        (BinOp::Ge, ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a >= b)),
        (BinOp::And, ConstValue::Bool(a), ConstValue::Bool(b)) => {
            Ok(ConstValue::Bool(*a && *b))
        }
        (BinOp::Or, ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(*a || *b)),
        _ => Err(ConstError::new(
            span,
            format!(
                "cannot apply `{}` to `{}` and `{}`",
                op.symbol(),
                left.kind(),
                right.kind()
            ),
        )),
    }
}

/// The message an integer operation that overflows produces.
fn overflow(span: Span) -> ConstError {
    ConstError::new(span, "integer overflow")
}
