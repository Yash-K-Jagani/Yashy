//! Types used by the type checker.
//!
//! A `Type` carries a `TypeKind` plus the span where it was written or
//! inferred. Two `Type`s are equal when their kinds are equal, regardless of
//! span: a type error points at the span of the expression that went wrong,
//! not at the span where a matching type was first declared.

use otter_common::Span;

/// The shape of a value. Every variant except `Never` is a real type a
/// program can contain; `Never` is the type of expressions that never
/// evaluate (diverging calls, unreachable code) and is never written by the
/// user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    I64,
    F64,
    Bool,
    Str,
    Unit,
    /// A declared struct, named by its declaration name.
    Struct(String),
    /// A declared enum, named by its declaration name.
    Enum(String),
    Option(Box<Type>),
    Result(Box<Type>, Box<Type>),
    /// A function value type. First-class functions do not exist in M0-M2;
    /// this is the type of builtin and user function signatures as they are
    /// tracked during checking.
    Func(Vec<Type>, Box<Type>),
    /// No value: the expression diverges. Not written by users.
    Never,
}

/// A type with the span where it appears. Equality ignores spans so that two
/// types written in different places still compare equal; the span is used
/// only to point diagnostics at the right source text.
#[derive(Clone, Debug)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for Type {}

impl Type {
    pub fn eq_ignoring_span(&self, other: &Type) -> bool {
        self.kind == other.kind
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            TypeKind::I64 => write!(f, "i64"),
            TypeKind::F64 => write!(f, "f64"),
            TypeKind::Bool => write!(f, "bool"),
            TypeKind::Str => write!(f, "string"),
            TypeKind::Unit => write!(f, "()"),
            TypeKind::Struct(name) => write!(f, "{name}"),
            TypeKind::Enum(name) => write!(f, "{name}"),
            TypeKind::Option(inner) => write!(f, "Option[{inner}]"),
            TypeKind::Result(ok, err) => write!(f, "Result[{ok}, {err}]"),
            TypeKind::Func(params, ret) => {
                write!(f, "(")?;
                let mut first = true;
                for p in params {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeKind::Never => write!(f, "!"),
        }
    }
}

/// Mutability of a binding. `Const` is the default; `Var` is explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mutability {
    Const,
    Var,
}

impl Mutability {
    pub fn is_mutable(self) -> bool {
        matches!(self, Mutability::Var)
    }
}

/// A type that never changes kind under unification; used internally when the
/// checker needs to say "this expression is known to diverge".
pub fn never(span: Span) -> Type {
    Type {
        kind: TypeKind::Never,
        span,
    }
}
