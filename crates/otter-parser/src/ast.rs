//! The Otter abstract syntax tree: what a parsed source file *is*.
//!
//! # Untyped on purpose
//!
//! Nothing here knows a type. A `var` node has an annotation slot because the
//! source had one, not because the compiler decided it; an expression is a
//! literal because the source said so, and says nothing about what type that
//! literal turned out to be. Type checking lives downstream in `otter-sema`,
//! which builds a *separate* tree with its own names (`HirProgram`, `HirExpr`,
//! `HirStmt`, …) precisely so that the two can never be confused for one
//! another. A type field added here would be a type the parser guessed at, and
//! a guess that a downstream stage then trusts is worse than no guess at all.
//!
//! # Every node carries a span
//!
//! Each variant holds the [`Span`] of the source text it came from, and the
//! span is part of its value: two structurally identical nodes from different
//! places in a file are different nodes, because they point at different
//! places. Every diagnostic the rest of the compiler will ever print is aimed
//! at one of these spans, so a node missing one is a diagnostic that cannot be
//! placed. [`Literal`] is the single exception, and only because every use of
//! one sits inside a variant that has a span of its own.
//!
//! # `Debug` is the interface
//!
//! Every node derives [`Debug`], and that output is load-bearing rather than
//! incidental: golden tests dump whole trees of these with `{:?}` and compare
//! the text. The derives are what make a whole-AST dump possible at all, and
//! `tests/parse.rs` pins every variant name and every field name in that output,
//! so a rename cannot pass unnoticed through the stages that code against it.
//!
//! ```
//! use otter_common::Span;
//! use otter_parser::ast::{BinOp, Expr, Literal, Program, Stmt};
//!
//! // An `if` statement's condition, built by hand.
//! let cond = Expr::Binary {
//!     op: BinOp::Gt,
//!     lhs: Box::new(Expr::Literal(Literal::I64(1), Span::new(4, 5))),
//!     rhs: Box::new(Expr::Literal(Literal::I64(2), Span::new(8, 9))),
//!     span: Span::new(4, 9),
//! };
//! let program = Program { items: Vec::new() };
//! let _ = (cond, program, Stmt::Break(Span::new(0, 0)));
//! ```
//!
//! The example is deliberately trivial -- building a tree by hand is what
//! Tasks 4 to 6 do with the parser's output -- but it is checked, so the
//! shapes documented here cannot drift from the shapes that exist.

use otter_common::Span;

pub use otter_lexer::StringLit;

/// A whole parsed source file.
///
/// The one type here with no span: it is the file, and a diagnostic about the
/// file as a whole is a diagnostic about the `Eof` token or about a node
/// inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

/// A top-level declaration.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// `import std.io;` -- a dotted path, one [`String`] per segment.
    Import { path: Vec<String>, span: Span },
    /// `struct P { x: f64 }`
    Struct {
        name: String,
        fields: Vec<FieldDef>,
        span: Span,
    },
    /// `enum C { Red, Green }`
    Enum {
        name: String,
        variants: Vec<VariantDef>,
        span: Span,
    },
    /// `fn d(a: P) -> f64 { ... }`
    ///
    /// `ret` is `None` when the source wrote no `->`; that is a missing
    /// annotation, which is different from one that says `-> ()`.
    Fn {
        name: String,
        params: Vec<Param>,
        ret: Option<TypeExpr>,
        body: Block,
        span: Span,
    },
    /// `extern "C" link "m" { unsafe fn puts(s: string) -> i64; }` and friends.
    ///
    /// Two names, two jobs, two fields:
    ///
    /// - `abi` is the **ABI** -- `Some("C")` for `extern "C"`. It says how to
    ///   call the declarations, not what to link against.
    /// - `link` is the **library to link** -- `Some("m")` for
    ///   `link "m"`. It says what the linker must resolve the names against.
    ///
    /// Each is `None` when the source does not write it, and neither is ever
    /// the empty string: a block that names no ABI is not a block whose ABI is
    /// `""`. Conflating them into one field would lose the library entirely,
    /// because a block may name an ABI and no library, a library and no ABI, or
    /// both. Each declaration under the block is an [`ExternDecl`].
    ///
    /// This variant is read from M5 on. `abi`, `link` and `decls` are whatever
    /// the source wrote; the checker is what rejects a declaration that is not
    /// `unsafe` and resolves the C types.
    Extern {
        abi: Option<String>,
        link: Option<String>,
        decls: Vec<ExternDecl>,
        span: Span,
    },
}

/// One field of a `struct`. Always named: a struct's fields are named in the
/// source, and there is no positional form to confuse them with.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

/// One field of an `enum` variant that has a payload.
///
/// **The two shapes are different variants, not one shape with an optional
/// name.** `Circle(f64)` and `Rect { w: f64 }` are written differently, mean
/// different things, and are matched differently: `Pattern::TupleVariant`
/// against `Pattern::StructVariant`. A single `FieldDef` with an empty `String`
/// standing for "positional" would make that distinction a *convention* -- one
/// `name.is_empty()` check in the type checker and one missed check is a
/// silently wrong program rather than a compile error. Two variants cannot be
/// confused, because reading the wrong one is a type error in Rust itself.
///
/// Both carry a span covering the whole field -- the type for a positional one,
/// `name: type` for a named one -- so a diagnostic about one field of a variant
/// does not point at its neighbour.
#[derive(Clone, Debug, PartialEq)]
pub enum VariantField {
    /// `Circle(f64)` -- a payload with no name, matched by position.
    Positional { ty: TypeExpr, span: Span },
    /// `Rect { w: f64, h: f64 }` -- a named payload, matched by name.
    Named {
        name: String,
        ty: TypeExpr,
        span: Span,
    },
}

impl VariantField {
    /// The type this field carries, whichever shape it is.
    ///
    /// For a `Named` field this is the field's declared type, which is what a
    /// binding in a `Pattern::StructVariant` arm is checked against.
    pub fn ty(&self) -> &TypeExpr {
        match self {
            VariantField::Positional { ty, .. } | VariantField::Named { ty, .. } => ty,
        }
    }

    /// The span of the field as it was written.
    pub fn span(&self) -> Span {
        match self {
            VariantField::Positional { span, .. } | VariantField::Named { span, .. } => *span,
        }
    }
}

/// One variant of an `enum`. An empty `fields` is a variant with no payload.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantDef {
    pub name: String,
    /// The variant's payload, in source order. Empty for a unit variant such
    /// as `Red`.
    pub fields: Vec<VariantField>,
    pub span: Span,
}

/// One parameter of a `fn`. A parameter is always named and always typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

/// One function declared inside an [`Item::Extern`] block.
///
/// An `extern` declaration has no body: the implementation is elsewhere, which
/// is the whole point of the block. It is otherwise a [`Param`] list and a
/// return type, exactly like the head of an [`Item::Fn`].
///
/// **There is deliberately no `unsafe` flag here.** Spec 8.1 makes `unsafe`
/// mandatory on every `extern` declaration, so a declaration that reached this
/// type was already checked and there is nothing left to store: a flag that is
/// always `true` is not information, and one that defaults to `false` is a
/// lie. M5 must not add one. The marker that *is* stored is the other one: an
/// [`Expr::Call`] carries `unsafe_call`, because at the call site the same
/// keyword is optional and its absence means something.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub span: Span,
}

/// The mutability a pointer spelling asks for.
///
/// `Bare` is `*T`, the C spelling that does not say. It is kept distinct from
/// `Mut` rather than folded into it so the parser records what was written;
/// the checker is what decides that both mean the same machine pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtrKind {
    /// `*T`
    Bare,
    /// `*mut T`
    Mut,
    /// `*const T`
    Const,
}

/// A type as the source wrote it.
///
/// These six forms are the whole type grammar. It gained `Ptr` with the memory
/// model in M3 and the C pointer spellings in M5; slices and maps arrive in
/// M6, and adding a variant is additive for the parser and a recompile for
/// everyone else.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    /// A named type, primitive or declared: `i64`, `f64`, `P`.
    Name(String, Span),
    /// A pointer: `*T`, `*mut T`, or `*const T`.
    ///
    /// Outside an `extern` declaration only `*mut T` is meaningful — it is
    /// Otter's owning pointer — and the checker rejects the other two. Inside
    /// one, all three are C pointers.
    Ptr {
        kind: PtrKind,
        pointee: Box<TypeExpr>,
        span: Span,
    },
    /// `[]T` — a growable, linearly owned sequence of `T`.
    ///
    /// The element type is written, never inferred: `[]T { ... }` names `T`
    /// before any element is seen, so an empty literal has a type and the
    /// checker never has to guess.
    Slice(Box<TypeExpr>, Span),
    /// `map[K]V` — a linearly owned table from `K` to `V`.
    ///
    /// Not a generic spelling: `map` is recognised only when it is directly
    /// followed by `[`, so an ordinary type named `map` still resolves by name.
    Map(Box<TypeExpr>, Box<TypeExpr>, Span),
    /// `Option[T]`
    Option(Box<TypeExpr>, Span),
    /// `Result[T, E]`
    Result(Box<TypeExpr>, Box<TypeExpr>, Span),
    /// `Channel[T]` - a bounded queue of `T`, handled by the runtime.
    ///
    /// The element type is written, never inferred: `chan_new` has nothing in
    /// its arguments to infer from, so the binding's annotation is what fixes
    /// the channel, exactly as `map[K]V` does for `map_new`.
    Channel(Box<TypeExpr>, Span),
    /// A parenthesised list of types, as in `(i64, f64)`.
    ///
    /// **Never empty.** `()` is [`TypeExpr::Unit`], always, and a parser that
    /// produced an empty `Tuple` would be describing the same type two ways:
    /// `PartialEq` would then call two identical types different, and every
    /// AST golden dump would depend on which spelling the parser happened to
    /// choose.
    Tuple(Vec<TypeExpr>, Span),
    /// The unit type, written `()`.
    Unit(Span),
}

/// A braced sequence of statements, braces included in its span.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

/// A statement: the things that can appear in a block in statement position.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// An expression evaluated for its effect, such as a call.
    Expr(Expr, Span),
    /// `const c = 1;` or `const c: i64 = 1;`.
    Const {
        name: String,
        ty: Option<TypeExpr>,
        init: Expr,
        span: Span,
    },
    /// `var v = 1;` or `var v: f64 = 1.0;`.
    Var {
        name: String,
        ty: Option<TypeExpr>,
        init: Expr,
        span: Span,
    },
    /// `x = expr;` -- assigning to a binding that already exists.
    ///
    /// Only a `var` binding in scope may be assigned; assigning a `const` is a
    /// diagnostic, and so is assigning a name that is not declared. Those are
    /// the *checker's* rules, which is why the target is an `Expr` and not a
    /// name: the parser accepts any expression the expression grammar can
    /// already produce and leaves to the checker the question of whether that
    /// expression names something assignable.
    ///
    /// That choice is what keeps this statement from having to be invented
    /// again. The day Otter has struct fields and subscripts to assign to, `s.x
    /// = 1` and `a[i] = 1` are new shapes of *target* -- one match arm each in
    /// the checker and a new way to compute an address in code generation --
    /// rather than new statements threaded through both.
    ///
    /// Assignment is a statement, not an expression, so it has no value and
    /// cannot appear as a subexpression. `x = y = 1` is not in the language,
    /// and neither is `f(x = 1)`.
    Assign {
        /// The expression being assigned to. Only certain shapes are
        /// assignable, and the checker is what says which.
        target: Expr,
        /// What is stored.
        value: Expr,
        span: Span,
    },
    /// `return;` or `return expr;`.
    Return(Option<Expr>, Span),
    /// `if cond { .. } else { .. }`, or with a condition binding:
    /// `if d := f(); d > 5.0 { .. }`.
    ///
    /// The binding is `init`, a node in its own right, and `cond` is then the
    /// ordinary condition that reads it: an [`Expr::Ident`] naming the bound
    /// variable on the left of a comparison. The binding is not folded into
    /// the condition's operand, because the type checker and code generation
    /// both have to find it, and a tree that hides it makes each of them
    /// rediscover the same rule.
    ///
    /// `init` is `None` for an `if` that binds nothing. Whatever it is, the
    /// binding is scoped to this statement's own blocks -- it is not visible
    /// after the `if`.
    If {
        init: Option<Box<Stmt>>,
        cond: Expr,
        then_block: Block,
        else_block: Option<Block>,
        span: Span,
    },
    /// `for i in 0..10 { .. }`
    ///
    /// Both bounds are expressions; the `..` itself is not a node, because
    /// nothing else in the language can produce one.
    ///
    /// **`end` is exclusive**: `for i in 0..10` runs with `i` taking 0 through 9,
    /// ten times. The rule is the half-open reading of `..` used everywhere
    /// else in the language, and the alternative -- inclusive -- would make
    /// `start..=end` and `start..end` differ by a character that is not in the
    /// grammar at all. Code generation (Task 12) lowers this by comparing
    /// against `end` before the body, not after.
    ForRange {
        name: String,
        start: Expr,
        end: Expr,
        body: Block,
        span: Span,
    },
    /// `for x in slice { .. }` -- every element of a slice, in order.
    ///
    /// The same head as [`Stmt::ForRange`] without a `..` after the first
    /// expression, so the parser decides between the two by one token. The
    /// iterable must stay a *binding*: the loop reads the header each
    /// iteration, which is what makes a `push` inside the body visible, and a
    /// temporary has no stable header to read. The checker enforces that and
    /// rejects moving the slice while the loop can still read it.
    ForIn {
        name: String,
        iterable: Expr,
        body: Block,
        span: Span,
    },
    /// `for i := 0; i < 10; step(i) { .. }`
    ///
    /// Each of the three parts is optional -- `for ;; { .. }` is legal -- so
    /// each is an `Option`, and `init` is boxed because a statement is already
    /// a large enum.
    ///
    /// `update` is an **expression**, so it cannot be an assignment: `i = i + 1`
    /// in that slot does not parse. A C-style loop therefore cannot advance a
    /// counter, and the accumulating loop this language has is
    /// `for i in 0..n { total = total + i; }` -- a [`Stmt::Assign`] in the body.
    /// Two things would make the C form useful and neither is in the grammar:
    /// widening `update` to a statement, and letting `:=` in a loop head bind a
    /// `var` (today it is a [`Stmt::Const`], so the binding it would assign to
    /// is immutable even once `update` could assign).
    ForC {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        update: Option<Expr>,
        body: Block,
        span: Span,
    },
    /// `switch subject { case .. { .. } default { .. } }`
    Switch {
        subject: Expr,
        arms: Vec<SwitchArm>,
        span: Span,
    },
    /// `break;` -- leaves the innermost loop or `switch`.
    Break(Span),
    /// `continue;`
    Continue(Span),
    /// `defer { .. }` -- always a braced block, never an expression.
    Defer { body: Block, span: Span },
}

/// One arm of a `switch`, in either statement or expression position.
///
/// `is_default` says whether the arm was written `default` rather than
/// `case`, which is a fact about the source: both spellings may match, and
/// which one the user typed is what a diagnostic about an unreachable arm
/// wants to point at.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub is_default: bool,
    pub span: Span,
}

/// What a `case` matches against.
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    /// `_` -- matches anything.
    Wildcard(Span),
    /// A variant with no payload: `Red`.
    UnitVariant { path: Vec<String>, span: Span },
    /// A variant with a positional payload: `RGB(1, 2, 3)`.
    TupleVariant {
        path: Vec<String>,
        args: Vec<Pattern>,
        span: Span,
    },
    /// A variant with a named payload: `RGB { red: r, .. }`, one triple of
    /// field name, sub-pattern and the span of that field's name.
    StructVariant {
        path: Vec<String>,
        fields: Vec<(String, Pattern, Span)>,
        span: Span,
    },
    /// A name that binds the whole matched value.
    Binding { name: String, span: Span },
    /// A literal to compare against.
    Literal(Literal, Span),
}

/// A literal, carrying its value rather than the text it was written as.
///
/// A `Str` holds a [`StringLit`], which is what the lexer decoded it into, so
/// escapes are resolved once and in one place.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(StringLit),
}

/// An expression: the things that can appear where a value is expected.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(Literal, Span),
    /// A possibly dotted name: `x`, `std.io.puts`.
    Ident(Vec<String>, Span),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
        span: Span,
    },
    /// `move expr` -- a linear-ownership transfer, not a value transformation.
    ///
    /// M3's linearity checker and codegen treat this as transferring ownership
    /// rather than applying an operator, so it is a distinct variant rather than
    /// an [`UnOp`].
    Move(Box<Expr>, Span),
    /// `f(1, 2)` or `unsafe f(1, 2)` -- a call by path.
    ///
    /// `unsafe_call` is true when the source wrote `unsafe` at the call site.
    /// Spec 8.1 requires that marker there, in addition to the `unsafe` that
    /// every [`ExternDecl`] carries, because handing a raw pointer across the C
    /// boundary is the dangerous act and it is the *call* that does it, not the
    /// declaration.
    ///
    /// It is a `bool` on the call rather than a wrapper node because the tree
    /// underneath is identical either way: the same callee, the same arguments,
    /// the same span. A node of its own would add a variant that every later
    /// stage -- parser, type checker, code generation -- has to match through to
    /// reach the arguments, and would give a call that differs from an ordinary
    /// call in one keyword an extra level of tree to walk.
    Call {
        callee: Vec<String>,
        args: Vec<Expr>,
        /// True only when the source wrote `unsafe` before this call.
        unsafe_call: bool,
        span: Span,
    },
    /// `P { x: 1.0, y: 2.0 }` -- one triple per field of name, value and the
    /// span of the name.
    StructLit {
        path: Vec<String>,
        fields: Vec<(String, Expr, Span)>,
        span: Span,
    },
    /// `p.x`
    FieldAccess {
        base: Box<Expr>,
        field: String,
        span: Span,
    },
    /// `[]T { a, b }` — a slice literal. The element type is explicit, so the
    /// literal is checked without inference and an empty `[]T {}` still has a
    /// type.
    SliceLit {
        elem: Box<TypeExpr>,
        elements: Vec<Expr>,
        span: Span,
    },
    /// `p[i]` -- reading (or, in an assignment target, writing) an element of
    /// a pointer. Otter has no arrays yet, so the base is always a `*mut i64`
    /// and the index an `i64`; the result is `i64`.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// `"far: ${d}!"` -- a string literal whose value is assembled at run time.
    ///
    /// `parts` is the literal in source order, text and expressions interleaved.
    /// A literal with no `${ ... }` is never this variant; it is
    /// [`Expr::Literal`] holding a [`StringLit`], so code that handles strings
    /// does not have to look for a case that cannot occur.
    ///
    /// The conversion of each part to text is *not* decided here. Whether `${d}`
    /// means `str_from_f64(d)` depends on the type of `d`, which the parser does
    /// not know, so the parts stay as written and the type checker (§check_expr)
    /// picks the conversion -- see [`InterpPart`].
    Interp {
        parts: Vec<InterpPart>,
        span: Span,
    },
    /// `x as f64`
    Cast {
        expr: Box<Expr>,
        ty: TypeExpr,
        span: Span,
    },
    /// `f()?` -- propagates the error out of a `Result`.
    Try {
        expr: Box<Expr>,
        span: Span,
    },
    /// `switch x { .. }` in value position, which yields the arm's expression.
    SwitchExpr {
        subject: Box<Expr>,
        arms: Vec<SwitchArm>,
        span: Span,
    },
}

/// One part of an [`Expr::Interp`].
#[derive(Clone, Debug, PartialEq)]
pub enum InterpPart {
    /// Literal text between interpolations, already decoded.
    Text(StringLit),
    /// `${ expr }`, the expression as written.
    Expr(Expr),
}

/// A binary operator. Copy because the parser holds one while deciding whether
/// the next token continues the expression it is building.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    /// The operator as it is written in source, which is what a diagnostic
    /// that blames the operator must quote.
    pub fn symbol(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

/// A unary operator, prefix only: Otter has no postfix `?` operator, which is
/// [`Expr::Try`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

impl UnOp {
    /// The operator as it is written in source.
    pub fn symbol(&self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
        }
    }
}
