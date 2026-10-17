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

use std::collections::HashMap;

use otter_common::{Diagnostic, FileId, SourceMap, Span};
use otter_parser::ast::{
    BinOp, Block, Expr, Item, Literal, Param, Pattern, Stmt, SwitchArm, TypeExpr, VariantDef,
    VariantField,
};

use crate::error::TypeError;
use crate::hir::*;
use crate::scope::{Binding, Scope};
use crate::ty::{Mutability, Type, TypeKind};

/// A declared struct: its fields, in declaration order.
#[derive(Clone, Debug)]
struct StructSig {
    fields: Vec<(String, Type)>,
}

/// A declared enum: its variant names.
#[derive(Clone, Debug)]
struct EnumSig {
    #[allow(dead_code)]
    variants: Vec<String>,
}

/// A function signature collected in pass 1.
#[derive(Clone, Debug)]
struct FnSig {
    params: Vec<Type>,
    ret: Type,
}

/// The type checker. Owns the scope stack, the collected signatures, and the
/// errors found so far.
pub struct Checker {
    scope: Scope,
    structs: HashMap<String, StructSig>,
    enums: HashMap<String, EnumSig>,
    fns: HashMap<String, FnSig>,
    errors: Vec<TypeError>,
    file: FileId,
}

impl Checker {
    pub fn new(_sm: &SourceMap, file: FileId) -> Self {
        let mut scope = Scope::new();
        // Builtins are predeclared in the root scope. They are functions, so a
        // call resolves through the same path as a user call; the names are
        // exactly these three.
        let builtins: [(&str, Vec<Type>, Type); 3] = [
            ("print_str", vec![prim(TypeKind::Str)], prim(TypeKind::Unit)),
            ("print_i64", vec![prim(TypeKind::I64)], prim(TypeKind::Unit)),
            ("print_f64", vec![prim(TypeKind::F64)], prim(TypeKind::Unit)),
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
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(TypeError::new(span, message));
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
                        .map(|Param { ty, .. }| {
                            self.resolve_type(ty).unwrap_or_else(|_| never(*span))
                        })
                        .collect();
                    let ret_ty = match ret {
                        Some(ty) => self.resolve_type(ty).unwrap_or_else(|_| never(*span)),
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
                    };
                    if let Err(e) = self.scope.declare(name.clone(), binding) {
                        self.error(e.span, e.message);
                    }
                    let field_tys = fields
                        .iter()
                        .map(|f| {
                            let ty = self.resolve_type(&f.ty).unwrap_or_else(|_| never(f.span));
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
                    };
                    if let Err(e) = self.scope.declare(name.clone(), binding) {
                        self.error(e.span, e.message);
                    }
                    self.enums.insert(
                        name.clone(),
                        EnumSig {
                            variants: variants
                                .iter()
                                .map(|VariantDef { name, .. }| name.clone())
                                .collect(),
                        },
                    );
                }
                Item::Import { .. } | Item::Extern { .. } => {}
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

    fn check_block(&mut self, block: &Block) -> Option<HirBlock> {
        self.check_block_with_ret(block, &prim(TypeKind::Unit))
    }

    /// Check a block in which `return` must yield `ret`.
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
                let init_expr = self.check_expr(init)?;
                let declared = ty.as_ref().and_then(|t| self.resolve_type(t).ok());
                if let Some(declared) = &declared {
                    if !declared.eq_ignoring_span(&expr_ty(&init_expr)) {
                        self.error(
                            expr_span(&init_expr),
                            format!("expected `{declared}`, found `{}`", expr_ty(&init_expr)),
                        );
                    }
                }
                if let Err(e) = self.scope.declare(
                    name.clone(),
                    Binding {
                        ty: expr_ty(&init_expr),
                        mutability: Mutability::Const,
                        span: *span,
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
                let init_expr = self.check_expr(init)?;
                let declared = ty.as_ref().and_then(|t| self.resolve_type(t).ok());
                if let Some(declared) = &declared {
                    if !declared.eq_ignoring_span(&expr_ty(&init_expr)) {
                        self.error(
                            expr_span(&init_expr),
                            format!("expected `{declared}`, found `{}`", expr_ty(&init_expr)),
                        );
                    }
                }
                if let Err(e) = self.scope.declare(
                    name.clone(),
                    Binding {
                        ty: expr_ty(&init_expr),
                        mutability: Mutability::Var,
                        span: *span,
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
            Stmt::Return(expr, span) => match expr {
                Some(e) => {
                    let checked = self.check_expr(e)?;
                    if !ret.eq_ignoring_span(&expr_ty(&checked)) {
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
                let then_block = self.check_block(then_block)?;
                let else_block = else_block.as_ref().and_then(|b| self.check_block(b));
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
                    },
                );
                let body = self.check_block(body)?;
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
                let body = self.check_block(body)?;
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
            } => {
                let subject_expr = self.check_expr(subject)?;
                let mut hir_arms = Vec::new();
                let mut arm_ty: Option<Type> = None;
                for arm in arms {
                    let checked = self.check_switch_arm(arm)?;
                    let body_ty = expr_ty(&checked.body);
                    match &arm_ty {
                        None => arm_ty = Some(body_ty),
                        Some(expected) if expected.eq_ignoring_span(&body_ty) => {}
                        Some(expected) => {
                            self.error(
                                checked.span,
                                format!("expected `{expected}`, found `{body_ty}`"),
                            );
                        }
                    }
                    hir_arms.push(checked);
                }
                Some(HirStmt::Switch {
                    subject: subject_expr,
                    arms: hir_arms,
                    span: *span,
                })
            }
            Stmt::Break(span) => Some(HirStmt::Break(*span)),
            Stmt::Continue(span) => Some(HirStmt::Continue(*span)),
            Stmt::Defer { body, span } => {
                let body = self.check_block(body)?;
                Some(HirStmt::Defer { body, span: *span })
            }
        }
    }

    fn check_switch_arm(&mut self, arm: &SwitchArm) -> Option<HirSwitchArm> {
        let body = self.check_expr(&arm.body)?;
        Some(HirSwitchArm {
            pattern: self.convert_pattern(&arm.pattern)?,
            body,
            is_default: arm.is_default,
            span: arm.span,
        })
    }

    /// Convert an AST pattern to its HIR form. `Binding` records the type of
    /// what it binds, which Task 9 needs to check payload types.
    fn convert_pattern(&mut self, pattern: &Pattern) -> Option<HirPattern> {
        match pattern {
            Pattern::Wildcard(span) => Some(HirPattern::Wildcard(*span)),
            Pattern::UnitVariant { path, span } => Some(HirPattern::UnitVariant {
                path: path.clone(),
                span: *span,
            }),
            Pattern::TupleVariant { path, args, span } => Some(HirPattern::TupleVariant {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|p| self.convert_pattern(p))
                    .collect::<Option<Vec<_>>>()?,
                span: *span,
            }),
            Pattern::StructVariant { path, fields, span } => Some(HirPattern::StructVariant {
                path: path.clone(),
                fields: fields
                    .iter()
                    .map(|(name, sub, span)| {
                        self.convert_pattern(sub)
                            .map(|checked| (name.clone(), checked, *span))
                    })
                    .collect::<Option<Vec<_>>>()?,
                span: *span,
            }),
            Pattern::Binding { name, span } => Some(HirPattern::Binding {
                name: name.clone(),
                ty: never(*span),
                mutability: Mutability::Const,
                span: *span,
            }),
            Pattern::Literal(lit, span) => Some(HirPattern::Literal(lit.clone(), *span)),
        }
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
                let name = path.last().cloned().unwrap_or_default();
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
                    Ok(ty) => Some(HirExpr::Binary {
                        op: *op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span: *span,
                        ty,
                    }),
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
                    Ok(ty) => Some(HirExpr::Unary {
                        op: *op,
                        operand: Box::new(operand),
                        span: *span,
                        ty,
                    }),
                    Err(message) => {
                        self.error(*span, message);
                        None
                    }
                }
            }
            Expr::Move(inner, span) => {
                let inner = self.check_expr(inner)?;
                let ty = expr_ty(&inner);
                Some(HirExpr::Move(Box::new(inner), *span, ty))
            }
            Expr::Call {
                callee,
                args,
                unsafe_call,
                span,
            } => {
                let name = callee.last().cloned().unwrap_or_default();
                let arg_exprs = args
                    .iter()
                    .map(|a| self.check_expr(a))
                    .collect::<Option<Vec<_>>>()?;
                let resolved = self.scope.resolve(&name).map(|b| b.ty.clone());
                match resolved {
                    Some(ty) => match ty.kind {
                        TypeKind::Func(params, ret) => {
                            if params.len() != arg_exprs.len() {
                                self.error(
                                    *span,
                                    format!(
                                        "expected {} arguments, found {}",
                                        params.len(),
                                        arg_exprs.len()
                                    ),
                                );
                                return None;
                            }
                            for (p, a) in params.iter().zip(arg_exprs.iter()) {
                                if !p.eq_ignoring_span(&expr_ty(a)) {
                                    self.error(
                                        expr_span(a),
                                        format!("expected `{p}`, found `{}`", expr_ty(a)),
                                    );
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
                        None
                    }
                }
            }
            Expr::StructLit { path, fields, span } => {
                let name = path.last().cloned().unwrap_or_default();
                let sig = self.structs.get(&name).cloned();
                match sig {
                    Some(sig) => {
                        let mut hir_fields = Vec::new();
                        for (fname, value, _) in fields {
                            let value = self.check_expr(value)?;
                            hir_fields.push((fname.clone(), value, Span::new(0, 0)));
                        }
                        let provided: Vec<&str> =
                            hir_fields.iter().map(|(n, _, _)| n.as_str()).collect();
                        for (fname, _) in &sig.fields {
                            if !provided.contains(&fname.as_str()) {
                                self.error(
                                    *span,
                                    format!("missing field `{fname}` in struct `{name}`"),
                                );
                            }
                        }
                        for (fname, fty) in &sig.fields {
                            if let Some((_, value, _)) =
                                hir_fields.iter().find(|(n, _, _)| n == fname)
                            {
                                let value_ty = expr_ty(value);
                                if !fty.eq_ignoring_span(&value_ty) {
                                    self.error(
                                        expr_span(value),
                                        format!("expected `{fty}`, found `{value_ty}`"),
                                    );
                                }
                            }
                        }
                        for (fname, _, _) in &hir_fields {
                            if !sig.fields.iter().any(|(n, _)| n == fname) {
                                self.error(
                                    *span,
                                    format!("unknown field `{fname}` in struct `{name}`"),
                                );
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
                            .and_then(|s| s.fields.iter().find(|(n, _)| n == field))
                        {
                            Some((_, fty)) => Some(HirExpr::FieldAccess {
                                base: Box::new(base),
                                field: field.clone(),
                                span: *span,
                                ty: fty.clone(),
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
                let mut hir_arms = Vec::new();
                let mut arm_ty: Option<Type> = None;
                for arm in arms {
                    let checked = self.check_switch_arm(arm)?;
                    let body_ty = expr_ty(&checked.body);
                    match &arm_ty {
                        None => arm_ty = Some(body_ty),
                        Some(expected) if expected.eq_ignoring_span(&body_ty) => {}
                        Some(expected) => {
                            self.error(
                                checked.span,
                                format!("expected `{expected}`, found `{body_ty}`"),
                            );
                        }
                    }
                    hir_arms.push(checked);
                }
                Some(HirExpr::SwitchExpr {
                    subject: Box::new(subject),
                    arms: hir_arms,
                    span: *span,
                    ty: arm_ty.unwrap_or_else(|| never(*span)),
                })
            }
        }
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

/// The type of a diverging expression, at the span where it diverged.
fn never(span: Span) -> Type {
    Type {
        kind: TypeKind::Never,
        span,
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
        TypeExpr::Result(_, _, span) => *span,
        TypeExpr::Tuple(_, span) => *span,
        TypeExpr::Unit(span) => *span,
    }
}
