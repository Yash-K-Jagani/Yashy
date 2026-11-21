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
//! - fixed arrays and checked indexing into them;
//! - calls to other `const fn`s;
//! - `if`, with a `return` in each arm;
//! - `let` and assignment to its binding, so a body can accumulate;
//! - the condition-only loop (`while c { .. }`, which the parser lowers to
//!   `for ; c; { .. }`) and its `break`/`continue`;
//! - struct literals and field reads, with field types checked against the
//!   declaration before the value is folded;
//! - enum constructors, with variant payload types checked the same way.
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
use yashy_parser::ast::{BinOp, Block, Expr, Literal, Pattern, Stmt, StringLit, UnOp};

use crate::error::TypeError;
use crate::hir::{HirBlock, HirExpr, HirPattern, HirStmt, HirSwitchArm};
use crate::ty::{Type, TypeKind};

/// A value a `const fn` can produce.
///
/// The four scalar kinds are the ones the plan names: `int`, `float`, `bool`
/// and `string`. An array is a fixed-length run of values of one kind; it is
/// how a `const fn` builds a lookup table, and indexing it is how the caller
/// reads one entry. Both stay values the compiler can write into the HIR as a
/// literal, so a folded call has no run-time body.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    /// One byte, from a char literal or a folded char comparison.
    Char(u8),
    /// A fixed array `[T; N]`: its element type is the common type of the
    /// values, which the checker has already proved is one of the scalar kinds.
    Array(Vec<ConstValue>),
    /// A struct value: field names and values in declaration order. The
    /// checker validates the field types against the declaration before the
    /// evaluator builds one, so a fold cannot put a `bool` where an `int`
    /// belongs.
    Struct {
        name: String,
        fields: Vec<(String, ConstValue)>,
    },
    /// An enum value: the enum name, the variant and its payload values.
    Enum {
        name: String,
        variant: String,
        payload: Vec<ConstValue>,
    },
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
            ConstValue::Array(elements) => TypeKind::FixedArray(
                Box::new(Type {
                    kind: elements
                        .first()
                        .map(ConstValue::kind)
                        .unwrap_or(TypeKind::Unit),
                    span: Span::new(0, 0),
                }),
                elements.len(),
            ),
            ConstValue::Struct { name, .. } => TypeKind::Struct(name.clone()),
            ConstValue::Enum { name, .. } => TypeKind::Enum(name.clone()),
        }
    }

    /// The literal the value is, for the tree the checker builds.
    ///
    /// The value *is* a literal after evaluation: the compiler computed it,
    /// so the program carries the number rather than the call that produced
    /// it.
    pub fn to_expr(&self, span: Span) -> HirExpr {
        match self {
            ConstValue::Enum {
                name,
                variant,
                payload,
            } => HirExpr::Call {
                callee: vec![name.clone(), variant.clone()],
                args: payload.iter().map(|value| value.to_expr(span)).collect(),
                unsafe_call: false,
                span,
                ty: Type {
                    kind: TypeKind::Enum(name.clone()),
                    span,
                },
            },
            ConstValue::Struct { name, fields } => HirExpr::StructLit {
                path: vec![name.clone()],
                fields: fields
                    .iter()
                    .map(|(field, value)| (field.clone(), value.to_expr(span), span))
                    .collect(),
                span,
                ty: Type {
                    kind: TypeKind::Struct(name.clone()),
                    span,
                },
            },
            ConstValue::Array(elements) => {
                // The checker proved one element type, so the first element's
                // kind is the array's and every later one agrees with it.
                let kind = elements
                    .first()
                    .map(ConstValue::kind)
                    .unwrap_or(TypeKind::Unit);
                let elem_ty = Type {
                    kind: kind.clone(),
                    span,
                };
                HirExpr::FixedArrayLit {
                    elem_ty,
                    elements: elements.iter().map(|value| value.to_expr(span)).collect(),
                    span,
                    ty: Type {
                        kind: TypeKind::FixedArray(Box::new(Type { kind, span }), elements.len()),
                        span,
                    },
                }
            }
            ConstValue::I64(value) => HirExpr::Literal(
                Literal::I64(*value),
                span,
                Type {
                    kind: TypeKind::I64,
                    span,
                },
            ),
            ConstValue::F64(value) => HirExpr::Literal(
                Literal::F64(*value),
                span,
                Type {
                    kind: TypeKind::F64,
                    span,
                },
            ),
            ConstValue::Bool(value) => HirExpr::Literal(
                Literal::Bool(*value),
                span,
                Type {
                    kind: TypeKind::Bool,
                    span,
                },
            ),
            ConstValue::Char(value) => HirExpr::Literal(
                Literal::Char(*value),
                span,
                Type {
                    kind: TypeKind::Char,
                    span,
                },
            ),
            ConstValue::Str(value) => {
                let text = StringLit {
                    value: value.clone(),
                    bytes: value.as_bytes().to_vec(),
                };
                HirExpr::Literal(
                    Literal::Str(text),
                    span,
                    Type {
                        kind: TypeKind::Str,
                        span,
                    },
                )
            }
        }
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
    /// The body, untyped. Retained as the declaration-time gate: `check_body`
    /// validates the restricted const language here before any lowering.
    pub body: Block,
    /// The same body lowered to typed HIR, when the checker has built it.
    ///
    /// The HIR evaluator consumes this form. It is optional because a `ConstFn`
    /// is first collected from its declaration (body only) and lowered later,
    /// once the checker's type tables are complete.
    pub hir_body: Option<crate::hir::HirBlock>,
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

/// The declarations a `const fn` may name: struct field types and enum
/// variants with their payload types. The checker builds this from its own
/// tables and passes it to both passes, so a literal in a `const fn` is checked
/// against the same declaration an ordinary literal would use.
#[derive(Clone, Debug, Default)]
pub struct ConstTypes {
    pub structs: BTreeMap<String, Vec<(String, Type)>>,
    pub enums: BTreeMap<String, Vec<(String, Vec<TypeKind>)>>,
}

impl ConstTypes {
    pub fn new() -> Self {
        Self::default()
    }
}

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
/// a constant expression needs. The test threads have a smaller stack than a
/// compiler main thread, so the cap stays well below the point where the
/// evaluator's own frames overflow it.
const MAX_CONST_DEPTH: usize = 32;

/// How many calls one evaluation may make in total.
///
/// The depth limit alone does not bound the *work*: a function that calls
/// itself twice at each level is only as deep as its argument is large, but it
/// does twice the work at every level. This budget is the second half of the
/// bound, and it is what a runaway program hits first.
const MAX_CONST_CALLS: usize = 1_000_000;

/// How many expression evaluations one `const fn` call may perform.
///
/// Calls bound recursion depth, but a loop-free body can still do a lot of
/// work in one expression tree; this counter bounds that work as well and
/// reports the expression that exceeded it.
const MAX_CONST_STEPS: usize = 1_000_000;

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
    /// How many expressions the evaluation has visited.
    pub steps: usize,
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
    /// The catalog code to report with, when the failure has one. A shift count
    /// outside its type's range is `E0105`; an overflow keeps the general
    /// (uncoded) path it has always had.
    pub code: Option<&'static str>,
}

impl ConstError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        ConstError {
            span,
            message: message.into(),
            code: None,
        }
    }

    /// Attach the catalog code this failure reports under.
    fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
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
    structs: &ConstTypes,
) -> Result<Vec<(String, Span)>, TypeError> {
    let mut env: Vec<(String, Type)> = function.params.clone();
    let mut work = Work::default();
    check_block(
        &function.body,
        &mut env,
        &function.ret,
        consts,
        "a const fn",
        &mut work,
        false,
        function.params.len(),
        structs,
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
///
/// `env` is the block's scope: a `let` adds to it, and it is truncated to the
/// length it had on entry when the block ends. The checker sees the same
/// language the evaluator does, so a body that passes here cannot surprise it.
fn check_block(
    block: &Block,
    env: &mut Vec<(String, Type)>,
    ret: &Type,
    consts: &ConstFns,
    context: &str,
    work: &mut Work,
    in_loop: bool,
    params: usize,
    structs: &ConstTypes,
) -> Result<(), TypeError> {
    let base = env.len();
    for statement in &block.stmts {
        check_stmt(
            statement, env, ret, consts, context, work, in_loop, params, structs,
        )?;
    }
    env.truncate(base);
    Ok(())
}

/// Check one statement of a `const fn` body.
///
/// `params` is how many leading entries of `env` are the function's
/// parameters: only a `let` may be assigned, exactly as in a run-time body.
#[allow(clippy::too_many_arguments)]
fn check_stmt(
    statement: &Stmt,
    env: &mut Vec<(String, Type)>,
    ret: &Type,
    consts: &ConstFns,
    context: &str,
    work: &mut Work,
    in_loop: bool,
    params: usize,
    structs: &ConstTypes,
) -> Result<(), TypeError> {
    match statement {
        Stmt::Return(value, span) => {
            let actual = match value {
                Some(expr) => check_expr(expr, env, consts, context, work, structs)?,
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
            Ok(())
        }
        // A local is the same declaration a run-time body writes, minus the
        // parts the evaluator has no value for: the initializer is computed
        // now, and a written type has to agree with it.
        Stmt::Let {
            name,
            ty,
            init,
            span,
        } => {
            let actual = check_expr(init, env, consts, context, work, structs)?;
            let kind = match ty {
                Some(written) => {
                    let Some(declared) = declared_kind(written) else {
                        return Err(TypeError::new(
                            *span,
                            format!(
                                "{} is not a type a `const fn` can declare: \
                                 use `int`, `float`, `bool`, `string` or `char`",
                                written_kind(written)
                            ),
                        ));
                    };
                    if declared != actual.kind {
                        return Err(TypeError::new(
                            *span,
                            format!("expected `{declared}`, found `{actual}`"),
                        ));
                    }
                    declared
                }
                None => actual.kind,
            };
            env.push((name.clone(), Type { kind, span: *span }));
            Ok(())
        }
        // `x = e;` and `x op= e;` (the parser lowers the compound form to a
        // plain assignment): the name has to be a binding this body declared.
        Stmt::Assign {
            target,
            value,
            span,
        } => {
            let Expr::Ident(path, name_span) = target else {
                return Err(TypeError::new(
                    *span,
                    "only a name can be assigned in a const fn".to_string(),
                ));
            };
            let name = path.last().cloned().unwrap_or_default();
            let actual = check_expr(value, env, consts, context, work, structs)?;
            let Some((at, (_, bound))) = env
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (bound, _))| bound == &name)
            else {
                return Err(TypeError::new(
                    *name_span,
                    format!("cannot find value `{name}`"),
                ));
            };
            if at < params {
                return Err(TypeError::new(
                    *span,
                    format!("cannot assign to `{name}`, which is immutable"),
                )
                .with_code(codes::CONST_REASSIGNMENT)
                .with_note("use `let` for a mutable binding"));
            }
            if bound.kind != actual.kind {
                return Err(TypeError::new(
                    *span,
                    format!("expected `{bound}`, found `{actual}`"),
                ));
            }
            Ok(())
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
            let condition = check_expr(cond, env, consts, context, work, structs)?;
            if condition.kind != TypeKind::Bool {
                return Err(TypeError::new(
                    expr_span(cond),
                    format!("a condition must be `bool`, found `{condition}`"),
                ));
            }
            check_block(
                then_block, env, ret, consts, context, work, in_loop, params, structs,
            )?;
            if let Some(else_block) = else_block {
                check_block(
                    else_block, env, ret, consts, context, work, in_loop, params, structs,
                )?;
            }
            Ok(())
        }
        // `while c { .. }` and `for ; c; u { .. }` are the same node: the body
        // is checked in a loop, which is what lets `break` and `continue`
        // appear in it.
        Stmt::ForC {
            init,
            cond,
            update,
            body,
            span,
            ..
        } => {
            if let Some(init) = init {
                check_stmt(
                    init, env, ret, consts, context, work, in_loop, params, structs,
                )?;
            }
            if let Some(cond) = cond {
                let condition = check_expr(cond, env, consts, context, work, structs)?;
                if condition.kind != TypeKind::Bool {
                    return Err(TypeError::new(
                        expr_span(cond),
                        format!("a condition must be `bool`, found `{condition}`"),
                    ));
                }
            }
            if let Some(update) = update {
                check_expr(update, env, consts, context, work, structs)?;
            }
            check_block(body, env, ret, consts, context, work, true, params, structs)?;
            let _ = span;
            Ok(())
        }
        Stmt::Break(label, span) => {
            if !in_loop {
                return Err(TypeError::new(
                    *span,
                    "`break` outside a loop in a const fn".to_string(),
                ));
            }
            if label.is_some() {
                return Err(TypeError::new(
                    *span,
                    "a labelled `break` is not allowed in a const fn".to_string(),
                ));
            }
            Ok(())
        }
        Stmt::Continue(label, span) => {
            if !in_loop {
                return Err(TypeError::new(
                    *span,
                    "`continue` outside a loop in a const fn".to_string(),
                ));
            }
            if label.is_some() {
                return Err(TypeError::new(
                    *span,
                    "a labelled `continue` is not allowed in a const fn".to_string(),
                ));
            }
            Ok(())
        }
        other => Err(TypeError::new(
            other.span(),
            format!("{} is not allowed in {context}", statement_kind(other)),
        )),
    }
}

/// The kind a written type names, when a `const fn` local may use it.
///
/// The evaluator has one width per scalar and no aggregate layout, so a local
/// declares one of the five scalar kinds; a fixed array comes from its
/// initializer, where the elements are all present.
fn declared_kind(ty: &yashy_parser::ast::TypeExpr) -> Option<TypeKind> {
    use yashy_parser::ast::TypeExpr;
    match ty {
        TypeExpr::Name(name, _) => match name.as_str() {
            "int" => Some(TypeKind::I64),
            "float" => Some(TypeKind::F64),
            "bool" => Some(TypeKind::Bool),
            "string" => Some(TypeKind::Str),
            "char" => Some(TypeKind::Char),
            _ => None,
        },
        _ => None,
    }
}

/// A written type, for a message that refuses it.
fn written_kind(ty: &yashy_parser::ast::TypeExpr) -> String {
    use yashy_parser::ast::TypeExpr;
    match ty {
        TypeExpr::Name(name, _) => format!("`{name}`"),
        _ => "this type".to_string(),
    }
}

/// The integer a literal denotes in a pattern test.
fn pattern_integer(literal: &Literal) -> Option<i64> {
    match literal {
        Literal::I64(value) => Some(*value),
        Literal::Char(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// Bind the names a pattern declares, all with the subject's type.
///
/// Only the forms that have a compile-time value are accepted; a farther
/// pattern is reported instead of being half-matched.
fn check_pattern_names(
    pattern: &Pattern,
    ty: &Type,
    env: &mut Vec<(String, Type)>,
) -> Result<(), TypeError> {
    match pattern {
        Pattern::Wildcard(_) => Ok(()),
        Pattern::Binding { name, span } | Pattern::BindAt { name, span, .. } => {
            if env.iter().any(|(bound, _)| bound == name) {
                return Err(TypeError::new(*span, format!("`{name}` is already bound")));
            }
            env.push((name.clone(), ty.clone()));
            if let Pattern::BindAt { pattern, .. } = pattern {
                check_pattern_names(pattern, ty, env)?;
            }
            Ok(())
        }
        Pattern::Literal(_, _) | Pattern::Range { .. } | Pattern::UnitVariant { .. } => Ok(()),
        other => Err(TypeError::new(
            Span::SYNTHETIC,
            format!(
                "{} is not a pattern a const fn can match",
                match other {
                    Pattern::TupleVariant { .. } => "a variant with a payload",
                    Pattern::StructVariant { .. } => "a struct variant",
                    Pattern::Tuple { .. } => "a tuple pattern",
                    Pattern::FixedArray { .. } => "an array pattern",
                    Pattern::Or { .. } => "an or-pattern",
                    _ => "this pattern",
                }
            ),
        )),
    }
}

/// Check one expression and report the type it has.
///
/// The type is what the evaluator needs: it is the value's type, and for a
/// call it is the called function's return type.
fn check_expr(
    expr: &Expr,
    env: &mut Vec<(String, Type)>,
    consts: &ConstFns,
    context: &str,
    work: &mut Work,
    structs: &ConstTypes,
) -> Result<Type, TypeError> {
    match expr {
        Expr::Literal(literal, span) => Ok(Type {
            kind: literal_kind(literal),
            span: *span,
        }),
        Expr::Ident(path, span) => {
            // A dotted name whose head is a local binding is a field read in
            // the main checker; the constant evaluator must agree.
            if path.len() >= 2 && env.iter().any(|(name, _)| name == &path[0]) {
                let mut current = env
                    .iter()
                    .rev()
                    .find(|(name, _)| name == &path[0])
                    .map(|(_, ty)| ty.clone())
                    .expect("the head was found above");
                for field in &path[1..] {
                    let TypeKind::Struct(name) = &current.kind else {
                        return Err(TypeError::new(
                            *span,
                            format!("only a struct has a field, found `{current}`"),
                        ));
                    };
                    let Some(declared) = structs.structs.get(name) else {
                        return Err(TypeError::new(
                            *span,
                            format!("`{name}` has no known fields in {context}"),
                        ));
                    };
                    let Some((_, ty)) = declared.iter().find(|(written, _)| written == field)
                    else {
                        return Err(TypeError::new(
                            *span,
                            format!("`{name}` has no field `{field}`"),
                        ));
                    };
                    current = ty.clone();
                }
                return Ok(current);
            }
            if let [enum_name, variant] = path.as_slice() {
                if let Some(variants) = structs.enums.get(enum_name) {
                    if let Some((_, payload_types)) =
                        variants.iter().find(|(name, _)| name == variant)
                    {
                        if !payload_types.is_empty() {
                            return Err(TypeError::new(
                                *span,
                                format!("variant `{variant}` needs a payload: write `{enum_name}.{variant}(..)`"),
                            ));
                        }
                        return Ok(Type {
                            kind: TypeKind::Enum(enum_name.clone()),
                            span: *span,
                        });
                    }
                }
            }
            let name = path.last().cloned().unwrap_or_default();
            match env.iter().rev().find(|(param, _)| param == &name) {
                Some((_, ty)) => Ok(ty.clone()),
                None => Err(TypeError::new(*span, format!("cannot find value `{name}`"))),
            }
        }
        Expr::Binary { op, lhs, rhs, span } => {
            let left = check_expr(lhs, env, consts, context, work, structs)?;
            let right = check_expr(rhs, env, consts, context, work, structs)?;
            let kind = binary_kind(*op, &left, &right, *span)?;
            // A literal count outside the type's range is known to fault, so the
            // declaration is rejected here rather than at every call. The
            // message is the runtime's, so both build modes say the same thing.
            if matches!(op, BinOp::Shl | BinOp::Shr) {
                if let (Some(count), Some(bits)) = (ast_int(rhs), kind.kind.bit_width()) {
                    if count < 0 || count as i128 >= bits as i128 {
                        return Err(TypeError::new(
                            *span,
                            format!(
                                "shift amount {count} is out of range for {} (0..{})",
                                kind.kind,
                                bits - 1
                            ),
                        )
                        .with_code(codes::SHIFT_OUT_OF_RANGE));
                    }
                }
            }
            Ok(kind)
        }
        Expr::Unary { op, operand, span } => {
            let inner = check_expr(operand, env, consts, context, work, structs)?;
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
                UnOp::BitNot if inner.kind.is_integer() => Ok(inner),
                UnOp::BitNot => Err(TypeError::new(
                    *span,
                    format!("cannot apply `~` to `{}`", inner.kind),
                )),
            }
        }
        Expr::FixedArrayLit { elements, span } => {
            let mut element_ty: Option<Type> = None;
            for element in elements {
                let actual = check_expr(element, env, consts, context, work, structs)?;
                match &element_ty {
                    None => {
                        if !const_array_element_ok(&actual.kind) {
                            return Err(TypeError::new(
                                *span,
                                format!("a fixed array cannot hold `{}`", actual.kind),
                            ));
                        }
                        element_ty = Some(actual);
                    }
                    Some(expected) if expected.kind != actual.kind => {
                        return Err(TypeError::new(
                            expr_span(element),
                            format!("expected `{expected}`, found `{actual}`"),
                        ));
                    }
                    _ => {}
                }
            }
            let Some(element_ty) = element_ty else {
                return Err(TypeError::new(
                    *span,
                    "a fixed array needs at least one element",
                ));
            };
            Ok(Type {
                kind: TypeKind::FixedArray(Box::new(element_ty), elements.len()),
                span: *span,
            })
        }
        Expr::Index { base, index, span } => {
            let base_ty = check_expr(base, env, consts, context, work, structs)?;
            let TypeKind::FixedArray(element, _) = &base_ty.kind else {
                return Err(TypeError::new(
                    *span,
                    format!("only a fixed array can be indexed in {context}, found `{base_ty}`"),
                ));
            };
            let index_ty = check_expr(index, env, consts, context, work, structs)?;
            if index_ty.kind != TypeKind::I64 {
                return Err(TypeError::new(
                    expr_span(index),
                    format!("an index must be `int`, found `{index_ty}`"),
                ));
            }
            Ok(element.as_ref().clone())
        }
        Expr::Call {
            callee, args, span, ..
        } => {
            // A dotted call on a local binding is a scalar conversion; the
            // main checker flattens it to `[binding, method]` just like the
            // enum constructor spelling, so the head tells them apart.
            if let [head, method] = callee.as_slice() {
                if env.iter().any(|(name, _)| name == head) {
                    let receiver_ty = check_expr(
                        &Expr::Ident(vec![head.clone()], *span),
                        env,
                        consts,
                        context,
                        work,
                        structs,
                    )?;
                    let known = match (&receiver_ty.kind, method.as_str()) {
                        (TypeKind::Char, "to_int") => TypeKind::I64,
                        (TypeKind::I64, "to_char") => TypeKind::Char,
                        _ => {
                            return Err(TypeError::new(
                                *span,
                                format!("`{method}` is not a method a const fn can call"),
                            ));
                        }
                    };
                    if !args.is_empty() {
                        return Err(TypeError::new(
                            *span,
                            format!("`{method}` takes no arguments"),
                        ));
                    }
                    return Ok(Type {
                        kind: known,
                        span: *span,
                    });
                }
            }
            if let [enum_name, variant] = callee.as_slice() {
                let Some(variants) = structs.enums.get(enum_name) else {
                    return Err(TypeError::new(
                        *span,
                        format!("`{enum_name}` is not an enum this const fn can build"),
                    ));
                };
                let Some((_, payload_types)) = variants.iter().find(|(name, _)| name == variant)
                else {
                    return Err(TypeError::new(
                        *span,
                        format!("`{enum_name}` has no variant `{variant}`"),
                    ));
                };
                if args.len() != payload_types.len() {
                    return Err(TypeError::new(
                        *span,
                        format!(
                            "variant `{variant}` takes {} payload value(s), found {}",
                            payload_types.len(),
                            args.len()
                        ),
                    ));
                }
                for (argument, expected) in args.iter().zip(payload_types) {
                    let actual = check_expr(&argument.value, env, consts, context, work, structs)?;
                    if actual.kind != *expected {
                        return Err(TypeError::new(
                            argument.span,
                            format!("expected `{expected}`, found `{actual}`"),
                        )
                        .with_code(codes::TYPE_MISMATCH));
                    }
                }
                return Ok(Type {
                    kind: TypeKind::Enum(enum_name.clone()),
                    span: *span,
                });
            }
            let name = match callee.as_slice() {
                [name] => name.clone(),
                _ => {
                    return Err(TypeError::new(
                        *span,
                        "a qualified call is not allowed in a const fn".to_string(),
                    ));
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
                let actual = check_expr(&argument.value, env, consts, context, work, structs)?;
                if actual.kind != param.kind {
                    return Err(TypeError::new(
                        argument.span,
                        format!("expected `{param}`, found `{actual}`"),
                    )
                    .with_code(codes::TYPE_MISMATCH));
                }
            }
            Ok(function.ret.clone())
        }
        Expr::StructLit {
            path,
            fields,
            spread,
            span,
        } => {
            let name = path.last().cloned().unwrap_or_default();
            let Some(declared) = structs.structs.get(&name) else {
                return Err(TypeError::new(
                    *span,
                    format!("`{name}` is not a struct this const fn can build"),
                ));
            };
            if spread.is_some() {
                return Err(TypeError::new(
                    *span,
                    "a struct spread is not allowed in a const fn".to_string(),
                ));
            }
            for (field, value, field_span) in fields {
                let Some((_, expected)) = declared.iter().find(|(written, _)| written == field)
                else {
                    return Err(TypeError::new(
                        *field_span,
                        format!("`{name}` has no field `{field}`"),
                    ));
                };
                let actual = check_expr(value, env, consts, context, work, structs)?;
                if actual.kind != expected.kind {
                    return Err(TypeError::new(
                        *field_span,
                        format!("expected `{expected}`, found `{actual}`"),
                    )
                    .with_code(codes::TYPE_MISMATCH));
                }
            }
            for (field, _) in declared {
                if !fields.iter().any(|(written, _, _)| written == field) {
                    return Err(TypeError::new(
                        *span,
                        format!("`{name}` needs a value for field `{field}`"),
                    )
                    .with_code(codes::MISSING_FIELD));
                }
            }
            Ok(Type {
                kind: TypeKind::Struct(name),
                span: *span,
            })
        }
        Expr::FieldAccess { base, field, span } => {
            let base_ty = check_expr(base, env, consts, context, work, structs)?;
            let TypeKind::Struct(name) = &base_ty.kind else {
                return Err(TypeError::new(
                    *span,
                    format!("only a struct has a field, found `{base_ty}`"),
                ));
            };
            let Some(declared) = structs.structs.get(name) else {
                return Err(TypeError::new(
                    *span,
                    format!("`{name}` has no known fields in {context}"),
                ));
            };
            let Some((_, ty)) = declared.iter().find(|(written, _)| written == field) else {
                return Err(TypeError::new(
                    *span,
                    format!("`{name}` has no field `{field}`"),
                ));
            };
            Ok(ty.clone())
        }
        Expr::MethodCall {
            receiver,
            method,
            args,
            span,
        } => {
            // The only method calls with a compile-time value are the scalar
            // conversions; every other method reaches storage the compiler
            // cannot read while folding.
            let receiver_ty = check_expr(receiver, env, consts, context, work, structs)?;
            let known = match (&receiver_ty.kind, method.as_str()) {
                (TypeKind::Char, "to_int") => TypeKind::I64,
                (TypeKind::I64, "to_char") => TypeKind::Char,
                _ => {
                    return Err(TypeError::new(
                        *span,
                        format!("`{method}` is not a method a const fn can call"),
                    ));
                }
            };
            if !args.is_empty() {
                return Err(TypeError::new(
                    *span,
                    format!("`{method}` takes no arguments"),
                ));
            }
            Ok(Type {
                kind: known,
                span: *span,
            })
        }
        Expr::SwitchExpr {
            subject,
            arms,
            span,
            ..
        } => {
            let subject_ty = check_expr(subject, env, consts, context, work, structs)?;
            let mut arm_ty: Option<Type> = None;
            for arm in arms {
                let before = env.len();
                check_pattern_names(&arm.pattern, &subject_ty, env)?;
                if let Some(guard) = &arm.guard {
                    let guard_ty = check_expr(guard, env, consts, context, work, structs)?;
                    env.truncate(before);
                    if guard_ty.kind != TypeKind::Bool {
                        return Err(TypeError::new(
                            *span,
                            format!("a switch guard must be `bool`, found `{guard_ty}`"),
                        ));
                    }
                }
                let body_ty = check_expr(&arm.body, env, consts, context, work, structs)?;
                env.truncate(before);
                match &arm_ty {
                    Some(expected) if expected.kind != body_ty.kind => {
                        return Err(TypeError::new(
                            *span,
                            format!(
                                "a switch has arms of `{expected}` and `{body_ty}`; a const fn \
                                 needs one result type"
                            ),
                        ));
                    }
                    None => arm_ty = Some(body_ty),
                    _ => {}
                }
            }
            arm_ty.ok_or_else(|| TypeError::new(*span, "a switch needs at least one arm"))
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

/// The value of a literal integer expression, or `None` when it is not one.
///
/// The constant language has one integer width, so a literal and a negated
/// literal are the only shapes that can be folded here.
fn ast_int(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Literal(Literal::I64(value), _) => Some(*value),
        Expr::Unary {
            op: UnOp::Neg,
            operand,
            ..
        } => ast_int(operand).map(i64::wrapping_neg),
        _ => None,
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
            let allowed =
                is_numeric(&left.kind) || (matches!(op, BinOp::Add) && left.kind == TypeKind::Str);
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
            if left.kind == right.kind && (is_numeric(&left.kind) || left.kind == TypeKind::Char) {
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
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            if left.kind == TypeKind::Bool && right.kind == TypeKind::Bool {
                return Ok(bool_type());
            }
            if left.kind.is_integer() && left.kind == right.kind {
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
        BinOp::Shl | BinOp::Shr => {
            if left.kind.is_integer() && left.kind == right.kind {
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
    }
}

/// Whether a type is one the evaluator can compute with.
/// The scalar kinds a fixed array can hold in a `const fn`.
///
/// A fixed array is copied by copying its bytes, so the same rule the checker
/// applies to a run-time `[T; N]` applies here: numbers, `bool` and `char`, no
/// references and no owners.
fn const_array_element_ok(kind: &TypeKind) -> bool {
    matches!(
        kind,
        TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Char
    )
}

fn is_numeric(kind: &TypeKind) -> bool {
    matches!(kind, TypeKind::I64 | TypeKind::F64)
}

/// The name of a statement, for the message that refuses it.
fn statement_kind(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::Expr(_, _) => "an expression statement",
        Stmt::Const { .. } => "a `const` binding",
        Stmt::Let { .. } => "a `let` binding",
        Stmt::LetPattern { .. } => "a destructuring binding",
        Stmt::Assign { .. } => "an assignment",
        Stmt::Return(_, _) => "a `return`",
        Stmt::If { .. } => "an `if`",
        Stmt::IfLet { .. } => "an `if let`",
        Stmt::WhileLet { .. } => "a `while let`",
        Stmt::ForRange { .. } => "a `for` loop",
        Stmt::ForIn { .. } => "a `for ... in` loop",
        Stmt::ForC { .. } => "a `for` loop",
        Stmt::Switch { .. } => "a `switch`",
        Stmt::Break(_, _) => "a `break`",
        Stmt::Continue(_, _) => "a `continue`",
        Stmt::Defer { .. } => "a `defer`",
        Stmt::Cfg { .. } => "a `cfg` block",
        Stmt::Arena { .. } => "an `arena` block",
        Stmt::Scope { .. } => "a `scope` block",
        Stmt::Invariant(_) => "a loop `invariant`",
    }
}

/// The name of an expression, for the message that refuses it.
fn expression_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Literal(_, _) => "a literal",
        Expr::UnsafeBlock { .. } => "an `unsafe` block",
        Expr::Ident(_, _) => "a name",
        Expr::Binary { .. } => "an arithmetic expression",
        Expr::CompareChain { .. } => "a comparison chain",
        Expr::Is { .. } => "an `is` test",
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
        Expr::Range { .. } => "a range",
        Expr::Interp { .. } => "an interpolation",
        Expr::Cast { .. } => "a cast",
        Expr::Try { .. } => "a `?`",
        Expr::Await { .. } => "an `await`",
        Expr::Spawn { .. } => "a `spawn`",
        Expr::SwitchExpr { .. } => "a `switch`",
        Expr::MethodCall { .. } => "a method call",
        Expr::Lambda { .. } => "a lambda",
        Expr::TypePath { .. } => "a type path",
        Expr::TypePathCall { .. } => "a type-path call",
        Expr::ArrayComp { .. } => "a comprehension",
        Expr::MapComp { .. } => "a comprehension",
    }
}

/// The span of an expression.
fn expr_span(expr: &Expr) -> Span {
    match expr {
        Expr::Literal(_, span)
        | Expr::Ident(_, span)
        | Expr::Binary { span, .. }
        | Expr::CompareChain { span, .. }
        | Expr::Is { span, .. }
        | Expr::UnsafeBlock { span, .. }
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
        | Expr::Range { span, .. }
        | Expr::Interp { span, .. }
        | Expr::Cast { span, .. }
        | Expr::Try { span, .. }
        | Expr::Await { span, .. }
        | Expr::Spawn { span, .. }
        | Expr::SwitchExpr { span, .. }
        | Expr::MethodCall { span, .. }
        | Expr::Lambda { span, .. }
        | Expr::TypePath { span, .. }
        | Expr::TypePathCall { span, .. }
        | Expr::ArrayComp { span, .. }
        | Expr::MapComp { span, .. } => *span,
    }
}

/// Whether an expression mentions a `const fn`.
///
/// A `const` binding whose initializer mentions one is evaluated at compile
/// time; one that does not is an ordinary run-time binding, as it always was.
pub fn mentions_const_fn(expr: &Expr, consts: &ConstFns) -> bool {
    match expr {
        Expr::Literal(_, _) | Expr::Ident(_, _) => false,
        Expr::UnsafeBlock { .. } => false,
        Expr::Binary { lhs, rhs, .. } => {
            mentions_const_fn(lhs, consts) || mentions_const_fn(rhs, consts)
        }
        Expr::CompareChain { operands, .. } => operands
            .iter()
            .any(|operand| mentions_const_fn(operand, consts)),
        Expr::Is { lhs, .. } => mentions_const_fn(lhs, consts),
        Expr::Range { start, end, .. } => {
            mentions_const_fn(start, consts) || mentions_const_fn(end, consts)
        }
        Expr::Unary { operand, .. } => mentions_const_fn(operand, consts),
        Expr::Move(inner, _) => mentions_const_fn(inner, consts),
        Expr::Call { callee, args, .. } => {
            matches!(callee.as_slice(), [name] if consts.contains_key(name))
                || args.iter().any(|arg| mentions_const_fn(&arg.value, consts))
        }
        Expr::StructLit { fields, spread, .. } => {
            fields
                .iter()
                .any(|(_, value, _)| mentions_const_fn(value, consts))
                || spread
                    .as_deref()
                    .is_some_and(|operand| mentions_const_fn(operand, consts))
        }
        Expr::FieldAccess { base, .. } => mentions_const_fn(base, consts),
        Expr::SliceLit { elements, .. } => elements.iter().any(|element| match element {
            yashy_parser::ast::SliceElement::Value(value) => mentions_const_fn(value, consts),
            yashy_parser::ast::SliceElement::Spread(operand) => mentions_const_fn(operand, consts),
        }),
        Expr::Index { base, index, .. } => {
            mentions_const_fn(base, consts) || mentions_const_fn(index, consts)
        }
        Expr::FixedArrayLit { elements, .. } => elements
            .iter()
            .any(|element| mentions_const_fn(element, consts)),
        Expr::TupleLit { elements, .. } => elements
            .iter()
            .any(|element| mentions_const_fn(element, consts)),
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
        Expr::Await { expr, .. } => mentions_const_fn(expr, consts),
        Expr::Spawn { callee, args, .. } => {
            mentions_const_fn(callee, consts)
                || args.iter().any(|arg| mentions_const_fn(&arg.value, consts))
        }
        Expr::SwitchExpr { subject, arms, .. } => {
            mentions_const_fn(subject, consts)
                || arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .is_some_and(|guard| mentions_const_fn(guard, consts))
                        || mentions_const_fn(&arm.body, consts)
                })
        }
        Expr::Lambda { .. } => false,
        Expr::MethodCall { .. } => false,
        Expr::TypePath { .. } => false,
        Expr::TypePathCall { args, .. } => {
            args.iter().any(|arg| mentions_const_fn(&arg.value, consts))
        }
        Expr::ArrayComp {
            expr,
            iterable,
            filter,
            ..
        } => {
            mentions_const_fn(expr, consts)
                || mentions_const_fn(iterable, consts)
                || filter
                    .as_deref()
                    .is_some_and(|f| mentions_const_fn(f, consts))
        }
        Expr::MapComp {
            key,
            value,
            iterable,
            filter,
            ..
        } => {
            mentions_const_fn(key, consts)
                || mentions_const_fn(value, consts)
                || mentions_const_fn(iterable, consts)
                || filter
                    .as_deref()
                    .is_some_and(|f| mentions_const_fn(f, consts))
        }
    }
}

/// Evaluate an expression at compile time.
///
/// `env` holds the values the enclosing `const fn` was called with, and
/// `work` the state that bounds the evaluation: how deep it may nest and how
/// many calls it may make. A call that would not terminate is reported rather
/// than followed.
///
/// The bindings are copied into a working environment, because a `const fn`
/// body may declare and assign its own: a call sees the arguments it was
/// given, and a local one of its callees declares never reaches back out.
pub fn evaluate(
    expr: &Expr,
    env: &[(String, ConstValue)],
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
    structs: &ConstTypes,
) -> Result<ConstValue, ConstError> {
    let mut bindings = env.to_vec();
    eval(expr, &mut bindings, consts, work, missing, structs)
}

/// Spend one step of the evaluation budget, or report that it is gone.
///
/// A loop iterates without an expression of its own to charge, so the loop
/// spends one step per iteration as well: a body that never terminates is
/// reported by the same budget as a wide expression tree.
fn tick_at(span: Span, work: &mut Work) -> Result<(), ConstError> {
    work.steps = work.steps.saturating_add(1);
    if work.steps > MAX_CONST_STEPS {
        return Err(ConstError::new(
            span,
            format!("const evaluation exceeded {MAX_CONST_STEPS} steps"),
        )
        .with_code(codes::CONST_FUEL));
    }
    Ok(())
}

/// Try one pattern against one folded value, binding names on success.
fn pattern_matches(
    pattern: &Pattern,
    value: &ConstValue,
    env: &mut Vec<(String, ConstValue)>,
) -> Result<bool, ConstError> {
    match pattern {
        Pattern::Wildcard(_) => Ok(true),
        Pattern::Binding { name, .. } => {
            env.push((name.clone(), value.clone()));
            Ok(true)
        }
        Pattern::BindAt { name, pattern, .. } => {
            if pattern_matches(pattern, value, env)? {
                env.push((name.clone(), value.clone()));
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Pattern::Literal(literal, _) => Ok(match (literal, value) {
            (Literal::I64(expected), ConstValue::I64(found)) => expected == found,
            (Literal::Bool(expected), ConstValue::Bool(found)) => expected == found,
            (Literal::Char(expected), ConstValue::Char(found)) => {
                i64::from(*expected) == i64::from(*found)
            }
            _ => false,
        }),
        Pattern::Range {
            lo, hi, inclusive, ..
        } => {
            let number = match value {
                ConstValue::I64(value) => Some(*value),
                ConstValue::Char(value) => Some(i64::from(*value)),
                _ => None,
            };
            let Some(number) = number else {
                return Ok(false);
            };
            let (Some(lo), Some(hi)) = (pattern_integer(lo), pattern_integer(hi)) else {
                return Ok(false);
            };
            Ok(number >= lo
                && (if *inclusive {
                    number <= hi
                } else {
                    number < hi
                }))
        }
        Pattern::UnitVariant { path, .. } => Ok(matches!(
            value,
            ConstValue::Enum { variant, payload, .. }
                if payload.is_empty() && path.last() == Some(variant)
        )),
        other => Err(ConstError::new(
            Span::SYNTHETIC,
            format!(
                "this pattern cannot be matched at compile time: {}",
                match other {
                    Pattern::TupleVariant { .. } => "a variant with a payload",
                    Pattern::StructVariant { .. } => "a struct variant",
                    Pattern::Tuple { .. } => "a tuple pattern",
                    Pattern::FixedArray { .. } => "an array pattern",
                    Pattern::Or { .. } => "an or-pattern",
                    _ => "this pattern",
                }
            ),
        )),
    }
}

/// Evaluate one expression against the bindings in scope.
fn eval(
    expr: &Expr,
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
    structs: &ConstTypes,
) -> Result<ConstValue, ConstError> {
    tick_at(expr_span(expr), work)?;
    match expr {
        Expr::Literal(literal, _) => Ok(match literal {
            Literal::I64(value) => ConstValue::I64(*value),
            Literal::F64(value) => ConstValue::F64(*value),
            Literal::Bool(value) => ConstValue::Bool(*value),
            Literal::Str(text) => ConstValue::Str(text.value.clone()),
            Literal::Char(byte) => ConstValue::Char(*byte),
        }),
        Expr::Ident(path, span) => {
            if path.len() >= 2 && env.iter().any(|(name, _)| name == &path[0]) {
                let mut value = env
                    .iter()
                    .rev()
                    .find(|(name, _)| name == &path[0])
                    .map(|(_, value)| value.clone())
                    .expect("the head was found above");
                for field in &path[1..] {
                    let ConstValue::Struct { name, fields } = value else {
                        return Err(ConstError::new(
                            *span,
                            format!("only a struct has a field, found `{}`", value.kind()),
                        ));
                    };
                    let Some((_, found)) = fields.into_iter().find(|(written, _)| written == field)
                    else {
                        return Err(ConstError::new(
                            *span,
                            format!("`{name}` has no field `{field}`"),
                        ));
                    };
                    value = found;
                }
                return Ok(value);
            }
            if let [enum_name, variant] = path.as_slice() {
                if let Some(variants) = structs.enums.get(enum_name) {
                    if variants.iter().any(|(name, _)| name == variant) {
                        return Ok(ConstValue::Enum {
                            name: enum_name.clone(),
                            variant: variant.clone(),
                            payload: Vec::new(),
                        });
                    }
                }
            }
            let name = path.last().cloned().unwrap_or_default();
            match env.iter().rev().find(|(param, _)| param == &name) {
                Some((_, value)) => Ok(value.clone()),
                None => Err(ConstError::new(*span, missing.message(&name))
                    .with_code(codes::CONST_NOT_CONSTANT)),
            }
        }
        Expr::Binary { op, lhs, rhs, span } => {
            let left = eval(lhs, env, consts, work, missing, structs)?;
            let right = eval(rhs, env, consts, work, missing, structs)?;
            evaluate_binary(*op, &left, &right, *span)
        }
        Expr::Unary { op, operand, span } => {
            let inner = eval(operand, env, consts, work, missing, structs)?;
            match (op, &inner) {
                (UnOp::Neg, ConstValue::I64(value)) => Ok(ConstValue::I64(value.wrapping_neg())),
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
                (UnOp::BitNot, ConstValue::I64(value)) => Ok(ConstValue::I64(!value)),
                (UnOp::BitNot, other) => Err(ConstError::new(
                    *span,
                    format!("cannot apply `~` to `{}`", other.kind()),
                )),
            }
        }
        Expr::FixedArrayLit { elements, span } => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(eval(element, env, consts, work, missing, structs)?);
            }
            if let Some(first) = values.first() {
                if values.iter().any(|value| value.kind() != first.kind()) {
                    return Err(ConstError::new(
                        *span,
                        "a fixed array needs elements of one type".to_string(),
                    ));
                }
            }
            Ok(ConstValue::Array(values))
        }
        Expr::Index { base, index, span } => {
            let base_value = eval(base, env, consts, work, missing, structs)?;
            let ConstValue::Array(values) = &base_value else {
                return Err(ConstError::new(
                    *span,
                    format!("cannot index `{}` in a const fn", base_value.kind()),
                ));
            };
            let ConstValue::I64(at) = eval(index, env, consts, work, missing, structs)? else {
                return Err(ConstError::new(
                    expr_span(index),
                    "an index must be `int`".to_string(),
                ));
            };
            if at < 0 || at as usize >= values.len() {
                return Err(ConstError::new(
                    *span,
                    format!(
                        "index {at} is outside a fixed array of length {}",
                        values.len()
                    ),
                ));
            }
            Ok(values[at as usize].clone())
        }
        Expr::Call {
            callee, args, span, ..
        } => {
            if let [head, method] = callee.as_slice() {
                if env.iter().any(|(name, _)| name == head) {
                    let value = env
                        .iter()
                        .rev()
                        .find(|(name, _)| name == head)
                        .map(|(_, value)| value.clone())
                        .expect("the head was found above");
                    return match (method.as_str(), value) {
                        ("to_int", ConstValue::Char(byte)) => Ok(ConstValue::I64(i64::from(byte))),
                        ("to_char", ConstValue::I64(number)) => {
                            let byte = u8::try_from(number).map_err(|_| {
                                ConstError::new(
                                    *span,
                                    format!("`{number}` is not a character code"),
                                )
                            })?;
                            Ok(ConstValue::Char(byte))
                        }
                        (_, value) => Err(ConstError::new(
                            *span,
                            format!(
                                "`{method}` has no compile-time value for `{}`",
                                value.kind()
                            ),
                        )),
                    };
                }
            }
            if let [enum_name, variant] = callee.as_slice() {
                let mut payload = Vec::with_capacity(args.len());
                for argument in args {
                    payload.push(eval(&argument.value, env, consts, work, missing, structs)?);
                }
                return Ok(ConstValue::Enum {
                    name: enum_name.clone(),
                    variant: variant.clone(),
                    payload,
                });
            }
            let name = match callee.as_slice() {
                [name] => name.clone(),
                _ => {
                    return Err(ConstError::new(
                        *span,
                        "a qualified call is not allowed in a const fn",
                    ));
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
                )
                .with_code(codes::CONST_RECURSION));
            }
            let mut values = Vec::with_capacity(args.len());
            for argument in args {
                values.push(eval(&argument.value, env, consts, work, missing, structs)?);
            }
            let mut bound = Vec::with_capacity(function.params.len());
            for ((param, _), value) in function.params.iter().zip(values.into_iter()) {
                bound.push((param.clone(), value));
            }
            work.depth = work.depth.saturating_add(1);
            // Prefer the typed HIR body: the whole point of Step 5 is that the
            // evaluator consumes the same typed tree the checker produces, not
            // a second untyped AST. The AST body stays as a fallback for a
            // `const fn` whose body could not be lowered (e.g. an unusual form
            // the const-mode checker refused), so a valid program never fails
            // just because lowering is incomplete.
            let returned = if let Some(hir_body) = &function.hir_body {
                match evaluate_hir_block(hir_body, &mut bound, consts, work, missing, structs)? {
                    HirFlow::Returned(value) => Some(value),
                    HirFlow::Fell => None,
                    HirFlow::Break | HirFlow::Continue => None,
                }
            } else {
                match evaluate_block(
                    &function.body,
                    &mut bound,
                    consts,
                    work,
                    false,
                    function.params.len(),
                    structs,
                )? {
                    Flow::Returned(value) => Some(value),
                    Flow::Fell | Flow::Break | Flow::Continue => None,
                }
            };
            work.depth = work.depth.saturating_sub(1);
            let value = match returned {
                Some(value) => value,
                None => {
                    return Err(ConstError::new(
                        *span,
                        format!("`{name}` does not return a value on every path"),
                    ));
                }
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
        Expr::StructLit {
            path, fields, span, ..
        } => {
            let name = path.last().cloned().unwrap_or_default();
            let mut built = Vec::with_capacity(fields.len());
            for (field, value, _) in fields {
                built.push((
                    field.clone(),
                    eval(value, env, consts, work, missing, structs)?,
                ));
            }
            let _ = span;
            Ok(ConstValue::Struct {
                name,
                fields: built,
            })
        }
        Expr::FieldAccess { base, field, span } => {
            let value = eval(base, env, consts, work, missing, structs)?;
            let ConstValue::Struct { name, fields } = value else {
                return Err(ConstError::new(
                    *span,
                    format!("only a struct has a field, found `{}`", value.kind()),
                ));
            };
            let Some((_, found)) = fields.into_iter().find(|(written, _)| written == field) else {
                return Err(ConstError::new(
                    *span,
                    format!("`{name}` has no field `{field}`"),
                ));
            };
            Ok(found)
        }
        Expr::MethodCall {
            receiver, method, ..
        } => {
            let value = eval(receiver, env, consts, work, missing, structs)?;
            match (method.as_str(), value) {
                ("to_int", ConstValue::Char(byte)) => Ok(ConstValue::I64(i64::from(byte))),
                ("to_char", ConstValue::I64(number)) => {
                    let byte = u8::try_from(number).map_err(|_| {
                        ConstError::new(
                            expr_span(expr),
                            format!("`{number}` is not a character code"),
                        )
                    })?;
                    Ok(ConstValue::Char(byte))
                }
                (_, value) => Err(ConstError::new(
                    expr_span(expr),
                    format!(
                        "`{method}` has no compile-time value for `{}`",
                        value.kind()
                    ),
                )),
            }
        }
        Expr::SwitchExpr {
            subject,
            arms,
            span,
            ..
        } => {
            let value = eval(subject, env, consts, work, missing, structs)?;
            for arm in arms {
                let before = env.len();
                let matched = if arm.is_default {
                    true
                } else {
                    pattern_matches(&arm.pattern, &value, env)?
                };
                if !matched {
                    env.truncate(before);
                    continue;
                }
                if let Some(guard) = &arm.guard {
                    let allowed = eval(guard, env, consts, work, missing, structs)?;
                    let ConstValue::Bool(allowed) = allowed else {
                        env.truncate(before);
                        return Err(ConstError::new(
                            *span,
                            "a switch guard must be `bool`".to_string(),
                        ));
                    };
                    if !allowed {
                        env.truncate(before);
                        continue;
                    }
                }
                let result = eval(&arm.body, env, consts, work, missing, structs);
                env.truncate(before);
                return result;
            }
            Err(ConstError::new(
                *span,
                "no switch arm matched the value in this const fn".to_string(),
            ))
        }
        other => Err(ConstError::new(
            expr_span(other),
            format!("{} is not allowed in a const fn", expression_kind(other)),
        )),
    }
}

/// What a statement did, as far as the statements after it are concerned.
#[derive(Clone, Debug)]
enum Flow {
    /// The statement ran to its end without returning or jumping: the next
    /// statement still runs. That is what makes `if c { return a; } return b;`
    /// work -- the `if` returns on one path and falls through on the other.
    Fell,
    /// A `return` left the function with this value.
    Returned(ConstValue),
    /// A `break` left the enclosing loop.
    Break,
    /// A `continue` went on to the enclosing loop's next iteration.
    Continue,
}

/// Evaluate the statements of a `const fn` body.
///
/// The block's own scope is the tail of `env`: whatever it declares is dropped
/// when it ends, so two sibling blocks may each declare `i`.
fn evaluate_block(
    block: &Block,
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    in_loop: bool,
    params: usize,
    structs: &ConstTypes,
) -> Result<Flow, ConstError> {
    let base = env.len();
    let mut flow = Flow::Fell;
    for statement in &block.stmts {
        flow = evaluate_stmt(statement, env, consts, work, in_loop, params, structs)?;
        if !matches!(flow, Flow::Fell) {
            break;
        }
    }
    env.truncate(base);
    Ok(flow)
}

/// Evaluate one statement of a `const fn` body.
#[allow(clippy::too_many_arguments)]
fn evaluate_stmt(
    statement: &Stmt,
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    in_loop: bool,
    params: usize,
    structs: &ConstTypes,
) -> Result<Flow, ConstError> {
    match statement {
        Stmt::Return(Some(expr), _) => Ok(Flow::Returned(eval(
            expr,
            env,
            consts,
            work,
            Missing::NotFound,
            structs,
        )?)),
        Stmt::Return(None, span) => Err(ConstError::new(
            *span,
            "a const fn must return a value".to_string(),
        )),
        // `let x = e;` is computed now and bound for the rest of the block.
        // The written type was checked when the function was declared.
        Stmt::Let { name, init, .. } => {
            let value = eval(init, env, consts, work, Missing::NotFound, structs)?;
            env.push((name.clone(), value));
            Ok(Flow::Fell)
        }
        // `x = e;` -- the innermost binding of that name takes the value. A
        // name that is not a local is not assignable, which the declaration
        // check already proved, so this is a value-level answer to the same
        // question.
        Stmt::Assign {
            target,
            value,
            span,
        } => {
            let Expr::Ident(path, _) = target else {
                return Err(ConstError::new(
                    *span,
                    "only a name can be assigned in a const fn".to_string(),
                ));
            };
            let name = path.last().cloned().unwrap_or_default();
            let new_value = eval(value, env, consts, work, Missing::NotFound, structs)?;
            let Some((at, (_, slot))) = env
                .iter_mut()
                .enumerate()
                .rev()
                .find(|(_, (bound, _))| bound == &name)
            else {
                return Err(ConstError::new(
                    *span,
                    format!("cannot find value `{name}`"),
                ));
            };
            if at < params {
                return Err(ConstError::new(
                    *span,
                    format!("cannot assign to `{name}`, which is immutable"),
                )
                .with_code(codes::CONST_REASSIGNMENT));
            }
            if slot.kind() != new_value.kind() {
                return Err(ConstError::new(
                    *span,
                    format!("expected `{}`, found `{}`", slot.kind(), new_value.kind()),
                ));
            }
            *slot = new_value;
            Ok(Flow::Fell)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            let condition = eval(cond, env, consts, work, Missing::NotFound, structs)?;
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
                // No `else`: the `if` did nothing, and the statements after it
                // still run.
                return Ok(Flow::Fell);
            };
            evaluate_block(taken, env, consts, work, in_loop, params, structs)
        }
        // `while c { .. }` is `for ; c; { .. }`, and both are one node. The
        // condition is re-read every iteration, so a body that assigns the
        // name it tests is a loop that ends.
        Stmt::ForC {
            init,
            cond,
            update,
            body,
            span,
            ..
        } => {
            if let Some(init) = init {
                if let Flow::Returned(value) =
                    evaluate_stmt(init, env, consts, work, in_loop, params, structs)?
                {
                    return Ok(Flow::Returned(value));
                }
            }
            loop {
                tick_at(*span, work)?;
                if let Some(cond) = cond {
                    match eval(cond, env, consts, work, Missing::NotFound, structs)? {
                        ConstValue::Bool(true) => {}
                        ConstValue::Bool(false) => break,
                        other => {
                            return Err(ConstError::new(
                                expr_span(cond),
                                format!("a condition must be `bool`, found `{}`", other.kind()),
                            ));
                        }
                    }
                }
                match evaluate_block(body, env, consts, work, true, params, structs)? {
                    Flow::Returned(value) => return Ok(Flow::Returned(value)),
                    Flow::Break => break,
                    Flow::Fell | Flow::Continue => {}
                }
                if let Some(update) = update {
                    eval(update, env, consts, work, Missing::NotFound, structs)?;
                }
            }
            Ok(Flow::Fell)
        }
        Stmt::Break(label, span) => {
            if !in_loop {
                return Err(ConstError::new(
                    *span,
                    "`break` outside a loop in a const fn".to_string(),
                ));
            }
            if label.is_some() {
                return Err(ConstError::new(
                    *span,
                    "a labelled `break` is not allowed in a const fn".to_string(),
                ));
            }
            Ok(Flow::Break)
        }
        Stmt::Continue(label, span) => {
            if !in_loop {
                return Err(ConstError::new(
                    *span,
                    "`continue` outside a loop in a const fn".to_string(),
                ));
            }
            if label.is_some() {
                return Err(ConstError::new(
                    *span,
                    "a labelled `continue` is not allowed in a const fn".to_string(),
                ));
            }
            Ok(Flow::Continue)
        }
        other => Err(ConstError::new(
            other.span(),
            format!("{} is not allowed in a const fn", statement_kind(other)),
        )),
    }
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
                return Err(ConstError::new(span, "division by zero".to_string())
                    .with_code(codes::CONST_DIVISION_BY_ZERO));
            }
            a.checked_div(*b)
                .map(ConstValue::I64)
                .ok_or_else(|| overflow(span))
        }
        (BinOp::Rem, ConstValue::I64(a), ConstValue::I64(b)) => {
            if *b == 0 {
                return Err(ConstError::new(span, "division by zero".to_string())
                    .with_code(codes::CONST_DIVISION_BY_ZERO));
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
        (BinOp::And, ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(*a && *b)),
        (BinOp::Or, ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(*a || *b)),
        (BinOp::BitAnd, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::I64(a & b)),
        (BinOp::BitOr, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::I64(a | b)),
        (BinOp::BitXor, ConstValue::I64(a), ConstValue::I64(b)) => Ok(ConstValue::I64(a ^ b)),
        (BinOp::BitAnd, ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(*a && *b)),
        (BinOp::BitOr, ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(*a || *b)),
        (BinOp::BitXor, ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(*a ^ *b)),
        (BinOp::Shl, ConstValue::I64(a), ConstValue::I64(b)) => {
            if *b < 0 || *b >= 64 {
                return Err(shift_out_of_range(span, *b));
            }
            a.checked_shl(*b as u32)
                .map(ConstValue::I64)
                .ok_or_else(|| overflow(span))
        }
        (BinOp::Shr, ConstValue::I64(a), ConstValue::I64(b)) => {
            if *b < 0 || *b >= 64 {
                return Err(shift_out_of_range(span, *b));
            }
            a.checked_shr(*b as u32)
                .map(ConstValue::I64)
                .ok_or_else(|| overflow(span))
        }
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
    ConstError::new(span, "integer overflow").with_code(codes::CONST_OVERFLOW)
}

/// The message a constant shift count outside `0..64` produces.
///
/// The wording matches the runtime's `yashy_panic_shift` so a program cannot
/// pass the check in one build mode and fail in another. The type is always
/// `int` here: the constant evaluator has one integer width.
fn shift_out_of_range(span: Span, count: i64) -> ConstError {
    ConstError::new(
        span,
        format!("shift amount {count} is out of range for int (0..63)"),
    )
    .with_code(codes::SHIFT_OUT_OF_RANGE)
}

// ---- The typed-HIR evaluator ---------------------------------------------
//
// This is the evaluator the design wants: it consumes the same typed HIR the
// rest of the compiler uses, not a second, untyped tree. `check_body` still
// gates the restricted const language at declaration time, and `lower_const_body`
// in the checker lowers an accepted body to `HirBlock`. `evaluate_hir` runs that
// typed tree. The AST evaluator (`evaluate`) stays as a differential oracle:
// until the two are proven equal on the corpus, the checker keeps using the AST
// path. Nothing here changes the value a program computes.

/// Flow through a lowered const body.
enum HirFlow {
    Fell,
    Returned(ConstValue),
    Break,
    Continue,
}

/// Evaluate one `const fn` both ways and return `(ast_value, hir_value)`.
///
/// This is the differential oracle: the AST path is the long-standing one, the
/// HIR path consumes the typed body the checker lowered. A disagreement means
/// the typed evaluator drifted from the untyped one and is a compiler bug.
pub fn differential_evaluate(
    function: &ConstFn,
    consts: &ConstFns,
    structs: &ConstTypes,
) -> Result<(ConstValue, ConstValue), ConstError> {
    // Call the function with its parameters' designated values: use 0 for each
    // int and the equivalent zero for scalars. The two evaluators get the same
    // environment, so any difference is in evaluation, not the inputs.
    let env: Vec<(String, ConstValue)> = function
        .params
        .iter()
        .map(|(name, ty)| {
            let value = match ty.kind {
                TypeKind::F64 => ConstValue::F64(0.0),
                TypeKind::Bool => ConstValue::Bool(false),
                TypeKind::Char => ConstValue::Char(0),
                TypeKind::Str => ConstValue::Str(String::new()),
                _ => ConstValue::I64(0),
            };
            (name.clone(), value)
        })
        .collect();

    let mut ast_work = Work::default();
    let mut ast_env = env.clone();
    let ast = evaluate_block(
        &function.body,
        &mut ast_env,
        consts,
        &mut ast_work,
        false,
        function.params.len(),
        structs,
    )
    .and_then(|flow| match flow {
        Flow::Returned(value) => Ok(value),
        Flow::Fell => Ok(ConstValue::Bool(true)),
        Flow::Break | Flow::Continue => Err(ConstError::new(
            function.span,
            "a loop jump escaped a const fn",
        )),
    });

    let hir = function
        .hir_body
        .as_ref()
        .ok_or_else(|| ConstError::new(function.span, "no typed HIR body for differential"))?;
    let mut hir_work = Work::default();
    let mut hir_env = env;
    let hir = evaluate_hir_block(
        hir,
        &mut hir_env,
        consts,
        &mut hir_work,
        Missing::NotConstant,
        structs,
    )
    .and_then(|flow| match flow {
        HirFlow::Returned(value) => Ok(value),
        HirFlow::Fell => Ok(ConstValue::Bool(true)),
        HirFlow::Break | HirFlow::Continue => Err(ConstError::new(
            function.span,
            "a loop jump escaped a const fn",
        )),
    });

    Ok((ast?, hir?))
}

/// Evaluate a typed HIR expression to a constant value.
///
/// `env` is the current binding stack (innermost last); `consts` supplies other
/// `const fn` bodies; `work` carries the shared fuel/depth/call caps.
pub fn evaluate_hir_expr(
    expr: &HirExpr,
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
    structs: &ConstTypes,
) -> Result<ConstValue, ConstError> {
    tick_at(expr_span_hir(expr), work)?;
    match expr {
        HirExpr::Literal(lit, _, _) => literal_value(lit, expr_span_hir(expr)),
        HirExpr::Ident(path, span, _) => {
            if path.len() == 1 {
                if let Some((_, value)) = env.iter().rev().find(|(name, _)| name == &path[0]) {
                    return Ok(value.clone());
                }
            }
            Err(
                ConstError::new(*span, format!("`{}` is not a constant", path.join(".")))
                    .with_code(codes::CONST_NOT_CONSTANT),
            )
        }
        HirExpr::Move(inner, _, _) | HirExpr::Try { expr: inner, .. } => {
            evaluate_hir_expr(inner, env, consts, work, missing, structs)
        }
        HirExpr::Binary {
            op, lhs, rhs, span, ..
        } => {
            let left = evaluate_hir_expr(lhs, env, consts, work, missing, structs)?;
            let right = evaluate_hir_expr(rhs, env, consts, work, missing, structs)?;
            evaluate_binary(*op, &left, &right, *span)
        }
        HirExpr::Unary {
            op, operand, span, ..
        } => {
            let inner = evaluate_hir_expr(operand, env, consts, work, missing, structs)?;
            match (op, &inner) {
                (UnOp::Neg, ConstValue::I64(value)) => Ok(ConstValue::I64(value.wrapping_neg())),
                (UnOp::Neg, ConstValue::F64(value)) => Ok(ConstValue::F64(-value)),
                (UnOp::Not, ConstValue::Bool(value)) => Ok(ConstValue::Bool(!value)),
                (UnOp::BitNot, ConstValue::I64(value)) => Ok(ConstValue::I64(!value)),
                (op, other) => Err(ConstError::new(
                    *span,
                    format!("cannot apply `{op:?}` to `{}`", other.kind()),
                )),
            }
        }
        HirExpr::FixedArrayLit { elements, .. } => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(evaluate_hir_expr(
                    element, env, consts, work, missing, structs,
                )?);
            }
            Ok(ConstValue::Array(values))
        }
        HirExpr::TupleLit { elements, .. } => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(evaluate_hir_expr(
                    element, env, consts, work, missing, structs,
                )?);
            }
            Ok(ConstValue::Array(values))
        }
        HirExpr::Index {
            base, index, span, ..
        } => {
            let base_value = evaluate_hir_expr(base, env, consts, work, missing, structs)?;
            let index_value = evaluate_hir_expr(index, env, consts, work, missing, structs)?;
            let at = match index_value {
                ConstValue::I64(n) => n,
                other => {
                    return Err(ConstError::new(
                        *span,
                        format!("an index must be `int`, found `{}`", other.kind()),
                    ))
                }
            };
            match base_value {
                ConstValue::Array(values) => {
                    if at < 0 || at as usize >= values.len() {
                        return Err(ConstError::new(
                            *span,
                            format!(
                                "index {at} is outside a fixed array of length {}",
                                values.len()
                            ),
                        ));
                    }
                    Ok(values[at as usize].clone())
                }
                other => Err(ConstError::new(
                    *span,
                    format!("cannot index `{}`", other.kind()),
                )),
            }
        }
        HirExpr::StructLit { path, fields, .. } => {
            let mut values = Vec::with_capacity(fields.len());
            for (name, value, _) in fields {
                values.push((
                    name.clone(),
                    evaluate_hir_expr(value, env, consts, work, missing, structs)?,
                ));
            }
            Ok(ConstValue::Struct {
                name: path.last().cloned().unwrap_or_default(),
                fields: values,
            })
        }
        HirExpr::FieldAccess {
            base, field, span, ..
        } => {
            let value = evaluate_hir_expr(base, env, consts, work, missing, structs)?;
            if let ConstValue::Struct { fields, .. } = value {
                if let Some((_, value)) = fields.into_iter().find(|(name, _)| name == field) {
                    return Ok(value);
                }
            }
            Err(ConstError::new(
                *span,
                format!("no field `{field}` on the constant"),
            ))
        }
        HirExpr::Call {
            callee, args, span, ..
        } => {
            // Scalar conversions are lowered by the checker: `to_int` becomes a
            // Cast, `to_char` a call to `char_from_int`. Handle the latter.
            if callee.len() == 1 && callee[0] == "char_from_int" {
                let value = match args.first() {
                    Some(arg) => evaluate_hir_expr(arg, env, consts, work, missing, structs)?,
                    None => {
                        return Err(ConstError::new(*span, "`char_from_int` needs one argument"))
                    }
                };
                return match value {
                    ConstValue::I64(n) => u8::try_from(n).map(ConstValue::Char).map_err(|_| {
                        ConstError::new(*span, format!("`{n}` is not a character code"))
                    }),
                    other => Err(ConstError::new(
                        *span,
                        format!("cannot convert `{}` to char", other.kind()),
                    )),
                };
            }
            // An enum constructor is a two-segment call: `[Enum, Variant]`.
            if callee.len() == 2 {
                let mut payload = Vec::with_capacity(args.len());
                for arg in args {
                    payload.push(evaluate_hir_expr(arg, env, consts, work, missing, structs)?);
                }
                return Ok(ConstValue::Enum {
                    name: callee[0].clone(),
                    variant: callee[1].clone(),
                    payload,
                });
            }
            // A `const fn` call resolves from the table. Recurse into its typed
            // body when available, else fall back to the AST evaluator.
            if callee.len() == 1 {
                let name = &callee[0];
                if let Some(function) = consts.get(name) {
                    if work.depth >= MAX_CONST_DEPTH || work.calls >= MAX_CONST_CALLS {
                        return Err(ConstError::new(
                            *span,
                            format!("`{name}` did not terminate: the evaluation is too deep"),
                        )
                        .with_code(codes::CONST_RECURSION));
                    }
                    work.calls = work.calls.saturating_add(1);
                    work.called.push((name.clone(), *span));
                    let mut inner_env: Vec<(String, ConstValue)> = Vec::new();
                    for (index, (param, _)) in function.params.iter().enumerate() {
                        let value = match args.get(index) {
                            Some(arg) => {
                                evaluate_hir_expr(arg, env, consts, work, missing, structs)?
                            }
                            None => {
                                return Err(ConstError::new(
                                    *span,
                                    format!("missing argument `{param}`"),
                                ))
                            }
                        };
                        inner_env.push((param.clone(), value));
                    }
                    if let Some(body) = &function.hir_body {
                        work.depth = work.depth.saturating_add(1);
                        let outcome = evaluate_hir_block(
                            body,
                            &mut inner_env,
                            consts,
                            work,
                            missing,
                            structs,
                        );
                        work.depth = work.depth.saturating_sub(1);
                        return outcome.and_then(|flow| match flow {
                            HirFlow::Returned(value) => Ok(value),
                            HirFlow::Fell => Err(ConstError::new(
                                *span,
                                format!("`{name}` does not return a value on every path"),
                            )),
                            HirFlow::Break | HirFlow::Continue => {
                                Err(ConstError::new(*span, "a loop jump escaped a const fn"))
                            }
                        });
                    }
                    // Fallback: no typed body was built, use the AST evaluator.
                    let ast_args = args
                        .iter()
                        .map(|arg| arg_to_ast(arg, *span))
                        .collect::<Vec<_>>();
                    let ast_expr = Expr::Call {
                        callee: vec![name.clone()],
                        type_args: Vec::new(),
                        args: ast_args
                            .into_iter()
                            .map(|value| yashy_parser::ast::CallArg {
                                name: None,
                                value,
                                span: *span,
                            })
                            .collect(),
                        unsafe_call: false,
                        span: *span,
                    };
                    return evaluate(&ast_expr, &inner_env, consts, work, missing, structs);
                }
            }
            Err(ConstError::new(
                *span,
                format!("`{}` has no compile-time value", callee.join(".")),
            ))
        }
        HirExpr::Cast { expr, ty, span } => {
            let value = evaluate_hir_expr(expr, env, consts, work, missing, structs)?;
            cast_const(value, &ty.kind, *span)
        }
        HirExpr::SwitchExpr {
            subject,
            arms,
            span,
            ..
        } => {
            let value = evaluate_hir_expr(subject, env, consts, work, missing, structs)?;
            evaluate_hir_switch(&value, arms, env, consts, work, missing, structs, *span)
        }
        other => Err(ConstError::new(
            expr_span_hir(other),
            format!("{} is not allowed in a const fn", hir_expr_kind(other)),
        )),
    }
}

/// Evaluate a lowered `const fn` block, returning its flow.
fn evaluate_hir_block(
    block: &HirBlock,
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
    structs: &ConstTypes,
) -> Result<HirFlow, ConstError> {
    for stmt in &block.stmts {
        match evaluate_hir_stmt(stmt, env, consts, work, missing, structs)? {
            HirFlow::Fell => {}
            flow => return Ok(flow),
        }
    }
    Ok(HirFlow::Fell)
}

fn evaluate_hir_stmt(
    stmt: &HirStmt,
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
    structs: &ConstTypes,
) -> Result<HirFlow, ConstError> {
    match stmt {
        HirStmt::Expr(expr, _, _) => {
            let _ = evaluate_hir_expr(expr, env, consts, work, missing, structs)?;
            Ok(HirFlow::Fell)
        }
        HirStmt::Const { name, init, .. } | HirStmt::Var { name, init, .. } => {
            let value = evaluate_hir_expr(init, env, consts, work, missing, structs)?;
            env.push((name.clone(), value));
            Ok(HirFlow::Fell)
        }
        HirStmt::Assign { name, value, .. } => {
            let value = evaluate_hir_expr(value, env, consts, work, missing, structs)?;
            if let Some((_, slot)) = env.iter_mut().rev().find(|(n, _)| n == name) {
                *slot = value;
                Ok(HirFlow::Fell)
            } else {
                Err(ConstError::new(
                    hir_stmt_span(stmt),
                    format!("`{name}` is not a constant binding"),
                ))
            }
        }
        HirStmt::Return(Some(expr), _) => {
            let value = evaluate_hir_expr(expr, env, consts, work, missing, structs)?;
            Ok(HirFlow::Returned(value))
        }
        HirStmt::Return(None, _) => Ok(HirFlow::Returned(ConstValue::Bool(true))),
        HirStmt::Break(_) => Ok(HirFlow::Break),
        HirStmt::Continue(_) => Ok(HirFlow::Continue),
        HirStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            let value = evaluate_hir_expr(cond, env, consts, work, missing, structs)?;
            let take_then = matches!(value, ConstValue::Bool(true));
            if take_then {
                evaluate_hir_block(then_block, env, consts, work, missing, structs)
            } else if let Some(else_block) = else_block {
                evaluate_hir_block(else_block, env, consts, work, missing, structs)
            } else {
                Ok(HirFlow::Fell)
            }
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            span,
        } => {
            if let Some(init) = init {
                evaluate_hir_stmt(init, env, consts, work, missing, structs)?;
            }
            loop {
                tick_at(*span, work)?;
                if let Some(cond) = cond {
                    let value = evaluate_hir_expr(cond, env, consts, work, missing, structs)?;
                    if !matches!(value, ConstValue::Bool(true)) {
                        break;
                    }
                }
                match evaluate_hir_block(body, env, consts, work, missing, structs)? {
                    HirFlow::Fell => {}
                    HirFlow::Break => break,
                    HirFlow::Returned(value) => return Ok(HirFlow::Returned(value)),
                    HirFlow::Continue => {}
                }
                if let Some(update) = update {
                    let _ = evaluate_hir_expr(update, env, consts, work, missing, structs)?;
                }
            }
            Ok(HirFlow::Fell)
        }
        other => Err(ConstError::new(
            hir_stmt_span(other),
            format!("{} is not allowed in a const fn", hir_stmt_kind(other)),
        )),
    }
}

fn evaluate_hir_switch(
    value: &ConstValue,
    arms: &[HirSwitchArm],
    env: &mut Vec<(String, ConstValue)>,
    consts: &ConstFns,
    work: &mut Work,
    missing: Missing,
    structs: &ConstTypes,
    span: Span,
) -> Result<ConstValue, ConstError> {
    for arm in arms {
        // Binding names from a pattern must not leak out of a failed arm.
        let before = env.len();
        let matches = hir_pattern_matches(&arm.pattern, value, env)?;
        if matches {
            if let Some(guard) = &arm.guard {
                let ok = evaluate_hir_expr(guard, env, consts, work, missing, structs)?;
                if !matches!(ok, ConstValue::Bool(true)) {
                    env.truncate(before);
                    continue;
                }
            }
            let result = evaluate_hir_expr(&arm.body, env, consts, work, missing, structs);
            env.truncate(before);
            return result;
        }
        env.truncate(before);
    }
    let _ = (arm_default(arms), span);
    Err(ConstError::new(span, "no switch arm matched the constant"))
}

fn arm_default(arms: &[HirSwitchArm]) -> bool {
    arms.iter().any(|arm| arm.is_default)
}

fn hir_pattern_matches(
    pattern: &HirPattern,
    value: &ConstValue,
    env: &mut Vec<(String, ConstValue)>,
) -> Result<bool, ConstError> {
    match pattern {
        HirPattern::Wildcard(_) => Ok(true),
        HirPattern::Binding { name, .. } => {
            env.push((name.clone(), value.clone()));
            Ok(true)
        }
        HirPattern::BindAt { name, pattern, .. } => {
            if hir_pattern_matches(pattern, value, env)? {
                env.push((name.clone(), value.clone()));
                Ok(true)
            } else {
                Ok(false)
            }
        }
        HirPattern::Literal(lit, ..) => Ok(literal_value(lit, Span::new(0, 0))? == *value),
        HirPattern::Range {
            lo, hi, inclusive, ..
        } => {
            let n = match value {
                ConstValue::I64(n) => *n,
                _ => return Ok(false),
            };
            let as_i64 = |lit: &Literal| match lit {
                Literal::I64(v) => Some(*v),
                Literal::Char(c) => Some(i64::from(*c)),
                _ => None,
            };
            let (Some(lo), Some(hi)) = (as_i64(lo), as_i64(hi)) else {
                return Ok(false);
            };
            Ok(if *inclusive {
                lo <= n && n <= hi
            } else {
                lo <= n && n < hi
            })
        }
        HirPattern::UnitVariant { path, .. } => Ok(matches!(
            value,
            ConstValue::Enum { variant, .. } if variant == path.last().unwrap_or(&String::new())
        )),
        _ => Err(ConstError::new(
            Span::new(0, 0),
            "this pattern has no compile-time value",
        )),
    }
}

fn literal_value(lit: &Literal, _span: Span) -> Result<ConstValue, ConstError> {
    match lit {
        Literal::I64(v) => Ok(ConstValue::I64(*v)),
        Literal::F64(v) => Ok(ConstValue::F64(*v)),
        Literal::Bool(v) => Ok(ConstValue::Bool(*v)),
        Literal::Str(v) => Ok(ConstValue::Str(v.value.clone())),
        Literal::Char(v) => Ok(ConstValue::Char(*v)),
    }
}

fn cast_const(value: ConstValue, target: &TypeKind, span: Span) -> Result<ConstValue, ConstError> {
    match (value, target) {
        (ConstValue::Char(byte), TypeKind::I64) => Ok(ConstValue::I64(i64::from(byte))),
        (ConstValue::I64(n), TypeKind::Char) => u8::try_from(n)
            .map(ConstValue::Char)
            .map_err(|_| ConstError::new(span, format!("`{n}` is not a character code"))),
        (value, _) => Ok(value),
    }
}

/// Build a loose AST expression from a typed HIR literal tree, for the fallback
/// path where no typed const-fn body was built. Only the forms a const argument
/// can carry are reconstructed.
fn arg_to_ast(expr: &HirExpr, span: Span) -> Expr {
    match expr {
        HirExpr::Literal(lit, _, _) => Expr::Literal(lit.clone(), span),
        HirExpr::Ident(path, span, _) => Expr::Ident(path.clone(), *span),
        _ => Expr::Literal(Literal::I64(0), span),
    }
}

fn expr_span_hir(expr: &HirExpr) -> Span {
    match expr {
        HirExpr::Literal(_, span, _) => *span,
        HirExpr::Ident(_, span, _) => *span,
        HirExpr::UnsafeBlock { span, .. } => *span,
        HirExpr::Binary { span, .. } | HirExpr::Unary { span, .. } => *span,
        HirExpr::Move(_, span, _) | HirExpr::AddrOf { span, .. } => *span,
        HirExpr::Call { span, .. } | HirExpr::StructLit { span, .. } => *span,
        HirExpr::SliceLit { span, .. } | HirExpr::FieldAccess { span, .. } => *span,
        HirExpr::Index { span, .. } | HirExpr::FixedArrayLit { span, .. } => *span,
        HirExpr::View { span, .. } | HirExpr::TupleLit { span, .. } => *span,
        HirExpr::Cast { span, .. } | HirExpr::Try { span, .. } => *span,
        HirExpr::Await { span, .. } | HirExpr::SwitchExpr { span, .. } => *span,
        _ => Span::new(0, 0),
    }
}

fn hir_stmt_span(stmt: &HirStmt) -> Span {
    crate::hir::stmt_span(stmt)
}

fn hir_expr_kind(expr: &HirExpr) -> &'static str {
    match expr {
        HirExpr::Literal(..) => "a literal",
        HirExpr::Ident(..) => "a name",
        HirExpr::Binary { .. } => "a binary operation",
        HirExpr::Unary { .. } => "a unary operation",
        HirExpr::Call { .. } => "a call",
        HirExpr::SwitchExpr { .. } => "a switch",
        _ => "this expression",
    }
}

fn hir_stmt_kind(stmt: &HirStmt) -> &'static str {
    match stmt {
        HirStmt::Return(..) => "a return",
        HirStmt::Var { .. } => "a let binding",
        HirStmt::Const { .. } => "a const binding",
        HirStmt::Assign { .. } => "an assignment",
        HirStmt::If { .. } => "an if",
        HirStmt::ForC { .. } => "a loop",
        _ => "this statement",
    }
}
