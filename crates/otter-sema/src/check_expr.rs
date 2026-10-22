//! Type checking: from the untyped AST to the typed HIR.
//!
//! Two passes over each item.
//!
//! 1. **Signatures.** Every `struct`, `enum` and `fn` is declared in the root
//!    scope, in one sweep, before any body is read. This is what makes mutual
//!    recursion work: `f` can call `g` whether `g` is declared before or after
//!    it. It is also what makes a duplicate item name a single, immediate error
//!    instead of "undefined" cascading into every use.
//! 2. **Bodies.** Each `fn` is checked with its own scope holding its
//!    parameters, then popped. Statement and expression checkers infer types
//!    bottom-up and unify against annotations where they exist.
//!
//! Errors are **collected**, not raised: a function with three type problems
//! reports all three, because a user fixing one at a time is a user who cannot
//! see the shape of the mistake. `check_program` returns `Err` with every
//! diagnostic when anything failed.
//!
//! # No input makes this module panic
//!
//! No `unwrap`, no `expect`, no `panic!`, no unchecked indexing. Every
//! fallible lookup returns `Result` or `Option`; every error becomes a
//! [`TypeError`], and every [`TypeError`] becomes a [`otter_common::Diagnostic`].

use std::collections::{BTreeSet, HashMap, HashSet};

use otter_common::{Diagnostic, FileId, SourceMap, Span};
use otter_parser::ast::{
    BinOp, BindingKeyword, Block, Expr, InterpFormat, InterpPart, Item, Literal, Param, Pattern,
    PtrKind, Stmt, StringLit, SwitchArm, TypeExpr, VariantDef, VariantField,
};

use crate::check_pat::check_pattern;
use crate::const_eval::{self, ConstFn, ConstFns, Missing};
use crate::error::TypeError;
use crate::exhaustive::missing_cases;
use crate::hir::*;
use crate::ownership::{linearity_of, Linearity, Ownership};
use crate::scope::{Binding, Scope};
use crate::ty::{Mutability, Type, TypeKind};

/// Debug formatting (`${value:?}`), which synthesizes one printer per type.
mod debug;

/// A declared struct: its fields, in declaration order.
#[derive(Clone, Debug)]
struct StructSig {
    fields: Vec<(String, Type)>,
    /// Whether each field may be used from another module, in the same order
    /// as `fields`.
    ///
    /// A field is private to the module that declares it unless it is marked
    /// `pub`, the same rule an item follows.
    field_public: Vec<bool>,
    /// Where the type came from, and whether another module may name it.
    origin: ItemOrigin,
}

/// The item inside a `pub` wrapper, or the item itself.
///
/// A method may be written `pub`, which says nothing the type does not
/// already say: a method is as visible as the type it belongs to. The
/// wrapper is therefore looked through rather than refused.
fn unwrap_public(item: &Item) -> &Item {
    match item {
        Item::Public { item, .. } => unwrap_public(item),
        other => other,
    }
}

/// Where an item was declared, and whether another module may use it.
///
/// A module's items belong to it: another module may use a `pub` item and may
/// not use a private one. Every use of a name therefore has to know where the
/// name came from, which is what this records.
#[derive(Clone, Copy, Debug)]
struct ItemOrigin {
    /// The module that declares the item.
    file: FileId,
    /// Whether the item is marked `pub`.
    public: bool,
}

/// A declared enum: its variants in declaration order, with payload types.
#[derive(Clone, Debug)]
struct EnumSig {
    variants: Vec<VariantDef>,
}

/// A function signature collected in pass 1.
#[derive(Clone, Debug)]
struct FnSig {
    params: Vec<Type>,
    ret: Type,
}

/// A generic function, kept until a call fixes its type parameters.
///
/// The body is not checked in pass 2 like an ordinary function's: it mentions
/// type variables, so there is no single set of types to check it against.
/// Each call records the substitution it inferred, and the body is checked
/// once per distinct substitution -- monomorphization. The HIR therefore holds
/// only concrete functions, which is why nothing downstream of the checker has
/// to know that generics exist.
#[derive(Clone, Debug)]
struct GenericFn {
    /// The type parameters, in the order written.
    type_params: Vec<(String, Span)>,
    /// The parameters, whose annotations may mention a type parameter.
    params: Vec<Param>,
    /// The return annotation, if the source wrote one.
    ret: Option<TypeExpr>,
    /// The body, checked once per substitution.
    body: Block,
    /// The item's span, used when the item itself is at fault.
    span: Span,
    /// The file the body came from, so its diagnostics name the right module.
    file: FileId,
}

/// One concrete substitution waiting to be checked.
///
/// A call records this instead of emitting the body directly: checking the
/// body may add more calls, and the queue is what lets one instantiation
/// discover another without recursing through the checker's own stack.
#[derive(Clone, Debug)]
struct Instantiation {
    /// The generic function being instantiated.
    name: String,
    /// One concrete type per type parameter, in the function's order.
    subst: Vec<Type>,
    /// The call site, for diagnostics about the instantiation itself.
    span: Span,
}

/// A method declared in an `impl` block.
#[derive(Clone, Debug)]
struct MethodSig {
    /// The type the method belongs to.
    type_name: String,
    /// The method's own name, without the type.
    name: String,
    /// The parameters, the receiver first.
    params: Vec<Type>,
    /// The return type.
    ret: Type,
    /// Whether the first parameter is the receiver.
    ///
    /// A method with no `self` is a static method: it is called on the type
    /// rather than on a value.
    has_self: bool,
    /// Whether the receiver is a pointer receiver -- `self: *mut T`.
    ///
    /// A pointer receiver is handed the address of the object it was called on,
    /// which is what lets the method mutate that object. The binding it is
    /// called on must therefore be a mutable `var`, and it must be a binding:
    /// there is no address of a temporary.
    receiver_is_pointer: bool,
}

/// The LLVM symbol a generic function has at one substitution.
///
/// `identity[T]` called with `i64` is `identity_i64`. The names are built from
/// the type names, so a symbol reads like the type it was made for. A struct's
/// own name is used as-is; the element types of a slice or a map are folded in,
/// which keeps `[]i64` and `[]string` apart.
fn mangled_generic_name(name: &str, subst: &[Type]) -> String {
    let mut mangled = name.to_string();
    for ty in subst {
        mangled.push('_');
        mangled.push_str(&mangled_type_name(ty));
    }
    mangled
}

fn mangled_type_name(ty: &Type) -> String {
    match &ty.kind {
        TypeKind::I64 => "i64".to_string(),
        TypeKind::F64 => "f64".to_string(),
        TypeKind::Bool => "bool".to_string(),
        TypeKind::Str => "string".to_string(),
        TypeKind::Unit => "unit".to_string(),
        TypeKind::Never => "never".to_string(),
        TypeKind::Struct(name) | TypeKind::Enum(name) => name.clone(),
        TypeKind::Option(inner) => format!("option_{}", mangled_type_name(inner)),
        TypeKind::Result(ok, err) => {
            format!("result_{}_{}", mangled_type_name(ok), mangled_type_name(err))
        }
        TypeKind::Slice(element) => format!("array_{}", mangled_type_name(element)),
        TypeKind::FixedArray(element, length) => {
            format!("fixed_{}_{length}", mangled_type_name(element))
        }
        TypeKind::View(element) => format!("view_{}", mangled_type_name(element)),
        TypeKind::Map(key, value) => {
            format!("map_{}_{}", mangled_type_name(key), mangled_type_name(value))
        }
        TypeKind::Mutex => "mutex".to_string(),
        TypeKind::Channel(element) => format!("channel_{}", mangled_type_name(element)),
        TypeKind::Ptr(pointee) => format!("ptr_{}", mangled_type_name(pointee)),
        TypeKind::CInt(width) => format!("cint{width}"),
        TypeKind::CUInt(width) => format!("cuint{width}"),
        TypeKind::CFloat(width) => format!("cfloat{width}"),
        TypeKind::CVoid => "void".to_string(),
        TypeKind::CPtr(pointee) => format!("cptr_{}", mangled_type_name(pointee)),
        TypeKind::CSize => "csize".to_string(),
        TypeKind::Func(params, ret) => {
            let mut mangled = "fn".to_string();
            for param in params {
                mangled.push('_');
                mangled.push_str(&mangled_type_name(param));
            }
            mangled.push_str("_to_");
            mangled.push_str(&mangled_type_name(ret));
            mangled
        }
        // A tuple's position is part of its type, so the mangled name has to
        // keep the order: `(i64, bool)` and `(bool, i64)` are different
        // instantiations of a generic function.
        TypeKind::Tuple(elements) => {
            let mut mangled = "tuple".to_string();
            for element in elements {
                mangled.push('_');
                mangled.push_str(&mangled_type_name(element));
            }
            mangled
        }
    }
}

/// How a declared type is written, for a diagnostic.
///
/// The checker's own [`Type`] describes the same thing, but a message about an
/// argument mismatch should show the annotation the program wrote -- `[]T`,
/// not a slice of some concrete element -- so the spelling is rebuilt from the
/// syntax tree.
fn type_expr_name(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Name(name, _) => name.clone(),
        TypeExpr::Ptr { pointee, .. } => format!("*mut {}", type_expr_name(pointee)),
        TypeExpr::Slice(element, _) => format!("[]{}", type_expr_name(element)),
        TypeExpr::FixedArray(element, length, _) => {
            format!("[{}; {}]", type_expr_name(element), length)
        }
        TypeExpr::View(element, _) => format!("[{}]", type_expr_name(element)),
        TypeExpr::Map(key, value, _) => {
            format!("map[{}]{}", type_expr_name(key), type_expr_name(value))
        }
        TypeExpr::Option(inner, _) => format!("Option[{}]", type_expr_name(inner)),
        TypeExpr::Result(ok, err, _) => {
            format!("Result[{}, {}]", type_expr_name(ok), type_expr_name(err))
        }
        TypeExpr::Channel(element, _) => format!("Channel[{}]", type_expr_name(element)),
        TypeExpr::Fn { params, ret, .. } => {
            let params: Vec<String> = params.iter().map(type_expr_name).collect();
            format!("fn({}) -> {}", params.join(", "), type_expr_name(ret))
        }
        TypeExpr::Tuple(items, _) => {
            let items: Vec<String> = items.iter().map(type_expr_name).collect();
            format!("({})", items.join(", "))
        }
        TypeExpr::Unit(_) => "()".to_string(),
    }
}

/// Whether a block returns on every path out of it.
///
/// This is what a "missing return" check needs: a function that returns a
/// value has to return one on every path, and the question is whether the
/// body manages it.
///
/// The analysis is deliberately conservative. A loop that might not run, an
/// `if` without an `else`, and a `switch` without a default all fall through,
/// so a body that ends in one of them does not always return.
fn block_always_returns(block: &HirBlock) -> bool {
    match block.stmts.last() {
        Some(HirStmt::Return(..)) => true,
        // A `panic` never comes back, so a body that ends in one returns.
        Some(HirStmt::Expr(_, _, ty)) => matches!(ty.kind, TypeKind::Never),
        Some(HirStmt::If {
            then_block,
            else_block,
            ..
        }) => match else_block {
            Some(else_block) => {
                block_always_returns(then_block) && block_always_returns(else_block)
            }
            None => false,
        },
        Some(HirStmt::Switch { arms, .. }) => {
            // A guarded `default` does not cover anything: its guard can be
            // false at run time. The switch only always returns when an
            // unguarded arm catches everything.
            arms.iter().any(|arm| arm.is_default && arm.guard.is_none())
                && arms.iter().all(|arm| expr_always_returns(&arm.body))
        }
        _ => false,
    }
}

/// Whether an expression always returns.
///
/// Only a `switch` can: every arm returns and there is a default, so the
/// switch has an answer on every path.
fn expr_always_returns(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::SwitchExpr { arms, .. } => {
            arms.iter().any(|arm| arm.is_default && arm.guard.is_none())
                && arms.iter().all(|arm| expr_always_returns(&arm.body))
        }
        _ => false,
    }
}

/// What a name lookup found.
enum NameLookup {
    /// The name is bound, and the binding may be used here.
    Found(Binding),
    /// The name is not bound at all.
    Missing,
    /// The name is bound, but it belongs to an enclosing scope and the closure
    /// being checked captures it. The capture's index and type are the value.
    Capture { index: usize, ty: Type },
}

/// One closure being checked: the scope its parameters live in, and the
/// values it has captured so far.
struct ClosureFrame {
    /// The depth of the scope the closure's own parameters are bound in.
    ///
    /// A name that resolves *above* this depth belongs to an enclosing scope
    /// and is a capture; a name at or below it is the closure's own.
    scope_depth: usize,
    /// The captures, in the order the environment holds them.
    captures: Vec<HirCapture>,
}

/// The type checker. Owns the scope stack, the collected signatures, and the
/// errors found so far.
pub struct Checker<'src> {
    scope: Scope,
    structs: HashMap<String, StructSig>,
    enums: HashMap<String, EnumSig>,
    fns: HashMap<String, FnSig>,
    /// The generic functions, keyed by name.
    ///
    /// A generic function is not a value and has no `FnSig`: its parameters
    /// mention type variables. A call finds it here, infers a substitution and
    /// queues an instantiation; the body is checked once per substitution.
    generics: HashMap<String, GenericFn>,
    /// The substitution in force while a generic body is being checked.
    ///
    /// Empty everywhere else. `resolve_type` consults it before the declared
    /// types, which is what makes `T` mean the instantiation's type inside a
    /// generic body and nowhere else.
    type_subst: HashMap<String, Type>,
    /// Instantiations discovered but not yet checked.
    pending_generics: Vec<Instantiation>,
    /// Mangled names already queued or emitted, so one substitution is checked
    /// once however many calls find it.
    instantiated: HashSet<String>,
    /// The instantiations currently being checked, innermost last.
    ///
    /// A call to a generic from inside its own instantiation is ordinary
    /// recursion when the substitution is the same and polymorphic recursion
    /// when it is not; the second has no finite monomorphization, so it is
    /// rejected here.
    active_generics: Vec<(String, Vec<Type>)>,
    /// The bindings a view in this function borrows, with the range's span.
    ///
    /// A view points into its array, and nothing in Otter can free the view
    /// before the function returns, so the borrow lasts as long as the body.
    /// The checker refuses to move or mutate a borrowed array: either would
    /// leave the view pointing at storage that is gone or has been replaced.
    borrowed_bindings: Vec<(String, Span)>,
    /// The methods declared in `impl` blocks, keyed by the mangled name
    /// `Type.method`.
    ///
    /// A method is an ordinary function with a receiver, so the signature is
    /// the same shape a function's is; what is different is where it is
    /// called from and that it is found by the type rather than by the name.
    methods: HashMap<String, MethodSig>,
    /// The type a method's `self` has, while its body is being checked.
    ///
    /// `Self` in a method's signature or body means the type the `impl` block
    /// is for, and this is where that answer is kept.
    self_type: Option<Type>,
    /// The return type of the function being checked, while its body is.
    ///
    /// `?` needs it: propagating an error means returning it, so the
    /// enclosing function has to be returning a `Result` of the same error
    /// type. The "missing return" check needs it too: a function that returns
    /// a value has to return one on every path.
    fn_ret: Option<Type>,
    /// Names declared inside an `extern` block. A call to one of these follows
    /// the FFI boundary rules, which user functions do not (spec §8.1).
    externs: HashSet<String>,
    /// Every error found, with the file it belongs to. A single-file check has
    /// one file and an import graph has several, so the file is recorded at the
    /// moment the error is reported rather than taken from `self.file` at the
    /// end.
    errors: Vec<(FileId, TypeError)>,
    /// The file currently being declared or checked. It is what `panic` and
    /// `assert` turn a span into a position with, and it moves from module to
    /// module as [`Checker::check_modules`] walks the graph.
    file: FileId,
    /// Whether this is a release build, in which a `debug_assert` compiles away.
    ///
    /// The only build-mode decision the checker makes. It cannot be made later:
    /// dropping the call is dropping the *tree*, so that an argument with side
    /// effects is not lowered either.
    release: bool,
    /// Whether any `thread_spawn` call was checked.
    ///
    /// Codegen uses this one bit to pick the atomic reference-count pair for
    /// the whole program: once a thread exists, a string built anywhere in the
    /// program can reach it through an argument or a call, and a per-binding
    /// answer would need the alias analysis this milestone does not have.
    uses_threads: bool,
    /// Names bound by an active `for x in slice` loop. Moving one of them while
    /// the loop can still read its header would free the array the next
    /// iteration indexes, so `move_out` rejects it (see [`Checker::move_out`]).
    iteration_borrows: Vec<String>,
    /// The closures being checked, innermost last.
    ///
    /// A closure's frame says which scope is its own and what it has
    /// captured. A name that resolves above a closure's own scope is a
    /// capture: the value is copied into the closure's environment when the
    /// closure is made, and the body reads it from there.
    ///
    /// The root scope is never a capture: a top-level function is a global,
    /// and a closure may call one.
    closures: Vec<ClosureFrame>,
    /// The debug printers `${value:?}` asked for, in the order they were
    /// discovered. Each is an ordinary function appended to the program after
    /// generics are instantiated, so the backend has no idea it was not
    /// written by hand.
    debug_printers: Vec<HirFn>,
    /// The printer names already queued or emitted. A type with more than one
    /// `:?` use gets one printer, and a printer that contains itself does not
    /// ask for a second body while the first is being built.
    debug_printer_names: HashSet<String>,
    /// The `const fn`s the program declares, by name.
    ///
    /// A `const fn` has no run-time existence: it is evaluated by the
    /// compiler, and the value it produces is what a `const` binding holds.
    /// They are therefore *not* in the root scope -- there is no runtime
    /// function of that name to call -- and a call to one is recognised by
    /// the initializer it appears in.
    const_fns: ConstFns,
    /// Where each declared item came from, keyed by name.
    ///
    /// A module's items are private to it unless they are marked `pub`, so a
    /// use of a name has to know which module declared it. The entry is
    /// missing for a name that is not an item -- a builtin, or a local -- and
    /// such a name is never private to anybody.
    item_origins: HashMap<String, ItemOrigin>,
    /// The source text and its line index.
    ///
    /// Only the diagnostics builtins read this: `panic` reports the position it
    /// was written at, and `assert`/`assert_eq`/`assert_ne` quote the
    /// expression back. Both are facts the checker holds and the backend does
    /// not -- it is handed IR text and a path, and no map -- so they are
    /// resolved here and travel in the tree.
    sm: &'src SourceMap,
}

impl<'src> Checker<'src> {
    pub fn new(sm: &'src SourceMap, file: FileId, options: CheckOptions) -> Self {
        let mut scope = Scope::new();
        // Builtins are predeclared in the root scope. They are functions, so a
        // call resolves through the same path as a user call.
        //
        // `alloc` returns a pointer to `i64`, not `u8`. Otter has no byte type,
        // and without indexing or pointer casts a `*mut u8` could not be used at
        // all; `*mut i64` is directly usable and matches the plan's own
        // `buffer.ot`. Recorded as a divergence from the master plan's L3.
        let builtins: [(&str, Vec<Type>, Type); 30] = [
            ("print_str", vec![prim(TypeKind::Str)], prim(TypeKind::Unit)),
            ("print_i64", vec![prim(TypeKind::I64)], prim(TypeKind::Unit)),
            ("print_f64", vec![prim(TypeKind::F64)], prim(TypeKind::Unit)),
            ("alloc", vec![prim(TypeKind::I64)], ptr(prim(TypeKind::I64))),
            ("dealloc", vec![ptr(prim(TypeKind::I64))], prim(TypeKind::Unit)),
            // Arenas (M3 Task 4): a bump allocator. `arena_new` hands out an
            // opaque handle; `arena_alloc(arena, n)` hands out room for `n`
            // `i64`s; `arena_free_all(arena)` releases every block at once.
            // No per-allocation free, and no automatic release -- the program
            // calls `arena_free_all` when it is done, or leaks until exit.
            (
                "arena_new",
                vec![],
                ptr(prim(TypeKind::I64)),
            ),
            (
                "arena_alloc",
                vec![ptr(prim(TypeKind::I64)), prim(TypeKind::I64)],
                ptr(prim(TypeKind::I64)),
            ),
            (
                "arena_free_all",
                vec![ptr(prim(TypeKind::I64))],
                prim(TypeKind::Unit),
            ),
            // Input. `read_line` yields a `string` (empty at end of input).
            // `parse_i64` and `parse_f64` are methods now: they lower to the
            // same runtime entry points, but the public spelling is
            // `s.parse_i64()` / `s.parse_f64()`. A program that can print but
            // not read is a report generator; this line is what makes it a
            // program.
            ("read_line", vec![], prim(TypeKind::Str)),
            // Files and the environment (M6 Task 10). `read_file` yields the
            // whole file as a fresh owned string, or the empty string when it
            // cannot be read; `write_file` reports 1 for success and 0 for
            // failure; `getenv` yields the variable's value, or the empty string
            // when it is unset. The ambiguities are deliberate and documented in
            // the plan: a builtin's signature cannot spell `Result` yet, and an
            // empty file (or an empty variable) is a legitimate value.
            ("read_file", vec![prim(TypeKind::Str)], prim(TypeKind::Str)),
            (
                "write_file",
                vec![prim(TypeKind::Str), prim(TypeKind::Str)],
                prim(TypeKind::I64),
            ),
            ("getenv", vec![prim(TypeKind::Str)], prim(TypeKind::Str)),
            // Out the other way: a number as text, so a program can build a
            // string that *contains* one (`"count = " + str_from_i64(n)`).
            // Without these the runtime's conversions existed and nothing could
            // call them, and `print_i64` was the only way to show a number.
            //
            // Three names rather than one `str`: the builtin table declares a
            // name once and duplicate declarations in a scope are an error, so
            // there is no overloading to hang three argument types on. Task 7's
            // interpolation is the ergonomic form of this ("n = ${n}") and will
            // call exactly these conversions.
            ("str_from_i64", vec![prim(TypeKind::I64)], prim(TypeKind::Str)),
            ("str_from_f64", vec![prim(TypeKind::F64)], prim(TypeKind::Str)),
            ("str_from_bool", vec![prim(TypeKind::Bool)], prim(TypeKind::Str)),
            // The way out of a situation the program cannot continue from.
            //
            // Its result type is `Never`, not `()`, and that is the whole point:
            // a call that cannot return has no value, so a program cannot go on
            // to use one, and the checker's `!` is where that is recorded. The
            // backend lowers it to a call that ends the process.
            //
            // It takes the message only. The file, line and column a panic
            // reports are facts the *checker* has and the program does not, so
            // they are added to the lowered call here rather than written down
            // by the user -- see `Checker::panic_call`.
            ("panic", vec![prim(TypeKind::Str)], prim(TypeKind::Never)),
            // The three ways out of the checked default (spec 6.1). Integer
            // arithmetic stops the program on overflow, which is the right
            // default and the wrong answer for code that has a reason to want
            // the other behaviour:
            //
            // - `wrapping_*` is what C, Go and Rust-in-release do all the time:
            //   two's-complement wraparound, no check, no branch. It is the
            //   fastest and it is a choice the program has to make out loud.
            // - `checked_*` answers with `None` instead of stopping, for code
            //   that can do something better than die. It is a `select`, not a
            //   branch: the caller decides what the answer means.
            // - `saturating_*` clamps to `i64::MIN`/`i64::MAX`, for accumulators
            //   where "as large as it goes" is more useful than a fault.
            //
            // Only `+`, `-` and `*`. A division has no wrapping answer worth
            // naming -- a wrapped quotient is not the quotient of anything, and
            // `i64::MIN / -1` is exactly the case the checked lowering already
            // handles -- so `/` and `%` keep their check with no opt-out. That
            // asymmetry is deliberate: a divisor of zero is not an overflow, and
            // a language that let a program wrap *that* would be lying about
            // what happened.
            //
            // Nine names, not one overloaded name: the table declares a name
            // once and a duplicate declaration is an error, so the family has to
            // be in the name. It reads the way the arithmetic reads
            // (`acc = wrapping_add(acc, x);`), which is the point.
            ("wrapping_add", i64_pair(), prim(TypeKind::I64)),
            ("wrapping_sub", i64_pair(), prim(TypeKind::I64)),
            ("wrapping_mul", i64_pair(), prim(TypeKind::I64)),
            ("checked_add", i64_pair(), option_i64()),
            ("checked_sub", i64_pair(), option_i64()),
            ("checked_mul", i64_pair(), option_i64()),
            ("saturating_add", i64_pair(), prim(TypeKind::I64)),
            ("saturating_sub", i64_pair(), prim(TypeKind::I64)),
            ("saturating_mul", i64_pair(), prim(TypeKind::I64)),
            // `assert` (spec 6.2). A condition that must hold, reported with
            // the expression that was written when it does not:
            //
            //     assert(n > 0);
            //     main.ot:3:5: panic: assertion failed: n > 0
            //
            // Like `panic`, the interesting arguments are added by the checker
            // (`Checker::assert_call`): the source text and the position are
            // facts only it holds.
            //
            // The type is `()`, not `Never`: an assertion that *passes* returns
            // normally, so treating it as diverging would let a program assign
            // its non-existent value to anything.
            ("assert", vec![prim(TypeKind::Bool)], prim(TypeKind::Unit)),
            // `debug_assert` is the same assertion, lowered only in a debug
            // build. The checker drops the whole call when `--release` asks for
            // a release build, so the condition -- and any side effect in it --
            // never reaches the backend. It is still checked first: source a
            // release build rejects would be the worst of both worlds.
            (
                "debug_assert",
                vec![prim(TypeKind::Bool)],
                prim(TypeKind::Unit),
            ),
            // `assert_eq(a, b)` and `assert_ne(a, b)` are the comparison forms.
            // The declared pair is an `i64` one, but the checker never consults
            // it: every call is built generically in `Checker::assert_cmp_call`,
            // so the two operands may be any pair of `i64`, `f64`, `bool` or
            // `string`. The entry is here for what the table is actually for --
            // a name, declared once, that a program therefore cannot redefine.
            ("assert_eq", i64_pair(), prim(TypeKind::Unit)),
            ("assert_ne", i64_pair(), prim(TypeKind::Unit)),
            // `os.exit(code)` (spec 6.3). The one builtin whose name has a dot
            // in it, because the specification writes it that way: it is the
            // exit *of the operating system*, not of the program's own control
            // flow, and the qualification is the documentation.
            //
            // Modules exist (M6 Task 9), but references are unqualified: a call
            // is resolved by its last segment like every other name, so
            // `os.exit` reaches this entry as `exit`. Both spellings therefore
            // work, and the checker cannot tell them apart -- which is recorded
            // here rather than silently relied on.
            //
            // The type is `!`: exiting produces no value, and the same argument
            // that lets `panic` stand in a `return` position applies. Taking an
            // `i64` rather than an `i32` is because `i64` is the only integer
            // type there is; the runtime narrows it.
            ("exit", vec![prim(TypeKind::I64)], never(Span::new(0, 0))),
        ];
        for (name, params, ret) in builtins {
            let _ = scope.declare(
                name,
                Binding {
                    ty: Type {
                        kind: TypeKind::Func(params, Box::new(ret)),
                        span: Span::new(0, 0),
                    },
                    mutability: Mutability::Const,
                    span: Span::new(0, 0),
                    ownership: Ownership::Live,
                },
            );
        }
        Self {
            scope,
            structs: HashMap::new(),
            enums: HashMap::new(),
            fns: HashMap::new(),
            generics: HashMap::new(),
            type_subst: HashMap::new(),
            pending_generics: Vec::new(),
            instantiated: HashSet::new(),
            active_generics: Vec::new(),
            borrowed_bindings: Vec::new(),
            methods: HashMap::new(),
            self_type: None,
            fn_ret: None,
            externs: HashSet::new(),
            errors: Vec::new(),
            file,
            release: options.release,
            uses_threads: false,
            iteration_borrows: Vec::new(),
            closures: Vec::new(),
            debug_printers: Vec::new(),
            debug_printer_names: HashSet::new(),
            const_fns: ConstFns::new(),
            item_origins: HashMap::new(),
            sm,
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push((self.file, TypeError::new(span, message)));
    }

    /// Look a name up for a read, a call or an assignment.
    ///
    /// The three outcomes are separate because the three callers say different
    /// things about a name that is not there: a read says "cannot find value",
    /// a call says "cannot find function", and neither should be said at all
    /// when the name *is* there but belongs to an enclosing scope -- a capture
    /// is a value, not a missing name.
    fn lookup(&mut self, name: &str, span: Span) -> NameLookup {
        let Some(binding) = self.scope.resolve(name).cloned() else {
            return NameLookup::Missing;
        };
        let Some(depth) = self.scope.resolve_depth(name) else {
            return NameLookup::Missing;
        };
        // A name in the root scope is an item, and an item belongs to the
        // module that declared it: another module may use it only if it is
        // `pub`. A name in an inner scope is a local, and a local is nobody's
        // private business.
        if depth == 0 {
            self.check_visibility(name, span);
        }
        // A name that resolves above the closure being checked belongs to an
        // enclosing scope: a capture. The root scope is exempt -- a top-level
        // function is a global, and a closure may call one.
        if let Some(frame) = self.closures.last() {
            if depth > 0 && depth < frame.scope_depth {
                let index = self.capture(name, depth, &binding.ty, span);
                return NameLookup::Capture {
                    index,
                    ty: binding.ty.clone(),
                };
            }
        }
        NameLookup::Found(binding)
    }

    /// Record that the closure being checked captures `name`, and return the
    /// capture's index.
    ///
    /// A linear value cannot be captured: the environment would have to own a
    /// value the program still owns, and Otter has no way to say which of the
    /// two owners releases it.
    fn capture(&mut self, name: &str, depth: usize, ty: &Type, span: Span) -> usize {
        if matches!(linearity_of(&ty.kind), Linearity::Linear) {
            self.error(
                span,
                format!("a linear value cannot be captured yet: `{name}`"),
            );
        }
        let frame_index = self.closures.len() - 1;
        self.capture_in(frame_index, name, depth, ty)
    }

    /// Record a capture in a particular closure, and return its index.
    ///
    /// The closure that is being checked captures the name; the closures
    /// between it and the scope the name belongs to capture it too, so that
    /// the value is there to be copied.
    fn capture_in(
        &mut self,
        frame_index: usize,
        name: &str,
        depth: usize,
        ty: &Type,
    ) -> usize {
        if let Some(index) = self.closures[frame_index]
            .captures
            .iter()
            .position(|capture| capture.name == name)
        {
            return index;
        }
        let source = self.capture_source(frame_index, name, depth, ty);
        self.closures[frame_index].captures.push(HirCapture {
            name: name.to_string(),
            ty: ty.clone(),
            source,
        });
        self.closures[frame_index].captures.len() - 1
    }

    /// Where the closure at `frame_index` reads the captured `name` from.
    ///
    /// The name belongs to a scope outside the closure. The scope that holds
    /// it is the innermost enclosing closure whose own scope reaches the
    /// name's depth -- or the function around them all, if there is no such
    /// closure. The value is read from that scope when the closure is made.
    fn capture_source(
        &mut self,
        frame_index: usize,
        name: &str,
        depth: usize,
        ty: &Type,
    ) -> CaptureSource {
        if frame_index == 0 {
            // The outermost closure: the name belongs to the function it is
            // written in, and the function's frame is live where the closure
            // is made.
            return CaptureSource::Scope;
        }
        let parent = frame_index - 1;
        if depth >= self.closures[parent].scope_depth {
            // A local of the enclosing closure's own scope: its parameter, or
            // a binding of its body.
            CaptureSource::Scope
        } else {
            // The enclosing closure captured the name too, so the value is
            // copied out of that closure's environment.
            let parent_index = self.capture_in(parent, name, depth, ty);
            CaptureSource::Enclosing(parent_index)
        }
    }

    /// Move `name` out of its binding, unless an enclosing `for-in` is reading
    /// it. The loop re-reads the header every iteration, so a `free_slice` or
    /// another linear transfer inside the body would leave it reading a header
    /// whose array is gone.
    /// Refuse a write to a binding a view borrows.
    ///
    /// A view is a pointer into the binding's storage, and Otter has no way to
    /// end the view early: it lives until the function returns. Rebinding,
    /// freeing, or mutating the array in place would all leave the view reading
    /// storage that was released or replaced, so every write to a borrowed
    /// binding is an error rather than a dangling read later.
    fn reject_borrowed_write(&mut self, name: &str, span: Span) {
        if let Some((_, borrow_span)) = self
            .borrowed_bindings
            .iter()
            .find(|(borrowed, _)| borrowed == name)
        {
            self.error(
                span,
                format!(
                    "cannot change `{name}` while a view of it is alive: the view at line {} \
                     points into it",
                    self.sm.locate(self.file, *borrow_span).line
                ),
            );
        }
    }

    fn move_out(&mut self, name: &str, span: Span) -> Result<(), TypeError> {
        if self.iteration_borrows.iter().any(|borrowed| borrowed == name) {
            return Err(TypeError::new(
                span,
                format!("cannot move `{name}` while it is being iterated"),
            ));
        }
        if let Some((_, borrow_span)) = self
            .borrowed_bindings
            .iter()
            .find(|(borrowed, _)| borrowed == name)
        {
            return Err(TypeError::new(
                span,
                format!(
                    "cannot move `{name}` while a view of it is alive: the view at \
                     {} points into it",
                    self.sm.locate(self.file, *borrow_span).line
                ),
            ));
        }
        self.scope.move_out(name, span)
    }

    /// A linear value may leave its binding only through `move`. Handing a live
    /// linear binding to a new binding (`var o = buf`) without `move` would
    /// leave two owners of one allocation, so it is an error rather than a
    /// silent copy. Only a bare name can be a copy of a binding; a call or a
    /// literal produces a value that already has exactly one owner, and a
    /// `move` says the transfer out loud.
    fn forbid_linear_copy(&mut self, source: &Expr, checked: &HirExpr) {
        if matches!(checked, HirExpr::Move(..)) {
            return;
        }
        if let Expr::Ident(path, span) = source {
            let name = path.last().cloned().unwrap_or_default();
            if let Some(binding) = self.scope.resolve(&name) {
                if matches!(linearity_of(&binding.ty.kind), Linearity::Linear) {
                    self.error(
                        *span,
                        format!("cannot assign linear value `{name}` without `move`"),
                    );
                }
            }
        }
    }

    /// Check a whole program. Returns the typed HIR when every item passed.
    pub fn check_program(
        &mut self,
        ast: &otter_parser::ast::Program,
    ) -> Result<HirProgram, Vec<Diagnostic>> {
        let file = self.file;
        self.check_modules(&[(file, ast)])
    }

    /// Check a whole import graph, in the order the driver loaded it.
    ///
    /// Every module's items are declared before any body is read, so a module
    /// can call a function another module defines regardless of which one comes
    /// first in the list (the loader orders dependencies before the entry, but
    /// mutual recursion through *types* would still need the two passes). Names
    /// share one root scope, so defining `Point` in two modules is the same
    /// duplicate-definition error as defining it twice in one file.
    ///
    /// `self.file` moves to each module before its items are touched, which is
    /// what makes a diagnostic -- and the file/line a `panic` reports -- name
    /// the module the mistake is in rather than the entry point.
    pub fn check_modules(
        &mut self,
        modules: &[(FileId, &otter_parser::ast::Program)],
    ) -> Result<HirProgram, Vec<Diagnostic>> {
        for (file, ast) in modules {
            self.file = *file;
            self.declare_items(&ast.items);
        }
        let mut items = Vec::new();
        for (file, ast) in modules {
            self.file = *file;
            for item in &ast.items {
                if let Some(hir) = self.check_item(item) {
                    items.extend(hir);
                }
            }
        }
        // Generic bodies are checked last, once per substitution the program
        // asked for. This has to follow every module: a generic body may call
        // anything the program declares.
        self.instantiate_generics(&mut items);
        // The printers are ordinary functions with synthesized bodies. They are
        // appended after every generic body has been checked, because a generic
        // instantiation is one of the places a `${value:?}` part can appear.
        items.extend(self.debug_printers.drain(..).map(HirItem::Fn));
        if self.errors.is_empty() {
            Ok(HirProgram {
                items,
                uses_threads: self.uses_threads,
            })
        } else {
            let diagnostics = self
                .errors
                .drain(..)
                .map(|(file, e)| e.into_diagnostic(file))
                .collect();
            Err(diagnostics)
        }
    }

    /// The type a name refers to, if it is one of the declared types.
    fn declared_type(&self, name: &str) -> Option<Type> {
        if self.structs.contains_key(name) {
            return Some(Type {
                kind: TypeKind::Struct(name.to_string()),
                span: Span::new(0, 0),
            });
        }
        if self.enums.contains_key(name) {
            return Some(Type {
                kind: TypeKind::Enum(name.to_string()),
                span: Span::new(0, 0),
            });
        }
        None
    }

    /// The signature of a method declared in an `impl` block.
    ///
    /// The first parameter is the receiver when it is written `self`; the
    /// receiver's type is the type the block is for, whatever the parameter
    /// says.
    fn method_signature(
        &mut self,
        type_name: &str,
        name: &str,
        params: &[Param],
        ret: &Option<TypeExpr>,
        receiver_ty: &Type,
        span: Span,
    ) -> MethodSig {
        let has_self = params
            .first()
            .map(|param| param.name == "self")
            .unwrap_or(false);
        let mut receiver_is_pointer = false;
        let mut param_tys = Vec::with_capacity(params.len());
        for (index, param) in params.iter().enumerate() {
            if index == 0 && has_self {
                // The receiver is `self` (a value) or `self: *mut T` (a
                // pointer). A pointer receiver's pointee must be the type the
                // block is for: `self: *mut Other` in `impl Counter` would let
                // a method of one type mutate another.
                match &param.ty {
                    TypeExpr::Name(written, _) if written == "Self" => {
                        param_tys.push(receiver_ty.clone());
                    }
                    TypeExpr::Ptr {
                        kind: PtrKind::Mut,
                        pointee,
                        ..
                    } => {
                        let pointee_ty = self.resolve_or_report(pointee, param.span);
                        if !pointee_ty.eq_ignoring_span(receiver_ty) {
                            self.error(
                                param.span,
                                format!(
                                    "a pointer receiver of `{type_name}` must point at \
                                     `{type_name}`, not `{pointee_ty}`"
                                ),
                            );
                        }
                        receiver_is_pointer = true;
                        param_tys.push(Type {
                            kind: TypeKind::Ptr(Box::new(pointee_ty)),
                            span: param.span,
                        });
                    }
                    _ => {
                        self.error(
                            param.span,
                            format!(
                                "a receiver is `self` or `self: *mut {type_name}`"
                            ),
                        );
                        param_tys.push(receiver_ty.clone());
                    }
                }
                continue;
            }
            param_tys.push(self.resolve_or_report(&param.ty, param.span));
        }
        let ret_ty = match ret {
            Some(ty) => self.resolve_or_report(ty, span),
            None => prim(TypeKind::Unit),
        };
        MethodSig {
            type_name: type_name.to_string(),
            name: name.to_string(),
            params: param_tys,
            ret: ret_ty,
            has_self,
            receiver_is_pointer,
        }
    }

    /// Record a generic function's declaration.
    ///
    /// The function is not bound in the root scope: a generic function is not
    /// a value, and its signature has type variables that only a call can
    /// replace. It is kept here until a call instantiates it.
    fn declare_generic(
        &mut self,
        name: &str,
        type_params: &[(String, Span)],
        params: &[Param],
        ret: &Option<TypeExpr>,
        body: &Block,
        span: Span,
    ) {
        if self.generics.contains_key(name) || self.scope.resolve(name).is_some() {
            self.error(span, format!("duplicate definition of `{name}`"));
        }
        for (index, (param, param_span)) in type_params.iter().enumerate() {
            if type_params[..index]
                .iter()
                .any(|(previous, _)| previous == param)
            {
                self.error(*param_span, format!("duplicate type parameter `{param}`"));
            }
            if param == "Self" {
                self.error(
                    *param_span,
                    "`Self` is not a type parameter name: it means the method's own type",
                );
            }
            // A type parameter shadows the type it names inside the body, so a
            // parameter that names one turns every mention of that type in the
            // body into something else. That is never what the author meant.
            if self
                .resolve_type(&TypeExpr::Name(param.clone(), *param_span))
                .is_ok()
            {
                self.error(
                    *param_span,
                    format!("type parameter `{param}` shadows a type of the same name"),
                );
            }
        }
        self.generics.insert(
            name.to_string(),
            GenericFn {
                type_params: type_params.to_vec(),
                params: params.to_vec(),
                ret: ret.clone(),
                body: body.clone(),
                span,
                file: self.file,
            },
        );
    }

    /// Where an item declared right now comes from.
    ///
    /// The module being declared is the one the checker is standing in, so
    /// this is the same for every item in a module.
    fn origin(&self, public: bool) -> ItemOrigin {
        ItemOrigin {
            file: self.file,
            public,
        }
    }

    /// Record where a declared item came from, so a use of it elsewhere can
    /// be checked.
    fn record_origin(&mut self, name: &str, public: bool) {
        self.item_origins
            .insert(name.to_string(), self.origin(public));
    }

    /// Report a use of an item that belongs to another module and is
    /// private.
    ///
    /// The use is where the mistake is visible: the item itself is fine, and
    /// it is another module's use of it that is not. A name that is not an
    /// item -- a builtin, or a local -- is nobody's private business.
    fn check_visibility(&mut self, name: &str, span: Span) {
        let Some(origin) = self.item_origins.get(name).copied() else {
            return;
        };
        if origin.public || origin.file == self.file {
            return;
        }
        let module = self.module_name(origin.file);
        self.error(
            span,
            format!("`{name}` is private to module `{module}`"),
        );
    }

    /// Report a use of a field that belongs to another module and is
    /// private.
    ///
    /// A field is private to the module that declares it unless it is marked
    /// `pub`, the same rule an item follows.
    fn check_field_visibility(
        &mut self,
        struct_name: &str,
        field: &str,
        sig: &StructSig,
        index: usize,
        span: Span,
    ) {
        if sig.origin.file == self.file {
            return;
        }
        if sig.field_public.get(index).copied().unwrap_or(false) {
            return;
        }
        let module = self.module_name(sig.origin.file);
        self.error(
            span,
            format!("field `{field}` of `{struct_name}` is private to module `{module}`"),
        );
    }

    /// The name a module is known by in a diagnostic.
    ///
    /// A module is one source file, so the name is that file's own name
    /// without its extension: `json_keys.ot` is the module `json_keys`.
    fn module_name(&self, file: FileId) -> String {
        let path = self.sm.path(file);
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        match name.rsplit_once('.') {
            Some((stem, _)) => stem.to_string(),
            None => name.to_string(),
        }
    }

    /// Pass 1: declare every item name so signatures can reference each other.
    fn declare_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                // `pub` is a modifier in front of an item: the item inside is
                // declared as usual, and marked as one another module may use.
                Item::Public { item, .. } => self.declare_item(item, true),
                other => self.declare_item(other, false),
            }
        }
    }

    /// Declare one item's name, and record where it came from.
    ///
    /// `public` says whether the item is marked `pub`, which is what decides
    /// whether another module may use it.
    fn declare_item(&mut self, item: &Item, public: bool) {
        match item {
            Item::Public { item, .. } => self.declare_item(item, public),
            Item::Fn {
                name,
                type_params,
                params,
                ret,
                body,
                span,
            } => {
                if !type_params.is_empty() {
                    // A generic function is not a value: its signature has
                    // type variables, so there is nothing to bind in the
                    // root scope until a call says what they are. The call
                    // path finds it in `generics` instead.
                    self.declare_generic(
                        name,
                        type_params,
                        params,
                        ret,
                        body,
                        *span,
                    );
                    self.record_origin(name, public);
                    return;
                }
                let param_tys: Vec<Type> = params
                    .iter()
                    .map(|Param { ty, .. }| self.resolve_or_report(ty, *span))
                    .collect();
                let ret_ty = match ret {
                    Some(ty) => self.resolve_or_report(ty, *span),
                    None => prim(TypeKind::Unit),
                };
                let sig = FnSig {
                    params: param_tys.clone(),
                    ret: ret_ty.clone(),
                };
                let binding = Binding {
                    ty: Type {
                        kind: TypeKind::Func(param_tys, Box::new(ret_ty)),
                        span: *span,
                    },
                    mutability: Mutability::Const,
                    span: *span,
                    ownership: Ownership::Live,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.error(e.span, e.message);
                }
                self.fns.insert(name.clone(), sig);
                self.record_origin(name, public);
            }
            Item::Impl {
                type_name,
                methods,
                span,
            } => {
                let Some(receiver_ty) = self.declared_type(type_name) else {
                    self.error(
                        *span,
                        format!("no type named `{type_name}` for these methods"),
                    );
                    return;
                };
                for method in methods {
                    let Item::Fn {
                        name,
                        type_params,
                        params,
                        ret,
                        body: _,
                        span: method_span,
                    } = unwrap_public(method)
                    else {
                        self.error(*span, "an `impl` block holds only methods");
                        continue;
                    };
                    if !type_params.is_empty() {
                        self.error(
                            *method_span,
                            "a method cannot have type parameters yet",
                        );
                        continue;
                    }
                    let sig = self.method_signature(
                        type_name,
                        name,
                        params,
                        ret,
                        &receiver_ty,
                        *method_span,
                    );
                    let mangled = format!("{type_name}.{name}");
                    if let Some(previous) = self.methods.insert(mangled.clone(), sig) {
                        if previous.name == *name {
                            self.error(
                                *method_span,
                                format!("`{type_name}` already has a method named `{name}`"),
                            );
                        }
                    }
                }
            }
            Item::Object {
                name,
                fields,
                methods,
                span,
            } => {
                // An object is a struct and an `impl` block written
                // together: the type is declared here, and the methods
                // are registered the way an `impl` block's are.
                let binding = Binding {
                    ty: Type {
                        kind: TypeKind::Struct(name.clone()),
                        span: *span,
                    },
                    mutability: Mutability::Const,
                    span: *span,
                    ownership: Ownership::Live,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.error(e.span, e.message);
                }
                let field_tys: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.resolve_or_report(&f.ty, f.span);
                        (f.name.clone(), ty)
                    })
                    .collect();
                let field_public: Vec<bool> = fields.iter().map(|f| f.public).collect();
                self.structs.insert(
                    name.clone(),
                    StructSig {
                        fields: field_tys,
                        field_public,
                        origin: self.origin(public),
                    },
                );
                self.record_origin(name, public);
                let receiver_ty = Type {
                    kind: TypeKind::Struct(name.clone()),
                    span: *span,
                };
                for method in methods {
                    let Item::Fn {
                        name: method_name,
                        type_params,
                        params,
                        ret,
                        body: _,
                        span: method_span,
                    } = unwrap_public(method)
                    else {
                        self.error(*span, "an `object` holds only methods");
                        continue;
                    };
                    if !type_params.is_empty() {
                        self.error(
                            *method_span,
                            "a method cannot have type parameters yet",
                        );
                        continue;
                    }
                    let sig = self.method_signature(
                        name,
                        method_name,
                        params,
                        ret,
                        &receiver_ty,
                        *method_span,
                    );
                    let mangled = format!("{name}.{method_name}");
                    if let Some(previous) = self.methods.insert(mangled.clone(), sig) {
                        if previous.name == *method_name {
                            self.error(
                                *method_span,
                                format!("`{name}` already has a method named `{method_name}`"),
                            );
                        }
                    }
                }
            }
            Item::Struct { name, fields, span } => {
                let binding = Binding {
                    ty: Type {
                        kind: TypeKind::Struct(name.clone()),
                        span: *span,
                    },
                    mutability: Mutability::Const,
                    span: *span,
                    ownership: Ownership::Live,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.error(e.span, e.message);
                }
                let field_tys: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.resolve_or_report(&f.ty, f.span);
                        // A function value in a struct would be copied into
                        // the aggregate without being counted, and the
                        // aggregate has no way to release it: the copy would
                        // outlive the environment it points at.
                        if matches!(ty.kind, TypeKind::Func(_, _)) {
                            self.error(
                                f.span,
                                "a struct field cannot have a function type yet",
                            );
                        }
                        (f.name.clone(), ty)
                    })
                    .collect();
                let field_public: Vec<bool> = fields.iter().map(|f| f.public).collect();
                self.structs.insert(
                    name.clone(),
                    StructSig {
                        fields: field_tys,
                        field_public,
                        origin: self.origin(public),
                    },
                );
                self.record_origin(name, public);
            }
            Item::Enum {
                name,
                variants,
                span,
            } => {
                let binding = Binding {
                    ty: Type {
                        kind: TypeKind::Enum(name.clone()),
                        span: *span,
                    },
                    mutability: Mutability::Const,
                    span: *span,
                    ownership: Ownership::Live,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.error(e.span, e.message);
                }
                for variant in variants {
                    for field in &variant.fields {
                        let ty = self.resolve_or_report(field.ty(), field.span());
                        // The same reason a struct field cannot have one:
                        // an enum value is an aggregate, and an aggregate
                        // does not count the references it holds.
                        if matches!(ty.kind, TypeKind::Func(_, _)) {
                            self.error(
                                field.span(),
                                "an enum field cannot have a function type yet",
                            );
                        }
                    }
                }
                self.enums.insert(
                    name.clone(),
                    EnumSig {
                        variants: variants.clone(),
                    },
                );
                self.record_origin(name, public);
            }
            Item::Extern { decls, .. } => {
                for decl in decls {
                    let param_tys: Vec<Type> = decl
                        .params
                        .iter()
                        .map(|p| self.resolve_or_report_in(&p.ty, decl.span, true, false))
                        .collect();
                    let ret_ty = match &decl.ret {
                        Some(ty) => self.resolve_or_report_in(ty, decl.span, true, true),
                        None => prim(TypeKind::Unit),
                    };
                    let sig = FnSig {
                        params: param_tys.clone(),
                        ret: ret_ty.clone(),
                    };
                    let binding = Binding {
                        ty: Type {
                            kind: TypeKind::Func(param_tys, Box::new(ret_ty)),
                            span: decl.span,
                        },
                        mutability: Mutability::Const,
                        span: decl.span,
                        ownership: Ownership::Live,
                    };
                    if let Err(e) = self.scope.declare(decl.name.clone(), binding) {
                        self.error(e.span, e.message);
                    }
                    self.fns.insert(decl.name.clone(), sig);
                    self.externs.insert(decl.name.clone());
                }
            }
            Item::Import { .. } => {}
            Item::ConstFn {
                name,
                params,
                ret,
                body,
                span,
            } => {
                // A `const fn` is declared here and *checked* here too:
                // its body is the restricted language, and the checker
                // that owns the declaration table is the only stage that
                // knows which names are `const fn`s.
                let param_types: Vec<(String, Type)> = params
                    .iter()
                    .map(|param| {
                        let ty = self.resolve_or_report(&param.ty, param.span);
                        (param.name.clone(), ty)
                    })
                    .collect();
                let ret_ty = match ret {
                    Some(annotation) => self.resolve_or_report(annotation, *span),
                    None => Type {
                        kind: TypeKind::Unit,
                        span: *span,
                    },
                };
                // A `const fn` has no run-time existence, so its name is
                // not in the root scope: there is no function of that
                // name for a call to reach.
                if self.const_fns.contains_key(name) {
                    self.error(
                        *span,
                        format!("duplicate definition of `{name}`"),
                    );
                }
                self.const_fns.insert(
                    name.clone(),
                    ConstFn {
                        name: name.clone(),
                        params: param_types,
                        ret: ret_ty,
                        body: body.clone(),
                        span: *span,
                        file: self.file,
                    },
                );
                self.record_origin(name, public);
            }
        }
    }
    /// Resolve a type in a declaration, with the FFI edge's context rules.
    ///
    /// `allow_c` turns on the C spellings; `is_return` turns on `void`. The two
    /// are separate because a `void` return is the one place the type is legal,
    /// and a parameter list needs to say so even when it is otherwise full of C
    /// types: `extern fn f(x: void)` is the mistake this catches.
    fn resolve_or_report_in(
        &mut self,
        ty: &TypeExpr,
        fallback_span: Span,
        allow_c: bool,
        is_return: bool,
    ) -> Type {
        match self.resolve_type_in(ty, allow_c) {
            Ok(resolved) => {
                if !is_return && is_void_value(&resolved.kind) {
                    self.error(
                        type_expr_span(ty),
                        "`void` is only allowed as the return type of an `extern` declaration"
                            .to_string(),
                    );
                    never(fallback_span)
                } else {
                    resolved
                }
            }
            Err(e) => {
                self.error(e.span, e.message);
                never(fallback_span)
            }
        }
    }

    /// Resolve a type expression, reporting a diagnostic and falling back to
    /// `Never` when the name is unknown.
    ///
    /// A silently swallowed resolution failure is how an unknown type name in
    /// a signature used to become `Never` and compile: the program was
    /// accepted and the mistake surfaced later as garbage IR. Reporting here
    /// is what makes `fn f(b: double)` a diagnostic rather than a `void`
    /// parameter.
    fn resolve_or_report(&mut self, ty: &TypeExpr, fallback_span: Span) -> Type {
        match self.resolve_type(ty) {
            Ok(resolved) => {
                // A named type is an item too, so it is private to the module
                // that declares it unless it is `pub`.
                if let TypeExpr::Name(name, span) = ty {
                    self.check_visibility(name, *span);
                }
                resolved
            }
            Err(e) => {
                self.error(e.span, e.message);
                never(fallback_span)
            }
        }
    }

    /// Resolve a type expression to a concrete type. `Option[T]` and
    /// `Result[T, E]` are resolved structurally, because their shape is
    /// self-describing and they need no scope entry.
    ///
    /// C-only spellings are rejected here: ordinary Otter code writes `i64`,
    /// never `i32`. An `extern` declaration resolves through
    /// [`CheckCtx::resolve_type_in`] with `allow_c` set.
    fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, TypeError> {
        self.resolve_type_in(ty, false)
    }

    /// Resolve a type expression, optionally allowing the C spellings.
    ///
    /// `allow_c` is true inside an `extern` declaration and in an `as` target,
    /// where the user is naming a C type for an explicit conversion. It changes two
    /// things: the FFI-only names (`i32`, `usize`, `void`, ...) resolve instead
    /// of being rejected, and `*T` / `*mut T` / `*const T` all resolve to
    /// [`TypeKind::CPtr`] instead of an owning Otter pointer.
    fn resolve_type_in(&self, ty: &TypeExpr, allow_c: bool) -> Result<Type, TypeError> {
        match ty {
            TypeExpr::Name(name, span) => {
                // A type parameter, while its generic body is being checked.
                // It shadows everything: the body was written meaning the
                // instantiation's type wherever the parameter's name appears.
                if let Some(concrete) = self.type_subst.get(name) {
                    return Ok(Type {
                        kind: concrete.kind.clone(),
                        span: *span,
                    });
                }
                if !allow_c && TypeKind::is_c_only_spelling(name) {
                    return Err(TypeError::new(
                        *span,
                        format!(
                            "`{name}` is a C type and is only allowed in an `extern` \
                             declaration; ordinary Otter code uses `i64`, `f64`, `bool`, \
                             or `string`"
                        ),
                    ));
                }
                // `Self` is the type the method being checked belongs to.
                if name == "Self" {
                    if let Some(receiver) = &self.self_type {
                        return Ok(Type {
                            kind: receiver.kind.clone(),
                            span: *span,
                        });
                    }
                    return Err(TypeError::new(
                        *span,
                        "`Self` is only meaningful inside a method".to_string(),
                    ));
                }
                let kind = match name.as_str() {
                    "bool" => Some(TypeKind::Bool),
                    "string" => Some(TypeKind::Str),
                    "()" => Some(TypeKind::Unit),
                    "Mutex" => Some(TypeKind::Mutex),
                    "Channel" => {
                        return Err(TypeError::new(
                            *span,
                            "`Channel` takes an element type, as in `Channel[i64]`".to_string(),
                        ))
                    }
                    _ => {
                        if let Some(kind) = TypeKind::from_c_spelling(name) {
                            Some(kind)
                        } else if self.structs.contains_key(name) {
                            Some(TypeKind::Struct(name.clone()))
                        } else if self.enums.contains_key(name) {
                            Some(TypeKind::Enum(name.clone()))
                        } else {
                            None
                        }
                    }
                };
                match kind {
                    Some(kind) => Ok(Type { kind, span: *span }),
                    None => Err(TypeError::new(*span, format!("cannot find type `{name}`"))),
                }
            }
            TypeExpr::Option(inner, span) => {
                let inner = self.resolve_type_in(inner, allow_c)?;
                Ok(Type {
                    kind: TypeKind::Option(Box::new(inner)),
                    span: *span,
                })
            }
            TypeExpr::Result(ok, err, span) => {
                let ok = self.resolve_type_in(ok, allow_c)?;
                let err = self.resolve_type_in(err, allow_c)?;
                Ok(Type {
                    kind: TypeKind::Result(Box::new(ok), Box::new(err)),
                    span: *span,
                })
            }
            TypeExpr::Channel(element, span) => {
                let element = self.resolve_type_in(element, allow_c)?;
                if matches!(
                    element.kind,
                    TypeKind::Unit | TypeKind::Never | TypeKind::CVoid
                ) {
                    return Err(TypeError::new(
                        *span,
                        format!(
                            "a channel cannot carry `{element}`: its slots would have no size"
                        ),
                    ));
                }
                if linearity_of(&element.kind) == Linearity::Linear {
                    return Err(TypeError::new(
                        element.span,
                        format!(
                            "a channel cannot carry `{element}` yet: the runtime copies each \
                             value into a slot, so an owned value would have two owners"
                        ),
                    ));
                }
                if matches!(element.kind, TypeKind::Str) {
                    return Err(TypeError::new(
                        element.span,
                        "a channel cannot carry `string` yet: a slot copy would share one \
                         refcount between the sender and the receiver"
                            .to_string(),
                    ));
                }
                Ok(Type {
                    kind: TypeKind::Channel(Box::new(element)),
                    span: *span,
                })
            }
            TypeExpr::Ptr {
                kind,
                pointee,
                span,
            } => {
                if !allow_c && *kind != PtrKind::Mut {
                    return Err(TypeError::new(
                        *span,
                        "`*const T` and `*T` are C pointer spellings and are only allowed in \
                         an `extern` declaration; ordinary Otter code writes `*mut T`"
                            .to_string(),
                    ));
                }
                let inner = self.resolve_type_in(pointee, allow_c)?;
                if allow_c {
                    Ok(Type {
                        kind: TypeKind::CPtr(Box::new(inner)),
                        span: *span,
                    })
                } else {
                    Ok(Type {
                        kind: TypeKind::Ptr(Box::new(inner)),
                        span: *span,
                    })
                }
            }
            TypeExpr::FixedArray(element, length, span) => {
                let inner = self.resolve_type_in(element, allow_c)?;
                if !fixed_array_element_is_supported(&inner.kind) {
                    return Err(TypeError::new(
                        *span,
                        format!(
                            "a fixed array cannot hold `{inner}`: a fixed array is copied by \
                             copying its bytes, which is exact for the built-in number and \
                             `bool` types and not yet defined for anything that owns a reference"
                        ),
                    ));
                }
                if *length == 0 {
                    return Err(TypeError::new(
                        *span,
                        "a fixed array needs at least one element: its length is part of its \
                         type, and a zero-length array has no value to store"
                            .to_string(),
                    ));
                }
                Ok(Type {
                    kind: TypeKind::FixedArray(Box::new(inner), *length),
                    span: *span,
                })
            }
            TypeExpr::View(_, span) => {
                // A view is produced by `a[start..end]` and inferred there. It
                // borrows storage in the current frame, and Otter has no
                // lifetime syntax to say which storage that is, so a written
                // `[T]` could not be checked: a parameter or a return type
                // would let the view outlive its source. The type exists
                // internally and in messages; it is not a spelling.
                Err(TypeError::new(
                    *span,
                    "a view type cannot be written: `[T]` is the type of `a[start..end]`, \
                     which borrows `a` and stays in the function that made it"
                        .to_string(),
                ))
            }
            TypeExpr::Slice(element, span) => {
                let inner = self.resolve_type_in(element, allow_c)?;
                if matches!(
                    inner.kind,
                    TypeKind::Unit | TypeKind::Never | TypeKind::CVoid
                ) {
                    return Err(TypeError::new(
                        *span,
                        format!(
                            "an array cannot hold `{inner}`: its elements would have no size"
                        ),
                    ));
                }
                Ok(Type {
                    kind: TypeKind::Slice(Box::new(inner)),
                    span: *span,
                })
            }
            TypeExpr::Map(key, value, span) => {
                let key = self.resolve_type_in(key, allow_c)?;
                let value = self.resolve_type_in(value, allow_c)?;
                if !map_key_is_supported(&key.kind) {
                    return Err(TypeError::new(
                        key.span,
                        format!(
                            "a map key must be `i64`, `bool`, or `string`, found `{key}`: \
                             those are the kinds the runtime can hash"
                        ),
                    ));
                }
                if !map_value_is_supported(&value.kind) {
                    return Err(TypeError::new(
                        value.span,
                        format!(
                            "a map value must be a number or `bool` for now, found `{value}`: \
                             an owned value would need a destructor on removal"
                        ),
                    ));
                }
                Ok(Type {
                    kind: TypeKind::Map(Box::new(key), Box::new(value)),
                    span: *span,
                })
            }
            TypeExpr::Fn { params, ret, span } => {
                let params = params
                    .iter()
                    .map(|param| self.resolve_type_in(param, allow_c))
                    .collect::<Result<Vec<_>, _>>()?;
                let ret = self.resolve_type_in(ret, allow_c)?;
                Ok(Type {
                    kind: TypeKind::Func(params, Box::new(ret)),
                    span: *span,
                })
            }
            TypeExpr::Tuple(items, span) => {
                let elements = items
                    .iter()
                    .map(|item| self.resolve_type_in(item, allow_c))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type {
                    kind: TypeKind::Tuple(elements),
                    span: *span,
                })
            }
            TypeExpr::Unit(span) => Ok(Type {
                kind: TypeKind::Unit,
                span: *span,
            }),
        }
    }

    /// Check one function body against concrete parameter and return types.
    ///
    /// Shared by ordinary functions and by each instantiation of a generic
    /// one: the two differ only in where the types come from.
    fn check_fn_body(
        &mut self,
        name: &str,
        params: &[Param],
        param_tys: &[Type],
        ret_ty: &Type,
        body: &Block,
        span: Span,
    ) -> Option<HirFn> {
        self.scope.push();
        for (param, ty) in params.iter().zip(param_tys.iter()) {
            let _ = self.scope.declare(
                param.name.clone(),
                Binding {
                    ty: ty.clone(),
                    mutability: Mutability::Const,
                    span: param.span,
                    ownership: Ownership::Live,
                },
            );
        }
        let outer_ret = self.fn_ret.replace(ret_ty.clone());
        // A view cannot leave the function that made it, so the borrows start
        // and end with the body. Lambdas are checked inside it and their own
        // borrows are cleared here too; a view in a lambda is over by the time
        // the closure returns, and holding the source for the rest of the
        // enclosing function is the conservative answer.
        let outer_borrows = std::mem::take(&mut self.borrowed_bindings);
        let checked = self.check_block_with_ret(body, ret_ty);
        self.borrowed_bindings = outer_borrows;
        self.fn_ret = outer_ret;
        self.scope.pop();
        let body = checked?;
        // A function that returns a value has to return one on every path out
        // of its body.
        if !matches!(ret_ty.kind, TypeKind::Unit | TypeKind::Never)
            && !block_always_returns(&body)
        {
            self.error(span, "missing return");
        }
        Some(HirFn {
            name: name.to_string(),
            params: params
                .iter()
                .zip(param_tys.iter())
                .map(|(param, ty)| HirParam {
                    name: param.name.clone(),
                    ty: ty.clone(),
                    span: param.span,
                })
                .collect(),
            ret: ret_ty.clone(),
            body,
            captures: Vec::new(),
            span,
        })
    }

    /// Check a call to a generic function and record its instantiation.
    ///
    /// The arguments are checked first, without an expected type: the expected
    /// type is what the call is trying to discover. Each argument's type is
    /// then unified with the parameter's declared type, which binds the type
    /// parameters; a parameter that stays unbound is an error at the call.
    fn check_generic_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let Some(generic) = self.generics.get(name).cloned() else {
            return None;
        };
        if generic.params.len() != args.len() {
            self.error(
                span,
                format!(
                    "expected {} arguments, found {}",
                    generic.params.len(),
                    args.len()
                ),
            );
            return None;
        }
        let mut subst: HashMap<String, Type> = HashMap::new();
        let mut checked_args = Vec::with_capacity(args.len());
        for (arg, param) in args.iter().zip(generic.params.iter()) {
            let checked = self.check_expr(arg)?;
            let actual = expr_ty(&checked);
            if !self.unify_type(&param.ty, &generic.type_params, &mut subst, &actual) {
                self.error(
                    expr_span(&checked),
                    format!(
                        "expected `{}`, found `{actual}`",
                        type_expr_name(&param.ty)
                    ),
                );
            }
            checked_args.push(checked);
        }
        // Every type parameter has to be pinned down by an argument. One that
        // appears only in the return type, or nowhere at all, has no answer.
        let mut ordered = Vec::with_capacity(generic.type_params.len());
        for (param, _) in &generic.type_params {
            match subst.get(param) {
                Some(concrete) => ordered.push(concrete.clone()),
                None => {
                    self.error(span, format!("cannot infer type parameter `{param}`"));
                    return None;
                }
            }
        }
        // Resolve the signature at the inferred substitution. The map is
        // installed only for the duration of the lookup: nothing outside a
        // generic body should see `T`.
        let outer_subst = std::mem::take(&mut self.type_subst);
        self.type_subst = generic
            .type_params
            .iter()
            .zip(ordered.iter())
            .map(|((param, _), concrete)| (param.clone(), concrete.clone()))
            .collect();
        let param_tys: Vec<Type> = generic
            .params
            .iter()
            .map(|param| self.resolve_or_report(&param.ty, param.span))
            .collect();
        let ret_ty = match &generic.ret {
            Some(ty) => self.resolve_or_report(ty, span),
            None => prim(TypeKind::Unit),
        };
        self.type_subst = outer_subst;
        // A parameter of linear type takes ownership, exactly as it does in an
        // ordinary call: passing a bare name moves it out.
        for (arg, param_ty) in args.iter().zip(param_tys.iter()) {
            if matches!(linearity_of(&param_ty.kind), Linearity::Linear) {
                if let Expr::Ident(path, arg_span) = arg {
                    let arg_name = path.last().cloned().unwrap_or_default();
                    if let Err(e) = self.move_out(&arg_name, *arg_span) {
                        self.error(e.span, e.message);
                    }
                }
            }
        }
        let mangled = self.queue_instantiation(name, &ordered, span);
        Some(HirExpr::Call {
            callee: vec![mangled],
            args: checked_args,
            unsafe_call: false,
            span,
            ty: ret_ty,
        })
    }

    /// Unify a declared parameter type with an argument's type, binding type
    /// parameters as they are found.
    ///
    /// This is a plain structural walk rather than full inference: a type
    /// parameter binds to whatever the argument has, and a repeated parameter
    /// has to have the same answer both times. There are no constraints and no
    /// bounds, so that is the whole algorithm.
    fn unify_type(
        &self,
        declared: &TypeExpr,
        params: &[(String, Span)],
        subst: &mut HashMap<String, Type>,
        actual: &Type,
    ) -> bool {
        match declared {
            TypeExpr::Name(name, _) if params.iter().any(|(param, _)| param == name) => {
                match subst.get(name) {
                    Some(bound) => bound.eq_ignoring_span(actual),
                    None => {
                        subst.insert(name.clone(), actual.clone());
                        true
                    }
                }
            }
            // Anything that is not a type parameter is a concrete type, and it
            // has to equal what the argument is.
            TypeExpr::Name(_, _) => self
                .resolve_type(declared)
                .is_ok_and(|ty| ty.eq_ignoring_span(actual)),
            TypeExpr::Slice(element, _) => match &actual.kind {
                TypeKind::Slice(actual_element) => {
                    self.unify_type(element, params, subst, actual_element)
                }
                _ => false,
            },
            TypeExpr::FixedArray(element, length, _) => match &actual.kind {
                TypeKind::FixedArray(actual_element, actual_length) => {
                    actual_length == length
                        && self.unify_type(element, params, subst, actual_element)
                }
                _ => false,
            },
            TypeExpr::View(element, _) => match &actual.kind {
                TypeKind::View(actual_element) => {
                    self.unify_type(element, params, subst, actual_element)
                }
                _ => false,
            },
            TypeExpr::Map(key, value, _) => match &actual.kind {
                TypeKind::Map(actual_key, actual_value) => {
                    self.unify_type(key, params, subst, actual_key)
                        && self.unify_type(value, params, subst, actual_value)
                }
                _ => false,
            },
            TypeExpr::Option(element, _) => match &actual.kind {
                TypeKind::Option(actual_element) => {
                    self.unify_type(element, params, subst, actual_element)
                }
                _ => false,
            },
            TypeExpr::Result(ok, err, _) => match &actual.kind {
                TypeKind::Result(actual_ok, actual_err) => {
                    self.unify_type(ok, params, subst, actual_ok)
                        && self.unify_type(err, params, subst, actual_err)
                }
                _ => false,
            },
            TypeExpr::Channel(element, _) => match &actual.kind {
                TypeKind::Channel(actual_element) => {
                    self.unify_type(element, params, subst, actual_element)
                }
                _ => false,
            },
            TypeExpr::Ptr { pointee, .. } => match &actual.kind {
                TypeKind::Ptr(actual_pointee) => {
                    self.unify_type(pointee, params, subst, actual_pointee)
                }
                _ => false,
            },
            TypeExpr::Fn { params: declared_params, ret, .. } => match &actual.kind {
                TypeKind::Func(actual_params, actual_ret) => {
                    declared_params.len() == actual_params.len()
                        && declared_params
                            .iter()
                            .zip(actual_params.iter())
                            .all(|(declared, actual)| {
                                self.unify_type(declared, params, subst, actual)
                            })
                        && self.unify_type(ret, params, subst, actual_ret)
                }
                _ => false,
            },
            TypeExpr::Unit(_) => matches!(actual.kind, TypeKind::Unit),
            TypeExpr::Tuple(items, _) => match &actual.kind {
                TypeKind::Tuple(elements) => {
                    items.len() == elements.len()
                        && items
                            .iter()
                            .zip(elements.iter())
                            .all(|(declared, actual)| {
                                self.unify_type(declared, params, subst, actual)
                            })
                }
                _ => false,
            },
        }
    }

    /// Record an instantiation, and answer with the symbol it will have.
    ///
    /// The same substitution is recorded once however many calls find it. A
    /// call from inside the function's own instantiation is allowed when the
    /// substitution is unchanged -- ordinary recursion -- and rejected when it
    /// is not: polymorphic recursion would need an unbounded family of
    /// functions, so there is no finite answer to emit.
    fn queue_instantiation(&mut self, name: &str, subst: &[Type], span: Span) -> String {
        let mangled = mangled_generic_name(name, subst);
        if self.instantiated.contains(&mangled) {
            return mangled;
        }
        if let Some((_, active)) = self
            .active_generics
            .iter()
            .find(|(active_name, _)| active_name == name)
        {
            if active.len() != subst.len()
                || !active
                    .iter()
                    .zip(subst.iter())
                    .all(|(active, actual)| active.eq_ignoring_span(actual))
            {
                self.error(
                    span,
                    format!(
                        "`{name}` calls itself with a different type: polymorphic \
                         recursion has no finite instantiation"
                    ),
                );
                return mangled;
            }
        }
        self.instantiated.insert(mangled.clone());
        self.pending_generics.push(Instantiation {
            name: name.to_string(),
            subst: subst.to_vec(),
            span,
        });
        mangled
    }

    /// Check every instantiation the program asked for, appending the concrete
    /// functions to `items`.
    ///
    /// This is monomorphization's second half. The worklist is what makes it
    /// terminate without recursion: an instantiation's body can ask for more,
    /// and they are appended to the same list. `queue_instantiation` has
    /// already refused polymorphic recursion, so the list is finite.
    fn instantiate_generics(&mut self, items: &mut Vec<HirItem>) {
        let outer_file = self.file;
        while let Some(job) = self.pending_generics.pop() {
            let Some(generic) = self.generics.get(&job.name).cloned() else {
                continue;
            };
            self.file = generic.file;
            self.type_subst = generic
                .type_params
                .iter()
                .zip(job.subst.iter())
                .map(|((param, _), concrete)| (param.clone(), concrete.clone()))
                .collect();
            let param_tys: Vec<Type> = generic
                .params
                .iter()
                .map(|param| self.resolve_or_report(&param.ty, param.span))
                .collect();
            let ret_ty = match &generic.ret {
                Some(ty) => self.resolve_or_report(ty, job.span),
                None => prim(TypeKind::Unit),
            };
            let mangled = mangled_generic_name(&job.name, &job.subst);
            self.active_generics.push((job.name.clone(), job.subst.clone()));
            let checked = self.check_fn_body(
                &mangled,
                &generic.params,
                &param_tys,
                &ret_ty,
                &generic.body,
                generic.span,
            );
            self.active_generics.pop();
            self.type_subst.clear();
            if let Some(hir) = checked {
                items.push(HirItem::Fn(hir));
            }
        }
        self.file = outer_file;
    }

    /// Build one item's HIR. `Fn` bodies are checked here.
    ///
    /// An `impl` block holds several methods, so the answer is a list: each
    /// method is an ordinary function item in the HIR, with a mangled name.
    fn check_item(&mut self, item: &Item) -> Option<Vec<HirItem>> {
        match item {
            // `pub` is a modifier in front of an item: the item inside is
            // checked as usual. The visibility was settled in the declaration
            // pass, where the item's name was recorded.
            Item::Public { item, .. } => self.check_item(item),
            Item::Fn {
                name,
                type_params,
                params,
                ret: _,
                body,
                span,
            } => {
                // A generic body mentions type variables, so it is checked at
                // each call's substitution -- see `instantiate_generics` -- and
                // there is no signature to look up here.
                if !type_params.is_empty() {
                    return None;
                }
                let sig = self.fns.get(name).cloned()?;
                let checked = self.check_fn_body(name, params, &sig.params, &sig.ret, body, *span)?;
                Some(vec![HirItem::Fn(checked)])
            }
            Item::Struct { name, fields, span } => {
                let sig = self.structs.get(name).cloned()?;
                Some(vec![HirItem::Struct(HirStruct {
                    name: name.clone(),
                    fields: fields
                        .iter()
                        .zip(sig.fields.iter())
                        .map(|(f, (_, ty))| HirFieldDef {
                            name: f.name.clone(),
                            ty: ty.clone(),
                            span: f.span,
                        })
                        .collect(),
                    span: *span,
                })])
            }
            Item::Enum {
                name,
                variants,
                span,
            } => Some(vec![HirItem::Enum(HirEnum {
                name: name.clone(),
                variants: variants
                    .iter()
                    .map(
                        |VariantDef {
                             name: vname,
                             fields,
                             span: vspan,
                         }| HirVariantDef {
                            name: vname.clone(),
                            fields: fields
                                .iter()
                                .map(|f| HirVariantField {
                                    name: match f {
                                        VariantField::Named { name, .. } => Some(name.clone()),
                                        VariantField::Positional { .. } => None,
                                    },
                                    ty: self.resolve_type(f.ty()).unwrap_or_else(|_| never(*vspan)),
                                    span: f.span(),
                                })
                                .collect(),
                            span: *vspan,
                        },
                    )
                    .collect(),
                span: *span,
            })]),
            Item::Import { path, span } => Some(vec![HirItem::Import(HirImport {                path: path.clone(),
                span: *span,
            })]),
            Item::Extern {
                abi,
                link,
                decls,
                span,
            } => Some(vec![HirItem::Extern(HirExternBlock {
                abi: abi.clone(),
                link: link.clone(),
                decls: decls
                    .iter()
                    .map(|decl| HirExternDecl {
                        name: decl.name.clone(),
                        params: decl
                            .params
                            .iter()
                            .map(|p| HirParam {
                                name: p.name.clone(),
                                ty: self
                                    .resolve_type_in(&p.ty, true)
                                    .unwrap_or_else(|_| never(p.span)),
                                span: p.span,
                            })
                            .collect(),
                        ret: match &decl.ret {
                            Some(ty) => {
                                self.resolve_type_in(ty, true).unwrap_or_else(|_| never(decl.span))
                            }
                            None => prim(TypeKind::Unit),
                        },
                        span: decl.span,
                    })
                    .collect(),
                span: *span,
            })]),
            Item::Impl {
                type_name,
                methods,
                span: _,
            } => self.check_impl_methods(type_name, methods),
            Item::ConstFn { name, .. } => {
                // The body is checked here and the function is *not* emitted:
                // a `const fn` exists only to be computed by the compiler, so
                // there is no run-time symbol for it.
                self.check_const_fn_body(name);
                None
            }
            Item::Object {
                name,
                fields,
                methods,
                span,
            } => {
                // The type, and then the methods: an object is a struct and
                // an `impl` block in one declaration.
                let sig = self.structs.get(name).cloned()?;
                let mut items = vec![HirItem::Struct(HirStruct {
                    name: name.clone(),
                    fields: fields
                        .iter()
                        .zip(sig.fields.iter())
                        .map(|(f, (_, ty))| HirFieldDef {
                            name: f.name.clone(),
                            ty: ty.clone(),
                            span: f.span,
                        })
                        .collect(),
                    span: *span,
                })];
                if let Some(methods) = self.check_impl_methods(name, methods) {
                    items.extend(methods);
                }
                Some(items)
            }
        }
    }

    /// Check the methods of an `impl` block.
    ///
    /// Each method is an ordinary function with a receiver: its first
    /// parameter is the receiver, and `Self` inside it is the type the block
    /// is for. The name the backend emits is the mangled `Type.method`, which
    /// is what the call site resolves to.
    fn check_impl_methods(
        &mut self,
        type_name: &str,
        methods: &[Item],
    ) -> Option<Vec<HirItem>> {
        let Some(receiver_ty) = self.declared_type(type_name) else {
            return None;
        };
        let mut items = Vec::new();
        for method in methods {
            let Item::Fn {
                name,
                type_params,
                params,
                ret,
                body,
                span,
            } = unwrap_public(method)
            else {
                continue;
            };
            if !type_params.is_empty() {
                self.error(*span, "a method cannot have type parameters yet");
                continue;
            }
            let sig = self.method_signature(
                type_name,
                name,
                params,
                ret,
                &receiver_ty,
                *span,
            );
            let ret_ty = sig.ret.clone();
            self.scope.push();
            for (param, param_ty) in params.iter().zip(sig.params.iter()) {
                let _ = self.scope.declare(
                    param.name.clone(),
                    Binding {
                        ty: param_ty.clone(),
                        mutability: Mutability::Const,
                        span: param.span,
                        ownership: Ownership::Live,
                    },
                );
            }
            let outer_self_type = self.self_type.replace(receiver_ty.clone());
            let outer_ret = self.fn_ret.replace(ret_ty.clone());
            let checked = self.check_block_with_ret(body, &ret_ty);
            self.fn_ret = outer_ret;
            self.self_type = outer_self_type;
            self.scope.pop();
            let body = checked?;
            // A method that returns a value has to return one on every path.
            if !matches!(ret_ty.kind, TypeKind::Unit | TypeKind::Never)
                && !block_always_returns(&body)
            {
                self.error(*span, "missing return");
            }
            items.push(HirItem::Fn(HirFn {
                name: format!("{type_name}.{name}"),
                params: params
                    .iter()
                    .zip(sig.params.iter())
                    .map(|(param, param_ty)| HirParam {
                        name: param.name.clone(),
                        ty: param_ty.clone(),
                        span: param.span,
                    })
                    .collect(),
                ret: ret_ty,
                body,
                captures: Vec::new(),
                span: *span,
            }));
        }
        Some(items)
    }

    /// Check a block in which `return` must yield `ret`.
    ///
    /// Every block in a function body — the body itself, an `if` arm, a loop
    /// body, a `defer` body — is checked with the *function's* return type, so
    /// threading `ret` down is what lets a `return` nested inside them mean
    /// what it says.
    fn check_block_with_ret(&mut self, block: &Block, ret: &Type) -> Option<HirBlock> {
        self.scope.push();
        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            let checked = self.check_stmt(stmt, ret)?;
            stmts.push(checked);
        }
        self.scope.pop();
        Some(HirBlock {
            stmts,
            span: block.span,
        })
    }

    /// Fold a `const` initializer that calls a `const fn` into the value the
    /// compiler computed.
    ///
    /// The result is a literal: the call is gone, and what is left is the
    /// number, the flag or the text it produced. A fault -- a division by
    /// zero, an overflow, a call that would not terminate -- is reported here,
    /// at the call, because that is where the value was wanted.
    fn fold_const_init(&mut self, init: &Expr, span: Span) -> Option<HirExpr> {
        // The initializer's own scope is empty: a `const fn` call is computed
        // from constants, so a name that is not one is a run-time value the
        // compiler cannot know.
        let mut work = const_eval::Work::default();
        match const_eval::evaluate(
            init,
            &[],
            &self.const_fns,
            &mut work,
            Missing::NotConstant,
        ) {
            Ok(value) => {
                // Every `const fn` the evaluation called has to be one this
                // module may use: a `const fn` belongs to its module like any
                // other item.
                for (name, call_span) in &work.called {
                    self.check_visibility(name, *call_span);
                }
                Some(value.to_expr(span))
            }
            Err(error) => {
                self.error(span, error.message);
                None
            }
        }
    }

    /// Check a `const fn` body against the restricted language.
    ///
    /// The body is never compiled, so this is the only check it gets: a
    /// `read_line` in it is a mistake the reader has to see, whether or not
    /// anything calls the function.
    fn check_const_fn_body(&mut self, name: &str) {
        let Some(function) = self.const_fns.get(name).cloned() else {
            return;
        };
        // The body belongs to the module that declared it, so a diagnostic
        // about it is attributed there and not to the module being checked.
        let outer = self.file;
        self.file = function.file;
        match const_eval::check_body(&function, &self.const_fns) {
            Ok(called) => {
                // A `const fn` body may call another `const fn`, and that one
                // has to be one this module may use.
                for (name, call_span) in &called {
                    self.check_visibility(name, *call_span);
                }
            }
            Err(error) => self.error(error.span, error.message),
        }
        self.file = outer;
    }

    fn check_stmt(&mut self, stmt: &Stmt, ret: &Type) -> Option<HirStmt> {
        match stmt {
            Stmt::Expr(expr, span) => {
                let checked = self.check_expr(expr)?;
                let ty = expr_ty(&checked);
                Some(HirStmt::Expr(checked, *span, ty))
            }
            Stmt::Const {
                name,
                ty,
                init,
                span,
            } => {
                // The annotation is resolved *before* the initializer, because
                // it is the expected type the initializer is checked against.
                // Comparing types after the fact works for concrete values but
                // leaves a constructor with nothing to infer from — `const x:
                // Option[i64] = None;` was reported as an ambiguous `None`
                // rather than typed by its own annotation.
                let declared = match ty {
                    Some(annotation) => match self.resolve_type(annotation) {
                        Ok(declared) => Some(declared),
                        Err(error) => {
                            self.error(error.span, error.message);
                            return None;
                        }
                    },
                    None => None,
                };
                // A `const` initializer that calls a `const fn` is computed by
                // the compiler, not at run time: the call is folded into the
                // value it produces, and the function itself has no run-time
                // existence.
                let init_expr = if const_eval::mentions_const_fn(init, &self.const_fns) {
                    let Some(folded) = self.fold_const_init(init, *span) else {
                        return None;
                    };
                    folded
                } else {
                    match &declared {
                        Some(declared) => self.check_expr_with_expected(init, declared)?,
                        None => self.check_expr(init)?,
                    }
                };
                if let Some(declared) = &declared {
                    if !declared.eq_ignoring_span(&expr_ty(&init_expr)) {
                        self.error(
                            expr_span(&init_expr),
                            format!("expected `{declared}`, found `{}`", expr_ty(&init_expr)),
                        );
                    }
                }
                // A linear value may leave its binding only through `move`.
                self.forbid_linear_copy(init, &init_expr);
                if let Err(e) = self.scope.declare(
                    name.clone(),
                    Binding {
                        ty: expr_ty(&init_expr),
                        mutability: Mutability::Const,
                        span: *span,
                        ownership: Ownership::Live,
                    },
                ) {
                    self.error(e.span, e.message);
                }
                Some(HirStmt::Const {
                    name: name.clone(),
                    ty: declared,
                    init: init_expr,
                    mutability: Mutability::Const,
                    span: *span,
                })
            }
            Stmt::Var {
                name,
                ty,
                init,
                span,
            }
            | Stmt::Let {
                name,
                ty,
                init,
                span,
            } => {
                // Same ordering as `Stmt::Const` above: the annotation is the
                // initializer's expected type.
                let declared = match ty {
                    Some(annotation) => match self.resolve_type(annotation) {
                        Ok(declared) => Some(declared),
                        Err(error) => {
                            self.error(error.span, error.message);
                            return None;
                        }
                    },
                    None => None,
                };
                let init_expr = match &declared {
                    Some(declared) => self.check_expr_with_expected(init, declared)?,
                    None => self.check_expr(init)?,
                };
                if let Some(declared) = &declared {
                    if !declared.eq_ignoring_span(&expr_ty(&init_expr)) {
                        self.error(
                            expr_span(&init_expr),
                            format!("expected `{declared}`, found `{}`", expr_ty(&init_expr)),
                        );
                    }
                }
                // Same as `Stmt::Const`: a linear initializer has to say
                // `move`.
                self.forbid_linear_copy(init, &init_expr);
                if let Err(e) = self.scope.declare(
                    name.clone(),
                    Binding {
                        ty: expr_ty(&init_expr),
                        mutability: Mutability::Var,
                        span: *span,
                        ownership: Ownership::Live,
                    },
                ) {
                    self.error(e.span, e.message);
                }
                Some(HirStmt::Var {
                    name: name.clone(),
                    ty: declared,
                    init: init_expr,
                    mutability: Mutability::Var,
                    span: *span,
                })
            }
            Stmt::Destructure {
                kind,
                names,
                init,
                span,
            } => {
                let init_expr = self.check_expr(init)?;
                let init_ty = expr_ty(&init_expr);
                let TypeKind::Tuple(elements) = &init_ty.kind else {
                    self.error(
                        expr_span(&init_expr),
                        format!(
                            "cannot destructure `{init_ty}`: a destructuring pattern needs a \
                             tuple, and this has no elements to bind"
                        ),
                    );
                    return None;
                };
                if elements.len() != names.len() {
                    self.error(
                        *span,
                        format!(
                            "this pattern binds {} name{}, but the tuple has {} element{}",
                            names.len(),
                            if names.len() == 1 { "" } else { "s" },
                            elements.len(),
                            if elements.len() == 1 { "" } else { "s" }
                        ),
                    );
                    return None;
                }
                // A tuple is a copyable value, but the rule is still stated the
                // same way every other binding states it: a linear initializer
                // has to say `move`. Nothing a tuple can currently hold is
                // linear, so this is future-proofing rather than a live check.
                self.forbid_linear_copy(init, &init_expr);
                let mutability = match kind {
                    BindingKeyword::Const => Mutability::Const,
                    BindingKeyword::Var | BindingKeyword::Let => Mutability::Var,
                };
                let mut bindings = Vec::with_capacity(names.len());
                for (name, element) in names.iter().zip(elements.iter()) {
                    match name {
                        Some(name) => {
                            if let Err(error) = self.scope.declare(
                                name.clone(),
                                Binding {
                                    ty: element.clone(),
                                    mutability,
                                    span: *span,
                                    ownership: Ownership::Live,
                                },
                            ) {
                                self.error(error.span, error.message);
                            }
                            bindings.push(Some((name.clone(), element.clone())));
                        }
                        None => bindings.push(None),
                    }
                }
                Some(HirStmt::Destructure {
                    init: init_expr,
                    bindings,
                    mutability,
                    span: *span,
                })
            }
            Stmt::Assign {
                target,
                value,
                span,
            } => {
                // The parser left the target an expression on purpose, so the
                // question "what may be assigned to?" is answered here. Three
                // shapes are assignable: a name bound by `var` in a scope that
                // encloses this statement, an index into a mutable `*mut i64`
                // binding (`p[i] = v`), and a field of a `var` binding
                // (`p.x = v`, `p.inner.x = v`).
                //
                // `p[i] = v` and `p.x = v` are checked before the name case
                // because neither target is a bare name -- each is an access
                // expression whose base has to resolve to a mutable binding.
                if matches!(target, Expr::FieldAccess { .. }) {
                    return self.check_field_assign(target, value, *span);
                }
                // A dotted name arrives as one `Ident` with several segments
                // (`p.x`, `p.inner.x`), so a multi-segment path is a field
                // target too; a one-segment one is the plain binding case below.
                if let Expr::Ident(path, _) = target {
                    if path.len() > 1 {
                        return self.check_field_assign(target, value, *span);
                    }
                }
                if let Expr::Index { base, index, span } = target {
                    let base = self.check_expr(base)?;
                    let base_ty = expr_ty(&base);
                    if matches!(base_ty.kind, TypeKind::Slice(_)) {
                        let _ = self.check_expr(value);
                        self.error(
                            *span,
                            "cannot assign to an array element: an array does not \
                             expose its storage; use the `set` method",
                        );
                        return None;
                    }
                    let TypeKind::Ptr(pointee) = &base_ty.kind else {
                        let _ = self.check_expr(value);
                        self.error(*span, format!("cannot assign to index of `{}`", base_ty));
                        return None;
                    };
                    if matches!(pointee.kind, TypeKind::Unit | TypeKind::Never) {
                        let _ = self.check_expr(value);
                        self.error(
                            *span,
                            format!("a pointer to nothing cannot be written through: `*mut {pointee}`"),
                        );
                        return None;
                    }
                    // The element type is whatever the pointer points at: a
                    // pointer is an address, and an address does not care
                    // whether it holds one `i64` or one struct.
                    let element = (**pointee).clone();
                    let index = self.check_expr(index)?;
                    if !expr_ty(&index).eq_ignoring_span(&prim(TypeKind::I64)) {
                        let _ = self.check_expr(value);
                        self.error(expr_span(&index), "index must be `i64`");
                        return None;
                    }
                    let value = self.check_expr(value)?;
                    if !element.eq_ignoring_span(&expr_ty(&value)) {
                        self.error(
                            expr_span(&value),
                            format!("expected `{element}`, found `{}`", expr_ty(&value)),
                        );
                        return None;
                    }
                    return Some(HirStmt::IndexAssign {
                        base,
                        index,
                        value,
                        span: *span,
                    });
                }
                let name = match target {
                    Expr::Ident(path, _) => path.last().cloned().unwrap_or_default(),
                    other => {
                        // Check the value first: a bad value is worth reporting
                        // in the same pass as a bad target.
                        let _ = self.check_expr(value);
                        let span = otter_parser::exprs::expr_span(other);
                        self.error(span, "only a `var` binding can be assigned");
                        return None;
                    }
                };

                let binding = match self.lookup(&name, otter_parser::exprs::expr_span(target)) {
                    NameLookup::Found(binding) => binding,
                    // A capture holds a copy: assigning to it would change the
                    // copy and not the binding it came from.
                    NameLookup::Capture { .. } => {
                        let _ = self.check_expr(value);
                        self.error(
                            otter_parser::exprs::expr_span(target),
                            format!("cannot assign to `{name}`, which is captured"),
                        );
                        return None;
                    }
                    NameLookup::Missing => {
                        let _ = self.check_expr(value);
                        let span = otter_parser::exprs::expr_span(target);
                        self.error(span, format!("cannot find value `{name}` in this scope"));
                        return None;
                    }
                };

                if !binding.mutability.is_mutable() {
                    let span = otter_parser::exprs::expr_span(target);
                    self.error(span, format!("cannot assign to `{name}`, which is immutable"));
                }

                // A view points into this binding's storage. Rebinding would
                // free what the view covers and leave the pointer dangling, so
                // the binding stays put until the view is out of scope -- which
                // in Otter means until the function returns.
                self.reject_borrowed_write(&name, otter_parser::exprs::expr_span(target));

                let checked = self.check_expr_with_expected(value, &binding.ty)?;
                let found = expr_ty(&checked);
                if !binding.ty.eq_ignoring_span(&found) {
                    self.error(
                        expr_span(&checked),
                        format!("expected `{}`, found `{found}`", binding.ty),
                    );
                }
                // Re-initialising a `var` gives it a fresh owner, so whatever it
                // held before -- live or moved -- it owns something now. The
                // source may not be a bare linear name without `move`, for the
                // same reason as a declaration.
                //
                // Whether the old value still needs releasing is decided here,
                // where the ownership state is still known: `release_previous`
                // is true only when the slot holds a live allocation. A binding
                // that was already moved (by `dealloc(buf)`, or by passing it to
                // a function that takes `*mut i64`) is not released again.
                let release_previous = matches!(linearity_of(&binding.ty.kind), Linearity::Linear)
                    && matches!(binding.ownership, Ownership::Live);
                self.forbid_linear_copy(value, &checked);
                self.scope.rebind(&name);

                Some(HirStmt::Assign {
                    name,
                    value: checked,
                    ty: binding.ty,
                    release_previous,
                    span: *span,
                })
            }
            Stmt::Return(expr, span) => match expr {
                Some(e) => {
                    let checked = self.check_expr_with_expected(e, ret)?;
                    if !fits(ret, &expr_ty(&checked)) {
                        self.error(
                            expr_span(&checked),
                            format!("expected `{ret}`, found `{}`", expr_ty(&checked)),
                        );
                    }
                    Some(HirStmt::Return(Some(checked), *span))
                }
                None => {
                    if !ret.eq_ignoring_span(&prim(TypeKind::Unit)) {
                        self.error(*span, format!("expected `{ret}`, found `()`"));
                    }
                    Some(HirStmt::Return(None, *span))
                }
            },
            Stmt::If {
                init,
                cond,
                then_block,
                else_block,
                span,
            } => {
                let init = init
                    .as_ref()
                    .and_then(|s| self.check_stmt(s, ret))
                    .map(Box::new);
                let cond_expr = self.check_expr(cond)?;
                if !expr_ty(&cond_expr).eq_ignoring_span(&prim(TypeKind::Bool)) {
                    self.error(
                        expr_span(&cond_expr),
                        format!("expected `bool`, found `{}`", expr_ty(&cond_expr)),
                    );
                }
                // Nested blocks belong to the same function, so `return` inside
                // them must still yield the *function's* return type. Checking
                // them against `()` rejected every `return` inside an `if` or a
                // loop body with a bogus "expected `()`, found `i64`".
                // Ownership across branches: check each branch from the state on
                // entry, then treat a binding as moved after the `if` if *either*
                // branch moved it. Without the restore, the `then` branch's moves
                // would leak into the `else` branch's check and reject a program
                // that frees in both branches -- which is exactly the shape a
                // `defer`-less program has.
                let entry_moved = self.scope.moved_bindings();
                let then_block = self.check_block_with_ret(then_block, ret)?;
                let after_then = self.scope.moved_bindings();
                self.scope.apply_moved(&entry_moved);
                let else_block = else_block
                    .as_ref()
                    .and_then(|b| self.check_block_with_ret(b, ret));
                let after_else = if else_block.is_some() {
                    self.scope.moved_bindings()
                } else {
                    // No `else` means the false path changed nothing, so its
                    // moved-set is the entry state.
                    entry_moved.clone()
                };
                let merged: BTreeSet<(usize, String)> =
                    after_then.union(&after_else).cloned().collect();
                self.scope.apply_moved(&merged);
                Some(HirStmt::If {
                    init,
                    cond: cond_expr,
                    then_block,
                    else_block,
                    span: *span,
                })
            }
            Stmt::ForRange {
                name,
                start,
                end,
                body,
                span,
            } => {
                let start_expr = self.check_expr(start)?;
                let end_expr = self.check_expr(end)?;
                if !expr_ty(&start_expr).eq_ignoring_span(&prim(TypeKind::I64)) {
                    self.error(
                        expr_span(&start_expr),
                        format!("expected `i64`, found `{}`", expr_ty(&start_expr)),
                    );
                }
                if !expr_ty(&end_expr).eq_ignoring_span(&prim(TypeKind::I64)) {
                    self.error(
                        expr_span(&end_expr),
                        format!("expected `i64`, found `{}`", expr_ty(&end_expr)),
                    );
                }
                self.scope.push();
                let _ = self.scope.declare(
                    name.clone(),
                    Binding {
                        ty: prim(TypeKind::I64),
                        mutability: Mutability::Const,
                        span: *span,
                        ownership: Ownership::Live,
                    },
                );
                let body = self.check_block_with_ret(body, ret)?;
                self.scope.pop();
                Some(HirStmt::ForRange {
                    name: name.clone(),
                    start: start_expr,
                    end: end_expr,
                    body,
                    span: *span,
                })
            }
            Stmt::ForIn {
                name,
                iterable,
                body,
                span,
            } => {
                let binding = match iterable {
                    Expr::Ident(path, _) if path.len() == 1 => {
                        Some(path.last().cloned().unwrap_or_default())
                    }
                    _ => None,
                };
                let iterable_expr = match self.check_expr(iterable) {
                    Some(checked) => checked,
                    None => return None,
                };
                let iterable_ty = expr_ty(&iterable_expr);
                let element = match &iterable_ty.kind {
                    TypeKind::Slice(element)
                    | TypeKind::FixedArray(element, _)
                    | TypeKind::View(element) => (**element).clone(),
                    _ => {
                        self.error(
                            *span,
                            format!(
                                "`for` over a value must iterate an array, found `{iterable_ty}`"
                            ),
                        );
                        return None;
                    }
                };
                let Some(binding) = binding else {
                    self.error(
                        *span,
                        "`for` needs an array binding to iterate, not a temporary or a field",
                    );
                    return None;
                };
                if !slice_element_is_operable(&element.kind) {
                    self.error(*span, unsupported_element_message(&element));
                    return None;
                }
                self.scope.push();
                let _ = self.scope.declare(
                    name.clone(),
                    Binding {
                        ty: element,
                        mutability: Mutability::Const,
                        span: *span,
                        ownership: Ownership::Live,
                    },
                );
                self.iteration_borrows.push(binding);
                let body = self.check_block_with_ret(body, ret);
                self.iteration_borrows.pop();
                self.scope.pop();
                let body = body?;
                Some(HirStmt::ForIn {
                    name: name.clone(),
                    iterable: iterable_expr,
                    body,
                    span: *span,
                })
            }
            Stmt::ForC {
                init,
                cond,
                update,
                body,
                span,
            } => {
                let init = init
                    .as_ref()
                    .and_then(|s| self.check_stmt(s, ret))
                    .map(Box::new);
                let cond_expr = cond.as_ref().and_then(|c| self.check_expr(c));
                if let Some(c) = &cond_expr {
                    if !expr_ty(c).eq_ignoring_span(&prim(TypeKind::Bool)) {
                        self.error(
                            expr_span(c),
                            format!("expected `bool`, found `{}`", expr_ty(c)),
                        );
                    }
                }
                let update_expr = update.as_ref().and_then(|u| self.check_expr(u));
                let body = self.check_block_with_ret(body, ret)?;
                Some(HirStmt::ForC {
                    init,
                    cond: cond_expr,
                    update: update_expr,
                    body,
                    span: *span,
                })
            }
            Stmt::Switch {
                subject,
                arms,
                span,
            } => self.check_switch_stmt(subject, arms, *span),
            Stmt::Break(span) => Some(HirStmt::Break(*span)),
            Stmt::Continue(span) => Some(HirStmt::Continue(*span)),
            Stmt::Defer { body, span } => {
                let body = self.check_block_with_ret(body, ret)?;
                Some(HirStmt::Defer { body, span: *span })
            }
        }
    }

    /// Check a statement-level `switch`: subject, then each arm's pattern
    /// against it (bindings scoped to the arm), then exhaustiveness.
    fn check_switch_stmt(
        &mut self,
        subject: &Expr,
        arms: &[SwitchArm],
        span: Span,
    ) -> Option<HirStmt> {
        let subject_expr = self.check_expr(subject)?;
        let subject_ty = expr_ty(&subject_expr);
        let mut hir_arms = Vec::with_capacity(arms.len());
        let mut arm_ty: Option<Type> = None;
        for arm in arms {
            let checked = self.check_arm_against(&subject_ty, arm)?;
            let body_ty = expr_ty(&checked.body);
            if let Some(message) = merge_arm_type(&mut arm_ty, body_ty) {
                self.error(checked.span, message);
            }
            hir_arms.push(checked);
        }
        self.check_arms_exhaustive(&subject_ty, arms, span)?;
        Some(HirStmt::Switch {
            subject: subject_expr,
            arms: hir_arms,
            span,
        })
    }

    /// Check one arm against the subject type: pattern bindings are declared
    /// in a scope pushed for the arm and popped after the body, so two arms
    /// can bind the same name.
    fn check_arm_against(&mut self, subject_ty: &Type, arm: &SwitchArm) -> Option<HirSwitchArm> {
        self.scope.push();
        let pattern = match self.check_arm_pattern(&arm.pattern, subject_ty) {
            Ok(pattern) => pattern,
            Err(error) => {
                self.error(error.span, error.message);
                self.scope.pop();
                return None;
            }
        };
        // The guard is checked with the pattern's names in scope -- that is the
        // point of a guard -- and must be a `bool`, because it decides a branch
        // with no conversion.
        let guard = match &arm.guard {
            Some(guard) => {
                let Some(checked) = self.check_expr(guard) else {
                    self.scope.pop();
                    return None;
                };
                let guard_ty = expr_ty(&checked);
                if !matches!(guard_ty.kind, TypeKind::Bool) {
                    self.error(
                        expr_span(&checked),
                        format!("a guard must be a `bool`, found `{guard_ty}`"),
                    );
                }
                Some(checked)
            }
            None => None,
        };
        let Some(body) = self.check_expr(&arm.body) else {
            self.scope.pop();
            return None;
        };
        self.scope.pop();
        Some(HirSwitchArm {
            pattern,
            guard,
            body,
            is_default: arm.is_default,
            span: arm.span,
        })
    }

    /// Run `check_pattern` with the checker's enum table and type resolver,
    /// declaring each binding in the current (arm) scope.
    fn check_arm_pattern(
        &mut self,
        pattern: &Pattern,
        subject_ty: &Type,
    ) -> Result<HirPattern, TypeError> {
        let mut bindings = Vec::new();
        let variants = |name: &str| self.enums.get(name).map(|sig| sig.variants.clone());
        // `resolve_type` borrows `self` immutably; wrap it so the closure
        // does not capture the whole checker mutably.
        let checked = {
            let this = &*self;
            check_pattern(pattern, subject_ty, &mut bindings, &variants, &|ty| {
                this.resolve_type(ty)
            })?
        };
        for (name, ty, mutability, span) in bindings {
            self.scope.declare(
                name,
                Binding {
                    ty,
                    mutability,
                    span,
                    ownership: Ownership::Live,
                },
            )?;
        }
        Ok(checked)
    }

    /// Enforce exhaustiveness: enums must cover every variant, non-enums need
    /// a `default`. Returns `None` (failing the arm list) on error.
    fn check_arms_exhaustive(
        &mut self,
        subject_ty: &Type,
        arms: &[SwitchArm],
        span: Span,
    ) -> Option<()> {
        // A guard does not cover the case where it is false, so a guarded arm
        // takes no part in exhaustiveness: `case A if c` leaves A uncovered.
        // The switch is exhaustive only if its unguarded arms are.
        let has_default = arms.iter().any(|arm| arm.is_default && arm.guard.is_none());
        if has_default {
            return Some(());
        }
        match &subject_ty.kind {
            TypeKind::Enum(enum_name) => match self.enums.get(enum_name) {
                Some(sig) => {
                    let sig = sig.clone();
                    self.check_missing(enum_name, &sig.variants, arms, span)?;
                    Some(())
                }
                None => {
                    self.error(span, format!("cannot find type `{enum_name}`"));
                    None
                }
            },
            // `Option[T]` and `Result[T, E]` are enums the user never
            // declares. Synthesize their variant lists so the same
            // declaration-order `missing_cases` applies.
            TypeKind::Option(_) => {
                let variants = [
                    otter_parser::ast::VariantDef {
                        name: "None".to_string(),
                        fields: Vec::new(),
                        span: Span::new(0, 0),
                    },
                    otter_parser::ast::VariantDef {
                        name: "Some".to_string(),
                        fields: vec![otter_parser::ast::VariantField::Positional {
                            ty: option_inner(subject_ty),
                            span: Span::new(0, 0),
                        }],
                        span: Span::new(0, 0),
                    },
                ];
                self.check_missing("Option", &variants, arms, span)?;
                Some(())
            }
            TypeKind::Result(_, _) => {
                let (ok_ty, err_ty) = result_inners(subject_ty);
                let variants = [
                    otter_parser::ast::VariantDef {
                        name: "Ok".to_string(),
                        fields: vec![otter_parser::ast::VariantField::Positional {
                            ty: ok_ty,
                            span: Span::new(0, 0),
                        }],
                        span: Span::new(0, 0),
                    },
                    otter_parser::ast::VariantDef {
                        name: "Err".to_string(),
                        fields: vec![otter_parser::ast::VariantField::Positional {
                            ty: err_ty,
                            span: Span::new(0, 0),
                        }],
                        span: Span::new(0, 0),
                    },
                ];
                self.check_missing("Result", &variants, arms, span)?;
                Some(())
            }
            _ => {
                self.error(
                    span,
                    "switch over a non-enum type is not exhaustive: add a `default` arm"
                        .to_string(),
                );
                None
            }
        }
    }

    /// Run `missing_cases` over one variant list and report the missing names.
    fn check_missing(
        &mut self,
        enum_name: &str,
        variants: &[VariantDef],
        arms: &[SwitchArm],
        span: Span,
    ) -> Option<()> {
        let has_default = arms.iter().any(|arm| arm.is_default && arm.guard.is_none());
        let patterns: Vec<Pattern> = arms
            .iter()
            .filter(|arm| arm.guard.is_none())
            .map(|arm| arm.pattern.clone())
            .collect();
        let missing = missing_cases(enum_name, variants, &patterns, has_default);
        if missing.is_empty() {
            Some(())
        } else {
            self.error(
                span,
                format!(
                    "switch is not exhaustive: missing case{} {}",
                    if missing.len() == 1 { "" } else { "s" },
                    missing
                        .iter()
                        .map(|m| format!("`{m}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            None
        }
    }

    /// Check an `Option`/`Result` constructor call.
    ///
    /// Returns `None` when `name` is not a constructor, so the caller falls
    /// through to ordinary call resolution. Otherwise checks the arity and
    /// builds the matching type, reporting failures through the error
    /// channel. `expected` is the context's demanded type.
    fn check_constructor(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> Option<Option<HirExpr>> {
        let arity = match name {
            "None" => Some(0),
            "Some" | "Ok" | "Err" => Some(1),
            _ => None,
        };
        let Some(want) = arity else {
            return Some(None);
        };
        if args.len() != want {
            self.error(
                span,
                format!(
                    "expected {want} argument{}, found {}",
                    if want == 1 { "" } else { "s" },
                    args.len()
                ),
            );
            return Some(None);
        }
        Some(self.build_constructor(name, args, span, expected))
    }

    /// Build the HIR for a constructor whose arity already checked.
    fn build_constructor(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> Option<HirExpr> {
        let mk_call = |args: Vec<HirExpr>, ty: Type| HirExpr::Call {
            callee: vec![name.to_string()],
            args,
            unsafe_call: false,
            span,
            ty,
        };
        match name {
            "None" => {
                let inner = match expected {
                    Some(ty) => match &ty.kind {
                        TypeKind::Option(inner) => (**inner).clone(),
                        _ => {
                            self.error(span, format!("expected `{ty}`, found `Option[?]`"));
                            return None;
                        }
                    },
                    None => {
                        self.error(span, "cannot infer the payload type of `None`".to_string());
                        return None;
                    }
                };
                Some(mk_call(
                    Vec::new(),
                    Type {
                        kind: TypeKind::Option(Box::new(inner)),
                        span,
                    },
                ))
            }
            "Some" => {
                let payload = self.check_expr(&args[0])?;
                let payload_ty = expr_ty(&payload);
                Some(mk_call(
                    vec![payload],
                    Type {
                        kind: TypeKind::Option(Box::new(payload_ty)),
                        span,
                    },
                ))
            }
            "Ok" | "Err" => {
                let payload = self.check_expr(&args[0])?;
                let payload_ty = expr_ty(&payload);
                let Some(ty) = expected else {
                    self.error(span, format!("cannot infer the full type of `{name}(..)`"));
                    return None;
                };
                let (ok_ty, err_ty) = match &ty.kind {
                    TypeKind::Result(ok, err) => ((**ok).clone(), (**err).clone()),
                    _ => {
                        self.error(span, format!("expected `{ty}`, found `Result[?, ?]`"));
                        return None;
                    }
                };
                let (mine, _) = if name == "Ok" {
                    (ok_ty.clone(), err_ty.clone())
                } else {
                    (err_ty.clone(), ok_ty.clone())
                };
                if !mine.eq_ignoring_span(&payload_ty) {
                    self.error(
                        expr_span(&payload),
                        format!("expected `{mine}`, found `{payload_ty}`"),
                    );
                    return None;
                }
                Some(mk_call(
                    vec![payload],
                    Type {
                        kind: TypeKind::Result(Box::new(ok_ty), Box::new(err_ty)),
                        span,
                    },
                ))
            }
            _ => None,
        }
    }

    /// `Enum.Variant(args)` for a user-declared enum.
    ///
    /// The outer `Option` says "the callee is an enum constructor" and the
    /// inner one is the checked expression, `None` after a reported error. A
    /// two-segment callee whose head is not an enum falls through to the
    /// ordinary call path, which reports an unknown function as it always did.
    fn check_enum_call(
        &mut self,
        callee: &[String],
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        if callee.len() != 2 {
            return None;
        }
        let sig = self.enums.get(&callee[0])?.clone();
        let variant = sig.variants.iter().find(|v| v.name == callee[1])?.clone();
        let enum_name = callee[0].clone();
        let variant_name = callee[1].clone();
        if variant
            .fields
            .iter()
            .any(|f| matches!(f, VariantField::Named { .. }))
        {
            self.error(
                span,
                format!(
                    "variant `{variant_name}` of `{enum_name}` has named fields; \
                     write `{enum_name}.{variant_name} {{ ... }}`"
                ),
            );
            return Some(None);
        }
        if variant.fields.len() != args.len() {
            self.error(
                span,
                format!(
                    "variant `{variant_name}` of `{enum_name}` takes {} field{}, found {}",
                    variant.fields.len(),
                    if variant.fields.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            );
            return Some(None);
        }
        let mut checked = Vec::with_capacity(args.len());
        for (arg, field) in args.iter().zip(variant.fields.iter()) {
            let expected = match self.resolve_type(field.ty()) {
                Ok(ty) => ty,
                Err(e) => {
                    self.error(e.span, e.message);
                    return Some(None);
                }
            };
            let value = match self.check_expr_with_expected(arg, &expected) {
                Some(value) => value,
                None => return Some(None),
            };
            if !fits(&expected, &expr_ty(&value)) {
                self.error(
                    expr_span(&value),
                    format!("expected `{expected}`, found `{}`", expr_ty(&value)),
                );
            }
            // A payload of linear type takes ownership, exactly as a function
            // parameter does, so a bare name is moved out at the construction
            // site. `Shape.Box(buf)` and `dealloc(buf)` agree about who owns it.
            if matches!(linearity_of(&expected.kind), Linearity::Linear) {
                if let Expr::Ident(path, arg_span) = arg {
                    let arg_name = path.last().cloned().unwrap_or_default();
                    if let Err(e) = self.move_out(&arg_name, *arg_span) {
                        self.error(e.span, e.message);
                    }
                }
            }
            checked.push(value);
        }
        Some(Some(HirExpr::Call {
            callee: callee.to_vec(),
            args: checked,
            unsafe_call: false,
            span,
            ty: Type {
                kind: TypeKind::Enum(enum_name),
                span,
            },
        }))
    }

    /// `Enum.Variant` for a variant with no payload.
    ///
    /// A dotted name is otherwise a field chain; an enum has no fields, so a
    /// two-segment name whose head is an enum can only be construction.
    fn check_enum_path(&mut self, path: &[String], span: Span) -> Option<Option<HirExpr>> {
        if path.len() != 2 {
            return None;
        }
        let sig = self.enums.get(&path[0])?.clone();
        let variant = sig.variants.iter().find(|v| v.name == path[1])?.clone();
        if !variant.fields.is_empty() {
            self.error(
                span,
                format!(
                    "variant `{}` of `{}` takes {} field{}; construct it with `{}.{}(...)`",
                    path[1],
                    path[0],
                    variant.fields.len(),
                    if variant.fields.len() == 1 { "" } else { "s" },
                    path[0],
                    path[1]
                ),
            );
            return Some(None);
        }
        Some(Some(HirExpr::Call {
            callee: path.to_vec(),
            args: Vec::new(),
            unsafe_call: false,
            span,
            ty: Type {
                kind: TypeKind::Enum(path[0].clone()),
                span,
            },
        }))
    }

    /// `Enum.Variant { field: value, ... }` for a variant with named fields.
    ///
    /// Fields are checked by name and lowered in declaration order, for the
    /// same reason a struct literal is: the layout must not depend on the order
    /// a particular literal happened to spell.
    fn check_enum_struct_lit(
        &mut self,
        path: &[String],
        fields: &[(String, Expr, Span)],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        if path.len() != 2 {
            return None;
        }
        let sig = self.enums.get(&path[0])?.clone();
        let variant = sig.variants.iter().find(|v| v.name == path[1])?.clone();
        let enum_name = path[0].clone();
        let variant_name = path[1].clone();
        if variant
            .fields
            .iter()
            .any(|f| matches!(f, VariantField::Positional { .. }))
        {
            self.error(
                span,
                format!(
                    "variant `{variant_name}` of `{enum_name}` takes positional fields; \
                     write `{enum_name}.{variant_name}(...)`"
                ),
            );
            return Some(None);
        }
        let mut written: Vec<(String, HirExpr)> = Vec::new();
        for (field_name, value, field_span) in fields {
            let Some(expected) = variant.fields.iter().find(|f| match f {
                VariantField::Named { name, .. } => name == field_name,
                VariantField::Positional { .. } => false,
            }) else {
                self.error(
                    *field_span,
                    format!("unknown field `{field_name}` in variant `{variant_name}`"),
                );
                let _ = self.check_expr(value);
                continue;
            };
            let expected = match self.resolve_type(expected.ty()) {
                Ok(ty) => ty,
                Err(e) => {
                    self.error(e.span, e.message);
                    return Some(None);
                }
            };
            let value = match self.check_expr_with_expected(value, &expected) {
                Some(value) => value,
                None => return Some(None),
            };
            if !fits(&expected, &expr_ty(&value)) {
                self.error(
                    expr_span(&value),
                    format!("expected `{expected}`, found `{}`", expr_ty(&value)),
                );
            }
            written.push((field_name.clone(), value));
        }
        for field in &variant.fields {
            let VariantField::Named { name, .. } = field else {
                continue;
            };
            if !written.iter().any(|(written_name, _)| written_name == name) {
                self.error(
                    span,
                    format!("missing field `{name}` in variant `{variant_name}`"),
                );
            }
        }
        let mut ordered = Vec::new();
        for field in &variant.fields {
            let VariantField::Named { name, .. } = field else {
                continue;
            };
            if let Some((_, value)) = written.iter().find(|(n, _)| n == name) {
                ordered.push(value.clone());
            }
        }
        Some(Some(HirExpr::Call {
            callee: path.to_vec(),
            args: ordered,
            unsafe_call: false,
            span,
            ty: Type {
                kind: TypeKind::Enum(enum_name),
                span,
            },
        }))
    }

    /// Like [`check_expr`], but the context's expected type is known: the
    /// `return` type of the enclosing function (or a `const` annotation).
    ///
    /// This is what lets `return Ok(7)` infer `Result[?, string]` — `Ok`
    /// alone knows only its own half. A direct constructor call is checked
    /// with the expectation; every other expression falls through unchanged,
    /// because a wrong type there is already reported by `check_expr`.
    ///
    /// A bare `None` is handled here for the same reason as a `None()` call:
    /// it is the one constructor with no payload to infer from, so it can only
    /// be typed from context. This is what makes `f(None)` work when `f`'s
    /// parameter is an `Option`.
    fn check_expr_with_expected(&mut self, expr: &Expr, expected: &Type) -> Option<HirExpr> {
        if let Expr::Call {
            callee, args, span, ..
        } = expr
        {
            if callee.len() == 1 {
                let name = callee.last().cloned().unwrap_or_default();
                if matches!(name.as_str(), "None" | "Some" | "Ok" | "Err") {
                    return self.check_constructor(&name, args, *span, Some(expected))?;
                }
                if name == "map_new" {
                    return self.check_map_new(args, *span, Some(expected));
                }
                if name == "chan_new" {
                    return self.check_chan_new(args, *span, Some(expected));
                }
            }
        }
        if let Expr::Ident(path, span) = expr {
            if path.len() == 1 && path.first().map(String::as_str) == Some("None") {
                return self.check_constructor("None", &[], *span, Some(expected))?;
            }
        }
        self.check_expr(expr)
    }

    /// Rewrite `receiver.method(args)` into the receiver-first builtin
    /// `builtin(receiver, args)` and check that.
    ///
    /// The parser flattens a dotted call into a callee path, so the method form
    /// reaches the checker as `["receiver", "method"]`. The first segment is a
    /// method receiver only when it resolves to a local binding; otherwise the
    /// path is left alone for module resolution.
    ///
    /// The outer `Option` says "this was a method-shaped call"; the inner one
    /// is the checked expression, `None` after a reported error.
    fn check_builtin_method(
        &mut self,
        callee: &[String],
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        if callee.len() != 2 {
            return None;
        }
        let receiver = &callee[0];
        let method = &callee[1];
        // `Counter.new(3)` is a call on the *type*, not on a value of it: a
        // static method, found by the type's name.
        if self.is_type_name(receiver) {
            return Some(self.check_static_method(receiver, method, args, span));
        }
        let binding = self.scope.resolve(receiver)?;
        let receiver_ty = binding.ty.clone();
        if matches!(
            receiver_ty.kind,
            TypeKind::FixedArray(_, _) | TypeKind::View(_)
        ) {
            if let Some(checked) =
                self.check_fixed_array_method(receiver, method, args, span, &receiver_ty)
            {
                return Some(checked);
            }
        }
        if matches!(receiver_ty.kind, TypeKind::Slice(_)) {
            if let Some(checked) = self.check_array_method(receiver, method, args, span) {
                return Some(checked);
            }
        }
        // Every mutating method -- pushing, removing, sorting, freeing -- would
        // invalidate a view of the same array. `len` and the read-only methods
        // are fine, and a fixed array has no mutating methods at all.
        if mutates_storage(method) {
            self.reject_borrowed_write(receiver, span);
            if self
                .borrowed_bindings
                .iter()
                .any(|(borrowed, _)| borrowed == receiver)
            {
                // One diagnostic is enough; checking the call would report the
                // same write again from inside the method.
                return Some(None);
            }
        }
        if matches!(receiver_ty.kind, TypeKind::Str) {
            if let Some(checked) = self.check_string_method(receiver, method, args, span) {
                return Some(checked);
            }
        }
        if let Some(checked) = self.check_user_method(receiver, method, args, span) {
            return Some(checked);
        }
        let Some(builtin) = builtin_method_name(method, &receiver_ty.kind) else {
            self.error(span, format!("no method `{method}` on `{receiver_ty}`"));
            return Some(None);
        };
        let mut call_args = Vec::with_capacity(args.len().saturating_add(1));
        call_args.push(Expr::Ident(vec![receiver.clone()], span));
        call_args.extend(args.iter().cloned());
        // The method layer still lowers to the receiver-first runtime calls,
        // but it does not go back through `Expr::Call`: the old free-function
        // spellings now report a migration error, and the method form must not
        // report one against itself. Calling the check helpers directly keeps
        // the runtime contract and skips the public spelling.
        let checked = match builtin {
            "len" => self.check_len_builtin(&call_args, span),
            "push" | "pop" | "free_slice" => {
                self.check_slice_builtin(builtin, &call_args, span).flatten()
            }
            "map_get" | "map_insert" | "map_remove" | "map_len" | "map_free"
            | "map_keys" | "map_values" | "map_contains_key" => {
                self.check_map_builtin(builtin, &call_args, span).flatten()
            }
            "chan_send" | "chan_recv" | "chan_close" | "chan_free" | "mutex_lock"
            | "mutex_unlock" | "mutex_free" => {
                self.check_thread_builtin(builtin, &call_args, span).flatten()
            }
            _ => None,
        };
        Some(checked)
    }

    /// Whether a name is one of the declared types.
    ///
    /// The types are declared in the root scope, so a name that resolves there
    /// and names a type is the type itself -- not a value of it. A local that
    /// shadows a type's name resolves deeper and is a value.
    fn is_type_name(&self, name: &str) -> bool {
        self.scope.resolve_depth(name) == Some(0)
            && (self.structs.contains_key(name) || self.enums.contains_key(name))
    }

    /// Check a call on a type: `Counter.new(3)`.
    ///
    /// A static method belongs to the type, not to a value of it, so the
    /// receiver is the type's name and the method takes no receiver of its
    /// own. A method that does have one is reported: it needs a value.
    fn check_static_method(
        &mut self,
        type_name: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let mangled = format!("{type_name}.{method}");
        let Some(sig) = self.methods.get(&mangled).cloned() else {
            self.error(span, format!("no method `{method}` on `{type_name}`"));
            return None;
        };
        if sig.has_self {
            self.error(
                span,
                format!("`{type_name}.{method}` is a method and needs a receiver"),
            );
            return None;
        }
        if sig.params.len() != args.len() {
            self.error(
                span,
                format!(
                    "`{method}` takes {} argument{}",
                    sig.params.len(),
                    if sig.params.len() == 1 { "" } else { "s" }
                ),
            );
            return None;
        }
        let mut arg_exprs = Vec::with_capacity(args.len());
        for (arg, param) in args.iter().zip(sig.params.iter()) {
            let checked = self.check_expr_with_expected(arg, param)?;
            if !fits(param, &expr_ty(&checked)) {
                self.error(
                    expr_span(&checked),
                    format!("expected `{param}`, found `{}`", expr_ty(&checked)),
                );
            }
            arg_exprs.push(checked);
        }
        Some(HirExpr::Call {
            callee: vec![mangled],
            args: arg_exprs,
            unsafe_call: false,
            span,
            ty: sig.ret.clone(),
        })
    }

    /// Build the comparison of two structs, field by field.
    ///
    /// Two structs are equal when every one of their fields is, so the
    /// comparison is a conjunction of the fields' comparisons. A field with no
    /// equality -- an array, a map -- makes the whole comparison impossible,
    /// and says so.
    fn struct_equality(
        &mut self,
        op: BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        name: &str,
        span: Span,
    ) -> Option<HirExpr> {
        let sig = self.structs.get(name)?.clone();
        let mut conjunction: Option<HirExpr> = None;
        for (index, (field_name, field_ty)) in sig.fields.iter().enumerate() {
            if !self.has_equality(&field_ty.kind) {
                self.error(
                    span,
                    format!("`==` needs fields with equality: `{field_ty}` has none"),
                );
                return None;
            }
            let left = HirExpr::FieldAccess {
                base: Box::new(lhs.clone()),
                field: field_name.clone(),
                index,
                ty: field_ty.clone(),
                span,
            };
            let right = HirExpr::FieldAccess {
                base: Box::new(rhs.clone()),
                field: field_name.clone(),
                index,
                ty: field_ty.clone(),
                span,
            };
            let compared = HirExpr::Binary {
                op: BinOp::Eq,
                lhs: Box::new(left),
                rhs: Box::new(right),
                span,
                ty: prim(TypeKind::Bool),
                fault: None,
            };
            conjunction = Some(match conjunction {
                None => compared,
                Some(previous) => HirExpr::Binary {
                    op: BinOp::And,
                    lhs: Box::new(previous),
                    rhs: Box::new(compared),
                    span,
                    ty: prim(TypeKind::Bool),
                    fault: None,
                },
            });
        }
        let equal = conjunction?;
        if op == BinOp::Ne {
            return Some(HirExpr::Unary {
                op: UnOp::Not,
                operand: Box::new(equal),
                span,
                ty: prim(TypeKind::Bool),
                fault: None,
            });
        }
        Some(equal)
    }

    /// Whether a type has an equality the compiler can lower.
    ///
    /// The numbers and the string have one; a struct has one when all of its
    /// fields do. An array, a map, a pointer and a channel have none: what
    /// they hold is behind a header, and two headers are never the same
    /// value.
    fn has_equality(&self, ty: &TypeKind) -> bool {
        match ty {
            TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Str => true,
            TypeKind::Struct(name) => self
                .structs
                .get(name)
                .map(|sig| sig.fields.iter().all(|(_, field)| self.has_equality(&field.kind)))
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Check a call to a method declared in an `impl` block.
    ///
    /// The receiver is a value of a struct or an enum type, and the method is
    /// found by the type: `point.sum()` is `Point.sum` with the receiver as
    /// its first argument. The method's own parameters are checked against the
    /// arguments the way a function's are.
    fn check_user_method(
        &mut self,
        receiver: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let receiver_expr = Expr::Ident(vec![receiver.to_string()], span);
        let Some(receiver_checked) = self.check_expr(&receiver_expr) else {
            return Some(None);
        };
        self.check_user_method_on(receiver_checked, method, args, span)
    }

    /// Check a method call whose receiver is a value rather than a name.
    ///
    /// `counter.add(4).get()` is a call on the result of a call: the receiver
    /// is an expression, and the method is found by its type exactly as it is
    /// when the receiver is a binding.
    fn check_user_method_on(
        &mut self,
        receiver_checked: HirExpr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let receiver_ty = expr_ty(&receiver_checked);
        let type_name = match &receiver_ty.kind {
            TypeKind::Struct(name) | TypeKind::Enum(name) => name.clone(),
            _ => return None,
        };
        let mangled = format!("{type_name}.{method}");
        let Some(sig) = self.methods.get(&mangled).cloned() else {
            return None;
        };
        if !sig.has_self {
            // A method with no `self` is a static one: it belongs to the type,
            // not to a value of it.
            self.error(
                span,
                format!(
                    "`{method}` is a static method of `{}` and needs the type, not a value",
                    sig.type_name
                ),
            );
            return Some(None);
        }
        // The method's own parameters, without the receiver.
        let own = sig.params.len().saturating_sub(1);
        if own != args.len() {
            self.error(
                span,
                format!(
                    "`{method}` takes {own} argument{}",
                    if own == 1 { "" } else { "s" }
                ),
            );
            return Some(None);
        }
        // A pointer receiver is handed the address of the object it is called
        // on, so the receiver must be a mutable binding: there is no address of
        // a temporary, and a `const` promises not to change.
        let receiver_checked = if sig.receiver_is_pointer {
            let HirExpr::Ident(path, _, _) = &receiver_checked else {
                self.error(
                    span,
                    format!("`{method}` needs a mutable object binding as the receiver"),
                );
                return Some(None);
            };
            let name = path.last().cloned().unwrap_or_default();
            let mutable = self
                .scope
                .resolve(&name)
                .map(|binding| binding.mutability.is_mutable())
                .unwrap_or(false);
            if !mutable {
                self.error(span, format!("`{method}` needs a mutable object binding"));
                return Some(None);
            }
            let pointee = expr_ty(&receiver_checked);
            HirExpr::AddrOf {
                name,
                span,
                ty: Type {
                    kind: TypeKind::Ptr(Box::new(pointee)),
                    span,
                },
            }
        } else {
            receiver_checked
        };
        let mut arg_exprs = Vec::with_capacity(args.len() + 1);
        arg_exprs.push(receiver_checked);
        for (arg, param) in args.iter().zip(sig.params.iter().skip(1)) {
            let checked = self.check_expr_with_expected(arg, param)?;
            if !fits(param, &expr_ty(&checked)) {
                self.error(
                    expr_span(&checked),
                    format!("expected `{param}`, found `{}`", expr_ty(&checked)),
                );
            }
            arg_exprs.push(checked);
        }
        Some(Some(HirExpr::Call {
            callee: vec![mangled],
            args: arg_exprs,
            unsafe_call: false,
            span,
            ty: sig.ret.clone(),
        }))
    }

    /// Report an old free-function spelling and show its method form.    ///
    /// The receiver is quoted from the caller's own source rather than named
    /// generically, so the hint is a line the caller can paste back. An
    /// argument-less call has no receiver to quote; the message then names the
    /// method without pretending to know one.
    fn report_old_builtin(&mut self, name: &str, method: &str, args: &[Expr], span: Span) {
        let Some(receiver) = args.first() else {
            self.error(
                span,
                format!(
                    "`{name}` has moved to method syntax: call `.{method}()` on the receiver"
                ),
            );
            return;
        };
        let receiver_text = self.snippet_text(otter_parser::exprs::expr_span(receiver));
        let rest: Vec<String> = args
            .iter()
            .skip(1)
            .map(|arg| self.snippet_text(otter_parser::exprs::expr_span(arg)))
            .collect();
        let arguments = format!("({})", rest.join(", "));
        self.error(
            span,
            format!(
                "`{name}` has moved to method syntax: write `{receiver_text}.{method}{arguments}`"
            ),
        );
    }

    /// String methods. The read-only helpers reuse the existing string
    /// builtins (`len`, `byte_at`, `str_slice`, `parse_*`); the transforms and
    /// searches are new runtime entry points and are built here so the method
    /// surface does not need a free-function spelling for each one.
    ///
    /// The outer `Option` says "this was a string method"; the inner one is the
    /// checked expression, `None` after a reported error.
    fn check_string_method(
        &mut self,
        receiver: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let arity = match method {
            "len" | "is_empty" | "trim" | "trim_start" | "trim_end" | "to_upper" | "to_lower"
            | "lines" | "parse_i64" | "parse_f64" => 0,
            "byte_at" | "get" | "contains" | "starts_with" | "ends_with" | "index_of"
            | "repeat" | "split" => 1,
            "slice" | "replace" => 2,
            _ => return None,
        };
        if args.len() != arity {
            self.error(
                span,
                format!(
                    "`{method}` takes {arity} argument{}",
                    if arity == 1 { "" } else { "s" }
                ),
            );
            return Some(None);
        }
        let receiver_expr = || Expr::Ident(vec![receiver.to_string()], span);
        match method {
            "len" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["len".to_string()],
                    args: vec![receiver_checked],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::I64),
                }))
            }
            "byte_at" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                let Some(index) = self.check_i64_argument(&args[0]) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["byte_at".to_string()],
                    args: vec![receiver_checked, index],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::I64),
                }))
            }
            "slice" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                let Some(start) = self.check_i64_argument(&args[0]) else {
                    return Some(None);
                };
                let Some(end) = self.check_i64_argument(&args[1]) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["str_slice".to_string()],
                    args: vec![receiver_checked, start, end],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::Str),
                }))
            }
            "parse_i64" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["parse_i64".to_string()],
                    args: vec![receiver_checked],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::I64),
                }))
            }
            "parse_f64" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["parse_f64".to_string()],
                    args: vec![receiver_checked],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::F64),
                }))
            }
            "is_empty" => {
                let Some(len) = self.check_len_builtin(&[receiver_expr()], span) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Binary {
                    op: BinOp::Eq,
                    lhs: Box::new(len),
                    rhs: Box::new(HirExpr::Literal(
                        Literal::I64(0),
                        span,
                        prim(TypeKind::I64),
                    )),
                    span,
                    ty: prim(TypeKind::Bool),
                    fault: None,
                }))
            }
            _ => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                let (callee, result_ty, checked_args) = match method {
                    "get" => {
                        let Some(index) = self.check_i64_argument(&args[0]) else {
                            return Some(None);
                        };
                        (
                            "str_get",
                            option(&prim(TypeKind::I64), span),
                            vec![receiver_checked, index],
                        )
                    }
                    "contains" | "starts_with" | "ends_with" => {
                        let Some(needle) = self.check_string_argument(&args[0]) else {
                            return Some(None);
                        };
                        (
                            match method {
                                "contains" => "str_contains",
                                "starts_with" => "str_starts_with",
                                _ => "str_ends_with",
                            },
                            prim(TypeKind::Bool),
                            vec![receiver_checked, needle],
                        )
                    }
                    "index_of" => {
                        let Some(needle) = self.check_string_argument(&args[0]) else {
                            return Some(None);
                        };
                        (
                            "str_index_of",
                            option(&prim(TypeKind::I64), span),
                            vec![receiver_checked, needle],
                        )
                    }
                    "trim" | "trim_start" | "trim_end" | "to_upper" | "to_lower" => (
                        match method {
                            "trim" => "str_trim",
                            "trim_start" => "str_trim_start",
                            "trim_end" => "str_trim_end",
                            "to_upper" => "str_to_upper",
                            _ => "str_to_lower",
                        },
                        prim(TypeKind::Str),
                        vec![receiver_checked],
                    ),
                    "replace" => {
                        let Some(old) = self.check_string_argument(&args[0]) else {
                            return Some(None);
                        };
                        let Some(new) = self.check_string_argument(&args[1]) else {
                            return Some(None);
                        };
                        (
                            "str_replace",
                            prim(TypeKind::Str),
                            vec![receiver_checked, old, new],
                        )
                    }
                    "repeat" => {
                        let Some(count) = self.check_i64_argument(&args[0]) else {
                            return Some(None);
                        };
                        ("str_repeat", prim(TypeKind::Str), vec![receiver_checked, count])
                    }
                    "split" => {
                        let Some(separator) = self.check_string_argument(&args[0]) else {
                            return Some(None);
                        };
                        let slice_ty = Type {
                            kind: TypeKind::Slice(Box::new(prim(TypeKind::Str))),
                            span,
                        };
                        ("str_split", slice_ty, vec![receiver_checked, separator])
                    }
                    "lines" => {
                        let slice_ty = Type {
                            kind: TypeKind::Slice(Box::new(prim(TypeKind::Str))),
                            span,
                        };
                        ("str_lines", slice_ty, vec![receiver_checked])
                    }
                    _ => return None,
                };
                Some(Some(HirExpr::Call {
                    callee: vec![callee.to_string()],
                    args: checked_args,
                    unsafe_call: false,
                    span,
                    ty: result_ty,
                }))
            }
        }
    }

    /// Check one argument that must be a `string`.
    fn check_string_argument(&mut self, arg: &Expr) -> Option<HirExpr> {
        let checked = self.check_expr_with_expected(arg, &prim(TypeKind::Str))?;
        if !fits(&prim(TypeKind::Str), &expr_ty(&checked)) {
            self.error(
                expr_span(&checked),
                format!("expected `string`, found `{}`", expr_ty(&checked)),
            );
            return None;
        }
        Some(checked)
    }

    /// Check one argument that must be an `i64`.
    fn check_i64_argument(&mut self, arg: &Expr) -> Option<HirExpr> {
        let checked = self.check_expr_with_expected(arg, &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&checked)) {
            self.error(
                expr_span(&checked),
                format!("expected `i64`, found `{}`", expr_ty(&checked)),
            );
            return None;
        }
        Some(checked)
    }

    /// Array query and update methods that do not lower to a single existing
    /// builtin.
    ///
    /// `first`, `last`, and `is_empty` are source-level compositions: they
    /// become an index expression or a comparison against `len`, so indexing
    /// remains the one place that defines the `Option` shape. `set` needs a
    /// runtime call because it writes through the binding's header.
    fn check_array_method(
        &mut self,
        receiver: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let receiver_expr = || Expr::Ident(vec![receiver.to_string()], span);
        let arity = match method {
            "is_empty" | "first" | "last" | "clear" | "reverse" | "sort" => 0,
            "get" | "remove" | "contains" | "index_of" | "extend" | "map" | "filter" => 1,
            "set" | "insert" | "slice" | "fold" => 2,
            _ => return None,
        };
        if args.len() != arity {
            self.error(
                span,
                format!(
                    "`{method}` takes {arity} argument{}",
                    if arity == 1 { "" } else { "s" }
                ),
            );
            return Some(None);
        }
        match method {
            "is_empty" => {
                let Some(len) = self.check_len_builtin(&[receiver_expr()], span) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Binary {
                    op: BinOp::Eq,
                    lhs: Box::new(len),
                    rhs: Box::new(HirExpr::Literal(
                        Literal::I64(0),
                        span,
                        prim(TypeKind::I64),
                    )),
                    span,
                    ty: prim(TypeKind::Bool),
                    fault: None,
                }))
            }
            "get" => {
                let index = Expr::Index {
                    base: Box::new(receiver_expr()),
                    index: Box::new(args[0].clone()),
                    span,
                };
                Some(self.check_expr(&index))
            }
            "first" => {
                let index = Expr::Index {
                    base: Box::new(receiver_expr()),
                    index: Box::new(Expr::Literal(Literal::I64(0), span)),
                    span,
                };
                Some(self.check_expr(&index))
            }
            "last" => {
                let Some((receiver_checked, receiver_ty)) =
                    self.check_array_receiver(receiver, span)
                else {
                    return Some(None);
                };
                let TypeKind::Slice(element) = receiver_ty.kind else {
                    return Some(None);
                };
                let len = HirExpr::Call {
                    callee: vec!["len".to_string()],
                    args: vec![receiver_checked.clone()],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::I64),
                };
                let index = HirExpr::Binary {
                    op: BinOp::Sub,
                    lhs: Box::new(len),
                    rhs: Box::new(HirExpr::Literal(
                        Literal::I64(1),
                        span,
                        prim(TypeKind::I64),
                    )),
                    span,
                    ty: prim(TypeKind::I64),
                    fault: None,
                };
                Some(Some(HirExpr::Index {
                    base: Box::new(receiver_checked),
                    index: Box::new(index),
                    span,
                    ty: option(&element, span),
                }))
            }
            "set" => Some(self.check_array_set(receiver, args, span)),
            "insert" => Some(self.check_array_insert(receiver, args, span)),
            "remove" => Some(self.check_array_remove(receiver, args, span)),
            "clear" => Some(self.check_array_mutation(receiver, "slice_clear", span)),
            "reverse" => Some(self.check_array_mutation(receiver, "slice_reverse", span)),
            "sort" => Some(self.check_array_sort(receiver, span)),
            "contains" => Some(self.check_array_contains(receiver, args, span, false)),
            "index_of" => Some(self.check_array_contains(receiver, args, span, true)),
            "extend" => Some(self.check_array_extend(receiver, args, span)),
            "slice" => Some(self.check_array_slice(receiver, args, span)),
            "map" => Some(self.check_array_map(receiver, args, span)),
            "filter" => Some(self.check_array_filter(receiver, args, span)),
            "fold" => Some(self.check_array_fold(receiver, args, span)),
            _ => None,
        }
    }

    /// The method surface a fixed array and a view share.
    ///
    /// Both know their length and can be indexed, which is enough for reading:
    /// `len`, `is_empty`, `get`, `first` and `last`. A fixed array's length is
    /// a constant in its type; a view's is a word it carries. The mutating and
    /// allocating methods belong to `[]T`, which owns its storage, so they are
    /// not offered here -- `[]T` is what a fixed array can be copied into if
    /// the program needs to grow.
    fn check_fixed_array_method(
        &mut self,
        receiver: &str,
        method: &str,
        args: &[Expr],
        span: Span,
        receiver_ty: &Type,
    ) -> Option<Option<HirExpr>> {
        let arity = match method {
            "len" | "is_empty" | "first" | "last" => 0,
            "get" => 1,
            _ => return None,
        };
        if args.len() != arity {
            self.error(
                span,
                format!(
                    "`{method}` takes {arity} argument{}",
                    if arity == 1 { "" } else { "s" }
                ),
            );
            return Some(None);
        }
        let receiver_expr = || HirExpr::Ident(
            vec![receiver.to_string()],
            span,
            receiver_ty.clone(),
        );
        let len = match &receiver_ty.kind {
            TypeKind::FixedArray(_, length) => HirExpr::Literal(
                Literal::I64(i64::try_from(*length).unwrap_or(i64::MAX)),
                span,
                prim(TypeKind::I64),
            ),
            _ => HirExpr::Call {
                callee: vec!["view_len".to_string()],
                args: vec![receiver_expr()],
                unsafe_call: false,
                span,
                ty: prim(TypeKind::I64),
            },
        };
        match method {
            "len" => Some(Some(len)),
            "is_empty" => Some(Some(HirExpr::Binary {
                op: BinOp::Eq,
                lhs: Box::new(len),
                rhs: Box::new(HirExpr::Literal(
                    Literal::I64(0),
                    span,
                    prim(TypeKind::I64),
                )),
                span,
                ty: prim(TypeKind::Bool),
                fault: None,
            })),
            "get" => {
                let index = Expr::Index {
                    base: Box::new(Expr::Ident(vec![receiver.to_string()], span)),
                    index: Box::new(args[0].clone()),
                    span,
                };
                Some(self.check_expr(&index))
            }
            "first" => {
                let index = Expr::Index {
                    base: Box::new(Expr::Ident(vec![receiver.to_string()], span)),
                    index: Box::new(Expr::Literal(Literal::I64(0), span)),
                    span,
                };
                Some(self.check_expr(&index))
            }
            "last" => {
                let index = HirExpr::Binary {
                    op: BinOp::Sub,
                    lhs: Box::new(len),
                    rhs: Box::new(HirExpr::Literal(
                        Literal::I64(1),
                        span,
                        prim(TypeKind::I64),
                    )),
                    span,
                    ty: prim(TypeKind::I64),
                    fault: None,
                };
                let base = self.check_expr(&Expr::Ident(vec![receiver.to_string()], span))?;
                Some(Some(HirExpr::Index {
                    base: Box::new(base),
                    index: Box::new(index),
                    span,
                    ty: option(
                        &match &receiver_ty.kind {
                            TypeKind::FixedArray(element, _) | TypeKind::View(element) => {
                                (**element).clone()
                            }
                            _ => prim(TypeKind::Unit),
                        },
                        span,
                    ),
                }))
            }
            _ => None,
        }
    }

    /// Check `xs.map(f)`: a new array holding the mapped values.
    ///
    /// The callback takes one element and returns the value that replaces it,
    /// so the result is an array of the callback's return type. The element type
    /// may differ from the source's: `[]i64` maps to `[]string` as well as to
    /// `[]i64`.
    fn check_array_map(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (receiver_checked, receiver_ty) = self.check_array_receiver(receiver, span)?;
        let TypeKind::Slice(element) = receiver_ty.kind else {
            return None;
        };
        let Some(func_checked) = self.check_callback(&args[0], span) else {
            return None;
        };
        let TypeKind::Func(params, ret) = expr_ty(&func_checked).kind else {
            return None;
        };
        if params.len() != 1 || !params[0].eq_ignoring_span(&element) {
            self.error(
                span,
                format!(
                    "expected a function of one `{}`, found `{}`",
                    element,
                    expr_ty(&func_checked)
                ),
            );
            return None;
        }
        let result_ty = Type {
            kind: TypeKind::Slice(Box::new((*ret).clone())),
            span,
        };
        Some(HirExpr::HigherOrder {
            op: HigherOrderOp::Map,
            receiver: Box::new(receiver_checked),
            func: Box::new(func_checked),
            init: None,
            span,
            ty: result_ty,
        })
    }

    /// Check `xs.filter(f)`: a new array holding the values that pass.
    ///
    /// The callback takes one element and says whether it stays, so the
    /// result is an array of the source's own element type.
    fn check_array_filter(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (receiver_checked, receiver_ty) = self.check_array_receiver(receiver, span)?;
        let TypeKind::Slice(element) = receiver_ty.kind else {
            return None;
        };
        let Some(func_checked) = self.check_callback(&args[0], span) else {
            return None;
        };
        let TypeKind::Func(params, ret) = expr_ty(&func_checked).kind else {
            return None;
        };
        if params.len() != 1
            || !params[0].eq_ignoring_span(&element)
            || !matches!(ret.kind, TypeKind::Bool)
        {
            self.error(
                span,
                format!(
                    "expected a function of one `{}` returning `bool`, found `{}`",
                    element,
                    expr_ty(&func_checked)
                ),
            );
            return None;
        }
        let result_ty = Type {
            kind: TypeKind::Slice(element),
            span,
        };
        Some(HirExpr::HigherOrder {
            op: HigherOrderOp::Filter,
            receiver: Box::new(receiver_checked),
            func: Box::new(func_checked),
            init: None,
            span,
            ty: result_ty,
        })
    }

    /// Check `xs.fold(init, f)`: one value, folded from the array.
    ///
    /// The accumulator's type is the type of `init`, and the callback takes
    /// the accumulator and the element and returns the next accumulator.
    fn check_array_fold(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (receiver_checked, receiver_ty) = self.check_array_receiver(receiver, span)?;
        let TypeKind::Slice(element) = receiver_ty.kind else {
            return None;
        };
        let Some(init_checked) = self.check_expr(&args[0]) else {
            return None;
        };
        let accumulator_ty = expr_ty(&init_checked);
        let Some(func_checked) = self.check_callback(&args[1], span) else {
            return None;
        };
        let TypeKind::Func(params, ret) = expr_ty(&func_checked).kind else {
            return None;
        };
        if params.len() != 2
            || !params[0].eq_ignoring_span(&accumulator_ty)
            || !params[1].eq_ignoring_span(&element)
            || !ret.eq_ignoring_span(&accumulator_ty)
        {
            self.error(
                span,
                format!(
                    "expected a function of `({accumulator_ty}, {element})` returning \
                     `{accumulator_ty}`, found `{}`",
                    expr_ty(&func_checked)
                ),
            );
            return None;
        }
        Some(HirExpr::HigherOrder {
            op: HigherOrderOp::Fold,
            receiver: Box::new(receiver_checked),
            func: Box::new(func_checked),
            init: Some(Box::new(init_checked)),
            span,
            ty: accumulator_ty,
        })
    }

    /// Check a callback argument and return its checked expression.
    ///
    /// The three higher-order methods all take a function, and all say so the
    /// same way: the argument has to be one, and a value that is not is
    /// reported here rather than deeper in.
    fn check_callback(&mut self, expr: &Expr, span: Span) -> Option<HirExpr> {
        let checked = self.check_expr(expr)?;
        let ty = expr_ty(&checked);
        if !matches!(ty.kind, TypeKind::Func(_, _)) {
            self.error(span, format!("expected a function, found `{ty}`"));
            return None;
        }
        Some(checked)
    }

    /// Resolve the receiver of an array method to its binding and slice type.
    fn check_array_receiver(
        &mut self,
        receiver: &str,
        span: Span,
    ) -> Option<(HirExpr, Type)> {
        let receiver_expr = Expr::Ident(vec![receiver.to_string()], span);
        let checked = self.check_expr(&receiver_expr)?;
        let ty = expr_ty(&checked);
        if !matches!(ty.kind, TypeKind::Slice(_)) {
            self.error(
                expr_span(&checked),
                format!("expected an array, found `{ty}`"),
            );
            return None;
        }
        Some((checked, ty))
    }

    /// The element type of a checked slice receiver.
    fn array_element(ty: &Type) -> Option<Type> {
        let TypeKind::Slice(element) = &ty.kind else {
            return None;
        };
        Some((**element).clone())
    }

    /// `xs.insert(index, value) -> bool`.
    fn check_array_insert(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&checked), unsupported_element_message(&element));
            return None;
        }
        if matches!(element.kind, TypeKind::Str) {
            self.error(
                expr_span(&checked),
                "inserting into a `[]string` is not supported yet: the array owns one reference per element, and the new element would need a retain",
            );
            return None;
        }
        let index = self.check_expr_with_expected(&args[0], &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&index)) {
            self.error(
                expr_span(&index),
                format!("expected `i64`, found `{}`", expr_ty(&index)),
            );
            return None;
        }
        let value = self.check_expr_with_expected(&args[1], &element)?;
        if !fits(&element, &expr_ty(&value)) {
            self.error(
                expr_span(&value),
                format!("expected `{element}`, found `{}`", expr_ty(&value)),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["slice_insert".to_string()],
            args: vec![checked, index, value],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Bool),
        })
    }

    /// `xs.remove(index) -> Option[T]`.
    fn check_array_remove(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&checked), unsupported_element_message(&element));
            return None;
        }
        if matches!(element.kind, TypeKind::Str) {
            self.error(
                expr_span(&checked),
                "removing from a `[]string` is not supported yet: the removed reference would leave the array without a matching release",
            );
            return None;
        }
        let index = self.check_expr_with_expected(&args[0], &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&index)) {
            self.error(
                expr_span(&index),
                format!("expected `i64`, found `{}`", expr_ty(&index)),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["slice_remove".to_string()],
            args: vec![checked, index],
            unsafe_call: false,
            span,
            ty: option(&element, span),
        })
    }

    /// `xs.clear()` and `xs.reverse()`: one receiver, no result.
    fn check_array_mutation(
        &mut self,
        receiver: &str,
        builtin: &str,
        span: Span,
    ) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&checked), unsupported_element_message(&element));
            return None;
        }
        Some(HirExpr::Call {
            callee: vec![builtin.to_string()],
            args: vec![checked],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Unit),
        })
    }

    /// `xs.sort()`, limited to `[]i64` in this phase.
    fn check_array_sort(&mut self, receiver: &str, span: Span) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !matches!(element.kind, TypeKind::I64) {
            self.error(
                expr_span(&checked),
                "`sort` is only implemented for arrays of i64",
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["slice_sort".to_string()],
            args: vec![checked],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Unit),
        })
    }

    /// `xs.contains(value) -> bool` and `xs.index_of(value) -> Option[i64]`.
    ///
    /// Both need an element type with equality; the runtime has one comparison
    /// per type it supports. `index_of` answers the first matching position, or
    /// `None` when the element is absent.
    fn check_array_contains(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
        want_index: bool,
    ) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !has_equality(&element) {
            self.error(
                expr_span(&checked),
                format!(
                    "`{}` needs an element type with equality",
                    if want_index { "index_of" } else { "contains" }
                ),
            );
            return None;
        }
        let value = self.check_expr_with_expected(&args[0], &element)?;
        if !fits(&element, &expr_ty(&value)) {
            self.error(
                expr_span(&value),
                format!("expected `{element}`, found `{}`", expr_ty(&value)),
            );
            return None;
        }
        let result_ty = if want_index {
            option(&prim(TypeKind::I64), span)
        } else {
            prim(TypeKind::Bool)
        };
        Some(HirExpr::Call {
            callee: vec![if want_index {
                "slice_index_of".to_string()
            } else {
                "slice_contains".to_string()
            }],
            args: vec![checked, value],
            unsafe_call: false,
            span,
            ty: result_ty,
        })
    }

    /// `xs.extend(other)` consumes `other` and appends its elements.
    ///
    /// A binding argument is moved out, and the runtime frees the consumed
    /// slice after copying, so the caller cannot read it again. A slice
    /// literal or a call result is a temporary with the same ownership. A
    /// field or index projection is refused: freeing its backing array would
    /// leave the field holding a dangling header.
    fn check_array_extend(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&checked), unsupported_element_message(&element));
            return None;
        }
        let other_expr = &args[0];
        let other = self.check_expr(other_expr)?;
        let other_ty = expr_ty(&other);
        let TypeKind::Slice(other_element) = &other_ty.kind else {
            self.error(
                expr_span(&other),
                format!("`extend` expects an array, found `{other_ty}`"),
            );
            return None;
        };
        if **other_element != element {
            self.error(
                expr_span(&other),
                format!("`extend` expects `[]{element}`, found `{other_ty}`"),
            );
            return None;
        }
        match other_expr {
            Expr::Ident(path, ident_span) if path.len() == 1 => {
                let name = path.last().cloned().unwrap_or_default();
                if name == receiver {
                    self.error(*ident_span, "an array cannot extend itself");
                    return None;
                }
                if let Err(error) = self.move_out(&name, *ident_span) {
                    self.error(error.span, error.message);
                }
            }
            Expr::SliceLit { .. } | Expr::Call { .. } => {}
            _ => {
                self.error(
                    expr_span(&other),
                    "`extend` needs an array binding, an array literal, or a call result",
                );
                return None;
            }
        }
        Some(HirExpr::Call {
            callee: vec!["slice_extend".to_string()],
            args: vec![checked, other],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Unit),
        })
    }

    /// `xs.slice(start, end) -> []T`, a fresh array over the clamped range.
    fn check_array_slice(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let (checked, ty) = self.check_array_receiver(receiver, span)?;
        let element = Self::array_element(&ty)?;
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&checked), unsupported_element_message(&element));
            return None;
        }
        let start = self.check_expr_with_expected(&args[0], &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&start)) {
            self.error(
                expr_span(&start),
                format!("expected `i64`, found `{}`", expr_ty(&start)),
            );
            return None;
        }
        let end = self.check_expr_with_expected(&args[1], &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&end)) {
            self.error(
                expr_span(&end),
                format!("expected `i64`, found `{}`", expr_ty(&end)),
            );
            return None;
        }
        let result_ty = Type {
            kind: TypeKind::Slice(Box::new(element)),
            span,
        };
        Some(HirExpr::Call {
            callee: vec!["slice_copy".to_string()],
            args: vec![checked, start, end],
            unsafe_call: false,
            span,
            ty: result_ty,
        })
    }

    /// `xs.set(index, value) -> bool`, where the receiver must be a plain
    /// binding because the runtime updates its header through the binding's
    /// slot.
    fn check_array_set(
        &mut self,
        receiver: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let receiver_expr = Expr::Ident(vec![receiver.to_string()], span);
        let checked = self.check_expr(&receiver_expr)?;
        let receiver_ty = expr_ty(&checked);
        let TypeKind::Slice(element) = &receiver_ty.kind else {
            self.error(
                expr_span(&checked),
                format!("`set` expects an array, found `{receiver_ty}`"),
            );
            return None;
        };
        let element = (**element).clone();
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&checked), unsupported_element_message(&element));
            return None;
        }
        if matches!(element.kind, TypeKind::Str) {
            self.error(
                expr_span(&checked),
                "setting a `string` element is not supported yet: the array owns the old reference, and the replacement would need a retain and a release",
            );
            return None;
        }
        let index = self.check_expr_with_expected(&args[0], &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&index)) {
            self.error(
                expr_span(&index),
                format!("expected `i64`, found `{}`", expr_ty(&index)),
            );
            return None;
        }
        let value = self.check_expr_with_expected(&args[1], &element)?;
        if !fits(&element, &expr_ty(&value)) {
            self.error(
                expr_span(&value),
                format!("expected `{element}`, found `{}`", expr_ty(&value)),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["slice_set".to_string()],
            args: vec![checked, index, value],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Bool),
        })
    }

    /// The receiver-first array builtins: `push`, `pop` and `free_slice`.
    ///
    /// These three cannot live in the fixed builtin table: their types mention
    /// the array's *element* type, and the table has no type parameters.
    /// `push` keeps the array (it may reallocate the backing store, and the
    /// header in the binding's slot is updated in place); `pop` also keeps it
    /// and hands the last element out as an `Option`; `free_slice` consumes it,
    /// so its receiver must be a binding and is moved out here.
    ///
    /// The outer `Option` says "this name is an array builtin"; the inner one
    /// is the checked expression, `None` after a reported error.
    fn check_slice_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let arity = if name == "push" { 2 } else { 1 };
        if args.len() != arity {
            self.error(
                span,
                format!(
                    "`{name}` takes {arity} argument{}",
                    if arity == 1 { "" } else { "s" }
                ),
            );
            return Some(None);
        }
        let Expr::Ident(path, ident_span) = &args[0] else {
            self.error(
                span,
                format!("`{name}` needs an array binding, not a temporary"),
            );
            return Some(None);
        };
        if path.len() != 1 {
            // A field's slice header has no binding slot to update in place,
            // and lowering looks the name up in the local map: allowing a
            // projection here would make the call silently do nothing.
            self.error(
                *ident_span,
                format!("`{name}` needs an array binding, not a field access"),
            );
            return Some(None);
        }
        let receiver = match self.check_expr(&args[0]) {
            Some(receiver) => receiver,
            None => return Some(None),
        };
        let receiver_ty = expr_ty(&receiver);
        let TypeKind::Slice(element) = &receiver_ty.kind else {
            self.error(
                expr_span(&receiver),
                format!("`{name}` expects an array, found `{receiver_ty}`"),
            );
            return Some(None);
        };
        let element = (**element).clone();
        if name == "free_slice" {
            let binding = path.last().cloned().unwrap_or_default();
            if let Err(error) = self.move_out(&binding, *ident_span) {
                self.error(error.span, error.message);
            }
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: unit(span),
            }));
        }
        if !slice_element_is_operable(&element.kind) {
            self.error(expr_span(&receiver), unsupported_element_message(&element));
            return Some(None);
        }
        if name == "pop" {
            // A `string` element is a reference the *slice* owns. Removing the
            // last one would leave the returned handle as the only owner, and
            // an `Option[string]` binding is not tracked for release yet, so
            // popping would leak the string. Reading through the index or a
            // `for-in` borrows instead, which is why those are allowed.
            if matches!(element.kind, TypeKind::Str) {
                self.error(
                    expr_span(&receiver),
                    "popping a `string` is not supported yet: the array owns the reference, and a removed element would be handed out unowned",
                );
                return Some(None);
            }
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: option(&element, span),
            }));
        }
        let value = match self.check_expr_with_expected(&args[1], &element) {
            Some(value) => value,
            None => return Some(None),
        };
        if !fits(&element, &expr_ty(&value)) {
            self.error(
                expr_span(&value),
                format!("expected `{element}`, found `{}`", expr_ty(&value)),
            );
            return Some(None);
        }
        Some(Some(HirExpr::Call {
            callee: vec![name.to_string()],
            args: vec![receiver, value],
            unsafe_call: false,
            span,
            ty: unit(span),
        }))
    }

    /// `len(slice) -> i64` and `len(string) -> i64`.
    ///
    /// The table declares only the string shape, so a slice would fail
    /// argument matching. Checking the argument here is also what lets both
    /// readings share one spelling without widening the table: the argument is
    /// checked once, then the *type* decides whether this is a string length or
    /// a header field.
    fn check_len_builtin(&mut self, args: &[Expr], span: Span) -> Option<HirExpr> {
        if args.len() != 1 {
            self.error(span, "`len` takes 1 argument");
            return None;
        }
        let argument = self.check_expr(&args[0])?;
        let ty = expr_ty(&argument);
        if !matches!(ty.kind, TypeKind::Str | TypeKind::Slice(_)) {
            self.error(
                expr_span(&argument),
                format!("`len` expects a string or an array, found `{ty}`"),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["len".to_string()],
            args: vec![argument],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::I64),
        })
    }

    /// `map_new()` — the empty table's key and value types come from context.
    ///
    /// There is no argument to infer from, so the call is only typed when the
    /// surrounding annotation, parameter, or return type is already a
    /// `map[K]V`. That keeps `var m = map_new();` an error instead of guessing
    /// a key type the rest of the program may not agree with.
    fn check_map_new(
        &mut self,
        args: &[Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> Option<HirExpr> {
        if !args.is_empty() {
            self.error(
                span,
                format!("`map_new` takes no arguments, found {}", args.len()),
            );
            return None;
        }
        let Some(expected) = expected else {
            self.error(
                span,
                "cannot infer the map's key and value types: write `var m: map[K]V = map_new()`",
            );
            return None;
        };
        if !matches!(expected.kind, TypeKind::Map(_, _)) {
            self.error(
                span,
                format!("`map_new` needs a `map[K]V` annotation, found `{expected}`"),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["map_new".to_string()],
            args: Vec::new(),
            unsafe_call: false,
            span,
            ty: expected.clone(),
        })
    }

    /// Check `map_get`, `map_insert`, `map_remove`, `map_len`, `map_free`,
    /// `map_keys`, `map_values`, and `map_contains_key`.
    ///
    /// Every one takes the map as its first argument, so the key and value
    /// types are known before the other arguments are checked. The receiver
    /// must be a plain binding: `map_free` moves it and the other operations
    /// look it up by name in lowering, so a temporary or a field would either
    /// leak or silently do nothing.
    ///
    /// The outer `Option` says "this name is a map builtin"; the inner one is
    /// the checked expression, `None` after a reported error.
    fn check_map_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let arity = match name {
            "map_insert" => 3,
            "map_get" | "map_remove" | "map_contains_key" => 2,
            _ => 1,
        };
        if args.len() != arity {
            self.error(
                span,
                format!(
                    "`{name}` takes {arity} argument{}",
                    if arity == 1 { "" } else { "s" }
                ),
            );
            return Some(None);
        }
        let Expr::Ident(path, ident_span) = &args[0] else {
            self.error(
                span,
                format!("`{name}` needs a map binding, not a temporary"),
            );
            return Some(None);
        };
        if path.len() != 1 {
            self.error(
                *ident_span,
                format!("`{name}` needs a map binding, not a field access"),
            );
            return Some(None);
        }
        let receiver = match self.check_expr(&args[0]) {
            Some(receiver) => receiver,
            None => return Some(None),
        };
        let receiver_ty = expr_ty(&receiver);
        let TypeKind::Map(key, value) = &receiver_ty.kind else {
            self.error(
                expr_span(&receiver),
                format!("`{name}` expects a map, found `{receiver_ty}`"),
            );
            return Some(None);
        };
        let key = (**key).clone();
        let value = (**value).clone();

        if name == "map_free" {
            let binding = path.last().cloned().unwrap_or_default();
            if let Err(error) = self.move_out(&binding, *ident_span) {
                self.error(error.span, error.message);
            }
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: unit(span),
            }));
        }
        if name == "map_len" {
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: prim(TypeKind::I64),
            }));
        }
        if name == "map_keys" {
            // The array is a fresh copy of the table's keys, so it is a linear
            // value the caller owns and frees.
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: Type {
                    kind: TypeKind::Slice(Box::new(key.clone())),
                    span,
                },
            }));
        }
        if name == "map_values" {
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: Type {
                    kind: TypeKind::Slice(Box::new(value.clone())),
                    span,
                },
            }));
        }

        let key_expr = match self.check_expr_with_expected(&args[1], &key) {
            Some(key_expr) => key_expr,
            None => return Some(None),
        };
        if !fits(&key, &expr_ty(&key_expr)) {
            self.error(
                expr_span(&key_expr),
                format!("expected `{key}`, found `{}`", expr_ty(&key_expr)),
            );
            return Some(None);
        }
        if name == "map_get" || name == "map_remove" {
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver, key_expr],
                unsafe_call: false,
                span,
                ty: option(&value, span),
            }));
        }
        if name == "map_contains_key" {
            return Some(Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver, key_expr],
                unsafe_call: false,
                span,
                ty: prim(TypeKind::Bool),
            }));
        }
        // `map_insert`
        let value_expr = match self.check_expr_with_expected(&args[2], &value) {
            Some(value_expr) => value_expr,
            None => return Some(None),
        };
        if !fits(&value, &expr_ty(&value_expr)) {
            self.error(
                expr_span(&value_expr),
                format!("expected `{value}`, found `{}`", expr_ty(&value_expr)),
            );
            return Some(None);
        }
        Some(Some(HirExpr::Call {
            callee: vec![name.to_string()],
            args: vec![receiver, key_expr, value_expr],
            unsafe_call: false,
            span,
            ty: unit(span),
        }))
    }

    /// The M7 builtins: threads, mutexes and channels.
    ///
    /// They are intercepted rather than declared in the fixed table for the
    /// same reason the slice and map builtins are. `thread_spawn` has no fixed
    /// arity -- its remaining arguments are the body's own parameters -- and
    /// every channel operation needs the element type that `Channel[T]` carries
    /// in the receiver's binding.
    ///
    /// The outer `Option` says "this name is a thread builtin"; the inner one
    /// is the checked expression, `None` after a reported error.
    fn check_thread_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        if name == "thread_spawn" {
            self.uses_threads = true;
        }
        let checked = match name {
            "thread_spawn" => self.check_thread_spawn(args, span),
            "thread_join" => self.check_thread_join(args, span),
            "mutex_new" | "mutex_lock" | "mutex_unlock" | "mutex_free" => {
                self.check_mutex_builtin(name, args, span)
            }
            "chan_new" => self.check_chan_new(args, span, None),
            _ => self.check_chan_builtin(name, args, span),
        };
        Some(checked)
    }

    /// `thread_spawn(body, args...)`.
    ///
    /// The first argument names an ordinary function, and the rest are checked
    /// against its parameters exactly as a direct call's arguments would be.
    /// The body must return `()`: a thread has no caller to hand a value back
    /// to, and silently dropping one (a linear value especially) would be a
    /// leak the source never spelled.
    ///
    /// Arguments are copied into the thread's entry record:
    ///
    /// - a `string` is retained for the thread, which drops its reference when
    ///   the body returns (the atomic form once Task 4 lands);
    /// - a `Mutex` or `Channel` is *borrowed*: the parent keeps the binding and
    ///   must free it only after the join, which is the whole point of sharing
    ///   a handle with a thread;
    /// - a linear container or pointer is rejected for now, because the byte
    ///   copy would leave the body and the parent both owning it.
    fn check_thread_spawn(&mut self, args: &[Expr], span: Span) -> Option<HirExpr> {
        let (first, rest) = match args.split_first() {
            Some(parts) => parts,
            None => {
                self.error(span, "`thread_spawn` takes a function and its arguments");
                return None;
            }
        };
        let Expr::Ident(path, ident_span) = first else {
            self.error(
                span,
                "`thread_spawn` needs a function as its first argument, not a temporary",
            );
            return None;
        };
        if path.len() != 1 {
            self.error(
                *ident_span,
                "`thread_spawn` needs a function name, not a field access",
            );
            return None;
        }
        let body_name = path.last().cloned().unwrap_or_default();
        let Some(body_ty) = self.scope.resolve(&body_name).map(|binding| binding.ty.clone()) else {
            self.error(
                *ident_span,
                format!("`thread_spawn` needs a function, `{body_name}` is not declared"),
            );
            return None;
        };
        let TypeKind::Func(params, ret) = &body_ty.kind else {
            self.error(
                *ident_span,
                format!("`thread_spawn` needs a function, `{body_name}` is `{body_ty}`"),
            );
            return None;
        };
        if !matches!(ret.kind, TypeKind::Unit | TypeKind::Never) {
            self.error(
                *ident_span,
                format!("a thread body must return `()`; `{body_name}` returns `{ret}`"),
            );
            return None;
        }
        if params.len() != rest.len() {
            self.error(
                span,
                format!(
                    "expected {} arguments, found {}",
                    params.len(),
                    rest.len()
                ),
            );
            return None;
        }

        let params = params.clone();
        let mut checked = Vec::with_capacity(args.len());
        checked.push(HirExpr::Ident(vec![body_name], *ident_span, body_ty));
        for (argument, parameter) in rest.iter().zip(params.iter()) {
            let value = self.check_expr_with_expected(argument, parameter)?;
            let found = expr_ty(&value);
            if !fits(parameter, &found) {
                self.error(
                    expr_span(&value),
                    format!("expected `{parameter}`, found `{found}`"),
                );
                return None;
            }
            if linearity_of(&parameter.kind) == Linearity::Linear
                && !matches!(parameter.kind, TypeKind::Mutex | TypeKind::Channel(_))
            {
                self.error(
                    expr_span(&value),
                    format!(
                        "a `{parameter}` argument cannot cross a thread boundary yet: the \
                         runtime copies the value, so the body and the parent would both own it"
                    ),
                );
                return None;
            }
            checked.push(value);
        }

        Some(HirExpr::Call {
            callee: vec!["thread_spawn".to_string()],
            args: checked,
            unsafe_call: false,
            span,
            ty: prim(TypeKind::I64),
        })
    }

    /// `thread_join(handle)`.
    ///
    /// The handle is an `i64` at the source level even though the runtime ABI
    /// is `i32`: ordinary Otter code cannot spell a C width, and a handle is
    /// only ever handed back to `thread_join` or forgotten.
    fn check_thread_join(&mut self, args: &[Expr], span: Span) -> Option<HirExpr> {
        if args.len() != 1 {
            self.error(
                span,
                format!("`thread_join` takes 1 argument, found {}", args.len()),
            );
            return None;
        }
        let handle = self.check_expr(&args[0])?;
        let found = expr_ty(&handle);
        if !fits(&prim(TypeKind::I64), &found) {
            self.error(
                expr_span(&handle),
                format!("expected `i64`, found `{found}`"),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["thread_join".to_string()],
            args: vec![handle],
            unsafe_call: false,
            span,
            ty: unit(span),
        })
    }

    /// `mutex_new()`, `mutex_lock(m)`, `mutex_unlock(m)` and `mutex_free(m)`.
    ///
    /// The receiver must be a plain binding: `mutex_free` moves it, and a
    /// temporary handle would leak. Locking and unlocking borrow, so the same
    /// binding can be locked, unlocked and later freed.
    fn check_mutex_builtin(&mut self, name: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        if name == "mutex_new" {
            if !args.is_empty() {
                self.error(
                    span,
                    format!("`mutex_new` takes no arguments, found {}", args.len()),
                );
                return None;
            }
            return Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: Vec::new(),
                unsafe_call: false,
                span,
                ty: prim(TypeKind::Mutex),
            });
        }

        if args.len() != 1 {
            self.error(
                span,
                format!("`{name}` takes 1 argument, found {}", args.len()),
            );
            return None;
        }
        let Expr::Ident(path, ident_span) = &args[0] else {
            self.error(
                span,
                format!("`{name}` needs a mutex binding, not a temporary"),
            );
            return None;
        };
        if path.len() != 1 {
            self.error(
                *ident_span,
                format!("`{name}` needs a mutex binding, not a field access"),
            );
            return None;
        }
        let receiver = self.check_expr(&args[0])?;
        let found = expr_ty(&receiver);
        if !matches!(found.kind, TypeKind::Mutex) {
            self.error(
                expr_span(&receiver),
                format!("`{name}` expects a Mutex, found `{found}`"),
            );
            return None;
        }
        if name == "mutex_free" {
            let binding = path.last().cloned().unwrap_or_default();
            if let Err(error) = self.move_out(&binding, *ident_span) {
                self.error(error.span, error.message);
            }
        }
        Some(HirExpr::Call {
            callee: vec![name.to_string()],
            args: vec![receiver],
            unsafe_call: false,
            span,
            ty: unit(span),
        })
    }

    /// `chan_new(capacity)` - the element type comes from the binding's
    /// `Channel[T]` annotation, exactly as `map_new`'s key and value do.
    fn check_chan_new(
        &mut self,
        args: &[Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> Option<HirExpr> {
        if args.len() != 1 {
            self.error(
                span,
                format!(
                    "`chan_new` takes 1 argument (the capacity), found {}",
                    args.len()
                ),
            );
            return None;
        }
        let capacity = self.check_expr_with_expected(&args[0], &prim(TypeKind::I64))?;
        let found = expr_ty(&capacity);
        if !fits(&prim(TypeKind::I64), &found) {
            self.error(
                expr_span(&capacity),
                format!("expected `i64`, found `{found}`"),
            );
            return None;
        }
        let Some(expected) = expected else {
            self.error(
                span,
                "cannot infer the channel's element type: write `var c: Channel[i64] = chan_new(4)`",
            );
            return None;
        };
        if !matches!(expected.kind, TypeKind::Channel(_)) {
            self.error(
                span,
                format!("`chan_new` needs a `Channel[T]` annotation, found `{expected}`"),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["chan_new".to_string()],
            args: vec![capacity],
            unsafe_call: false,
            span,
            ty: expected.clone(),
        })
    }

    /// `chan_send(c, value)`, `chan_recv(c)`, `chan_close(c)` and
    /// `chan_free(c)`. Only `chan_free` consumes the handle.
    fn check_chan_builtin(&mut self, name: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        let arity = if name == "chan_send" { 2 } else { 1 };
        if args.len() != arity {
            self.error(
                span,
                format!(
                    "`{name}` takes {arity} argument{}",
                    if arity == 1 { "" } else { "s" }
                ),
            );
            return None;
        }
        let Expr::Ident(path, ident_span) = &args[0] else {
            self.error(
                span,
                format!("`{name}` needs a channel binding, not a temporary"),
            );
            return None;
        };
        if path.len() != 1 {
            self.error(
                *ident_span,
                format!("`{name}` needs a channel binding, not a field access"),
            );
            return None;
        }
        let receiver = self.check_expr(&args[0])?;
        let found = expr_ty(&receiver);
        let TypeKind::Channel(element) = &found.kind else {
            self.error(
                expr_span(&receiver),
                format!("`{name}` expects a channel, found `{found}`"),
            );
            return None;
        };
        let element = (**element).clone();

        if name == "chan_free" {
            let binding = path.last().cloned().unwrap_or_default();
            if let Err(error) = self.move_out(&binding, *ident_span) {
                self.error(error.span, error.message);
            }
            return Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: unit(span),
            });
        }
        if name == "chan_close" {
            return Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: unit(span),
            });
        }
        if name == "chan_recv" {
            let ty = option(&element, span);
            return Some(HirExpr::Call {
                callee: vec![name.to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty,
            });
        }

        // `chan_send`
        let value = self.check_expr_with_expected(&args[1], &element)?;
        let value_ty = expr_ty(&value);
        if !fits(&element, &value_ty) {
            self.error(
                expr_span(&value),
                format!("expected `{element}`, found `{value_ty}`"),
            );
            return None;
        }
        Some(HirExpr::Call {
            callee: vec![name.to_string()],
            args: vec![receiver, value],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Bool),
        })
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<HirExpr> {
        match expr {
            Expr::Literal(Literal::I64(v), span) => Some(HirExpr::Literal(
                Literal::I64(*v),
                *span,
                prim(TypeKind::I64),
            )),
            Expr::Literal(Literal::F64(v), span) => Some(HirExpr::Literal(
                Literal::F64(*v),
                *span,
                prim(TypeKind::F64),
            )),
            Expr::Literal(Literal::Bool(v), span) => Some(HirExpr::Literal(
                Literal::Bool(*v),
                *span,
                prim(TypeKind::Bool),
            )),
            Expr::Literal(Literal::Str(lit), span) => Some(HirExpr::Literal(
                Literal::Str(lit.clone()),
                *span,
                prim(TypeKind::Str),
            )),
            Expr::Ident(path, span) => {
                // `None` is a value constructor, not a variable. It has no
                // payload to infer from on its own, so `check_constructor`
                // reports "cannot infer the payload type of `None`" — which is
                // the useful message. Resolving it as an ordinary name instead
                // produced "cannot find value `None` in this scope", which
                // names a spelling problem where the real problem is context.
                if path.len() == 1 && path.first().map(String::as_str) == Some("None") {
                    return self.check_constructor("None", &[], *span, None)?;
                }
                // `Color.Red` is a unit variant's constructor. An enum has no
                // fields, so a two-segment name whose head names one cannot be
                // anything else.
                if path.len() == 2 {
                    if let Some(constructed) = self.check_enum_path(path, *span) {
                        return constructed;
                    }
                }
                // A dotted name is a chain of field accesses: `p.x`, `a.b.c`.
                // The parser produces one `Ident` for the whole dotted name (see
                // its `x`, `std.io.puts`), so the decision lands here, where the
                // types are — a dotted name is a module path only once modules
                // exist, and today the only dotted name that resolves to a value
                // is a field chain.
                if path.len() > 1 {
                    return self.check_field_path(path, *span);
                }
                let name = path.last().cloned().unwrap_or_default();
                if let Err(e) = self.scope.check_live(&name, *span) {
                    self.error(e.span, e.message);
                    return None;
                }
                match self.lookup(&name, *span) {
                    NameLookup::Found(binding) => {
                        // `main` is the entry point, and it is emitted with the
                        // operating system's `i32` signature rather than its
                        // Otter one. It can be called; it cannot be a value.
                        if name == "main" && matches!(binding.ty.kind, TypeKind::Func(_, _)) {
                            self.error(
                                *span,
                                "`main` is the entry point and cannot be used as a value",
                            );
                            return None;
                        }
                        Some(HirExpr::Ident(path.clone(), *span, binding.ty.clone()))
                    }
                    NameLookup::Capture { index, ty } => Some(HirExpr::Capture {
                        index,
                        span: *span,
                        ty,
                    }),
                    NameLookup::Missing => {
                        self.error(*span, format!("cannot find value `{name}` in this scope"));
                        None
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let lhs = self.check_expr(lhs)?;
                let rhs = self.check_expr(rhs)?;
                let lhs_ty = expr_ty(&lhs);
                let rhs_ty = expr_ty(&rhs);
                match binary_result_type(*op, &lhs_ty, &rhs_ty) {
                    Ok(ty) => {
                        // A struct's equality is its fields' equality, so the
                        // comparison is built here rather than left to the
                        // backend, which would have to compare the aggregate
                        // as a whole.
                        if matches!(op, BinOp::Eq | BinOp::Ne) {
                            if let TypeKind::Struct(name) = &lhs_ty.kind {
                                if let Some(compared) =
                                    self.struct_equality(*op, &lhs, &rhs, name, *span)
                                {
                                    return Some(compared);
                                }
                            }
                        }
                        let checked =
                            is_checked_arithmetic(*op) && matches!(ty.kind, TypeKind::I64);
                        let fault = self.fault_site(*span, checked);
                        Some(HirExpr::Binary {
                            op: *op,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                            span: *span,
                            ty,
                            fault,
                        })
                    }
                    Err(message) => {
                        self.error(*span, message);
                        None
                    }
                }
            }
            Expr::Unary { op, operand, span } => {
                let operand = self.check_expr(operand)?;
                let operand_ty = expr_ty(&operand);
                let result = match op {
                    UnOp::Neg if is_numeric(&operand_ty) => Ok(operand_ty),
                    UnOp::Not if operand_ty.eq_ignoring_span(&prim(TypeKind::Bool)) => {
                        Ok(prim(TypeKind::Bool))
                    }
                    UnOp::Neg => Err(format!("cannot apply `-` to `{operand_ty}`")),
                    UnOp::Not => Err(format!("cannot apply `!` to `{operand_ty}`")),
                };
                match result {
                    Ok(ty) => {
                        let checked = matches!(op, UnOp::Neg) && matches!(ty.kind, TypeKind::I64);
                        let fault = self.fault_site(*span, checked);
                        Some(HirExpr::Unary {
                            op: *op,
                            operand: Box::new(operand),
                            span: *span,
                            ty,
                            fault,
                        })
                    }
                    Err(message) => {
                        self.error(*span, message);
                        None
                    }
                }
            }
            Expr::Move(inner, span) => {
                // Check the operand first. A read of an already-moved binding
                // must be reported as such, and it must be reported *before*
                // anything spends the value a second time.
                let checked = self.check_expr(inner)?;
                let ty = expr_ty(&checked);
                // `move` transfers ownership only when the operand names a
                // linear binding. Everything else -- a call, a literal, a copy
                // type -- already produces a value with one owner, so `move`
                // there is a no-op rather than an error: being explicit about
                // intent is not a mistake.
                if let Expr::Ident(path, ident_span) = inner.as_ref() {
                    if matches!(linearity_of(&ty.kind), Linearity::Linear) {
                        let name = path.last().cloned().unwrap_or_default();
                        if let Err(e) = self.move_out(&name, *ident_span) {
                            self.error(e.span, e.message);
                        }
                    }
                }
                Some(HirExpr::Move(Box::new(checked), *span, ty))
            }
            Expr::MethodCall {
                receiver,
                method,
                args,
                span,
            } => {
                // A method called on a value: the result of a call, say. The
                // method is found by the receiver's type, exactly as it is
                // when the receiver is a binding.
                let Some(receiver_checked) = self.check_expr(receiver) else {
                    return None;
                };
                let receiver_ty = expr_ty(&receiver_checked);
                match self.check_user_method_on(receiver_checked, method, args, *span) {
                    Some(checked) => checked,
                    None => {
                        // The receiver is a value, not a binding. The builtin
                        // methods -- an array's, a map's -- need a binding they
                        // can update, and a user's method needs a value of the
                        // type it belongs to.
                        self.error(
                            *span,
                            format!(
                                "a method needs a binding as the receiver, not a value \
                                 of type `{receiver_ty}`"
                            ),
                        );
                        None
                    }
                }
            }
            Expr::Call {
                callee,
                args,
                unsafe_call,
                span,
            } => {
                // `Enum.Variant(args)` constructs a user enum. Checked first:
                // the ordinary path resolves only the last segment, so `A` in
                // `Shape.Circle(...)` would otherwise be reported as an unknown
                // function and `Shape` as an unknown value.
                if callee.len() == 2 {
                    if let Some(constructed) = self.check_enum_call(callee, args, *span) {
                        return constructed;
                    }
                }
                // A method receiver has to be a local binding. The parser keeps
                // a field receiver as a longer path (`s.items.push`), which is
                // not a module path once its first segment resolves locally:
                // report the missing binding instead of looking for a function
                // named `push`. A non-local first segment is left alone, so
                // `std.io.print(...)` is untouched.
                if callee.len() > 2 && self.scope.resolve(&callee[0]).is_some() {
                    self.error(
                        *span,
                        "method calls need a local binding as the receiver: \
                         a field access is not a binding",
                    );
                    return None;
                }
                // `xs.len()` arrives as the two-segment callee `["xs", "len"]`,
                // because the parser flattens a dotted call. If the first
                // segment names a local value, this is a receiver method, not a
                // module path: rewrite it to the receiver-first builtin and
                // check that. A first segment that is not a local stays on the
                // ordinary path, so `std.io.print(...)` is untouched.
                if let Some(checked) = self.check_builtin_method(callee, args, *span) {
                    return checked;
                }
                let name = callee.last().cloned().unwrap_or_default();
                // A generic function is found by name like an ordinary one, but
                // its signature has type variables: the substitution comes from
                // the argument types, and the body is checked at that
                // substitution rather than here.
                if callee.len() == 1 && self.generics.contains_key(&name) {
                    return self.check_generic_call(&name, args, *span);
                }
                // The method surface replaced these free-function spellings.
                // They are not aliases: a call written this way points at the
                // method form with the receiver the caller already wrote. This
                // is checked before constructors so `push`/`len` cannot fall
                // through to a scope lookup that no longer has them.
                if callee.len() == 1 {
                    if let Some((method, arity)) = old_builtin_method_name(&name) {
                        if args.len() == arity {
                            self.report_old_builtin(&name, method, args, *span);
                            return None;
                        }
                    }
                }
                // `None`, `Some(x)`, `Ok(v)` and `Err(e)` are value
                // constructors, not calls. They resolve by name and arity,
                // not by scope lookup — `None` takes no arguments and the
                // others take exactly one.
                if callee.len() == 1 {
                    if let Some(constructed) = self.check_constructor(&name, args, *span, None)? {
                        return Some(constructed);
                    }
                    if name == "map_new" {
                        return self.check_map_new(args, *span, None);
                    }
                    if matches!(
                        name.as_str(),
                        "thread_spawn" | "thread_join" | "mutex_new" | "chan_new"
                    ) {
                        return self.check_thread_builtin(&name, args, *span).flatten();
                    }
                }
                // A `const fn` has no run-time existence, so a call to one
                // outside a `const` initializer cannot be compiled: there is
                // no function of that name to call. The message says so
                // rather than leaving the reader with "cannot find function".
                if callee.len() == 1 && self.const_fns.contains_key(&name) {
                    self.error(
                        *span,
                        format!(
                            "`{name}` is a `const fn`: it can only be called in a \
                             `const` initializer"
                        ),
                    );
                    return None;
                }
                // The callee is resolved *before* the arguments are checked,
                // because each argument is checked against its parameter's
                // declared type. Checking arguments first and comparing
                // afterwards works for concrete arguments but cannot type a
                // constructor that has nothing to infer from: `f(None)` needs
                // `f`'s parameter type to know what `None` is an `Option` of.
                let resolved = match self.lookup(&name, *span) {
                    NameLookup::Found(binding) => Some(binding.ty),
                    // A capture is a value, not a missing name: the call goes
                    // through the captured function.
                    NameLookup::Capture { index, ty } => {
                        return self.check_capture_call(index, ty, args, *span);
                    }
                    NameLookup::Missing => None,
                };
                match resolved {
                    Some(ty) => match ty.kind {
                        TypeKind::Func(params, ret) => {
                            if params.len() != args.len() {
                                self.error(
                                    *span,
                                    format!(
                                        "expected {} arguments, found {}",
                                        params.len(),
                                        args.len()
                                    ),
                                );
                                return None;
                            }
                            // `assert_eq` and `assert_ne` compare two values of
                            // *the same* type, which is any of the four with an
                            // equality, so they are built before the declared
                            // parameter types are applied and the table's `i64`
                            // pair is never consulted. See
                            // `Checker::assert_cmp_call`.
                            if callee.len() == 1 && (name == "assert_eq" || name == "assert_ne") {
                                return self.assert_cmp_call(&name, args, *span);
                            }
                            let mut arg_exprs = Vec::with_capacity(args.len());
                            for (i, (arg, param)) in args.iter().zip(params.iter()).enumerate() {
                                let checked = self.check_expr_with_expected(arg, param)?;
                                if !fits(param, &expr_ty(&checked)) {
                                    self.error(
                                        expr_span(&checked),
                                        format!("expected `{param}`, found `{}`", expr_ty(&checked)),
                                    );
                                }
                                // A parameter of linear type takes ownership, so
                                // passing a bare name moves it out --
                                // `dealloc(buf)`, and a user function taking
                                // `*mut T`, both consume without a `move` at the
                                // call site. An argument that is not a bare name
                                // (a fresh `alloc(...)`, say) owns nothing yet and
                                // is left alone.
                                //
                                // `arena_alloc` is the exception: an arena is
                                // reusable across many allocations, so its handle
                                // is not consumed by `alloc`. Only
                                // `arena_free_all` consumes the handle, which is
                                // what makes a double free a type error.
                                let consumes = !(name == "arena_alloc" && i == 0);
                                if consumes
                                    && matches!(linearity_of(&param.kind), Linearity::Linear)
                                {
                                    if let Expr::Ident(path, arg_span) = arg {
                                        let arg_name = path.last().cloned().unwrap_or_default();
                                        if let Err(e) = self.move_out(&arg_name, *arg_span) {
                                            self.error(e.span, e.message);
                                        }
                                    }
                                }
                                arg_exprs.push(checked);
                            }
                            // A raw pointer is an address, so there is nothing
                            // to copy at the boundary: C may keep or free what
                            // it names. The call site must say `unsafe`, which
                            // is what makes that risk visible where it is taken.
                            if self.externs.contains(&name) && !*unsafe_call {
                                for checked in &arg_exprs {
                                    let arg_ty = expr_ty(checked);
                                    if matches!(linearity_of(&arg_ty.kind), Linearity::Linear) {
                                        self.error(
                                            expr_span(checked),
                                            format!(
                                                "passing `{arg_ty}` to `{name}` (an `extern` \
                                                 function) requires an `unsafe` call: C may keep \
                                                 or free the pointer"
                                            ),
                                        );
                                    }
                                }
                            }
                            // `panic` and `assert` are sugar in the same way
                            // interpolation is: the position they report is a
                            // fact only this stage holds, so it is resolved here
                            // and the call handed down carries it. The arity
                            // check above already pinned `arg_exprs` to one
                            // element. Every other builtin passes through
                            // untouched.
                            if callee.len() == 1 && name == "panic" {
                                if let Some(message) = arg_exprs.first().cloned() {
                                    return Some(self.panic_call(message, *span));
                                }
                            }
                            if callee.len() == 1 && name == "assert" {
                                if let Some(condition) = arg_exprs.first().cloned() {
                                    return Some(self.assert_call(condition, *span));
                                }
                            }
                            if callee.len() == 1 && name == "debug_assert" {
                                // A release build drops the call entirely: the
                                // condition is checked (above) and its tree is
                                // discarded, so nothing is lowered and nothing
                                // is printed. That is the whole point of the
                                // name -- an assertion you pay for in a debug
                                // build and not in a release one.
                                if self.release {
                                    return Some(HirExpr::Call {
                                        callee: vec![name.clone()],
                                        args: Vec::new(),
                                        unsafe_call: false,
                                        span: *span,
                                        ty: unit(*span),
                                    });
                                }
                                if let Some(condition) = arg_exprs.first().cloned() {
                                    return Some(self.assert_call(condition, *span));
                                }
                            }
                            Some(HirExpr::Call {
                                callee: callee.clone(),
                                args: arg_exprs,
                                unsafe_call: *unsafe_call,
                                span: *span,
                                ty: (*ret).clone(),
                            })
                        }
                        _ => {
                            self.error(*span, format!("`{name}` is not a function (it is `{ty}`)"));
                            None
                        }
                    },
                    None => {
                        self.error(*span, format!("cannot find function `{name}`"));
                        // Check the arguments anyway, each one standing alone.
                        //
                        // Resolving the callee first is what lets `f(None)`
                        // infer what `None` is an `Option` of, but *stopping*
                        // here when the callee is unknown meant a typo in the
                        // function and a typo in an argument took two builds to
                        // find, one error at a time. There is no parameter type
                        // to check against, so this pass asks only whether each
                        // argument is valid by itself — and an argument that is
                        // fine reports nothing, so the common case of a
                        // misspelled callee with good arguments is unchanged.
                        for arg in args {
                            let _ = self.check_expr(arg);
                        }
                        None
                    }
                }
            }
            Expr::StructLit { path, fields, span } => {
                // `Shape.Rect { w, h }` constructs a variant with named fields.
                if path.len() == 2 {
                    if let Some(constructed) = self.check_enum_struct_lit(path, fields, *span) {
                        return constructed;
                    }
                }
                let name = path.last().cloned().unwrap_or_default();
                let sig = self.structs.get(&name).cloned();
                match sig {
                    Some(sig) => {
                        // The type is an item, and so are its fields: both are
                        // private to the module that declares them unless they
                        // are `pub`.
                        self.check_visibility(&name, *span);
                        // Every value the literal wrote is type-checked, whether
                        // or not its field name belongs to the struct; the
                        // diagnostics below need the written order to report what
                        // is missing and what is unknown.
                        let mut written: Vec<(String, HirExpr)> = Vec::new();
                        for (fname, value, _) in fields {
                            let value = self.check_expr(value)?;
                            written.push((fname.clone(), value));
                        }
                        let provided: Vec<&str> =
                            written.iter().map(|(n, _)| n.as_str()).collect();
                        for (fname, _) in &sig.fields {
                            if !provided.contains(&fname.as_str()) {
                                self.error(
                                    *span,
                                    format!("missing field `{fname}` in struct `{name}`"),
                                );
                            }
                        }
                        for (fname, fty) in &sig.fields {
                            if let Some((_, value)) = written.iter().find(|(n, _)| n == fname) {
                                let value_ty = expr_ty(value);
                                if !fty.eq_ignoring_span(&value_ty) {
                                    self.error(
                                        expr_span(value),
                                        format!("expected `{fty}`, found `{value_ty}`"),
                                    );
                                }
                            }
                        }
                        for (fname, _) in &written {
                            if !sig.fields.iter().any(|(n, _)| n == fname) {
                                self.error(
                                    *span,
                                    format!("unknown field `{fname}` in struct `{name}`"),
                                );
                                continue;
                            }
                            // A field that is not `pub` may not be written from
                            // another module.
                            if let Some(index) =
                                sig.fields.iter().position(|(n, _)| n == fname)
                            {
                                self.check_field_visibility(
                                    &name, fname, &sig, index, *span,
                                );
                            }
                        }
                        // Laid out in **declaration** order, not the order the
                        // literal spelled the fields in. Two literals of one
                        // struct written differently would otherwise define two
                        // different IR layouts for the same named type, and a
                        // field's index would depend on which literal happened to
                        // be lowered first.
                        let mut hir_fields = Vec::new();
                        for (fname, _) in &sig.fields {
                            if let Some((_, value)) = written.iter().find(|(n, _)| n == fname) {
                                hir_fields.push((
                                    fname.clone(),
                                    value.clone(),
                                    Span::new(0, 0),
                                ));
                            }
                        }
                        Some(HirExpr::StructLit {
                            path: path.clone(),
                            fields: hir_fields,
                            span: *span,
                            ty: Type {
                                kind: TypeKind::Struct(name),
                                span: *span,
                            },
                        })
                    }
                    None => {
                        self.error(*span, format!("cannot find struct `{name}`"));
                        None
                    }
                }
            }
            Expr::SliceLit {
                elem,
                elements,
                span,
            } => {
                let elem_ty = match self.resolve_type_in(elem, false) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.error(error.span, error.message);
                        return None;
                    }
                };
                if matches!(
                    elem_ty.kind,
                    TypeKind::Unit | TypeKind::Never | TypeKind::CVoid
                ) {
                    self.error(
                        *span,
                        format!(
                            "an array cannot hold `{elem_ty}`: its elements would have no size"
                        ),
                    );
                    return None;
                }
                if matches!(elem_ty.kind, TypeKind::Func(_, _)) {
                    // The same reason a struct cannot hold one: the array would
                    // hold a copy the environment does not know about.
                    self.error(
                        *span,
                        "an array cannot hold a function value yet",
                    );
                    return None;
                }
                let mut checked = Vec::with_capacity(elements.len());
                for element in elements {
                    let value = self.check_expr_with_expected(element, &elem_ty)?;
                    if !fits(&elem_ty, &expr_ty(&value)) {
                        self.error(
                            expr_span(&value),
                            format!("expected `{elem_ty}`, found `{}`", expr_ty(&value)),
                        );
                    }
                    checked.push(value);
                }
                let slice_ty = Type {
                    kind: TypeKind::Slice(Box::new(elem_ty.clone())),
                    span: *span,
                };
                Some(HirExpr::SliceLit {
                    elem_ty,
                    elements: checked,
                    span: *span,
                    ty: slice_ty,
                })
            }
            Expr::Index { base, index, span } => {
                // `p[i]` reads (or, in an assignment target, writes) an
                // element of a pointer, and always an `i64`. `xs[i]` reads an
                // element of an array -- growable, fixed, or a view -- and is
                // *always* an `Option`: indexing never traps, and out of range
                // including negative is `None`.
                let base = self.check_expr(base)?;
                let base_ty = expr_ty(&base);
                let (element, checked) = match &base_ty.kind {
                    TypeKind::Ptr(pointee) => ((**pointee).clone(), false),
                    TypeKind::Slice(element)
                    | TypeKind::FixedArray(element, _)
                    | TypeKind::View(element) => ((**element).clone(), true),
                    _ => {
                        self.error(*span, format!("cannot index `{base_ty}`"));
                        return None;
                    }
                };
                let index = self.check_expr(index)?;
                if !expr_ty(&index).eq_ignoring_span(&prim(TypeKind::I64)) {
                    self.error(
                        expr_span(&index),
                        "index must be `i64`",
                    );
                    return None;
                }
                let ty = if checked {
                    if !slice_element_is_operable(&element.kind) {
                        self.error(*span, unsupported_element_message(&element));
                        return None;
                    }
                    option(&element, *span)
                } else {
                    if matches!(element.kind, TypeKind::Unit | TypeKind::Never) {
                        self.error(
                            *span,
                            format!("a pointer to nothing cannot be read through: `*mut {element}`"),
                        );
                        return None;
                    }
                    // A pointer is an address; what it holds is whatever it
                    // points at. `p[0]` is the value at that address, and for
                    // a pointer to a struct that is the struct.
                    element.clone()
                };
                Some(HirExpr::Index {
                    base: Box::new(base),
                    index: Box::new(index),
                    span: *span,
                    ty,
                })
            }
            Expr::FixedArrayLit { elements, span } => {
                // The first element fixes the type; the rest have to fit it.
                // There is no empty spelling, so the type is never guessed.
                let Some(first) = elements.first() else {
                    self.error(
                        *span,
                        "a fixed array needs at least one element: write `[]T { ... }` for a \
                         growable array"
                            .to_string(),
                    );
                    return None;
                };
                let first = self.check_expr(first)?;
                let elem_ty = expr_ty(&first);
                if !fixed_array_element_is_supported(&elem_ty.kind) {
                    self.error(
                        expr_span(&first),
                        format!(
                            "a fixed array cannot hold `{elem_ty}`: a fixed array is copied by \
                             copying its bytes, which is exact for the built-in number and \
                             `bool` types and not yet defined for anything that owns a reference"
                        ),
                    );
                    return None;
                }
                let mut checked = Vec::with_capacity(elements.len());
                checked.push(first);
                for element in elements.iter().skip(1) {
                    checked.push(self.check_expr_with_expected(element, &elem_ty)?);
                }
                let ty = Type {
                    kind: TypeKind::FixedArray(Box::new(elem_ty.clone()), elements.len()),
                    span: *span,
                };
                Some(HirExpr::FixedArrayLit {
                    elem_ty,
                    elements: checked,
                    span: *span,
                    ty,
                })
            }
            Expr::TupleLit { elements, span } => {
                // Every element says its own type; the tuple is the product of
                // them. There is no inference to do and no empty spelling, so
                // the type is never guessed.
                let mut checked = Vec::with_capacity(elements.len());
                for (index, element) in elements.iter().enumerate() {
                    let value = self.check_expr(element)?;
                    let element_ty = expr_ty(&value);
                    if !tuple_element_is_supported(&element_ty.kind) {
                        self.error(
                            expr_span(&value),
                            format!(
                                "a tuple cannot hold `{element_ty}` (element {}): a tuple is \
                                 copied by copying its bytes, which is exact for the built-in \
                                 number and `bool` types and for other tuples of them, and not \
                                 yet defined for anything that owns a reference",
                                index + 1
                            ),
                        );
                        return None;
                    }
                    checked.push(value);
                }
                let element_types = checked.iter().map(|value| expr_ty(value)).collect();
                Some(HirExpr::TupleLit {
                    elements: checked,
                    span: *span,
                    ty: Type {
                        kind: TypeKind::Tuple(element_types),
                        span: *span,
                    },
                })
            }
            Expr::IndexRange { base, start, end, span } => {
                // A view borrows the array's storage, so the source has to be
                // a binding the checker can follow: a temporary would be gone
                // at the end of the expression and a field access is not
                // something the borrow rule tracks. The binding is marked
                // borrowed until the function returns, which is what makes
                // moving or mutating the array while the view exists an error.
                let base_checked = self.check_expr(base)?;
                let base_ty = expr_ty(&base_checked);
                let element = match &base_ty.kind {
                    TypeKind::Slice(element)
                    | TypeKind::FixedArray(element, _)
                    | TypeKind::View(element) => (**element).clone(),
                    _ => {
                        self.error(*span, format!("cannot take a view of `{base_ty}`"));
                        return None;
                    }
                };
                if !fixed_array_element_is_supported(&element.kind) {
                    self.error(
                        *span,
                        format!(
                            "a view of `{element}` is not supported yet: viewing a value that \
                             owns a reference would lend it out without saying so"
                        ),
                    );
                    return None;
                }
                let HirExpr::Ident(path, _, _) = &base_checked else {
                    self.error(
                        *span,
                        "a view needs a binding as its source: a field or a temporary may not \
                         outlive the view"
                            .to_string(),
                    );
                    return None;
                };
                if let Some(binding) = path.last() {
                    self.borrowed_bindings.push((binding.clone(), *span));
                }
                let start = self.check_expr_with_expected(start, &prim(TypeKind::I64))?;
                let end = self.check_expr_with_expected(end, &prim(TypeKind::I64))?;
                let ty = Type {
                    kind: TypeKind::View(Box::new(element)),
                    span: *span,
                };
                Some(HirExpr::View {
                    base: Box::new(base_checked),
                    start: Box::new(start),
                    end: Box::new(end),
                    span: *span,
                    ty,
                })
            }
            Expr::FieldAccess { base, field, span } => {
                let base = self.check_expr(base)?;
                let base_ty = expr_ty(&base);
                match &base_ty.kind {
                    TypeKind::Struct(sname) => {
                        match self
                            .structs
                            .get(sname)
                            .and_then(|s| s.fields.iter().position(|(n, _)| n == field))
                            .map(|index| (index, self.structs[sname].fields[index].1.clone()))
                        {
                            Some((index, fty)) => Some(HirExpr::FieldAccess {
                                base: Box::new(base),
                                field: field.clone(),
                                index,
                                span: *span,
                                ty: fty,
                            }),
                            None => {
                                self.error(
                                    *span,
                                    format!("struct `{sname}` has no field `{field}`"),
                                );
                                None
                            }
                        }
                    }
                    TypeKind::Enum(ename) => {
                        self.error(*span, format!("enum `{ename}` has no field `{field}`"));
                        None
                    }
                    _ => {
                        self.error(*span, format!("cannot access a field on `{}`", base_ty));
                        None
                    }
                }
            }
            Expr::Interp { parts, span } => {
                // The parser kept the parts as written because choosing a
                // conversion needs the type of each expression, which is what
                // this pass knows and the parser did not. Desugaring *here* --
                // into the `str_from_*` call and the `+` the language already
                // has -- means code generation never learns that interpolation
                // exists and so cannot lower it differently from the equivalent
                // written concatenation.
                let mut pieces: Vec<HirExpr> = Vec::new();
                for part in parts {
                    match part {
                        InterpPart::Text(lit) => pieces.push(HirExpr::Literal(
                            Literal::Str(lit.clone()),
                            *span,
                            prim(TypeKind::Str),
                        )),
                        InterpPart::Expr(expr, format) => {
                            let checked = self.check_expr(expr)?;
                            let piece = match format {
                                // Display is the form `str_from_*` builds.
                                InterpFormat::Display => self.as_string(checked, *span),
                                // Debug has no single conversion: the shape of
                                // the value decides, and aggregates get a
                                // synthesized printer. The diagnostic points at
                                // the expression, not at the whole literal.
                                InterpFormat::Debug => {
                                    let site = expr_span(&checked);
                                    self.debug_string(checked, site)
                                }
                            };
                            match piece {
                                Some(piece) => pieces.push(piece),
                                None => return None,
                            }
                        }
                    }
                }

                // A literal with no `${ ... }` is never an `Interp`, so there is
                // at least one piece; if a future edit ever made that false, the
                // empty string is the honest value rather than a panic.
                let mut joined = pieces.pop().unwrap_or_else(|| {
                    HirExpr::Literal(Literal::Str(empty_string()), *span, prim(TypeKind::Str))
                });
                while let Some(left) = pieces.pop() {
                    joined = HirExpr::Binary {
                        op: BinOp::Add,
                        lhs: Box::new(left),
                        rhs: Box::new(joined),
                        span: *span,
                        ty: prim(TypeKind::Str),
                        // A concatenation of strings cannot fault, which the
                        // type says as plainly as the comment would.
                        fault: None,
                    };
                }
                Some(joined)
            }
            Expr::Cast { expr, ty, span } => {
                let inner = self.check_expr(expr)?;
                // A cast target may name a C type: `malloc` takes a `usize`,
                // and the only way an Otter program can produce one is to say
                // so. This is also the one place a C spelling is legal outside
                // an `extern` block, because the conversion is explicit.
                let target = self.resolve_or_report_in(ty, *span, true, false);
                let source = expr_ty(&inner);
                if !is_numeric(&source) || !is_numeric(&target) || source.eq_ignoring_span(&target)
                {
                    self.error(*span, format!("cannot cast `{source}` to `{target}`"));
                    None
                } else {
                    Some(HirExpr::Cast {
                        expr: Box::new(inner),
                        ty: target,
                        span: *span,
                    })
                }
            }
            Expr::Try { expr, span } => {
                let inner = self.check_expr(expr)?;
                let inner_ty = expr_ty(&inner);
                let TypeKind::Result(ok, err) = &inner_ty.kind else {
                    self.error(
                        *span,
                        format!("`?` can only be used on a `Result`, not `{inner_ty}`"),
                    );
                    return None;
                };
                // Propagating an error means returning it, so the function
                // around the `?` has to be returning a `Result` -- and one
                // that can carry this error.
                match self.fn_ret.clone() {
                    Some(ret) => match &ret.kind {
                        TypeKind::Result(_, ret_err) => {
                            if !ret_err.eq_ignoring_span(err) {
                                self.error(
                                    *span,
                                    format!("error type mismatch: `{err}` vs `{ret_err}`"),
                                );
                                return None;
                            }
                        }
                        _ => {
                            self.error(*span, "`?` needs a function returning `Result`");
                            return None;
                        }
                    },
                    None => {
                        self.error(*span, "`?` needs a function returning `Result`");
                        return None;
                    }
                }
                Some(HirExpr::Try {
                    expr: Box::new(inner),
                    span: *span,
                    ty: (**ok).clone(),
                })
            }
            Expr::SwitchExpr {
                subject,
                arms,
                span,
            } => {
                let subject = self.check_expr(subject)?;
                let subject_ty = expr_ty(&subject);
                let mut hir_arms = Vec::new();
                let mut arm_ty: Option<Type> = None;
                for arm in arms {
                    let checked = self.check_arm_against(&subject_ty, arm)?;
                    let body_ty = expr_ty(&checked.body);
                    if let Some(message) = merge_arm_type(&mut arm_ty, body_ty) {
                        self.error(checked.span, message);
                    }
                    hir_arms.push(checked);
                }
                self.check_arms_exhaustive(&subject_ty, arms, *span)?;
                Some(HirExpr::SwitchExpr {
                    subject: Box::new(subject),
                    arms: hir_arms,
                    span: *span,
                    ty: arm_ty.unwrap_or_else(|| never(*span)),
                })
            }
            Expr::Lambda {
                params,
                ret,
                body,
                span,
            } => {
                // A lambda is a function without a name, so it is checked the
                // way a function is: its own scope, its parameters bound in it,
                // and its body checked against its declared return type.
                let param_tys: Vec<Type> = params
                    .iter()
                    .map(|param| self.resolve_or_report(&param.ty, param.span))
                    .collect();
                let ret_ty = match ret {
                    Some(ty) => self.resolve_or_report(ty, *span),
                    None => prim(TypeKind::Unit),
                };

                self.scope.push();
                for (param, param_ty) in params.iter().zip(param_tys.iter()) {
                    let _ = self.scope.declare(
                        param.name.clone(),
                        Binding {
                            ty: param_ty.clone(),
                            mutability: Mutability::Const,
                            span: param.span,
                            ownership: Ownership::Live,
                        },
                    );
                }
                // A name that resolves above this depth belongs to an enclosing
                // scope: a capture. The frame's depth is the scope the
                // parameters live in, so the closure's own names are fine and
                // a global is fine, and only an enclosing local is a capture.
                self.closures.push(ClosureFrame {
                    scope_depth: self.scope.depth() - 1,
                    captures: Vec::new(),
                });
                let checked_body = self.check_block_with_ret(body, &ret_ty);
                let frame = self.closures.pop().unwrap_or(ClosureFrame {
                    scope_depth: 0,
                    captures: Vec::new(),
                });
                self.scope.pop();

                let checked_body = checked_body?;
                let ty = Type {
                    kind: TypeKind::Func(param_tys.clone(), Box::new(ret_ty.clone())),
                    span: *span,
                };
                Some(HirExpr::Lambda {
                    f: Box::new(HirFn {
                        name: String::new(),
                        params: params
                            .iter()
                            .zip(param_tys.iter())
                            .map(|(param, param_ty)| HirParam {
                                name: param.name.clone(),
                                ty: param_ty.clone(),
                                span: param.span,
                            })
                            .collect(),
                        ret: ret_ty,
                        body: checked_body,
                        captures: frame.captures,
                        span: *span,
                    }),
                    span: *span,
                    ty,
                })
            }
        }
    }

    /// Check a call whose callee is a captured function value.
    ///
    /// The call is through the value, so the arguments are checked against the
    /// captured function's own parameter types, exactly as a call to a binding
    /// would be.
    fn check_capture_call(
        &mut self,
        index: usize,
        ty: Type,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let TypeKind::Func(params, ret) = ty.kind else {
            self.error(
                span,
                format!("this captured value is not a function (it is `{ty}`)"),
            );
            return None;
        };
        if params.len() != args.len() {
            self.error(
                span,
                format!(
                    "expected {} arguments, found {}",
                    params.len(),
                    args.len()
                ),
            );
            return None;
        }
        let mut arg_exprs = Vec::with_capacity(args.len());
        for (arg, param) in args.iter().zip(params.iter()) {
            let checked = self.check_expr_with_expected(arg, param)?;
            if !fits(param, &expr_ty(&checked)) {
                self.error(
                    expr_span(&checked),
                    format!("expected `{param}`, found `{}`", expr_ty(&checked)),
                );
            }
            arg_exprs.push(checked);
        }
        Some(HirExpr::CallCapture {
            index,
            args: arg_exprs,
            span,
            ty: (*ret).clone(),
        })
    }

    /// Resolve a dotted name as the field-access chain it is: `p.x`, `a.b.c`.
    ///
    /// The parser hands a dotted name over whole, so this is where the chain is
    /// taken apart. The first segment is an ordinary name — resolved through the
    /// same path a bare `p` takes, so a moved binding or an unbound name gets
    /// the diagnostic it would get on its own — and every later segment is a
    /// field of the struct the previous value has. Deeper chains are the same
    /// step repeated, so `a.b.c` needs no special handling.
    fn check_field_path(&mut self, path: &[String], span: Span) -> Option<HirExpr> {
        let first = path.first()?.clone();
        let mut current = self.check_expr(&Expr::Ident(vec![first], span))?;

        for field in path.iter().skip(1) {
            let base_ty = expr_ty(&current);
            let struct_name = match &base_ty.kind {
                TypeKind::Struct(name) => name.clone(),
                _ => {
                    self.error(span, format!("cannot access a field on `{base_ty}`"));
                    return None;
                }
            };
            let Some(sig) = self.structs.get(&struct_name).cloned() else {
                self.error(span, format!("struct `{struct_name}` has no field `{field}`"));
                return None;
            };
            let resolved = sig
                .fields
                .iter()
                .position(|(name, _)| name == field)
                .map(|index| (index, sig.fields[index].1.clone()));
            let (index, field_ty) = match resolved {
                Some(found) => found,
                None => {
                    self.error(span, format!("struct `{struct_name}` has no field `{field}`"));
                    return None;
                }
            };
            // A field is private to the module that declares it unless it is
            // `pub`, so reading one from another module is the mistake.
            self.check_field_visibility(&struct_name, field, &sig, index, span);
            current = HirExpr::FieldAccess {
                base: Box::new(current),
                field: field.clone(),
                index,
                span,
                ty: field_ty,
            };
        }
        Some(current)
    }

    /// Check `p.x = v` and `p.inner.x = v`, the field-assignment form of
    /// [`Stmt::Assign`].
    ///
    /// A field is a *place*, so the rules are the binding's rules one level
    /// down: the bottom of the chain has to be a `var` binding (a field of a
    /// `const` is as immutable as the `const`), every `.field` has to exist on
    /// the type the one before it produced, and the value has to fit the
    /// innermost field. What the checker resolves here -- each field's position
    /// and type -- is what keeps code generation from looking the struct up a
    /// second time.
    ///
    /// Returns `None` after reporting, like every other arm of `Stmt::Assign`.
    fn check_field_assign(&mut self, target: &Expr, value: &Expr, span: Span) -> Option<HirStmt> {
        // A field write through a pointer -- `self[0].value = v` -- is a
        // *store*, not a read-modify-write, so it takes a different shape.
        if let Some((base, index, names)) = split_pointer_target(target) {
            if !names.is_empty() {
                return self.check_pointer_field_assign(&base, &index, &names, value, span);
            }
        }
        let (root, names) = self.split_field_target(target)?;

        // The bottom of the chain is an ordinary name: resolve it the way a bare
        // `p` would be, and require it to be mutable.
        let Some(binding) = self.scope.resolve(&root).cloned() else {
            let _ = self.check_expr(value);
            self.error(
                otter_parser::exprs::expr_span(target),
                format!("cannot find value `{root}` in this scope"),
            );
            return None;
        };
        if !binding.mutability.is_mutable() {
            self.error(
                otter_parser::exprs::expr_span(target),
                format!("cannot assign to a field of `{root}`, which is immutable"),
            );
            return None;
        }

        // Walk the chain against the type each step produced. `steps` is what
        // the backend rebuilds the aggregates from, so it is built here, where
        // the declarations are known.
        let (steps, aggregate) = self.field_steps(binding.ty.clone(), &names)?;

        let checked = self.check_expr_with_expected(value, &aggregate)?;
        let found = expr_ty(&checked);
        if !aggregate.eq_ignoring_span(&found) {
            self.error(
                expr_span(&checked),
                format!("expected `{aggregate}`, found `{found}`"),
            );
        }
        // A field of a linear type may not be overwritten with a bare name any
        // more than a binding of one may: the same copy would be a second owner.
        self.forbid_linear_copy(value, &checked);

        Some(HirStmt::FieldAssign {
            root,
            steps,
            value: checked,
            ty: aggregate,
            span,
        })
    }

    /// `self[0].value = v` -- a field write through a pointer.
    ///
    /// The pointer is an address the program already has, so there is no
    /// binding to look for and no aggregate to rebuild: the write is a store at
    /// the field's offset. What the checker still owes the backend is the
    /// element type (to take the address of) and the field chain (to step
    /// through), and it resolves both here.
    fn check_pointer_field_assign(
        &mut self,
        base: &Expr,
        index: &Expr,
        names: &[(String, Span)],
        value: &Expr,
        span: Span,
    ) -> Option<HirStmt> {
        let pointer = self.check_expr(base)?;
        let pointer_ty = expr_ty(&pointer);
        let TypeKind::Ptr(element) = pointer_ty.kind.clone() else {
            self.error(
                expr_span(&pointer),
                format!("cannot write through `{pointer_ty}`: it is not a raw pointer"),
            );
            return None;
        };
        let index_checked = self.check_expr(index)?;
        let index_ty = expr_ty(&index_checked);
        if !matches!(index_ty.kind, TypeKind::I64) {
            self.error(
                expr_span(&index_checked),
                format!("a pointer index must be an `i64`, found `{index_ty}`"),
            );
            return None;
        }
        let (steps, aggregate) = self.field_steps(*element, names)?;

        let checked = self.check_expr_with_expected(value, &aggregate)?;
        let found = expr_ty(&checked);
        if !aggregate.eq_ignoring_span(&found) {
            self.error(
                expr_span(&checked),
                format!("expected `{aggregate}`, found `{found}`"),
            );
        }
        Some(HirStmt::FieldPtrAssign {
            pointer,
            index: index_checked,
            steps,
            value: checked,
            ty: aggregate,
            span,
        })
    }

    /// Resolve a chain of field names against `aggregate`, outermost first.
    ///
    /// Returns the chain the backend steps through and the type the last field
    /// has; `None` after reporting when a step does not name a struct field.
    fn field_steps(
        &mut self,
        aggregate: Type,
        names: &[(String, Span)],
    ) -> Option<(Vec<FieldStep>, Type)> {
        let mut steps: Vec<FieldStep> = Vec::new();
        let mut current = aggregate;
        for (field, fspan) in names {
            let TypeKind::Struct(sname) = &current.kind else {
                self.error(*fspan, format!("cannot access a field on `{current}`"));
                return None;
            };
            if !self.structs.contains_key(sname) {
                self.error(*fspan, format!("unknown struct `{sname}`"));
                return None;
            }
            let resolved = self.structs.get(sname).and_then(|sig| {
                sig.fields
                    .iter()
                    .position(|(name, _)| name == field)
                    .map(|index| (index, sig.fields[index].1.clone()))
            });
            let (index, field_ty) = match resolved {
                Some(found) => found,
                None => {
                    self.error(*fspan, format!("struct `{sname}` has no field `{field}`"));
                    return None;
                }
            };
            steps.push(FieldStep {
                index,
                ty: field_ty.clone(),
            });
            current = field_ty;
        }
        Some((steps, current))
    }

    /// Split an assignment target into the binding at the bottom of the chain and
    /// the field names above it, **root-first**.
    ///
    /// Two spellings reach here for the same program text: `p.x` is usually one
    /// [`Expr::Ident`] with two segments (the parser hands a dotted name over
    /// whole), while `move p.x` and a postfix `(expr).x` produce an
    /// [`Expr::FieldAccess`] chain. Both are taken apart here so the checker
    /// above has one shape to work with.
    ///
    /// Returns `None` after reporting when the target is not a chain of fields
    /// over a plain name.
    fn split_field_target(&mut self, target: &Expr) -> Option<(String, Vec<(String, Span)>)> {
        match target {
            Expr::FieldAccess { base, field, span } => {
                let (root, mut names) = self.split_field_target(base)?;
                names.push((field.clone(), *span));
                Some((root, names))
            }
            Expr::Ident(path, _) if path.len() > 1 => {
                let mut segments = path.iter();
                let root = segments.next()?.clone();
                let span = otter_parser::exprs::expr_span(target);
                let names = segments.map(|field| (field.clone(), span)).collect();
                Some((root, names))
            }
            // A one-segment name: the bottom of a `FieldAccess` chain, reached
            // when the chain was built by `move` or a postfix `.`.
            Expr::Ident(path, _) => Some((path.last()?.clone(), Vec::new())),
            other => {
                self.error(
                    otter_parser::exprs::expr_span(other),
                    "only a `var` binding or one of its fields can be assigned",
                );
                None
            }
        }
    }

    /// The `string` an interpolated expression contributes, converting with the
    /// `str_from_*` builtin that matches its type.
    ///
    /// A `string` needs no conversion and is returned as it is. `i64`, `f64` and
    /// `bool` each have exactly one conversion, named here so the piece and the
    /// builtin table cannot drift. Anything else -- a struct, an enum, a pointer,
    /// `()` -- has no text form, and formatting its bytes or its address would be
    /// a wrong answer that looks like a right one, so it is reported instead.
    ///
    /// Returns `None` after reporting when there is no conversion.
    fn as_string(&mut self, expr: HirExpr, span: Span) -> Option<HirExpr> {
        let ty = expr_ty(&expr);
        let callee = match &ty.kind {
            TypeKind::Str => return Some(expr),
            TypeKind::I64 => "str_from_i64",
            TypeKind::F64 => "str_from_f64",
            TypeKind::Bool => "str_from_bool",
            _ => {
                self.error(
                    expr_span(&expr),
                    format!(
                        "cannot interpolate a `{ty}` into a string: only `string`, `i64`, `f64` and `bool` have a text form"
                    ),
                );
                return None;
            }
        };
        Some(HirExpr::Call {
            callee: vec![callee.to_string()],
            args: vec![expr],
            unsafe_call: false,
            span,
            ty: prim(TypeKind::Str),
        })
    }

    /// The position a runtime fault at `span` reports itself from, when the
    /// operation there is one that can fault.
    ///
    /// Integer arithmetic is **checked**: `+`, `-`, `*`, `/` and `%` stop the
    /// program with a message and a position rather than continuing with a
    /// wrong number, and so does negating the smallest `i64`. Which operations
    /// those are is a decision about the *language*, so it is made here and
    /// recorded in the tree; the backend only decides how to check.
    ///
    /// Nothing else faults, so nothing else gets a position: a comparison, a
    /// boolean `and`/`or`, a float operation (an infinity is a float answer, not
    /// a fault) and a string concatenation all cannot go wrong.
    ///
    /// The resolution itself happens here for the reason `panic_call` resolves
    /// its own: the checker has the source map and the backend does not.
    fn fault_site(&self, span: Span, checked: bool) -> Option<FaultSite> {
        if !checked {
            return None;
        }
        let location = self.sm.locate(self.file, span);
        Some(FaultSite {
            file: self.sm.path(self.file).to_string(),
            line: location.line,
            column: location.col,
        })
    }

    /// `panic(message)` as the call the backend lowers: the message the program
    /// wrote, then the file, line and column it was written at.
    ///
    /// Resolving the position here rather than in code generation is the same
    /// trade interpolation made: the stage that holds a fact owns it. The
    /// checker has the source map, so it can turn the call's span into a line
    /// and column, and the backend -- which is handed IR text and a path, and no
    /// map at all -- is left lowering a call it already knows how to lower.
    ///
    /// The type stays `Never`, so nothing downstream can use a value that does
    /// not exist, and the extra arguments are ordinary literals: a `string` and
    /// two `i64`s, which the runtime prints.
    fn panic_call(&mut self, message: HirExpr, span: Span) -> HirExpr {
        let mut args = vec![message];
        args.extend(self.position_args(span));
        HirExpr::Call {
            callee: vec!["panic".to_string()],
            args,
            unsafe_call: false,
            span,
            ty: never(span),
        }
    }

    /// `assert(condition)` as the call the backend lowers: the condition, the
    /// message to report when it is false, and the position.
    ///
    /// The message quotes the condition's own source text, which is the other
    /// fact only this stage holds. It is a single literal -- everything in it is
    /// known now -- so the backend lowers it exactly as it lowers a `panic`.
    ///
    /// The result type is `()` rather than `Never`: a condition that holds is
    /// not a fault, and typing it as diverging would let the value that is never
    /// produced flow into anything at all.
    fn assert_call(&mut self, condition: HirExpr, span: Span) -> HirExpr {
        let text = self.snippet_text(expr_span(&condition));
        let message = format!("assertion failed: {text}");
        let mut args = vec![
            condition,
            HirExpr::Literal(Literal::Str(str_lit(message)), span, prim(TypeKind::Str)),
        ];
        args.extend(self.position_args(span));
        HirExpr::Call {
            callee: vec!["assert".to_string()],
            args,
            unsafe_call: false,
            span,
            ty: unit(span),
        }
    }

    /// `assert_eq(a, b)` and `assert_ne(a, b)` as the call the backend lowers:
    /// the two operands, the two halves of the message written around them, and
    /// the position.
    ///
    /// The operands travel down as expressions rather than as values because
    /// the *backend* has to evaluate them once and use the result twice -- once
    /// for the comparison and once for the message that prints it -- and it is
    /// the stage that owns the registers. Building the message here would either
    /// duplicate an operand that is a call (running it twice, on the failure
    /// path with a value that may differ) or need a temporary this tree has no
    /// way to name.
    ///
    /// Returns `None` when it has reported an error, so the caller does not
    /// lower a call it has no types for.
    fn assert_cmp_call(&mut self, name: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        // Both operands are checked even when the first one fails, so a program
        // with two mistakes reports two errors rather than taking two builds to
        // say so -- the same rule the argument loop above follows.
        let left = self.check_expr(args.first()?);
        let right = self.check_expr(args.get(1)?);
        let (Some(left), Some(right)) = (left, right) else {
            return None;
        };
        let left_ty = expr_ty(&left);
        let right_ty = expr_ty(&right);
        // The "cannot compare" case is checked first so that a pair with one
        // `Never` in it -- `assert_eq(panic("x"), 1)` -- is reported as an
        // uncomparable operand rather than as a type mismatch against `!`.
        if !has_equality(&left_ty) {
            self.error(
                expr_span(&left),
                format!(
                    "`{name}` cannot compare `{left_ty}` values: it compares `i64`, \
                     `f64`, `bool` and `string`"
                ),
            );
            return None;
        }
        if !left_ty.eq_ignoring_span(&right_ty) {
            self.error(
                expr_span(&right),
                format!("expected `{left_ty}`, found `{right_ty}`"),
            );
            return None;
        }
        let symbol = if name == "assert_ne" { "!=" } else { "==" };
        let text = format!(
            "{} {symbol} {}",
            self.snippet_text(expr_span(&left)),
            self.snippet_text(expr_span(&right))
        );
        let mut built = vec![
            left,
            right,
            HirExpr::Literal(
                Literal::Str(str_lit(format!("assertion failed: {text}, left = "))),
                span,
                prim(TypeKind::Str),
            ),
            HirExpr::Literal(
                Literal::Str(str_lit(", right = ".to_string())),
                span,
                prim(TypeKind::Str),
            ),
        ];
        built.extend(self.position_args(span));
        Some(HirExpr::Call {
            callee: vec![name.to_string()],
            args: built,
            unsafe_call: false,
            span,
            ty: unit(span),
        })
    }

    /// The file, line and column a runtime report names, as literals.
    ///
    /// The tail of every call this stage hands the backend for a fault, so that
    /// the position is resolved in the one stage that has a source map.
    fn position_args(&self, span: Span) -> Vec<HirExpr> {
        let location = self.sm.locate(self.file, span);
        vec![
            HirExpr::Literal(
                Literal::Str(str_lit(self.sm.path(self.file).to_string())),
                span,
                prim(TypeKind::Str),
            ),
            HirExpr::Literal(
                Literal::I64(i64::from(location.line)),
                span,
                prim(TypeKind::I64),
            ),
            HirExpr::Literal(
                Literal::I64(i64::from(location.col)),
                span,
                prim(TypeKind::I64),
            ),
        ]
    }

    /// The source text a span covers, as one trimmed line.
    ///
    /// Any run of whitespace -- the indent of a continuation line as much as the
    /// line break itself -- becomes a single space, so an assertion written
    /// across two lines still reports as one. The message is printed after a
    /// position and a label, and a second line of it would read as a second
    /// diagnostic. A span with no text -- a synthetic one -- yields the empty
    /// string, and the message degrades to `assertion failed: ` rather than to a
    /// compiler fault.
    fn snippet_text(&self, span: Span) -> String {
        let mut out = String::new();
        let mut pending_space = false;
        for ch in self.sm.snippet(self.file, span).chars() {
            if ch.is_whitespace() {
                pending_space = true;
                continue;
            }
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch);
        }
        out
    }
}

/// The method name an old free-function spelling moved to, or `None` when the
/// name was never a builtin.
///
/// This is the public migration table, not the lowering table: `str_slice`
/// becomes `slice`, while the HIR still calls the runtime symbol
/// `str_slice`. Keeping the two tables separate is what lets the method form
/// keep the old ABI without keeping the old spelling.
fn old_builtin_method_name(name: &str) -> Option<(&'static str, usize)> {
    match name {
        "push" => Some(("push", 2)),
        "pop" => Some(("pop", 1)),
        "free_slice" => Some(("free", 1)),
        "len" => Some(("len", 1)),
        "byte_at" => Some(("byte_at", 2)),
        "str_slice" => Some(("slice", 3)),
        "parse_i64" => Some(("parse_i64", 1)),
        "parse_f64" => Some(("parse_f64", 1)),
        "map_get" => Some(("get", 2)),
        "map_insert" => Some(("insert", 3)),
        "map_remove" => Some(("remove", 2)),
        "map_len" => Some(("len", 1)),
        "map_free" => Some(("free", 1)),
        "chan_send" => Some(("send", 2)),
        "chan_recv" => Some(("recv", 1)),
        "chan_close" => Some(("close", 1)),
        "chan_free" => Some(("free", 1)),
        "mutex_lock" => Some(("lock", 1)),
        "mutex_unlock" => Some(("unlock", 1)),
        "mutex_free" => Some(("free", 1)),
        _ => None,
    }
}

/// The receiver-first builtin a method name stands for, or `None` when the
/// receiver's type has no such method.
///
/// The builtin names are the old flat spellings. Task 6 removed those spellings
/// from the surface, but the lowering keeps using them as the runtime ABI.
fn builtin_method_name(method: &str, receiver: &TypeKind) -> Option<&'static str> {
    match receiver {
        TypeKind::Slice(_) => match method {
            "len" => Some("len"),
            "push" => Some("push"),
            "pop" => Some("pop"),
            "free" => Some("free_slice"),
            _ => None,
        },
        TypeKind::Str => match method {
            "len" => Some("len"),
            "byte_at" => Some("byte_at"),
            "slice" => Some("str_slice"),
            _ => None,
        },
        TypeKind::Map(_, _) => match method {
            "get" => Some("map_get"),
            "insert" => Some("map_insert"),
            "remove" => Some("map_remove"),
            "len" => Some("map_len"),
            "free" => Some("map_free"),
            "keys" => Some("map_keys"),
            "values" => Some("map_values"),
            "contains_key" => Some("map_contains_key"),
            _ => None,
        },
        TypeKind::Channel(_) => match method {
            "send" => Some("chan_send"),
            "recv" => Some("chan_recv"),
            "close" => Some("chan_close"),
            "free" => Some("chan_free"),
            _ => None,
        },
        TypeKind::Mutex => match method {
            "lock" => Some("mutex_lock"),
            "unlock" => Some("mutex_unlock"),
            "free" => Some("mutex_free"),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `==` is defined for a type, which is what `assert_eq` needs.
///
/// The four the comparison emitters support. An aggregate has no equality in
/// Otter yet, and comparing one by its first field would be a wrong answer
/// rather than a missing one, so asking is reported.
fn has_equality(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Str
    )
}

/// Whether a binary operation is integer arithmetic, and therefore checked.
///
/// The set is the five that can overflow or trap. Comparisons produce a `bool`
/// and cannot; `&&`/`||` are not arithmetic at all; and a `+` on strings is a
/// concatenation, which the backend routes away before it reaches the numeric
/// table -- the result type is what keeps it out of here.
fn is_checked_arithmetic(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
    )
}

/// A [`StringLit`] from its decoded text, keeping `value` and `bytes` in step.
///
/// Sema synthesises exactly one string literal -- the file name a panic reports
/// -- and the two fields have to agree for the backend, which interns `bytes`.
fn str_lit(value: String) -> StringLit {
    let bytes = value.as_bytes().to_vec();
    StringLit { value, bytes }
}

/// The empty string literal, for a piece that must be a string and is not there.
fn empty_string() -> StringLit {
    StringLit {
        value: String::new(),
        bytes: Vec::new(),
    }
}

/// The build-mode choices that reach the checker.
///
/// One flag today. It is here rather than as a bare `bool` parameter because the
/// call sites read `check_program_with(&ast, &sm, file, CheckOptions {
/// release: true })` and not `... , true)` -- and because the next one will be
/// added to a struct rather than to every call.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckOptions {
    /// A release build, in which `debug_assert` is not lowered at all.
    ///
    /// The condition is still *checked* -- a release build that accepts source a
    /// debug build rejects would be the worst of both -- but its tree is
    /// dropped, so an argument with side effects is dropped with it.
    pub release: bool,
}

/// The public entry point for the type checker, in a debug build.
pub fn check_program(
    ast: &otter_parser::ast::Program,
    sm: &SourceMap,
    file: FileId,
) -> Result<HirProgram, Vec<Diagnostic>> {
    check_program_with(ast, sm, file, CheckOptions::default())
}

/// The public entry point for the type checker, in the build mode `options`
/// asks for.
pub fn check_program_with(
    ast: &otter_parser::ast::Program,
    sm: &SourceMap,
    file: FileId,
    options: CheckOptions,
) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut checker = Checker::new(sm, file, options);
    checker.check_program(ast)
}

/// The public entry point for checking an import graph.
///
/// Each `(file, program)` pair is one module, with spans that index that
/// module's own text; the loader produces the list with dependencies before the
/// entry point. All the modules share one root scope, so the result behaves
/// like a single program whose items were written in this order.
pub fn check_modules_with(
    modules: &[(FileId, &otter_parser::ast::Program)],
    sm: &SourceMap,
    options: CheckOptions,
) -> Result<HirProgram, Vec<Diagnostic>> {
    let first = modules
        .first()
        .map(|(file, _)| *file)
        .unwrap_or(FileId::from_index(0));
    let mut checker = Checker::new(sm, first, options);
    checker.check_modules(modules)
}

/// A primitive type at a synthetic span.
fn prim(kind: TypeKind) -> Type {
    Type {
        kind,
        span: Span::new(0, 0),
    }
}

/// A pointer to `pointee`, at a synthetic span. Used for the builtin
/// signatures, where there is no source type expression to point at.
fn ptr(pointee: Type) -> Type {
    Type {
        kind: TypeKind::Ptr(Box::new(pointee)),
        span: Span::new(0, 0),
    }
}

/// The two `i64` parameters shared by all nine arithmetic builtins.
fn i64_pair() -> Vec<Type> {
    vec![prim(TypeKind::I64), prim(TypeKind::I64)]
}

/// `Option[i64]`, the answer `checked_add` and its siblings give instead of
/// stopping the program.
fn option_i64() -> Type {
    prim(TypeKind::Option(Box::new(prim(TypeKind::I64))))
}

/// `Option[payload]`, with a real span for the diagnostic that points at it.
fn option(payload: &Type, span: Span) -> Type {
    Type {
        kind: TypeKind::Option(Box::new(payload.clone())),
        span,
    }
}

/// Whether a slice of `kind` can be pushed to, indexed, popped and iterated in
/// M6.
///
/// Numbers, `bool`, and `string` qualify. A compound element (`Option`,
/// `Result`, a struct, an enum) needs its real layout as a stride, which
/// `otter_codegen::ir::size_of` does not have. A `string` element is a
/// reference the slice owns: `push` and a literal retain it, `free_slice`
/// releases each element, and reading one (index or `for-in`) *borrows* the
/// handle, the same shallow copy a struct field makes. `pop` is refused for
/// strings because the removed handle would leave the slice with nothing that
/// releases it (see `check_slice_builtin`).
/// Whether a slice may hold values of this kind.
///
/// A slice is storing elements it will free as one block, and (for `string`) one
/// reference per element, so the runtime only needs to know the stride and
/// whether the elements are handles. `string` joined the set with M6 Task 10;
/// the element operations that are still missing (`pop`) are refused at the call
/// site, not here.
/// Whether a method changes the storage behind its receiver.
///
/// These are the array methods that write through the binding's slot; a view of
/// the array would be left pointing at storage that moved, shrank, or was
/// freed.
fn mutates_storage(method: &str) -> bool {
    matches!(
        method,
        "push" | "pop" | "set" | "insert" | "remove" | "clear" | "reverse" | "sort" | "extend"
            | "free"
    )
}

/// Whether a fixed array can hold a value of this kind.
///
/// The element has to be a plain machine value: copying the array copies the
/// bytes, and every kind listed here is exactly its bytes with no reference to
/// retain. `string` is the notable exclusion -- a `[string; 3]` copy would
/// duplicate three handles without taking a reference on any of them.
fn fixed_array_element_is_supported(kind: &TypeKind) -> bool {
    matches!(
        kind,
        TypeKind::I64
            | TypeKind::F64
            | TypeKind::Bool
            | TypeKind::CInt(_)
            | TypeKind::CUInt(_)
            | TypeKind::CFloat(_)
            | TypeKind::CSize
    )
}

/// Whether a tuple may hold `kind`.
///
/// The byte-copy rule is the same one fixed arrays use, with one addition:
/// a tuple may hold another tuple of supported elements, because the nested
/// value is itself plain bytes. Nothing that owns a reference or a linear
/// pointer is allowed -- copying a tuple copies its bytes, so a `string`
/// element would hand out a second reference the compiler never retained,
/// and a `*mut T` element would produce two owners of one allocation.
fn tuple_element_is_supported(kind: &TypeKind) -> bool {
    matches!(
        kind,
        TypeKind::I64
            | TypeKind::F64
            | TypeKind::Bool
            | TypeKind::CInt(_)
            | TypeKind::CUInt(_)
            | TypeKind::CFloat(_)
            | TypeKind::CSize
    ) || matches!(
        kind,
        TypeKind::Tuple(elements)
            if elements.iter().all(|element| tuple_element_is_supported(&element.kind))
    )
}

fn slice_element_is_operable(kind: &TypeKind) -> bool {
    matches!(
        kind,
        TypeKind::I64
            | TypeKind::F64
            | TypeKind::Bool
            | TypeKind::Str
            | TypeKind::CInt(_)
            | TypeKind::CUInt(_)
            | TypeKind::CFloat(_)
            | TypeKind::CSize
    )
}

/// Whether a map value can be stored by value.
///
/// Numbers and `bool` are plain machine words. A `string` is *not* accepted:
/// the M6 map would have to retain the handle on insert and release it on
/// overwrite, removal, and free, and it does none of those. Slice elements
/// gained exactly that treatment in Task 10; map values did not, so the
/// restriction stands rather than shipping a dangling reference.
fn map_value_is_supported(kind: &TypeKind) -> bool {
    matches!(
        kind,
        TypeKind::I64
            | TypeKind::F64
            | TypeKind::Bool
            | TypeKind::CInt(_)
            | TypeKind::CUInt(_)
            | TypeKind::CFloat(_)
            | TypeKind::CSize
    )
}

/// Whether a map key can be hashed and compared by the runtime.
///
/// The key travels as a machine word, so the runtime needs to know how to read
/// it: `i64` bytes, a `bool` byte, or a `string` handle it hashes by content.
/// Anything else would need a user-supplied hash and equality, which M6 does
/// not have.
fn map_key_is_supported(kind: &TypeKind) -> bool {
    matches!(kind, TypeKind::I64 | TypeKind::Bool | TypeKind::Str)
}

/// The diagnostic for an element kind [`slice_element_is_operable`] rejects.
fn unsupported_element_message(element: &Type) -> String {
    format!("an array of `{element}` is not supported yet: only numbers, `bool` and `string` can be elements")
}

/// Split `p[i].field` into the pointer it starts from, its index, and the
/// fields above them, outermost first.
///
/// This is the target shape a *pointer* receiver's body uses to write a field:
/// the address is already a value the program holds, so the write is a store
/// through it rather than a read-modify-write of a binding. `None` when the
/// target is not a chain of fields over an index.
fn split_pointer_target(target: &Expr) -> Option<(Expr, Expr, Vec<(String, Span)>)> {
    let mut names = Vec::new();
    let mut current = target;
    loop {
        match current {
            Expr::FieldAccess { base, field, span } => {
                names.insert(0, (field.clone(), *span));
                current = base;
            }
            Expr::Index { base, index, .. } => {
                return Some((base.as_ref().clone(), index.as_ref().clone(), names));
            }
            _ => return None,
        }
    }
}

/// Whether a value of type `found` may stand where `expected` is wanted.
///
/// Types otherwise have to match exactly; `!` -- [`TypeKind::Never`] -- is the
/// one exception. An expression that never produces a value cannot disagree
/// about what kind of value it is, so it fits wherever one is wanted: the code
/// that would have used it never runs. That is what lets `return panic("...")`
/// stand in for any return type, and a diverging argument stand in for any
/// parameter. Everything else has to match exactly, so this widens nothing else.
fn fits(expected: &Type, found: &Type) -> bool {
    expected.eq_ignoring_span(found)
        || matches!(found.kind, TypeKind::Never)
        || ffi_string_fits(expected, found)
}

/// Whether a `string` may stand where a C string pointer is wanted.
///
/// This is the FFI boundary's one deliberate coercion: `*const u8` and `*mut u8`
/// describe a NUL-terminated C string, and Otter's `string` is the value a
/// program has. Codegen copies the bytes before the call (Task 3), so no Otter
/// reference escapes. Only the two byte types qualify; a `*const i64` is a
/// pointer to machine words, not text.
fn ffi_string_fits(expected: &Type, found: &Type) -> bool {
    matches!(found.kind, TypeKind::Str)
        && matches!(
            &expected.kind,
            TypeKind::CPtr(inner)
                if matches!(inner.kind, TypeKind::CUInt(8) | TypeKind::CInt(8))
        )
}

/// Whether a type is `void` in value position.
///
/// Pointers to `void` are not: `void*` is the C spelling of "untyped pointer",
/// and it is a perfectly good parameter type.
fn is_void_value(kind: &TypeKind) -> bool {
    match kind {
        TypeKind::CVoid => true,
        TypeKind::Option(inner) => is_void_value(&inner.kind),
        TypeKind::Result(ok, err) => is_void_value(&ok.kind) || is_void_value(&err.kind),
        _ => false,
    }
}

/// Fold one `switch` arm's body type into the type every arm has to agree on.
///
/// Returns the message for an arm that does not fit, so the caller reports it
/// against the span it already has. `!` follows the same rule as everywhere
/// else ([`fits`]): a `panic` in the `default` arm makes that one arm
/// unreachable rather than making the whole switch `!`.
fn merge_arm_type(slot: &mut Option<Type>, body: Type) -> Option<String> {
    let Some(expected) = slot.as_ref() else {
        *slot = Some(body);
        return None;
    };
    if fits(expected, &body) {
        return None;
    }
    // The first arm was the diverging one and this arm is the first to actually
    // produce something, so it becomes the type the later arms agree with.
    if matches!(expected.kind, TypeKind::Never) {
        *slot = Some(body);
        return None;
    }
    let expected = expected.to_string();
    Some(format!("expected `{expected}`, found `{body}`"))
}

/// The type of a diverging expression, at the span where it diverged.
fn never(span: Span) -> Type {
    Type {
        kind: TypeKind::Never,
        span,
    }
}

/// The type of a value-producing expression with nothing in it, at its span.
///
/// Used for the assertions, which return normally when they hold: `()` is the
/// honest type, and it is what keeps `assert(cond)` out of value positions.
fn unit(span: Span) -> Type {
    Type {
        kind: TypeKind::Unit,
        span,
    }
}

/// The `T` of an `Option[T]`, as a [`TypeExpr`] for synthesized variant defs.
/// Falls back to `()` when the subject is not an `Option` (which cannot
/// happen: callers only call this under a `TypeKind::Option` match).
fn option_inner(subject_ty: &Type) -> TypeExpr {
    match &subject_ty.kind {
        TypeKind::Option(inner) => type_to_expr(inner),
        _ => TypeExpr::Unit(Span::new(0, 0)),
    }
}

/// The `(T, E)` of a `Result[T, E]`, as [`TypeExpr`]s. Same fallback rule as
/// [`option_inner`].
fn result_inners(subject_ty: &Type) -> (TypeExpr, TypeExpr) {
    match &subject_ty.kind {
        TypeKind::Result(ok, err) => (type_to_expr(ok), type_to_expr(err)),
        _ => (
            TypeExpr::Unit(Span::new(0, 0)),
            TypeExpr::Unit(Span::new(0, 0)),
        ),
    }
}

/// Render a checked [`Type`] back as syntax. Only primitives round-trip
/// exactly; compound types fall back to their names, which is enough for
/// synthesized `Option`/`Result` variant defs (whose payload types are never
/// re-resolved — they exist only so `missing_cases` sees the right arity).
fn type_to_expr(ty: &Type) -> TypeExpr {
    let span = Span::new(0, 0);
    match &ty.kind {
        TypeKind::I64 => TypeExpr::Name("i64".to_string(), span),
        TypeKind::F64 => TypeExpr::Name("f64".to_string(), span),
        TypeKind::Bool => TypeExpr::Name("bool".to_string(), span),
        TypeKind::Str => TypeExpr::Name("string".to_string(), span),
        TypeKind::Unit => TypeExpr::Unit(span),
        TypeKind::Struct(name) | TypeKind::Enum(name) => TypeExpr::Name(name.clone(), span),
        TypeKind::Option(inner) => TypeExpr::Option(Box::new(type_to_expr(inner)), span),
        TypeKind::Result(ok, err) => TypeExpr::Result(
            Box::new(type_to_expr(ok)),
            Box::new(type_to_expr(err)),
            span,
        ),
        TypeKind::Ptr(pointee) => TypeExpr::Ptr {
            kind: PtrKind::Mut,
            pointee: Box::new(type_to_expr(pointee)),
            span,
        },
        TypeKind::Slice(element) => TypeExpr::Slice(Box::new(type_to_expr(element)), span),
        TypeKind::FixedArray(element, length) => {
            TypeExpr::FixedArray(Box::new(type_to_expr(element)), *length, span)
        }
        TypeKind::View(element) => TypeExpr::View(Box::new(type_to_expr(element)), span),
        TypeKind::Map(key, value) => TypeExpr::Map(
            Box::new(type_to_expr(key)),
            Box::new(type_to_expr(value)),
            span,
        ),
        TypeKind::Mutex => TypeExpr::Name("Mutex".to_string(), span),
        TypeKind::Channel(element) => TypeExpr::Channel(Box::new(type_to_expr(element)), span),
        TypeKind::CInt(width) => TypeExpr::Name(format!("i{width}"), span),
        TypeKind::CUInt(width) => TypeExpr::Name(format!("u{width}"), span),
        TypeKind::CFloat(width) => TypeExpr::Name(format!("f{width}"), span),
        TypeKind::CVoid => TypeExpr::Name("void".to_string(), span),
        TypeKind::CPtr(pointee) => TypeExpr::Ptr {
            kind: PtrKind::Bare,
            pointee: Box::new(type_to_expr(pointee)),
            span,
        },
        TypeKind::CSize => TypeExpr::Name("usize".to_string(), span),
        TypeKind::Func(params, ret) => TypeExpr::Fn {
            params: params.iter().map(type_to_expr).collect(),
            ret: Box::new(type_to_expr(ret)),
            span,
        },
        TypeKind::Tuple(elements) => {
            TypeExpr::Tuple(elements.iter().map(type_to_expr).collect(), span)
        }
        TypeKind::Never => TypeExpr::Unit(span),
    }
}

/// True for the numeric primitives: `i64` and `f64`, plus the C numeric kinds.
///
/// The C kinds are numeric only in an `as` target and in values that came back
/// from an `extern` call, which is exactly where they are useful: `malloc`
/// wants a `usize`, `strlen` returns one, and narrowing to `u8` is how a
/// program reaches a `char` parameter. Equality still governs arithmetic, so a
/// `i32` and an `i64` need an explicit cast rather than silently widening.
fn is_numeric(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::I64
            | TypeKind::F64
            | TypeKind::CInt(_)
            | TypeKind::CUInt(_)
            | TypeKind::CFloat(_)
            | TypeKind::CSize
    )
}

/// Compute the result type of a binary operator, reporting a message when
/// the operands do not fit it.
fn binary_result_type(op: BinOp, lhs: &Type, rhs: &Type) -> Result<Type, String> {
    let numeric = is_numeric(lhs) && is_numeric(rhs);
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
            // `+` is the one arithmetic operator that means something else for
            // a second type: two strings concatenate. Keeping it inside this
            // arm rather than before the table means `-`, `*`, `/` and `%` stay
            // numeric-only, so a typo cannot silently become a string operation.
            if op == BinOp::Add
                && lhs.eq_ignoring_span(&prim(TypeKind::Str))
                && rhs.eq_ignoring_span(&prim(TypeKind::Str))
            {
                return Ok(prim(TypeKind::Str));
            }
            if !numeric || !lhs.eq_ignoring_span(rhs) {
                return Err(format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    binop_symbol(op),
                    lhs,
                    rhs
                ));
            }
            Ok(lhs.clone())
        }
        BinOp::Eq | BinOp::Ne => {
            if !lhs.eq_ignoring_span(rhs) {
                return Err(format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    binop_symbol(op),
                    lhs,
                    rhs
                ));
            }
            Ok(prim(TypeKind::Bool))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            if !numeric || !lhs.eq_ignoring_span(rhs) {
                return Err(format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    binop_symbol(op),
                    lhs,
                    rhs
                ));
            }
            Ok(prim(TypeKind::Bool))
        }
        BinOp::And | BinOp::Or => {
            if !lhs.eq_ignoring_span(&prim(TypeKind::Bool))
                || !rhs.eq_ignoring_span(&prim(TypeKind::Bool))
            {
                return Err(format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    binop_symbol(op),
                    lhs,
                    rhs
                ));
            }
            Ok(prim(TypeKind::Bool))
        }
    }
}

/// The symbol for a binary operator, used in error messages.
fn binop_symbol(op: BinOp) -> &'static str {
    match op {
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

/// The span of an AST type expression.
fn type_expr_span(ty: &TypeExpr) -> Span {
    match ty {
        TypeExpr::Name(_, span) => *span,
        TypeExpr::Option(_, span) => *span,
        TypeExpr::Ptr { span, .. } => *span,
        TypeExpr::Slice(_, span) => *span,
        TypeExpr::FixedArray(_, _, span) => *span,
        TypeExpr::View(_, span) => *span,
        TypeExpr::Map(_, _, span) => *span,
        TypeExpr::Result(_, _, span) => *span,
        TypeExpr::Channel(_, span) => *span,
        TypeExpr::Fn { span, .. } => *span,
        TypeExpr::Tuple(_, span) => *span,
        TypeExpr::Unit(span) => *span,
    }
}
