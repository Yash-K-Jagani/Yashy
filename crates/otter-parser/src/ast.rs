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
    /// This variant is **declared but never constructed yet**: `parse_program`
    /// rejects an `extern` block with a diagnostic saying the feature arrives
    /// in M5. It is here so that M5 adds a variant rather than changing this
    /// one, which four later stages code against.
    Extern {
        abi: Option<String>,
        link: Option<String>,
        decls: Vec<ExternDecl>,
        span: Span,
    },
}

/// One field of a `struct`, or one field of an enum variant that has payload.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

/// One variant of an `enum`. An empty `fields` is a variant with no payload.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
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
/// lie. M5 must not add one. (The *call site* still needs marking -- see the
/// note on [`Expr::Call`] -- which is a different question from what the
/// declaration itself says.)
#[derive(Clone, Debug, PartialEq)]
pub struct ExternDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub span: Span,
}

/// A type as the source wrote it.
///
/// These five forms are the whole type grammar. There is no pointer, slice or
/// reference form: those arrive with the memory model in M3, and adding a
/// variant later is additive for the parser and a recompile for everyone else.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    /// A named type, primitive or declared: `i64`, `f64`, `P`.
    Name(String, Span),
    /// `Option[T]`
    Option(Box<TypeExpr>, Span),
    /// `Result[T, E]`
    Result(Box<TypeExpr>, Box<TypeExpr>, Span),
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
    /// `for i := 0; i < 10; i = i + 1 { .. }`
    ///
    /// Each of the three parts is optional -- `for ;; { .. }` is legal -- so
    /// each is an `Option`, and `init` is boxed because a statement is already
    /// a large enum.
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
    /// `f(1, 2)` -- a call by path.
    ///
    /// There is **no `unsafe` marker** here either, for a different reason
    /// than [`ExternDecl`] has none: calling an `extern` function is what
    /// spec 3.1 makes `unsafe` mark, and whether this call needs the marker
    /// depends on what `callee` resolves to -- a fact only the type checker
    /// knows. M5 will have to add a way to say it; that is a variant change,
    /// and this is the note saying so in advance.
    Call {
        callee: Vec<String>,
        args: Vec<Expr>,
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
        arms: Vec<SwitchArm>,
        span: Span,
    },
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
