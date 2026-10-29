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

use yashy_common::{FileId, Span};

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

/// One contract clause, after checking: `requires n > 0`.
///
/// The expression is checked as `bool`, so the verifier can read it as a
/// formula without asking what type it has. The span is the clause
/// expression's, which is where a proof that fails is reported.
#[derive(Clone, Debug, PartialEq)]
pub struct HirContract {
    pub expr: HirExpr,
    pub span: Span,
}

/// A function item: its signature and its checked body.
#[derive(Clone, Debug, PartialEq)]
pub struct HirFn {
    pub name: String,
    pub params: Vec<HirParam>,
    pub ret: Type,
    pub body: HirBlock,
    /// `requires` clauses: what the caller must establish.
    pub requires: Vec<HirContract>,
    /// `ensures` clauses: what the callee promises. `result` names the value
    /// being returned inside one.
    pub ensures: Vec<HirContract>,
    /// The values the function captured, in the order the environment holds
    /// them. Empty for every function that is not a closure.
    pub captures: Vec<HirCapture>,
    /// The module the function was written in.
    ///
    /// A program's items arrive flattened, one list in dependency order, so a
    /// later stage that reports a problem inside a body cannot recover the file
    /// from the item list alone. Spans index their own module's text, and two
    /// modules' spans can name the same bytes, which is why the file travels
    /// with the function rather than being guessed.
    pub file: FileId,
    pub span: Span,
}

/// One value a closure captured.
///
/// A capture is a name from an enclosing scope that the closure's body reads.
/// The closure cannot see that scope, so the value is copied into the
/// closure's environment when the closure is made, and the body reads it from
/// there.
#[derive(Clone, Debug, PartialEq)]
pub struct HirCapture {
    /// The name the capture was read under.
    pub name: String,
    /// The captured value's type.
    pub ty: Type,
    /// Where the value comes from when the closure is made.
    pub source: CaptureSource,
}

/// Where a captured value is read from at the point the closure is made.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureSource {
    /// A local of the scope the closure is written in: its parameter, or a
    /// binding of the body around it.
    Scope,
    /// A capture of the enclosing closure, at that closure's index. The
    /// enclosing closure holds it in its own environment, and the value is
    /// copied from there.
    Enclosing(usize),
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
    /// The file the import was written in.
    pub file: FileId,
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
    /// The file the block was written in.
    ///
    /// Items are flattened across modules, so a block that did not carry its
    /// file could not say where it came from. The inventory needs that: an
    /// assumption is evidence, and evidence without a position is not
    /// checkable.
    pub file: FileId,
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
    /// `const (a, b) = pair;` -- one binding per tuple element.
    ///
    /// `bindings` is aligned with the tuple's elements, so code generation
    /// extracts element `i` for the binding at position `i`. `None` is `_`:
    /// the element is extracted by nothing and simply dropped, which is free
    /// because a tuple element is a copyable value.
    Destructure {
        init: HirExpr,
        bindings: Vec<Option<(String, Type)>>,
        mutability: Mutability,
        span: Span,
    },
    /// `x = value;`.
    ///
    /// The target is gone by now: the checker proved it named a `let` binding,
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
        /// `buf.free()` the binding still names a value in the slot, but the
        /// pointer was already freed, so `buf = alloc(32)` must not free it
        /// again. Computed from the binding's ownership *before* the rebind.
        release_previous: bool,
        span: Span,
    },
    /// `p[i] = value;`
    ///
    /// Like `Assign`, the target is validated by the checker before it
    /// reaches here: `p` must be a mutable `*mut i64` binding and `i` an
    /// `int`. What remains is the three pieces of the store.
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
    /// what [`FieldStep`] carries; `root` is the `let` binding at the bottom of
    /// the chain, and it is the only thing the backend has to look up.
    FieldAssign {
        root: String,
        /// The chain of fields from `root`, outermost first. Never empty.
        steps: Vec<FieldStep>,
        value: HirExpr,
        ty: Type,
        span: Span,
    },
    /// `p[i].field = value;` -- a field write through a pointer.
    ///
    /// A pointer is an address, so unlike [`HirStmt::FieldAssign`] this is a
    /// plain store rather than a read-modify-write: the backend takes the
    /// element's address, steps to the field's offset, and stores there. The
    /// checker has already proved that `pointer` is a `*mut T` and resolved
    /// each `.field` to its position, which is what `steps` carries.
    FieldPtrAssign {
        pointer: HirExpr,
        index: HirExpr,
        /// The chain of fields from the element, outermost first. Never empty.
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
    /// `invariant n >= 0` -- a loop's promise at its head.
    ///
    /// The checker accepts one only among the first statements of a loop body,
    /// which is where the parser puts the clause that was written in the loop
    /// header. The verifier reads it in order, so the invariant is available
    /// from the first body statement on.
    LoopInvariant(HirContract),
}

/// One arm of a `switch`: the pattern, its body, and whether it is `default`.
#[derive(Clone, Debug, PartialEq)]
pub struct HirSwitchArm {
    pub pattern: HirPattern,
    /// The `if` condition, checked after the pattern's names are in scope.
    ///
    /// `None` for an unguarded arm. A guard is evaluated in the arm's own
    /// block: when it is false the arm does not match after all, and the test
    /// chain continues exactly as if the pattern had failed.
    pub guard: Option<HirExpr>,
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
        /// negation of the smallest `int`. See [`FaultSite`].
        fault: Option<FaultSite>,
    },
    Move(Box<HirExpr>, Span, Type),
    /// The address of a local binding: `&name`, written by the checker rather
    /// than by the source.
    ///
    /// A pointer receiver is called on the address of the object it mutates,
    /// so `c.bump()` lowers to a call whose first argument is `&c`. There is no
    /// way to write an address of a local in the language, which is why this
    /// variant is produced only by the checker's method resolution and has no
    /// source spelling of its own.
    AddrOf {
        /// The binding whose storage is the value.
        name: String,
        span: Span,
        /// The pointer type, `*mut T`.
        ty: Type,
    },
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
    /// `[1, 2, 3]` -- a fixed array built in the enclosing frame.
    ///
    /// The elements are all of `elem_ty`, and there are as many as the type's
    /// length says. Lowered as an `insertvalue` chain, so the value itself is
    /// an LLVM aggregate and no allocation happens.
    FixedArrayLit {
        elem_ty: Type,
        elements: Vec<HirExpr>,
        span: Span,
        ty: Type,
    },
    /// `a[0..2]` -- a borrowed view of part of an array.
    ///
    /// `base` is a binding: a view points into storage that outlives it, so
    /// the checker insists the source is a local it can hold onto. The result
    /// is a length and a pointer, and it owns neither.
    View {
        base: Box<HirExpr>,
        start: Box<HirExpr>,
        end: Box<HirExpr>,
        span: Span,
        ty: Type,
    },
    /// `(a, b)` -- an anonymous product.
    ///
    /// `ty` is the tuple type the elements produced, not a declared type: the
    /// elements say what the tuple is, exactly as they do for a fixed array.
    TupleLit {
        elements: Vec<HirExpr>,
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
    /// `|x: i64| -> i64 { ... }` -- an anonymous function.
    ///
    /// The body is a checked function, carried whole: a lambda is a function
    /// that has no name, and the backend needs the same things a named
    /// function needs -- its parameters, its return type and its body -- to
    /// emit it as a private function.
    ///
    /// The function's `name` is empty. The backend gives it one when it
    /// emits it, because the name has to be unique in the module and only the
    /// backend knows how many lambdas a program has.
    Lambda {
        f: Box<HirFn>,
        span: Span,
        ty: Type,
    },
    /// A read of a value the closure captured.
    ///
    /// The value is not in a slot: it is in the closure's environment, at the
    /// capture's index. The backend reads it from there.
    Capture {
        /// The capture's index in the function's capture list.
        index: usize,
        span: Span,
        ty: Type,
    },
    /// A call through a captured function value.
    ///
    /// The callee is not a name the backend can look up: it is a value in the
    /// closure's environment, and the call goes through it.
    CallCapture {
        /// The capture's index in the function's capture list.
        index: usize,
        args: Vec<HirExpr>,
        span: Span,
        ty: Type,
    },
    /// `xs.map(f)`, `xs.filter(f)`, `xs.fold(init, f)`.
    ///
    /// The three are one node because they are one shape: an array, a function
    /// to call, and a loop that calls it. The backend lowers the node into
    /// that loop, and the checker has already said what the loop produces.
    HigherOrder {
        /// Which of the three it is.
        op: HigherOrderOp,
        /// The array the method is called on.
        receiver: Box<HirExpr>,
        /// The function the method calls.
        func: Box<HirExpr>,
        /// The value `fold` starts from. `None` for the other two.
        init: Option<Box<HirExpr>>,
        span: Span,
        ty: Type,
    },
}

/// The three higher-order array methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HigherOrderOp {
    /// `xs.map(f)`: a new array holding the mapped values.
    Map,
    /// `xs.filter(f)`: a new array holding the values that pass.
    Filter,
    /// `xs.fold(init, f)`: one value, folded from the array.
    Fold,
}

/// Re-exported so the HIR can reference the AST's operator enums without
/// repeating the parser crate's paths everywhere.
pub use yashy_parser::ast::{BinOp, Literal, UnOp};

/// Whether `name` is the arena's allocation intrinsic, `arena_alloc`.
///
/// `arena_alloc(arena, n)` is the one call in the language that neither takes
/// nor gives ownership: it **borrows** the arena handle and returns a block the
/// arena owns. The handle stays usable for the next allocation, and the block
/// stays valid until `arena_free_all` releases every block at once.
///
/// Both halves of the compiler need that fact and neither may invent its own
/// version of it: the checker so it does not move the handle at the call
/// (`check_expr.rs`, where `arena_alloc` is the exception to "a linear
/// parameter consumes its argument"), and the verifier so it does not treat the
/// returned block as a value the binding has to release. It lives here, next to
/// the HIR both of them read, so the two cannot disagree.
pub fn is_arena_alloc(name: &str) -> bool {
    name == "arena_alloc"
}

/// The span of a typed expression.
pub fn expr_span(expr: &HirExpr) -> Span {
    match expr {
        HirExpr::Literal(_, span, _) => *span,
        HirExpr::Ident(_, span, _) => *span,
        HirExpr::Binary { span, .. } => *span,
        HirExpr::Unary { span, .. } => *span,
        HirExpr::Move(_, span, _) => *span,
        HirExpr::AddrOf { span, .. } => *span,
        HirExpr::Call { span, .. } => *span,
        HirExpr::StructLit { span, .. } => *span,
        HirExpr::SliceLit { span, .. } => *span,
        HirExpr::FieldAccess { span, .. } => *span,
        HirExpr::Index { span, .. } => *span,
        HirExpr::FixedArrayLit { span, .. } => *span,
        HirExpr::TupleLit { span, .. } => *span,
        HirExpr::View { span, .. } => *span,
        HirExpr::Cast { span, .. } => *span,
        HirExpr::Try { span, .. } => *span,
        HirExpr::SwitchExpr { span, .. } => *span,
        HirExpr::Lambda { span, .. } => *span,
        HirExpr::Capture { span, .. } => *span,
        HirExpr::CallCapture { span, .. } => *span,
        HirExpr::HigherOrder { span, .. } => *span,
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
        HirExpr::AddrOf { ty, .. } => ty.clone(),
        HirExpr::Call { ty, .. } => ty.clone(),
        HirExpr::StructLit { ty, .. } => ty.clone(),
        HirExpr::SliceLit { ty, .. } => ty.clone(),
        HirExpr::FieldAccess { ty, .. } => ty.clone(),
        HirExpr::Index { ty, .. } => ty.clone(),
        HirExpr::FixedArrayLit { ty, .. } => ty.clone(),
        HirExpr::TupleLit { ty, .. } => ty.clone(),
        HirExpr::View { ty, .. } => ty.clone(),
        HirExpr::Cast { ty, .. } => ty.clone(),
        HirExpr::Try { ty, .. } => ty.clone(),
        HirExpr::SwitchExpr { ty, .. } => ty.clone(),
        HirExpr::Lambda { ty, .. } => ty.clone(),
        HirExpr::Capture { ty, .. } => ty.clone(),
        HirExpr::CallCapture { ty, .. } => ty.clone(),
        HirExpr::HigherOrder { ty, .. } => ty.clone(),
    }
}

impl HirExpr {
    /// The position this expression reports a runtime fault from, when the
    /// operation it is can fault.
    ///
    /// Only checked integer arithmetic carries a [`FaultSite`]: `+`, `-`, `*`,
    /// `/`, `%` on `int`, and unary `-`. A comparison, a boolean operator, a
    /// float operation and a string concatenation cannot fault and answer
    /// `None`.
    ///
    /// The field stays private so this is the one way to read it. The verifier
    /// uses it to tell an operation that can overflow from one that cannot,
    /// without duplicating the checker's decision about which operations are
    /// checked.
    pub fn fault(&self) -> Option<&FaultSite> {
        match self {
            HirExpr::Binary { fault, .. } | HirExpr::Unary { fault, .. } => fault.as_ref(),
            _ => None,
        }
    }
}
