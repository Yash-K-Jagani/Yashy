//! The typed high-level IR.
//!
//! This tree mirrors the AST in shape but is a **separate type tree with
//! distinct names** so that code can never confuse a `Stmt` with a
//! [`HirStmt`]. Every expression node carries the [`Type`] the checker
//! inferred for it, and every binding carries its [`Mutability`]. Patterns
//! additionally carry the type and mutability of what a `Binding` arm binds.
//!
//! No untyped AST node appears here: the only way to build a `HirExpr` is
//! to infer it, and the only way to build a `HirStmt` is to check one.

use otter_common::Span;
use otter_parser::ast::Literal;

use crate::ty::{Mutability, Type};

/// A whole program after type checking.
#[derive(Clone, Debug, PartialEq)]
pub struct HirProgram {
    pub items: Vec<HirItem>,
    /// Whether the program contains a `thread_spawn` call.
    ///
    /// The backend reads this to choose the atomic string refcount pair. It is
    /// one bit for the whole program rather than a per-function answer: once a
    /// thread exists, any string can cross into it, and proving which ones
    /// cannot is interprocedural analysis this milestone does not attempt.
    pub uses_threads: bool,
}

/// A top-level item after type checking.
#[derive(Clone, Debug, PartialEq)]
pub enum HirItem {
    Struct(HirStruct),
    Enum(HirEnum),
    Fn(HirFn),
    Import(HirImport),
    Extern(HirExternBlock),
}

/// A `struct` item: its name and its named fields, in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct HirStruct {
    pub name: String,
    pub fields: Vec<HirFieldDef>,
    pub span: Span,
}

/// One field of a struct.
#[derive(Clone, Debug, PartialEq)]
pub struct HirFieldDef {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// An `enum` item: its name and its variants, in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct HirEnum {
    pub name: String,
    pub variants: Vec<HirVariantDef>,
    pub span: Span,
}

/// One variant of an enum, in either of the two payload shapes the AST
/// already distinguishes: `Circle(f64)` and `Rect { w: f64 }`.
#[derive(Clone, Debug, PartialEq)]
pub struct HirVariantDef {
    pub name: String,
    pub fields: Vec<HirVariantField>,
    pub span: Span,
}

/// One payload field of an enum variant, positional or named.
#[derive(Clone, Debug, PartialEq)]
pub struct HirVariantField {
    pub name: Option<String>,
    pub ty: Type,
    pub span: Span,
}

impl HirVariantField {
    /// The field's name when it has one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// A function item: its signature and its checked body.
#[derive(Clone, Debug, PartialEq)]
pub struct HirFn {
    pub name: String,
    pub params: Vec<HirParam>,
    pub ret: Type,
    pub body: HirBlock,
    pub span: Span,
}

/// A function parameter: always named, always typed.
#[derive(Clone, Debug, PartialEq)]
pub struct HirParam {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// An import: the dotted path, unchanged from the AST.
#[derive(Clone, Debug, PartialEq)]
pub struct HirImport {
    pub path: Vec<String>,
    pub span: Span,
}

/// An `extern` block after type checking.
///
/// `abi` and `link` are carried through unchanged: the ABI says how to call the
/// declarations and the library says what the driver must link against. Neither
/// is interpreted by the checker.
#[derive(Clone, Debug, PartialEq)]
pub struct HirExternBlock {
    pub abi: Option<String>,
    pub link: Option<String>,
    pub decls: Vec<HirExternDecl>,
    pub span: Span,
}

/// One C function declaration: a name, checked parameter types, a return type.
///
/// There is no body and no `unsafe` flag: the parser already required the
/// marker, so a declaration that reaches the HIR is a declaration the user
/// accepted the FFI risk for.
#[derive(Clone, Debug, PartialEq)]
pub struct HirExternDecl {
    pub name: String,
    pub params: Vec<HirParam>,
    pub ret: Type,
    pub span: Span,
}

/// A block of statements: its span includes the braces.
#[derive(Clone, Debug, PartialEq)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub span: Span,
}

/// One `.field` of a [`HirStmt::FieldAssign`] path: where the field sits in the
/// aggregate it belongs to, and what type it is.
///
/// The type is what lets the backend rebuild the enclosing aggregates on the
/// way out of a nested write (`p.inner.x = 1`): the `insertvalue` at each level
/// names the aggregate it produces, and the level below names this field's type
/// as the aggregate it started from. Both come from here rather than from a
/// second lookup of the struct declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldStep {
    /// Position in the owning aggregate's fields, in declaration order.
    pub index: usize,
    /// The field's type.
    pub ty: Type,
}

/// A statement after type checking.
///
/// Expression statements carry the inferred type; bindings carry mutability.
/// Everything else has the same shape as its AST counterpart.
#[derive(Clone, Debug, PartialEq)]
pub enum HirStmt {
    Expr(HirExpr, Span, Type),
    Const {
        name: String,
        ty: Option<Type>,
        init: HirExpr,
        mutability: Mutability,
        span: Span,
    },
    Var {
        name: String,
        ty: Option<Type>,
        init: HirExpr,
        mutability: Mutability,
        span: Span,
    },
    /// `x = value;`.
    ///
    /// The target is gone by now: the checker proved it named a `var` binding,
    /// so what is left is the name to store through and the type of the slot.
    /// Carrying the name rather than the `HirExpr` is deliberate -- it makes it
    /// unrepresentable in HIR to assign to something the checker did not
    /// approve, so code generation has no case to write for a bad target. When
    /// fields and subscripts become assignable, the target reappears here as a
    /// shape the checker has already validated.
    Assign {
        name: String,
        value: HirExpr,
        /// The binding's declared type, so code generation stores the right
        /// width without looking the name up again.
        ty: Type,
        /// True when the binding held a live linear value at this point, so the
        /// store overwrites an owning pointer and the backend must release it
        /// first. The checker is the only thing that knows this: after
        /// `dealloc(buf)` the binding still names a value in the slot, but the
        /// pointer was already freed, so `buf = alloc(32)` must not free it
        /// again. Computed from the binding's ownership *before* the rebind.
        release_previous: bool,
        span: Span,
    },
    /// `p[i] = value;`
    ///
    /// Like `Assign`, the target is validated by the checker before it
    /// reaches here: `p` must be a mutable `*mut i64` binding and `i` an
    /// `i64`. What remains is the three pieces of the store.
    IndexAssign {
        base: HirExpr,
        index: HirExpr,
        value: HirExpr,
        span: Span,
    },
    /// `p.x = value;`, and `p.inner.x = value;` for a nested field.
    ///
    /// A struct value is an aggregate, not an address, so this is a
    /// read-modify-write: the root binding's aggregate is loaded, the field is
    /// replaced with `insertvalue` (rebuilding the enclosing aggregates on the
    /// way out of a nested chain), and the result is stored back. The checker
    /// has already resolved each `.field` to its position and type, which is
    /// what [`FieldStep`] carries; `root` is the `var` binding at the bottom of
    /// the chain, and it is the only thing the backend has to look up.
    FieldAssign {
        root: String,
        /// The chain of fields from `root`, outermost first. Never empty.
        steps: Vec<FieldStep>,
        value: HirExpr,
        ty: Type,
        span: Span,
    },
    Return(Option<HirExpr>, Span),
    If {
        init: Option<Box<HirStmt>>,
        cond: HirExpr,
        then_block: HirBlock,
        else_block: Option<HirBlock>,
        span: Span,
    },
    ForRange {
        name: String,
        start: HirExpr,
        end: HirExpr,
        body: HirBlock,
        span: Span,
    },
    /// `for x in slice { .. }`: the loop variable takes each element in order.
    ///
    /// The backend keeps the iterable's header slot and re-reads length and data
    /// every iteration, so a `push` in the body is visible to the next test. The
    /// checker made sure the binding is not moved while the loop can still read
    /// it.
    ForIn {
        name: String,
        iterable: HirExpr,
        body: HirBlock,
        span: Span,
    },
    ForC {
        init: Option<Box<HirStmt>>,
        cond: Option<HirExpr>,
        update: Option<HirExpr>,
        body: HirBlock,
        span: Span,
    },
    Switch {
        subject: HirExpr,
        arms: Vec<HirSwitchArm>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Defer {
        body: HirBlock,
        span: Span,
    },
}

/// One arm of a `switch`: the pattern, its body, and whether it is `default`.
#[derive(Clone, Debug, PartialEq)]
pub struct HirSwitchArm {
    pub pattern: HirPattern,
    pub body: HirExpr,
    pub is_default: bool,
    pub span: Span,
}

/// A pattern after type checking.
///
/// `Binding` additionally carries the type and mutability of what the name
/// binds, because that is the only place the type checker can record it.
#[derive(Clone, Debug, PartialEq)]
pub enum HirPattern {
    Wildcard(Span),
    UnitVariant {
        path: Vec<String>,
        span: Span,
    },
    TupleVariant {
        path: Vec<String>,
        args: Vec<HirPattern>,
        span: Span,
    },
    StructVariant {
        path: Vec<String>,
        fields: Vec<(String, HirPattern, Span)>,
        span: Span,
    },
    Binding {
        name: String,
        ty: Type,
        mutability: Mutability,
        span: Span,
    },
    Literal(Literal, Span),
}

/// Where a runtime fault inside an expression reports itself from.
///
/// A resolved *position*, plus the file it indexes. This is the one thing the
/// backend cannot work out for itself: it is handed IR text and a path, and no
/// source map at all, so a span means nothing to it. The checker is the last
/// stage that holds the map, so it resolves the span once, here, and the backend
/// prints the numbers as constants.
///
/// [`HirExpr::Binary`] and [`HirExpr::Unary`] carry one when the operation can
/// fault -- integer arithmetic, which is checked -- and `None` when it cannot:
/// a comparison, a boolean `and`, a float operation, a string concatenation.
/// Which operations those are is the *language's* decision, made in the checker
/// (`Checker::fault_site`), not a detail the backend is free to reinterpret.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultSite {
    /// The path the compiler was handed, exactly as it was given.
    pub file: String,
    /// One-based line, counting characters.
    pub line: u32,
    /// One-based column, counting characters.
    pub column: u32,
}

/// An expression after type checking: the AST shape plus its inferred type.
#[derive(Clone, Debug, PartialEq)]
pub enum HirExpr {
    Literal(Literal, Span, Type),
    Ident(Vec<String>, Span, Type),
    Binary {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
        span: Span,
        ty: Type,
        /// `Some` when this operation has a runtime fault to report, with the
        /// position to report it at. See [`FaultSite`].
        fault: Option<FaultSite>,
    },
    Unary {
        op: UnOp,
        operand: Box<HirExpr>,
        span: Span,
        ty: Type,
        /// `Some` when this operation has a runtime fault to report -- the
        /// negation of the smallest `i64`. See [`FaultSite`].
        fault: Option<FaultSite>,
    },
    Move(Box<HirExpr>, Span, Type),
    Call {
        callee: Vec<String>,
        args: Vec<HirExpr>,
        unsafe_call: bool,
        span: Span,
        ty: Type,
    },
    StructLit {
        path: Vec<String>,
        fields: Vec<(String, HirExpr, Span)>,
        span: Span,
        ty: Type,
    },
    /// A slice literal, already checked against its written element type.
    ///
    /// `elem_ty` is the element type the source named, not inferred from the
    /// elements, so the backend can size the backing array and store each
    /// element without asking what an empty literal means.
    SliceLit {
        elem_ty: Type,
        elements: Vec<HirExpr>,
        span: Span,
        ty: Type,
    },
    FieldAccess {
        base: Box<HirExpr>,
        field: String,
        /// The field's position in the struct's **declaration** order, which is
        /// the order the IR aggregate is laid out in.
        ///
        /// Resolved here rather than in the backend so the name-versus-index
        /// resolution happens once, against the declaration table that only the
        /// checker has, and the IR layout cannot depend on how a particular
        /// struct literal happened to spell its fields.
        index: usize,
        span: Span,
        ty: Type,
    },
    Index {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
        span: Span,
        ty: Type,
    },
    Cast {
        expr: Box<HirExpr>,
        ty: Type,
        span: Span,
    },
    Try {
        expr: Box<HirExpr>,
        span: Span,
        ty: Type,
    },
    SwitchExpr {
        subject: Box<HirExpr>,
        arms: Vec<HirSwitchArm>,
        span: Span,
        ty: Type,
    },
}

/// Re-exported so the HIR can reference the AST's operator enums without
/// repeating the parser crate's paths everywhere.
pub use otter_parser::ast::{BinOp, UnOp};

/// The span of a typed expression.
pub fn expr_span(expr: &HirExpr) -> Span {
    match expr {
        HirExpr::Literal(_, span, _) => *span,
        HirExpr::Ident(_, span, _) => *span,
        HirExpr::Binary { span, .. } => *span,
        HirExpr::Unary { span, .. } => *span,
        HirExpr::Move(_, span, _) => *span,
        HirExpr::Call { span, .. } => *span,
        HirExpr::StructLit { span, .. } => *span,
        HirExpr::SliceLit { span, .. } => *span,
        HirExpr::FieldAccess { span, .. } => *span,
        HirExpr::Index { span, .. } => *span,
        HirExpr::Cast { span, .. } => *span,
        HirExpr::Try { span, .. } => *span,
        HirExpr::SwitchExpr { span, .. } => *span,
    }
}

/// The type of a typed expression.
pub fn expr_ty(expr: &HirExpr) -> Type {
    match expr {
        HirExpr::Literal(_, _, ty) => ty.clone(),
        HirExpr::Ident(_, _, ty) => ty.clone(),
        HirExpr::Binary { ty, .. } => ty.clone(),
        HirExpr::Unary { ty, .. } => ty.clone(),
        HirExpr::Move(_, _, ty) => ty.clone(),
        HirExpr::Call { ty, .. } => ty.clone(),
        HirExpr::StructLit { ty, .. } => ty.clone(),
        HirExpr::SliceLit { ty, .. } => ty.clone(),
        HirExpr::FieldAccess { ty, .. } => ty.clone(),
        HirExpr::Index { ty, .. } => ty.clone(),
        HirExpr::Cast { ty, .. } => ty.clone(),
        HirExpr::Try { ty, .. } => ty.clone(),
        HirExpr::SwitchExpr { ty, .. } => ty.clone(),
    }
}
