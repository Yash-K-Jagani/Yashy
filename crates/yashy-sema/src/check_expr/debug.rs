//! Debug formatting: the `${value:?}` form of string interpolation.
//!
//! Display (`${value}`) answers "what does this say?" and has a form for the
//! four scalar types only. Debug answers "what is this value?" and has a form
//! for the aggregate the language can build: structs and enums show their name
//! and their fields, tuples and arrays show their elements, and a `string` is
//! quoted and escaped so that whitespace and delimiters in it stay visible.
//!
//! # Why synthesized functions
//!
//! A debug form is recursive, and two of the shapes it walks -- tuples and
//! arrays -- need a statement to read an element (a destructuring binding, a
//! `for` loop). An expression cannot hold a statement, so the printer is a
//! function: one per type, appended to the program after the bodies are
//! checked. The backend sees an ordinary `fn` and needs no new node, and a
//! nested aggregate costs a call rather than a copy of the printer.
//!
//! Scalars and `string` are not printers: they map to the runtime conversions
//! the checker already has (`str_from_i64`, and so on), plus `str_debug` for
//! the quoted form.
//!
//! # Borrowing, not moving
//!
//! The call is built here rather than by [`Checker::check_expr`], so the
//! argument is not marked moved: `"${p:?}"` reads `p` and leaves it owned. The
//! backend's parameter slots are not owned-reference slots, so the callee does
//! not release what it was handed either. The two halves agree, and a linear
//! value can be printed without being consumed.
//!
//! # What has no debug form
//!
//! Maps, pointers, function values, `mutex`, channels and `void` have no debug
//! form yet. Formatting their address or their bytes would be a wrong answer
//! that looks like a right one, so those are reported instead of guessed at.

use yashy_common::Span;

use super::*;

impl Checker<'_> {
    /// The `string` one `${value:?}` part contributes.
    ///
    /// `span` is the expression's own span, so a diagnostic about an
    /// unformattable type points at the value rather than at the whole literal.
    /// Returns `None` after reporting when there is no form.
    pub(super) fn debug_string(&mut self, expr: HirExpr, span: Span) -> Option<HirExpr> {
        let ty = expr_ty(&expr);
        self.debug_expr(&ty, expr, span)
    }

    /// The string expression for one value of `ty`.
    fn debug_expr(&mut self, ty: &Type, value: HirExpr, span: Span) -> Option<HirExpr> {
        match &ty.kind {
            TypeKind::I64 => Some(runtime_call("str_from_i64", value, span)),
            TypeKind::F64 => Some(runtime_call("str_from_f64", value, span)),
            TypeKind::Bool => Some(runtime_call("str_from_bool", value, span)),
            TypeKind::Str => Some(runtime_call("str_debug", value, span)),
            TypeKind::Struct(_)
            | TypeKind::Enum(_)
            | TypeKind::Option(_)
            | TypeKind::Result(_, _)
            | TypeKind::Tuple(_)
            | TypeKind::FixedArray(_, _)
            | TypeKind::Slice(_)
            | TypeKind::View(_) => {
                let printer = self.debug_printer(ty, span)?;
                Some(HirExpr::Call {
                    callee: vec![printer],
                    args: vec![value],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::Str),
                })
            }
            _ => {
                self.error(
                    span,
                    format!(
                        "cannot format a `{ty}` with `:?`: only numbers, `bool`, `string`, \
                         structs, enums, tuples and arrays have a debug form"
                    ),
                );
                None
            }
        }
    }

    /// The name of the printer for `ty`, building and queueing it on first use.
    ///
    /// The name is `__debug.<mangled type>`. The dot is what makes a collision
    /// with a user function impossible: an identifier the source can write
    /// never contains one, and the backend already emits `Type.method` symbols,
    /// so a dotted name is an ordinary symbol to it.
    fn debug_printer(&mut self, ty: &Type, span: Span) -> Option<String> {
        let name = format!("__debug.{}", mangled_type_name(ty));
        if self.debug_printer_names.contains(&name) {
            return Some(name);
        }
        // Recorded before the body is built. A type that contains itself would
        // otherwise ask for its own printer while the first one is half-built;
        // such a type has no finite layout either, so the guard is about
        // remaining total on input rather than about accepting more programs.
        self.debug_printer_names.insert(name.clone());
        let Some(body) = self.debug_body(ty, span) else {
            self.debug_printer_names.remove(&name);
            return None;
        };
        self.debug_printers.push(HirFn {
            name: name.clone(),
            params: vec![HirParam {
                name: "value".to_string(),
                ty: ty.clone(),
                span,
            }],
            ret: prim(TypeKind::Str),
            body,
            captures: Vec::new(),
            span,
        });
        Some(name)
    }

    /// The body of the printer for `ty`: a block that returns the debug text.
    fn debug_body(&mut self, ty: &Type, span: Span) -> Option<HirBlock> {
        let value = ident("value", ty, span);
        match &ty.kind {
            TypeKind::Tuple(elements) => self.tuple_body(elements, value, span),
            TypeKind::FixedArray(element, _) => self.array_body(element, ty, span),
            TypeKind::Slice(element) | TypeKind::View(element) => {
                self.array_body(element, ty, span)
            }
            TypeKind::Struct(name) => {
                let text = self.struct_text(name, value, span)?;
                Some(returns(text, span))
            }
            TypeKind::Enum(name) => {
                let text = self.enum_text(name, value, span)?;
                Some(returns(text, span))
            }
            // The built-in sum types are enums with their variants fixed here,
            // because there is no declaration to read their fields from.
            TypeKind::Option(inner) => {
                let text = self.builtin_enum_text(
                    ("None", Vec::new()),
                    ("Some", vec![(None, (**inner).clone())]),
                    value,
                    span,
                )?;
                Some(returns(text, span))
            }
            TypeKind::Result(ok, err) => {
                let text = self.builtin_enum_text(
                    ("Ok", vec![(None, (**ok).clone())]),
                    ("Err", vec![(None, (**err).clone())]),
                    value,
                    span,
                )?;
                Some(returns(text, span))
            }
            // Not reachable: [`Checker::debug_expr`] answers these without a
            // printer. Keeping a total arm means a change there cannot turn
            // into a missing body.
            _ => {
                let text = self.debug_expr(ty, value, span)?;
                Some(returns(text, span))
            }
        }
    }

    /// `Name { field: value, .. }`, or `Name` when the struct has no fields.
    fn struct_text(&mut self, name: &str, value: HirExpr, span: Span) -> Option<HirExpr> {
        let Some(signature) = self.structs.get(name) else {
            self.error(span, format!("cannot find a declaration for `{name}`"));
            return None;
        };
        let fields = signature.fields.clone();
        if fields.is_empty() {
            return Some(text(name, span));
        }
        let mut out = text(&format!("{name} {{ "), span);
        for (index, (field, field_ty)) in fields.iter().enumerate() {
            if index > 0 {
                out = concat(out, text(", ", span), span);
            }
            out = concat(out, text(&format!("{field}: "), span), span);
            let read = HirExpr::FieldAccess {
                base: Box::new(value.clone()),
                field: field.clone(),
                index,
                span,
                ty: field_ty.clone(),
            };
            let piece = self.debug_expr(field_ty, read, span)?;
            out = concat(out, piece, span);
        }
        Some(concat(out, text(" }", span), span))
    }

    /// A `switch` over every variant of the declared enum `name`.
    fn enum_text(&mut self, name: &str, value: HirExpr, span: Span) -> Option<HirExpr> {
        let Some(signature) = self.enums.get(name) else {
            self.error(span, format!("cannot find a declaration for `{name}`"));
            return None;
        };
        let variants = signature.variants.clone();
        let mut arms = Vec::with_capacity(variants.len());
        for variant in &variants {
            // The payload's types are resolved here, before any printer is
            // built, so the borrow of `self.enums` is over before the body
            // builder needs `&mut self`.
            let mut fields = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let field_ty = match self.resolve_type(field.ty()) {
                    Ok(ty) => ty,
                    Err(error) => {
                        self.errors.push((self.file, error));
                        return None;
                    }
                };
                let field_name = match field {
                    VariantField::Positional { .. } => None,
                    VariantField::Named { name, .. } => Some(name.clone()),
                };
                fields.push((field_name, field_ty));
            }
            let arm = self.variant_arm(name, &variant.name, &fields, span)?;
            arms.push(arm);
        }
        Some(HirExpr::SwitchExpr {
            subject: Box::new(value),
            arms,
            span,
            ty: prim(TypeKind::Str),
        })
    }

    /// The two-variant form of [`Self::enum_text`] for `Option` and `Result`.
    fn builtin_enum_text(
        &mut self,
        first: (&str, Vec<(Option<String>, Type)>),
        second: (&str, Vec<(Option<String>, Type)>),
        value: HirExpr,
        span: Span,
    ) -> Option<HirExpr> {
        let mut arms = Vec::with_capacity(2);
        for (name, fields) in [first, second] {
            arms.push(self.variant_arm("", name, &fields, span)?);
        }
        Some(HirExpr::SwitchExpr {
            subject: Box::new(value),
            arms,
            span,
            ty: prim(TypeKind::Str),
        })
    }

    /// One `switch` arm: the variant's pattern and the text it produces.
    ///
    /// `owner` is the enum's own name, and is empty for the built-in sum types,
    /// whose variants are written without one (`Some(x)`, not `Option.Some(x)`).
    fn variant_arm(
        &mut self,
        owner: &str,
        variant: &str,
        fields: &[(Option<String>, Type)],
        span: Span,
    ) -> Option<HirSwitchArm> {
        let path = if owner.is_empty() {
            vec![variant.to_string()]
        } else {
            vec![owner.to_string(), variant.to_string()]
        };
        if fields.is_empty() {
            return Some(HirSwitchArm {
                pattern: HirPattern::UnitVariant { path, span },
                guard: None,
                body: text(variant, span),
                is_default: false,
                span,
            });
        }
        let named = fields.iter().any(|(name, _)| name.is_some());
        // Each binding gets a name of its own: two arms of one switch, or two
        // fields of one variant, must not share a slot in the backend's map.
        let mut bindings = Vec::with_capacity(fields.len());
        for (index, (_, ty)) in fields.iter().enumerate() {
            bindings.push(HirPattern::Binding {
                name: format!("__d{index}"),
                ty: ty.clone(),
                mutability: Mutability::Const,
                span,
            });
        }
        let mut out = text(&format!("{variant}("), span);
        if named {
            out = text(&format!("{variant} {{ "), span);
        }
        for (index, ((field_name, ty), binding)) in fields.iter().zip(bindings.iter()).enumerate() {
            if index > 0 {
                out = concat(out, text(", ", span), span);
            }
            if let Some(field_name) = field_name {
                out = concat(out, text(&format!("{field_name}: "), span), span);
            }
            let read = match binding {
                HirPattern::Binding { name, ty, .. } => ident(name, ty, span),
                _ => return None,
            };
            let piece = self.debug_expr(ty, read, span)?;
            out = concat(out, piece, span);
        }
        let pattern = if named {
            let mut names = Vec::with_capacity(fields.len());
            for ((field_name, _), binding) in fields.iter().zip(bindings.into_iter()) {
                let Some(field_name) = field_name else {
                    return None;
                };
                names.push((field_name.clone(), binding, span));
            }
            HirPattern::StructVariant {
                path,
                fields: names,
                span,
            }
        } else {
            HirPattern::TupleVariant {
                path,
                args: bindings,
                span,
            }
        };
        let body = if named {
            concat(out, text(" }", span), span)
        } else {
            concat(out, text(")", span), span)
        };
        Some(HirSwitchArm {
            pattern,
            guard: None,
            body,
            is_default: false,
            span,
        })
    }

    /// `(a, b, c)` -- the elements are copied out by a destructuring binding,
    /// because a tuple has no element accessor of its own.
    fn tuple_body(
        &mut self,
        elements: &[Type],
        value: HirExpr,
        span: Span,
    ) -> Option<HirBlock> {
        let mut bindings = Vec::with_capacity(elements.len());
        let mut reads = Vec::with_capacity(elements.len());
        for (index, ty) in elements.iter().enumerate() {
            let name = format!("__t{index}");
            bindings.push(Some((name.clone(), ty.clone())));
            reads.push(ident(&name, ty, span));
        }
        let mut stmts = vec![HirStmt::Destructure {
            init: value,
            bindings,
            mutability: Mutability::Const,
            span,
        }];
        let mut out = text("(", span);
        for (index, (ty, read)) in elements.iter().zip(reads.into_iter()).enumerate() {
            if index > 0 {
                out = concat(out, text(", ", span), span);
            }
            let piece = self.debug_expr(ty, read, span)?;
            out = concat(out, piece, span);
        }
        // `(3,)`, not `(3)`: the trailing comma is what tells a reader of the
        // output that the value is a one-tuple rather than a grouped number.
        if elements.len() == 1 {
            out = concat(out, text(",", span), span);
        }
        out = concat(out, text(")", span), span);
        stmts.push(HirStmt::Return(Some(out), span));
        Some(HirBlock { stmts, span })
    }

    /// `[a, b, c]` for a fixed array, a slice or a view.
    ///
    /// The loop is the language's own `for x in value`, so the printer works
    /// for a length that is only known at run time as well as for one the type
    /// spells out. `out` is an owned `string` binding: the backend releases the
    /// text it replaces on every assignment and the text it returns is a fresh
    /// concatenation.
    fn array_body(&mut self, element: &Type, iterable_ty: &Type, span: Span) -> Option<HirBlock> {
        let out_name = "__out";
        let first_name = "__first";
        let item_name = "__item";
        let bool_ty = prim(TypeKind::Bool);

        let mut stmts = vec![
            HirStmt::Var {
                name: out_name.to_string(),
                ty: Some(prim(TypeKind::Str)),
                init: text("[", span),
                mutability: Mutability::Var,
                span,
            },
            HirStmt::Var {
                name: first_name.to_string(),
                ty: Some(bool_ty.clone()),
                init: HirExpr::Literal(Literal::Bool(true), span, bool_ty.clone()),
                mutability: Mutability::Var,
                span,
            },
        ];

        let separator = HirStmt::If {
            init: None,
            cond: ident(first_name, &bool_ty, span),
            then_block: block(
                vec![assign(
                    first_name,
                    HirExpr::Literal(Literal::Bool(false), span, bool_ty.clone()),
                    &bool_ty,
                    span,
                )],
                span,
            ),
            else_block: Some(block(
                vec![assign_string(
                    out_name,
                    concat(ident(out_name, &prim(TypeKind::Str), span), text(", ", span), span),
                    span,
                )],
                span,
            )),
            span,
        };
        let item = ident(item_name, element, span);
        let piece = self.debug_expr(element, item, span)?;
        let append = assign_string(
            out_name,
            concat(ident(out_name, &prim(TypeKind::Str), span), piece, span),
            span,
        );
        let loop_body = block(vec![separator, append], span);
        stmts.push(HirStmt::ForIn {
            name: item_name.to_string(),
            iterable: ident("value", iterable_ty, span),
            body: loop_body,
            span,
        });
        let tail = concat(ident(out_name, &prim(TypeKind::Str), span), text("]", span), span);
        stmts.push(HirStmt::Return(Some(tail), span));
        Some(HirBlock { stmts, span })
    }
}

/// A call to one of the checker's runtime conversions, typed `string`.
fn runtime_call(name: &str, value: HirExpr, span: Span) -> HirExpr {
    HirExpr::Call {
        callee: vec![name.to_string()],
        args: vec![value],
        unsafe_call: false,
        span,
        ty: prim(TypeKind::Str),
    }
}

/// A `string` literal expression.
fn text(value: &str, span: Span) -> HirExpr {
    HirExpr::Literal(Literal::Str(str_lit(value.to_string())), span, prim(TypeKind::Str))
}

/// The concatenation the language's `+` performs on two `string`s. It cannot
/// fault, so it carries no [`FaultSite`].
fn concat(lhs: HirExpr, rhs: HirExpr, span: Span) -> HirExpr {
    HirExpr::Binary {
        op: BinOp::Add,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
        ty: prim(TypeKind::Str),
        fault: None,
    }
}

/// A read of a binding.
fn ident(name: &str, ty: &Type, span: Span) -> HirExpr {
    HirExpr::Ident(vec![name.to_string()], span, ty.clone())
}

/// A block with one statement: `return value;`.
fn returns(value: HirExpr, span: Span) -> HirBlock {
    block(vec![HirStmt::Return(Some(value), span)], span)
}

fn block(stmts: Vec<HirStmt>, span: Span) -> HirBlock {
    HirBlock { stmts, span }
}

/// `name = value;` for a `var` of type `ty`.
fn assign(name: &str, value: HirExpr, ty: &Type, span: Span) -> HirStmt {
    HirStmt::Assign {
        name: name.to_string(),
        value,
        ty: ty.clone(),
        // The synthesized bindings never hold a linear value that a second
        // owner could free; the string case is tracked by the backend's slot
        // ownership rather than by this flag.
        release_previous: false,
        span,
    }
}

/// The `string` form of [`assign`].
fn assign_string(name: &str, value: HirExpr, span: Span) -> HirStmt {
    HirStmt::Assign {
        name: name.to_string(),
        value,
        ty: prim(TypeKind::Str),
        release_previous: false,
        span,
    }
}
