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
    BinOp, Block, Expr, InterpPart, Item, Literal, Param, Pattern, PtrKind, Stmt, StringLit,
    SwitchArm, TypeExpr, VariantDef, VariantField,
};

use crate::check_pat::check_pattern;
use crate::error::TypeError;
use crate::exhaustive::missing_cases;
use crate::hir::*;
use crate::ownership::{linearity_of, Linearity, Ownership};
use crate::scope::{Binding, Scope};
use crate::ty::{Mutability, Type, TypeKind};

/// A declared struct: its fields, in declaration order.
#[derive(Clone, Debug)]
struct StructSig {
    fields: Vec<(String, Type)>,
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

/// The type checker. Owns the scope stack, the collected signatures, and the
/// errors found so far.
pub struct Checker<'src> {
    scope: Scope,
    structs: HashMap<String, StructSig>,
    enums: HashMap<String, EnumSig>,
    fns: HashMap<String, FnSig>,
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
    /// Names bound by an active `for x in slice` loop. Moving one of them while
    /// the loop can still read its header would free the array the next
    /// iteration indexes, so `move_out` rejects it (see [`Checker::move_out`]).
    iteration_borrows: Vec<String>,
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
        let builtins: [(&str, Vec<Type>, Type); 35] = [
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
            // `len(string) -> i64` is the byte length. The master plan lists
            // string `len` in M6 alongside slice `len`; taking the string case
            // now costs nothing and is what a program printing text actually
            // needs. M6 widens the same name to slices.
            ("len", vec![prim(TypeKind::Str)], prim(TypeKind::I64)),
            // `byte_at(string, i) -> i64` is the byte at `i`, or 0 when `i` is
            // negative or past the end (ledger L1). A free function rather than
            // the sketched `s.byte_at(i)`: there is no method-call syntax yet,
            // and the runtime symbol already had the semantics. Returning 0
            // rather than faulting is deliberate -- the byte type does not exist
            // yet, so a program inspects bytes as integers, and "out of range"
            // has one answer instead of a check every caller must remember.
            (
                "byte_at",
                vec![prim(TypeKind::Str), prim(TypeKind::I64)],
                prim(TypeKind::I64),
            ),
            // `str_slice(s, start, end) -> string` is the bytes in `[start, end)`
            // as a new owned string. The bounds are clamped by the runtime
            // (negative start is 0, an end past the length is the length, an
            // inverted range is empty), so a program cannot get a fault out of
            // arithmetic on indices -- the same "one answer instead of a check"
            // rule `byte_at` follows. The stdlib's `trim` and `split` are built
            // on it.
            (
                "str_slice",
                vec![prim(TypeKind::Str), prim(TypeKind::I64), prim(TypeKind::I64)],
                prim(TypeKind::Str),
            ),

            // Input. `read_line` yields a `string` (empty at end of input);
            // `parse_i64`/`parse_f64` yield zero when their argument is not a
            // number. A program that can print but not read is a report
            // generator; these three lines are what make it a program.
            ("read_line", vec![], prim(TypeKind::Str)),
            ("parse_i64", vec![prim(TypeKind::Str)], prim(TypeKind::I64)),
            ("parse_f64", vec![prim(TypeKind::Str)], prim(TypeKind::F64)),
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
            externs: HashSet::new(),
            errors: Vec::new(),
            file,
            release: options.release,
            iteration_borrows: Vec::new(),
            sm,
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push((self.file, TypeError::new(span, message)));
    }

    /// Move `name` out of its binding, unless an enclosing `for-in` is reading
    /// it. The loop re-reads the header every iteration, so a `free_slice` or
    /// another linear transfer inside the body would leave it reading a header
    /// whose array is gone.
    fn move_out(&mut self, name: &str, span: Span) -> Result<(), TypeError> {
        if self.iteration_borrows.iter().any(|borrowed| borrowed == name) {
            return Err(TypeError::new(
                span,
                format!("cannot move `{name}` while it is being iterated"),
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
                    items.push(hir);
                }
            }
        }
        if self.errors.is_empty() {
            Ok(HirProgram { items })
        } else {
            let diagnostics = self
                .errors
                .drain(..)
                .map(|(file, e)| e.into_diagnostic(file))
                .collect();
            Err(diagnostics)
        }
    }

    /// Pass 1: declare every item name so signatures can reference each other.
    fn declare_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn {
                    name,
                    params,
                    ret,
                    body: _,
                    span,
                } => {
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
                    let field_tys = fields
                        .iter()
                        .map(|f| {
                            let ty = self.resolve_or_report(&f.ty, f.span);
                            (f.name.clone(), ty)
                        })
                        .collect();
                    self.structs
                        .insert(name.clone(), StructSig { fields: field_tys });
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
                    self.enums.insert(
                        name.clone(),
                        EnumSig {
                            variants: variants.clone(),
                        },
                    );
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
            Ok(resolved) => resolved,
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
            TypeExpr::Slice(element, span) => {
                let inner = self.resolve_type_in(element, allow_c)?;
                if matches!(
                    inner.kind,
                    TypeKind::Unit | TypeKind::Never | TypeKind::CVoid
                ) {
                    return Err(TypeError::new(
                        *span,
                        format!(
                            "a slice cannot hold `{inner}`: its elements would have no size"
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
            TypeExpr::Tuple(_, span) => Err(TypeError::new(
                *span,
                "tuple types are not value types in M0-M2".to_string(),
            )),
            TypeExpr::Unit(span) => Ok(Type {
                kind: TypeKind::Unit,
                span: *span,
            }),
        }
    }

    /// Build one item's HIR. `Fn` bodies are checked here.
    fn check_item(&mut self, item: &Item) -> Option<HirItem> {
        match item {
            Item::Fn {
                name,
                params,
                ret: _,
                body,
                span,
            } => {
                let sig = self.fns.get(name).cloned()?;
                self.scope.push();
                for (
                    Param {
                        name: pname,
                        ty: _,
                        span: pspan,
                    },
                    pty,
                ) in params.iter().zip(sig.params.iter())
                {
                    let _ = self.scope.declare(
                        pname.clone(),
                        Binding {
                            ty: pty.clone(),
                            mutability: Mutability::Const,
                            span: *pspan,
                            ownership: Ownership::Live,
                        },
                    );
                }
                let ret_ty = sig.ret.clone();
                let body = self.check_block_with_ret(body, &ret_ty)?;
                self.scope.pop();
                Some(HirItem::Fn(HirFn {
                    name: name.clone(),
                    params: params
                        .iter()
                        .zip(sig.params.iter())
                        .map(|(p, ty)| HirParam {
                            name: p.name.clone(),
                            ty: ty.clone(),
                            span: p.span,
                        })
                        .collect(),
                    ret: ret_ty,
                    body,
                    span: *span,
                }))
            }
            Item::Struct { name, fields, span } => {
                let sig = self.structs.get(name).cloned()?;
                Some(HirItem::Struct(HirStruct {
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
                }))
            }
            Item::Enum {
                name,
                variants,
                span,
            } => Some(HirItem::Enum(HirEnum {
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
            })),
            Item::Import { path, span } => Some(HirItem::Import(HirImport {
                path: path.clone(),
                span: *span,
            })),
            Item::Extern {
                abi,
                link,
                decls,
                span,
            } => Some(HirItem::Extern(HirExternBlock {
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
            })),
        }
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
                            "cannot assign to a slice element: the slice does not \
                             expose its storage; use `push` to append",
                        );
                        return None;
                    }
                    let TypeKind::Ptr(pointee) = &base_ty.kind else {
                        let _ = self.check_expr(value);
                        self.error(*span, format!("cannot assign to index of `{}`", base_ty));
                        return None;
                    };
                    if !matches!(pointee.kind, TypeKind::I64) {
                        let _ = self.check_expr(value);
                        self.error(
                            *span,
                            format!("only `*mut i64` can be indexed, found `*mut {}`", pointee),
                        );
                        return None;
                    }
                    let index = self.check_expr(index)?;
                    if !expr_ty(&index).eq_ignoring_span(&prim(TypeKind::I64)) {
                        let _ = self.check_expr(value);
                        self.error(expr_span(&index), "index must be `i64`");
                        return None;
                    }
                    let value = self.check_expr(value)?;
                    if !expr_ty(&value).eq_ignoring_span(&prim(TypeKind::I64)) {
                        self.error(
                            expr_span(&value),
                            format!("expected `i64`, found `{}`", expr_ty(&value)),
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

                let Some(binding) = self.scope.resolve(&name).cloned() else {
                    let _ = self.check_expr(value);
                    let span = otter_parser::exprs::expr_span(target);
                    self.error(span, format!("cannot find value `{name}` in this scope"));
                    return None;
                };

                if !binding.mutability.is_mutable() {
                    let span = otter_parser::exprs::expr_span(target);
                    self.error(span, format!("cannot assign to `{name}`, which is immutable"));
                }

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
                let TypeKind::Slice(element) = &iterable_ty.kind else {
                    self.error(
                        *span,
                        format!("`for` over a value must iterate a slice, found `{iterable_ty}`"),
                    );
                    return None;
                };
                let Some(binding) = binding else {
                    self.error(
                        *span,
                        "`for` needs a slice binding to iterate, not a temporary or a field",
                    );
                    return None;
                };
                let element = (**element).clone();
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
        let body = self.check_expr(&arm.body)?;
        self.scope.pop();
        Some(HirSwitchArm {
            pattern,
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
        let has_default = arms.iter().any(|arm| arm.is_default);
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
        let has_default = arms.iter().any(|arm| arm.is_default);
        let patterns: Vec<Pattern> = arms.iter().map(|arm| arm.pattern.clone()).collect();
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

    /// `push(slice, value)`, `pop(slice)` and `free_slice(slice)`.
    ///
    /// These three cannot live in the fixed builtin table: their types mention
    /// the slice's *element* type, and the table has no type parameters.
    /// `push` keeps the slice (it may reallocate the backing array, and the
    /// header in the binding's slot is updated in place); `pop` also keeps it
    /// and hands the last element out as an `Option`; `free_slice` consumes it,
    /// so its receiver must be a binding and is moved out here.
    ///
    /// The outer `Option` says "this name is a slice builtin"; the inner one is
    /// the checked expression, `None` after a reported error.
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
                format!("`{name}` needs a slice binding, not a temporary"),
            );
            return Some(None);
        };
        if path.len() != 1 {
            // A field's slice header has no binding slot to update in place,
            // and lowering looks the name up in the local map: allowing a
            // projection here would make the call silently do nothing.
            self.error(
                *ident_span,
                format!("`{name}` needs a slice binding, not a field access"),
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
                format!("`{name}` expects a slice, found `{receiver_ty}`"),
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
                    "popping a `string` is not supported yet: the slice owns the reference, and a removed element would be handed out unowned",
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
                format!("`len` expects a string or a slice, found `{ty}`"),
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

    /// Check `map_get`, `map_insert`, `map_remove`, `map_len`, and `map_free`.
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
            "map_get" | "map_remove" => 2,
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
                match self.scope.resolve(&name) {
                    Some(binding) => Some(HirExpr::Ident(path.clone(), *span, binding.ty.clone())),
                    None => {
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
            Expr::Call {
                callee,
                args,
                unsafe_call,
                span,
            } => {
                let name = callee.last().cloned().unwrap_or_default();
                // `None`, `Some(x)`, `Ok(v)` and `Err(e)` are value
                // constructors, not calls. They resolve by name and arity,
                // not by scope lookup — `None` takes no arguments and the
                // others take exactly one.
                if callee.len() == 1 {
                    if let Some(constructed) = self.check_constructor(&name, args, *span, None)? {
                        return Some(constructed);
                    }
                    if name == "len" {
                        return self.check_len_builtin(args, *span);
                    }
                    if matches!(name.as_str(), "push" | "free_slice" | "pop") {
                        return self.check_slice_builtin(&name, args, *span).flatten();
                    }
                    if name == "map_new" {
                        return self.check_map_new(args, *span, None);
                    }
                    if matches!(
                        name.as_str(),
                        "map_get" | "map_insert" | "map_remove" | "map_len" | "map_free"
                    ) {
                        return self.check_map_builtin(&name, args, *span).flatten();
                    }
                    if matches!(
                        name.as_str(),
                        "thread_spawn"
                            | "thread_join"
                            | "mutex_new"
                            | "mutex_lock"
                            | "mutex_unlock"
                            | "mutex_free"
                            | "chan_new"
                            | "chan_send"
                            | "chan_recv"
                            | "chan_close"
                            | "chan_free"
                    ) {
                        return self.check_thread_builtin(&name, args, *span).flatten();
                    }
                }
                // The callee is resolved *before* the arguments are checked,
                // because each argument is checked against its parameter's
                // declared type. Checking arguments first and comparing
                // afterwards works for concrete arguments but cannot type a
                // constructor that has nothing to infer from: `f(None)` needs
                // `f`'s parameter type to know what `None` is an `Option` of.
                let resolved = self.scope.resolve(&name).map(|b| b.ty.clone());
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
                let name = path.last().cloned().unwrap_or_default();
                let sig = self.structs.get(&name).cloned();
                match sig {
                    Some(sig) => {
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
                            "a slice cannot hold `{elem_ty}`: its elements would have no size"
                        ),
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
                // element of a slice and is *always* an `Option`: indexing
                // never traps, and out of range including negative is `None`.
                let base = self.check_expr(base)?;
                let base_ty = expr_ty(&base);
                let (element, is_slice) = match &base_ty.kind {
                    TypeKind::Ptr(pointee) => ((**pointee).clone(), false),
                    TypeKind::Slice(element) => ((**element).clone(), true),
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
                let ty = if is_slice {
                    if !slice_element_is_operable(&element.kind) {
                        self.error(*span, unsupported_element_message(&element));
                        return None;
                    }
                    option(&element, *span)
                } else {
                    if !matches!(element.kind, TypeKind::I64) {
                        self.error(
                            *span,
                            format!("only `*mut i64` can be indexed, found `*mut {element}`"),
                        );
                        return None;
                    }
                    prim(TypeKind::I64)
                };
                Some(HirExpr::Index {
                    base: Box::new(base),
                    index: Box::new(index),
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
                        InterpPart::Expr(expr) => {
                            let checked = self.check_expr(expr)?;
                            match self.as_string(checked, *span) {
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
                match &inner_ty.kind {
                    TypeKind::Result(ok, _) => Some(HirExpr::Try {
                        expr: Box::new(inner),
                        span: *span,
                        ty: (**ok).clone(),
                    }),
                    _ => {
                        self.error(
                            *span,
                            format!("`?` can only be used on a `Result`, not `{inner_ty}`"),
                        );
                        None
                    }
                }
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
        }
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
            let resolved = self.structs.get(&struct_name).and_then(|sig| {
                sig.fields
                    .iter()
                    .position(|(name, _)| name == field)
                    .map(|index| (index, sig.fields[index].1.clone()))
            });
            let (index, field_ty) = match resolved {
                Some(found) => found,
                None => {
                    self.error(span, format!("struct `{struct_name}` has no field `{field}`"));
                    return None;
                }
            };
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
        let mut steps: Vec<FieldStep> = Vec::new();
        let mut aggregate = binding.ty.clone();
        for (field, fspan) in &names {
            let TypeKind::Struct(sname) = &aggregate.kind else {
                self.error(*fspan, format!("cannot access a field on `{aggregate}`"));
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
            aggregate = field_ty;
        }

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
    format!("a slice of `{element}` is not supported yet: only numbers, `bool` and `string` can be elements")
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
        TypeKind::Func(_, _) | TypeKind::Never => TypeExpr::Unit(span),
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
        TypeExpr::Map(_, _, span) => *span,
        TypeExpr::Result(_, _, span) => *span,
        TypeExpr::Channel(_, span) => *span,
        TypeExpr::Tuple(_, span) => *span,
        TypeExpr::Unit(span) => *span,
    }
}
