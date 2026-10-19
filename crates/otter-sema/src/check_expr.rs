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

use std::collections::{BTreeSet, HashMap};

use otter_common::{Diagnostic, FileId, SourceMap, Span};
use otter_parser::ast::{
    BinOp, Block, Expr, InterpPart, Item, Literal, Param, Pattern, Stmt, StringLit, SwitchArm,
    TypeExpr, VariantDef, VariantField,
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
    errors: Vec<TypeError>,
    file: FileId,
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
    pub fn new(sm: &'src SourceMap, file: FileId) -> Self {
        let mut scope = Scope::new();
        // Builtins are predeclared in the root scope. They are functions, so a
        // call resolves through the same path as a user call.
        //
        // `alloc` returns a pointer to `i64`, not `u8`. Otter has no byte type,
        // and without indexing or pointer casts a `*mut u8` could not be used at
        // all; `*mut i64` is directly usable and matches the plan's own
        // `buffer.ot`. Recorded as a divergence from the master plan's L3.
        let builtins: [(&str, Vec<Type>, Type); 25] = [
            ("print_str", vec![prim(TypeKind::Str)], prim(TypeKind::Unit)),
            ("print_i64", vec![prim(TypeKind::I64)], prim(TypeKind::Unit)),
            ("print_f64", vec![prim(TypeKind::F64)], prim(TypeKind::Unit)),
            ("alloc", vec![prim(TypeKind::I64)], ptr(prim(TypeKind::I64))),
            ("dealloc", vec![ptr(prim(TypeKind::I64))], prim(TypeKind::Unit)),
            // `len(string) -> i64` is the byte length. The master plan lists
            // string `len` in M6 alongside slice `len`; taking the string case
            // now costs nothing and is what a program printing text actually
            // needs. M6 widens the same name to slices.
            ("len", vec![prim(TypeKind::Str)], prim(TypeKind::I64)),
            // Input. `read_line` yields a `string` (empty at end of input);
            // `parse_i64`/`parse_f64` yield zero when their argument is not a
            // number. A program that can print but not read is a report
            // generator; these three lines are what make it a program.
            ("read_line", vec![], prim(TypeKind::Str)),
            ("parse_i64", vec![prim(TypeKind::Str)], prim(TypeKind::I64)),
            ("parse_f64", vec![prim(TypeKind::Str)], prim(TypeKind::F64)),
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
            // `assert_eq(a, b)` and `assert_ne(a, b)` are the comparison forms.
            // The declared pair is an `i64` one, but the checker never consults
            // it: every call is built generically in `Checker::assert_cmp_call`,
            // so the two operands may be any pair of `i64`, `f64`, `bool` or
            // `string`. The entry is here for what the table is actually for --
            // a name, declared once, that a program therefore cannot redefine.
            ("assert_eq", i64_pair(), prim(TypeKind::Unit)),
            ("assert_ne", i64_pair(), prim(TypeKind::Unit)),
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
            errors: Vec::new(),
            file,
            sm,
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(TypeError::new(span, message));
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
        self.declare_items(&ast.items);
        let mut items = Vec::new();
        for item in &ast.items {
            if let Some(hir) = self.check_item(item) {
                items.push(hir);
            }
        }
        if self.errors.is_empty() {
            Ok(HirProgram { items })
        } else {
            let file = self.file;
            let diagnostics = self
                .errors
                .drain(..)
                .map(|e| e.into_diagnostic(file))
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
                Item::Import { .. } | Item::Extern { .. } => {}
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
    fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, TypeError> {
        match ty {
            TypeExpr::Name(name, span) => {
                let kind = match name.as_str() {
                    "i64" => Some(TypeKind::I64),
                    "f64" => Some(TypeKind::F64),
                    "bool" => Some(TypeKind::Bool),
                    "string" => Some(TypeKind::Str),
                    "()" => Some(TypeKind::Unit),
                    _ => {
                        if self.structs.contains_key(name) {
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
                let inner = self.resolve_type(inner)?;
                Ok(Type {
                    kind: TypeKind::Option(Box::new(inner)),
                    span: *span,
                })
            }
            TypeExpr::Result(ok, err, span) => {
                let ok = self.resolve_type(ok)?;
                let err = self.resolve_type(err)?;
                Ok(Type {
                    kind: TypeKind::Result(Box::new(ok), Box::new(err)),
                    span: *span,
                })
            }
            TypeExpr::Ptr(inner, span) => {
                let inner = self.resolve_type(inner)?;
                Ok(Type {
                    kind: TypeKind::Ptr(Box::new(inner)),
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
            Item::Extern { .. } => None,
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
                let declared = ty.as_ref().and_then(|t| self.resolve_type(t).ok());
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
                let declared = ty.as_ref().and_then(|t| self.resolve_type(t).ok());
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
                // question "what may be assigned to?" is answered here. Today
                // one shape is assignable: a name bound by `var` in a scope that
                // encloses this statement. Fields and subscripts become further
                // arms in this match the day they exist.
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
            }
        }
        if let Expr::Ident(path, span) = expr {
            if path.len() == 1 && path.first().map(String::as_str) == Some("None") {
                return self.check_constructor("None", &[], *span, Some(expected))?;
            }
        }
        self.check_expr(expr)
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
                        if let Err(e) = self.scope.move_out(&name, *ident_span) {
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
                            for (arg, param) in args.iter().zip(params.iter()) {
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
                                if matches!(linearity_of(&param.kind), Linearity::Linear) {
                                    if let Expr::Ident(path, arg_span) = arg {
                                        let arg_name = path.last().cloned().unwrap_or_default();
                                        if let Err(e) = self.scope.move_out(&arg_name, *arg_span) {
                                            self.error(e.span, e.message);
                                        }
                                    }
                                }
                                arg_exprs.push(checked);
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
                let target = self.resolve_type(ty).ok()?;
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

/// The public entry point for the type checker.
pub fn check_program(
    ast: &otter_parser::ast::Program,
    sm: &SourceMap,
    file: FileId,
) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut checker = Checker::new(sm, file);
    checker.check_program(ast)
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

/// Whether a value of type `found` may stand where `expected` is wanted.
///
/// Types otherwise have to match exactly; `!` -- [`TypeKind::Never`] -- is the
/// one exception. An expression that never produces a value cannot disagree
/// about what kind of value it is, so it fits wherever one is wanted: the code
/// that would have used it never runs. That is what lets `return panic("...")`
/// stand in for any return type, and a diverging argument stand in for any
/// parameter. Everything else has to match exactly, so this widens nothing else.
fn fits(expected: &Type, found: &Type) -> bool {
    expected.eq_ignoring_span(found) || matches!(found.kind, TypeKind::Never)
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
        TypeKind::Ptr(pointee) => TypeExpr::Ptr(Box::new(type_to_expr(pointee)), span),
        TypeKind::Func(_, _) | TypeKind::Never => TypeExpr::Unit(span),
    }
}

/// True for the numeric primitives: `i64` and `f64`.
fn is_numeric(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::I64 | TypeKind::F64)
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
#[allow(dead_code)]
fn type_expr_span(ty: &TypeExpr) -> Span {
    match ty {
        TypeExpr::Name(_, span) => *span,
        TypeExpr::Option(_, span) => *span,
        TypeExpr::Ptr(_, span) => *span,
        TypeExpr::Result(_, _, span) => *span,
        TypeExpr::Tuple(_, span) => *span,
        TypeExpr::Unit(span) => *span,
    }
}
