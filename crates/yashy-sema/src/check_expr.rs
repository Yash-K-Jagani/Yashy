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
//! [`TypeError`], and every [`TypeError`] becomes a [`yashy_common::Diagnostic`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use yashy_common::{codes, Diagnostic, FileId, SourceMap, Span};
use yashy_hir::layout::{self as type_layout, Decls, Layout, NamedType};
use yashy_parser::ast::{
    BinOp, BindingKeyword, Block, CallArg, Contract, Expr, ExternDecl, InterpFormat, InterpPart,
    Item, LambdaParam, Literal, Param, Pattern, PtrKind, SliceElement, Stmt, StringLit, SwitchArm,
    TypeExpr, TypeParam, VariantDef, VariantField,
};

use crate::check_pat::{check_pattern, hir_pattern_span, is_irrefutable};
use crate::const_eval::{self, ConstFn, ConstFns, Missing};
use crate::error::TypeError;
use crate::exhaustive::missing_witnesses_with;
use crate::hir::*;
use crate::ownership::{linearity_of, Linearity, Ownership};
use crate::scope::{Binding, Scope, Visibility};
use crate::ty::{Mutability, Type, TypeKind};

/// Debug formatting (`${value:?}`), which synthesizes one printer per type.
mod debug;

/// The most distinct substitutions one generic function may be
/// instantiated at. Past it the program is rejected rather than compiled:
/// unbounded monomorphisation is how compile times become unbounded, and
/// the language would rather reject than hang.
pub const MAX_INSTANTIATIONS_PER_FN: usize = 64;

/// The most instantiations one program may hold across all generic
/// functions, for the same reason at program scale.
pub const MAX_INSTANTIATIONS_TOTAL: usize = 4096;

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
    /// A checked default per field, in the same order: `None` when the field
    /// wrote none. Checked once, when the struct is declared, so a default
    /// cannot be checked differently at two construction sites.
    defaults: Vec<Option<HirExpr>>,
    /// Whether the declaration asked for C's layout with `repr(C)`.
    ///
    /// The checker records it rather than recomputing the layout: `offsetof`
    /// accepts only a `repr(C)` struct, and that rule is about what the
    /// declaration promised, not about the bytes.
    repr_c: bool,
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
        Item::Effect { item, .. } => unwrap_public(item),
        // A doc block documents the item inside it; visibility looks through.
        Item::Documented { item, .. } => unwrap_public(item),
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
    /// Each variant's payload types, resolved, in declaration order and in the
    /// same order as `variants`.
    ///
    /// `sizeof` and `offsetof` need the resolved types, and resolving them a
    /// second time at the query would report the same unknown name twice.
    payloads: Vec<Vec<TypeKind>>,
    /// Whether the declaration asked for C's layout with `repr(C)`, which makes
    /// the tag a C `int` and forbids payloads.
    repr_c: bool,
}

/// A generic struct: its fields unresolved, mentions of its type parameters
/// still in them.
///
/// A generic type is a scheme: it has no layout until its type arguments
/// pick one out. Instantiating it at `Pair[i64]` produces one monomorphic
/// object type per argument tuple, interned under the mangled name in
/// `structs`, and every existing rule -- layout, ownership, code generation
/// -- applies to the monomorph unchanged.
#[derive(Clone, Debug)]
struct StructScheme {
    /// The type parameters, in declaration order, with their bounds.
    type_params: Vec<TypeParam>,
    fields: Vec<yashy_parser::ast::FieldDef>,
    repr_c: bool,
    origin: ItemOrigin,
    span: Span,
}

/// A generic enum, as [`StructScheme`] is for structs.
#[derive(Clone, Debug)]
struct EnumScheme {
    type_params: Vec<TypeParam>,
    variants: Vec<VariantDef>,
    repr_c: bool,
    span: Span,
}

/// The checker's declaration tables, seen through [`Decls`].
///
/// The layout walk cannot see the program, so the checker hands it the two
/// tables it already has. A declared type is looked up by name and reduced to
/// the field types the walk needs; everything else -- spans, visibility,
/// origins -- is the checker's business and stays out of the layout.
struct CheckerDecls<'a> {
    structs: &'a HashMap<String, StructSig>,
    enums: &'a HashMap<String, EnumSig>,
}

impl Decls for CheckerDecls<'_> {
    fn named(&self, name: &str) -> Option<NamedType> {
        if let Some(sig) = self.structs.get(name) {
            return Some(NamedType::Struct {
                fields: sig.fields.iter().map(|(_, ty)| ty.kind.clone()).collect(),
            });
        }
        let sig = self.enums.get(name)?;
        Some(NamedType::Enum {
            payloads: sig.payloads.clone(),
            repr_c: sig.repr_c,
        })
    }
}

/// A function signature collected in pass 1.
#[derive(Clone, Debug)]
struct FnSig {
    params: Vec<Type>,
    param_names: Vec<String>,
    param_defaults: Vec<Option<Box<Expr>>>,
    ret: Type,
    /// Whether the function was declared `async`.
    is_async: bool,
}

/// One linker symbol an `extern` declaration resolved to.
///
/// Two declarations may share a symbol when they describe the same C function,
/// so the first one is kept and a second is compared with it. The Yashy name and
/// the span are kept for the `E0900` message: the reader has to know which two
/// declarations disagree, and where the second one is.
#[derive(Clone, Debug)]
struct ExternSymbol {
    name: String,
    sig: FnSig,
    /// Whether the declaration ended in `...`. Two declarations of one symbol
    /// that disagree about this describe different calling conventions.
    variadic: bool,
    span: Span,
    /// The module the declaration was written in, so the collision diagnostic
    /// names that file rather than whichever module was read last.
    file: FileId,
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
    type_params: Vec<TypeParam>,
    /// The parameters, whose annotations may mention a type parameter.
    params: Vec<Param>,
    /// The return annotation, if the source wrote one.
    ret: Option<TypeExpr>,
    /// `requires` clauses, checked per substitution like the body.
    requires: Vec<Contract>,
    /// `ensures` clauses, checked per substitution like the body.
    ensures: Vec<Contract>,
    /// The body, checked once per substitution.
    body: Block,
    /// Whether the function was declared `async`.
    is_async: bool,
    /// The item's span, used when the item itself is at fault.
    span: Span,
    /// The file the body came from, so its diagnostics name the right module.
    file: FileId,
}

/// One parameter binding whose declared type is a type parameter with
/// bounds, while a generic body is being checked.
///
/// A method call on such a binding resolves through the bounds in force,
/// never through the substituted concrete type's own methods: the call
/// site guaranteed the bounds, and the body may rely on nothing more.
#[derive(Clone, Debug)]
struct BoundBinding {
    /// The parameter's binding name.
    name: String,
    /// The type parameter the binding's declared type is.
    param: String,
    /// The bounds its type parameter carries.
    bounds: Vec<String>,
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
    /// called on must therefore be a mutable `let`, and it must be a binding:
    /// there is no address of a temporary.
    receiver_is_pointer: bool,
    /// Whether the method was declared `async`. An async inherent method is
    /// allowed; an async trait method is `E1703` at the trait declaration.
    is_async: bool,
    /// Where the method was declared, and whether it is `pub`.
    ///
    /// A method is private to the module that declares it unless it is
    /// marked `pub`, the same rule an item follows. Visibility is checked
    /// against this record at every call site.
    origin: ItemOrigin,
}

/// A trait declaration: the requirement set `impl`s are checked against.
///
/// The methods are already resolved, with the placeholder
/// `TypeKind::Struct("Self")` wherever the trait wrote `Self`: a trait has no
/// implementing type yet, and resolving where the trait is declared keeps
/// errors about its signatures in the trait's own file.
#[derive(Clone, Debug)]
struct TraitSig {
    methods: Vec<MethodSig>,
    /// The trait's own type parameters, in declaration order. Only the
    /// stdlib's `Iterator[T]` carries any; user generic traits are `E1511`.
    type_params: Vec<String>,
    /// The effect the trait was declared with (`pure trait`), stamped onto
    /// every impl body as its declared effect so an `io` impl is `E1300`
    /// at the impl, in the verifier.
    declared: yashy_common::DeclaredEffect,
}

/// A comprehension's synthesized builder function, checked after the
/// program's own items, like a generic instantiation.
#[derive(Clone, Debug)]
struct CompJob {
    name: String,
    params: Vec<Param>,
    param_tys: Vec<Type>,
    ret_ty: Type,
    body: Block,
    span: Span,
    file: FileId,
}

/// One `impl Trait for Type` block that passed the coherence check.
#[derive(Clone, Debug)]
struct TraitImpl {
    /// (method name, emitted `HirFn` symbol) per method, in the impl's order.
    methods: Vec<(String, String)>,
}

/// Whether two types agree, with `Struct("Self")` in `expected` standing for
/// `self_ty`.
///
/// A trait's method signatures are resolved where the trait is declared, and
/// there `Self` names no concrete type yet. An impl's signatures resolve with
/// `Self` already the implementing type. This compares the two spellings apart
/// from that one placeholder, structurally, ignoring spans.
fn type_eq_with_self(expected: &Type, actual: &Type, self_ty: &Type) -> bool {
    fn kind_eq(expected: &TypeKind, actual: &TypeKind, self_ty: &TypeKind) -> bool {
        if let TypeKind::Struct(name) = expected {
            if name == "Self" {
                return kind_eq(self_ty, actual, self_ty);
            }
        }
        match (expected, actual) {
            (TypeKind::Option(e), TypeKind::Option(a)) => kind_eq(&e.kind, &a.kind, self_ty),
            (TypeKind::Result(eo, ee), TypeKind::Result(ao, ae)) => {
                kind_eq(&eo.kind, &ao.kind, self_ty) && kind_eq(&ee.kind, &ae.kind, self_ty)
            }
            (TypeKind::FixedArray(e, n), TypeKind::FixedArray(a, m)) => {
                n == m && kind_eq(&e.kind, &a.kind, self_ty)
            }
            (TypeKind::View(e), TypeKind::View(a))
            | (TypeKind::Slice(e), TypeKind::Slice(a))
            | (TypeKind::Channel(e), TypeKind::Channel(a))
            | (TypeKind::Ptr(e), TypeKind::Ptr(a))
            | (TypeKind::CPtr(e), TypeKind::CPtr(a)) => kind_eq(&e.kind, &a.kind, self_ty),
            (TypeKind::Map(ek, ev), TypeKind::Map(ak, av)) => {
                kind_eq(&ek.kind, &ak.kind, self_ty) && kind_eq(&ev.kind, &av.kind, self_ty)
            }
            (TypeKind::Func(ep, er), TypeKind::Func(ap, ar)) => {
                ep.len() == ap.len()
                    && ep
                        .iter()
                        .zip(ap.iter())
                        .all(|(e, a)| kind_eq(&e.kind, &a.kind, self_ty))
                    && kind_eq(&er.kind, &ar.kind, self_ty)
            }
            (TypeKind::Tuple(et), TypeKind::Tuple(at)) => {
                et.len() == at.len()
                    && et
                        .iter()
                        .zip(at.iter())
                        .all(|(e, a)| kind_eq(&e.kind, &a.kind, self_ty))
            }
            _ => expected == actual,
        }
    }
    kind_eq(&expected.kind, &actual.kind, &self_ty.kind)
}

/// Whether an impl method's signature is the one its trait declares: same
/// receiver kind, same parameters, same return, apart from `Self`.
fn same_method_signature(expected: &MethodSig, got: &MethodSig, self_ty: &Type) -> bool {
    expected.has_self == got.has_self
        && expected.receiver_is_pointer == got.receiver_is_pointer
        && expected.params.len() == got.params.len()
        && expected
            .params
            .iter()
            .zip(got.params.iter())
            .all(|(e, a)| type_eq_with_self(e, a, self_ty))
        && type_eq_with_self(&expected.ret, &got.ret, self_ty)
}

/// A method signature as source text: `fn show(self) -> string`.
fn describe_method(sig: &MethodSig, type_name: &str) -> String {
    let mut params = Vec::new();
    if sig.has_self {
        if sig.receiver_is_pointer {
            params.push(format!("self: *mut {type_name}"));
        } else {
            params.push("self".to_string());
        }
        for ty in sig.params.iter().skip(1) {
            params.push(format!("{ty}"));
        }
    } else {
        for ty in &sig.params {
            params.push(format!("{ty}"));
        }
    }
    format!("fn {}({}) -> {}", sig.name, params.join(", "), sig.ret)
}

/// The LLVM symbol a generic function has at one substitution.
///
/// `identity[T]` called with `int` is `identity_i64`. The names are built from
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
        TypeKind::Char => "char".to_string(),
        TypeKind::Str => "string".to_string(),
        TypeKind::Unit => "void".to_string(),
        TypeKind::Never => "never".to_string(),
        TypeKind::Struct(name) | TypeKind::Enum(name) => name.clone(),
        TypeKind::Option(inner) => format!("option_{}", mangled_type_name(inner)),
        TypeKind::Result(ok, err) => {
            format!(
                "result_{}_{}",
                mangled_type_name(ok),
                mangled_type_name(err)
            )
        }
        TypeKind::Slice(element) => format!("array_{}", mangled_type_name(element)),
        TypeKind::FixedArray(element, length) => {
            format!("fixed_{}_{length}", mangled_type_name(element))
        }
        TypeKind::View(element) => format!("view_{}", mangled_type_name(element)),
        TypeKind::Map(key, value) => {
            format!(
                "map_{}_{}",
                mangled_type_name(key),
                mangled_type_name(value)
            )
        }
        TypeKind::Mutex => "mutex".to_string(),
        TypeKind::Task(inner) => format!("task_{}", mangled_type_name(inner)),
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

/// The type names a type expression mentions: a generic's base and its
/// arguments included, so an unused-parameter check sees the `T` inside
/// `Pair[T]`.
fn collect_type_names(ty: &TypeExpr, out: &mut Vec<String>) {
    match ty {
        TypeExpr::Name(name, _) => out.push(name.clone()),
        TypeExpr::Generic { name, args, .. } => {
            out.push(name.clone());
            for arg in args {
                collect_type_names(arg, out);
            }
        }
        TypeExpr::Ptr { pointee, .. }
        | TypeExpr::FixedArray(pointee, _, _)
        | TypeExpr::View(pointee, _)
        | TypeExpr::Slice(pointee, _)
        | TypeExpr::Option(pointee, _)
        | TypeExpr::Channel(pointee, _) => collect_type_names(pointee, out),
        TypeExpr::Result(ok, err, _) | TypeExpr::Map(ok, err, _) => {
            collect_type_names(ok, out);
            collect_type_names(err, out);
        }
        TypeExpr::Fn { params, ret, .. } => {
            for param in params {
                collect_type_names(param, out);
            }
            collect_type_names(ret, out);
        }
        TypeExpr::Tuple(items, _) => {
            for item in items {
                collect_type_names(item, out);
            }
        }
        TypeExpr::Unit(_) => {}
    }
}

/// Replace the type parameters in a type expression with the arguments of
/// the instantiation it belongs to, by name.
///
/// Used to store a monomorph enum's variants in terms of its arguments, so
/// a pattern resolves its payloads against the monomorph rather than the
/// scheme.
fn substitute_type_expr(ty: &TypeExpr, subst: &[(String, TypeExpr)]) -> TypeExpr {
    match ty {
        TypeExpr::Name(name, span) if subst.iter().any(|(param, _)| param == name) => subst
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, arg)| arg.clone())
            .unwrap_or_else(|| TypeExpr::Name(name.clone(), *span)),
        TypeExpr::Ptr {
            kind,
            pointee,
            span,
        } => TypeExpr::Ptr {
            kind: *kind,
            pointee: Box::new(substitute_type_expr(pointee, subst)),
            span: *span,
        },
        TypeExpr::FixedArray(element, length, span) => TypeExpr::FixedArray(
            Box::new(substitute_type_expr(element, subst)),
            *length,
            *span,
        ),
        TypeExpr::View(element, span) => {
            TypeExpr::View(Box::new(substitute_type_expr(element, subst)), *span)
        }
        TypeExpr::Slice(element, span) => {
            TypeExpr::Slice(Box::new(substitute_type_expr(element, subst)), *span)
        }
        TypeExpr::Option(inner, span) => {
            TypeExpr::Option(Box::new(substitute_type_expr(inner, subst)), *span)
        }
        TypeExpr::Channel(element, span) => {
            TypeExpr::Channel(Box::new(substitute_type_expr(element, subst)), *span)
        }
        TypeExpr::Result(ok, err, span) => TypeExpr::Result(
            Box::new(substitute_type_expr(ok, subst)),
            Box::new(substitute_type_expr(err, subst)),
            *span,
        ),
        TypeExpr::Map(key, value, span) => TypeExpr::Map(
            Box::new(substitute_type_expr(key, subst)),
            Box::new(substitute_type_expr(value, subst)),
            *span,
        ),
        TypeExpr::Fn { params, ret, span } => TypeExpr::Fn {
            params: params
                .iter()
                .map(|param| substitute_type_expr(param, subst))
                .collect(),
            ret: Box::new(substitute_type_expr(ret, subst)),
            span: *span,
        },
        TypeExpr::Tuple(items, span) => TypeExpr::Tuple(
            items
                .iter()
                .map(|item| substitute_type_expr(item, subst))
                .collect(),
            *span,
        ),
        TypeExpr::Generic { name, args, span } => TypeExpr::Generic {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_type_expr(arg, subst))
                .collect(),
            span: *span,
        },
        TypeExpr::Unit(_) | TypeExpr::Name(_, _) => ty.clone(),
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
        TypeExpr::Generic { name, args, .. } => {
            let args: Vec<String> = args.iter().map(type_expr_name).collect();
            format!("{name}[{}]", args.join(", "))
        }
        TypeExpr::Unit(_) => "void".to_string(),
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

/// Names a call resolves before the scope lookup: the print family and the
/// value constructors. They are callable but are not bindings, so a "did you
/// mean" suggestion has to offer them explicitly.
const CALLABLE_EXTRAS: &[&str] = &[
    "print",
    "eprint",
    "None",
    "Some",
    "Ok",
    "Err",
    "map_new",
    "thread_spawn",
    "thread_join",
    "mutex_new",
    "chan_new",
    "block_on",
];

/// The type names a program can write without declaring them.
///
/// `resolve_type_in` resolves these before it looks at the declared structs and
/// enums, so a "did you mean" suggestion has to offer them too.
const BUILTIN_TYPE_NAMES: &[&str] = &[
    "int", "float", "bool", "char", "string", "void", "Mutex", "Channel", "Task", "Self",
    "io.Error",
];

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
    /// The parameter bindings whose type is a type parameter of the generic
    /// function being checked, with the bounds in force.
    ///
    /// Only set while an instantiation's body is checked. Method calls on
    /// these bindings resolve through the bounds, so the body may call only
    /// what the bounds guarantee.
    bound_context: Vec<BoundBinding>,
    /// Instantiations discovered but not yet checked.
    pending_generics: Vec<Instantiation>,
    /// Mangled names already queued or emitted, so one substitution is checked
    /// once however many calls find it.
    instantiated: HashSet<String>,
    /// Instantiations per generic function, for the budget.
    ///
    /// [`MAX_INSTANTIATIONS_PER_FN`] bounds one function; the total bounds
    /// the program. Both are correctness devices as much as speed ones:
    /// unbounded monomorphisation makes compile times unbounded, and the
    /// language would rather reject than hang.
    fn_instantiations: HashMap<String, usize>,
    /// The call site that most recently pushed a function's count up, so a
    /// budget error can name both sites that pushed it over.
    fn_instantiation_site: HashMap<String, Span>,
    /// The instantiations currently being checked, innermost last.
    ///
    /// A call to a generic from inside its own instantiation is ordinary
    /// recursion when the substitution is the same and polymorphic recursion
    /// when it is not; the second has no finite monomorphization, so it is
    /// rejected here.
    active_generics: Vec<(String, Vec<Type>)>,
    /// The bindings a view in this function borrows, with the range's span.
    ///
    /// A view points into its array, and nothing in Yashy can free the view
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
    /// The generic structs, by declared name.
    struct_schemes: HashMap<String, StructScheme>,
    /// The generic enums, by declared name.
    enum_schemes: HashMap<String, EnumScheme>,
    /// Monomorphised generic types: mangled name to (scheme name, type
    /// arguments). One entry per argument tuple, so one type per tuple --
    /// never one per use site.
    type_monomorphs: HashMap<String, Vec<Type>>,
    /// Mangled name to the scheme it was instantiated from, so a monomorph
    /// can be told from its scheme at a unification site.
    monomorph_schemes: HashMap<String, String>,
    /// The monomorphs currently being interned, outermost last. A type that
    /// names itself in one of its own fields (`struct Node { next: Node }`)
    /// instantiates forever without this guard.
    instantiating: Vec<String>,
    /// The monomorphised types, in first-use order, so the HIR emits one
    /// definition per monomorph after everything is checked.
    instantiated_types: Vec<String>,
    /// The traits declared in the program, by name: the requirement sets
    /// `impl Trait for Type` blocks are checked against.
    traits: BTreeMap<String, TraitSig>,
    /// The trait impls accepted, keyed by (trait, type). Coherence is one
    /// impl per pair, so the key is also the duplicate test.
    trait_impls: BTreeMap<(String, String), TraitImpl>,
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
    /// The `extern` names declared with a trailing `...`.
    ///
    /// The declaration's type is still `fn(fixed...) -> ret`; the flag is kept
    /// separately because a call to a variadic function accepts arguments beyond
    /// the fixed parameters, which no ordinary function type does.
    variadic_externs: HashSet<String>,
    /// Every error found, with the file it belongs to. A single-file check has
    /// one file and an import graph has several, so the file is recorded at the
    /// moment the error is reported rather than taken from `self.file` at the
    /// end.
    errors: Vec<(FileId, TypeError)>,
    /// The file currently being declared or checked. It is what `panic` and
    /// `assert` turn a span into a position with, and it moves from module to
    /// module as [`Checker::check_modules`] walks the graph.
    file: FileId,
    /// Import aliases per module: this file's `import std.io as io;` maps
    /// `io` to the module named by the path's last segment (`io`). An alias
    /// renames the name a module is known by in this file; the items
    /// themselves still live in the one shared namespace.
    aliases: Vec<(FileId, HashMap<String, String>)>,
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
    /// Whether the program uses async (`async fn`, `await`, or `spawn`).
    uses_async: bool,
    /// Whether the function whose body is being checked is `async`, and its
    /// enclosing contexts, innermost last. An `await` or `spawn` in a context
    /// whose top is `false` is `E1700`.
    ///
    /// A closure resets this: there is no async closure, so a lambda body is
    /// checked as a synchronous function even when it is written inside an
    /// `async fn`.
    async_ctx: Vec<bool>,
    /// Whether the expression being checked is a `const` initializer.
    ///
    /// A literal shift count outside its type's range is a compile-time fault
    /// there and only there: in ordinary code the same count is left to the
    /// run-time check, so the program has one rule and two build modes agree
    /// about it.
    in_const_init: bool,
    /// Return types being inferred for lambdas whose callee does not fix one
    /// (the `map` callback), innermost last. Each entry is the type the first
    /// `return` fixed, or `None` until one is seen.
    lambda_ret_infer: Vec<Option<Type>>,
    /// The linker symbols the `extern` declarations resolved to, keyed by the
    /// symbol string.
    ///
    /// Two declarations that name the same symbol describe one C function, so
    /// the second is checked against the first: agreeing signatures are the same
    /// function described twice and are allowed, disagreeing ones are `E0900`.
    extern_symbols: HashMap<String, ExternSymbol>,
    /// The names of the functions this program defines.
    ///
    /// A defined function's linker symbol is its name, and the extern
    /// declarations share that one flat namespace. A symbol claimed by both is
    /// the same function twice -- so a call through the declaration would reach
    /// the Yashy body instead of the C one, which is `E0900`.
    defined_fns: HashSet<String>,
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
    /// Comprehension builder functions waiting to be checked, like generic
    /// instantiations: a comprehension lowers to a synthesized function
    /// whose body is an ordinary builder loop.
    pending_comps: Vec<CompJob>,
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
    /// The labels of the loops enclosing the statement being checked,
    /// innermost last. `None` is an unlabeled loop (the variable type is
    /// `None`: it never appears in source, only in the model).
    loop_labels: Vec<Option<String>>,
    /// Labels whose flags have been written into the HIR, innermost last.
    /// A `break L` from two loops deep writes its flag, so an enclosing loop
    /// knows to check the flag at the end of its body even before the
    /// `break` itself has been lowered.
    labeled_loops: Vec<String>,
}

impl<'src> Checker<'src> {
    pub fn new(sm: &'src SourceMap, file: FileId, options: CheckOptions) -> Self {
        let mut scope = Scope::new();
        // Builtins are predeclared in the root scope. They are functions, so a
        // call resolves through the same path as a user call.
        //
        // `alloc` returns a pointer to `int`, not `u8`. Yashy has no byte type,
        // and without indexing or pointer casts a `*mut u8` could not be used at
        // all; `*mut i64` is directly usable and matches the plan's own
        // `buffer.ya`. Recorded as a divergence from the master plan's L3.
        let builtins: [(&str, Vec<Type>, Type); 38] = [
            ("alloc", vec![prim(TypeKind::I64)], ptr(prim(TypeKind::I64))),
            // Byte buffers (phase 3b). `alloc_bytes(n)` is `n` *bytes*, not `n`
            // elements, and the result is a pointer to one byte (`char`), so
            // `buf[i]` is a `char` and a program that wants an `int` has to
            // convert. The two file calls report failure as `io.Error` instead
            // of the empty-string convention `read_file` uses, because an empty
            // file and a missing one are different answers.
            (
                "alloc_bytes",
                vec![prim(TypeKind::I64)],
                ptr(prim(TypeKind::Char)),
            ),
            (
                "read_bytes",
                vec![prim(TypeKind::Str)],
                result_of(slice_of(prim(TypeKind::Char)), io_error()),
            ),
            (
                "write_bytes",
                vec![prim(TypeKind::Str), slice_of(prim(TypeKind::Char))],
                result_of(prim(TypeKind::Unit), io_error()),
            ),
            // Arenas (M3 Task 4): a bump allocator. `arena_new` hands out an
            // opaque handle; `arena_alloc(arena, n)` hands out room for `n`
            // `int`s; `arena_free_all(arena)` releases every block at once.
            // No per-allocation free, and no automatic release -- the program
            // calls `arena_free_all` when it is done, or leaks until exit.
            ("arena_new", vec![], ptr(prim(TypeKind::I64))),
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
            // Input. `read_line` yields `Option[string]`: `None` at end of
            // input and `Some("")` for an empty line, so a loop can tell the
            // two apart. `parse_i64` and `parse_f64` are methods now: they
            // lower to the same runtime entry points, but the public spelling
            // is `s.parse_i64()` / `s.parse_f64()`, and a malformed or
            // overflowing string is `None` rather than a plausible zero. A
            // program that can print but not read is a report generator; this
            // line is what makes it a program.
            ("read_line", vec![], option_of(prim(TypeKind::Str))),
            // Observability (phase 11). `log_info`/`log_error` write one JSON
            // line to stderr; `metrics_add`/`metrics_set` are counters and
            // gauges in the same sink. All four are `io` effects.
            ("log_info", vec![prim(TypeKind::Str)], prim(TypeKind::Unit)),
            ("log_error", vec![prim(TypeKind::Str)], prim(TypeKind::Unit)),
            ("eprint", vec![prim(TypeKind::Str)], prim(TypeKind::Unit)),
            (
                "metrics_add",
                vec![prim(TypeKind::Str), prim(TypeKind::I64)],
                prim(TypeKind::Unit),
            ),
            (
                "metrics_set",
                vec![prim(TypeKind::Str), prim(TypeKind::I64)],
                prim(TypeKind::Unit),
            ),
            // Files and the environment (M6 Task 10, honest signatures in
            // phase 3b). A missing file and an empty file are different
            // answers, so failure is `io.Error` rather than an empty string;
            // `write_file` reports the same error instead of a 1/0 flag.
            // `getenv` is `Option` because an unset variable and an empty one
            // are also different answers.
            (
                "read_file",
                vec![prim(TypeKind::Str)],
                result_of(prim(TypeKind::Str), io_error()),
            ),
            (
                "write_file",
                vec![prim(TypeKind::Str), prim(TypeKind::Str)],
                result_of(prim(TypeKind::Unit), io_error()),
            ),
            (
                "getenv",
                vec![prim(TypeKind::Str)],
                option_of(prim(TypeKind::Str)),
            ),
            // The process and the clock (R1 slice 2). `args()` is a fresh
            // owned `[]string` of the user arguments, excluding the executable
            // path; free it with `.free()`, exactly like `split`/`lines`.
            // `getcwd()` is `Result[string, io.Error]`, the same convention
            // `read_file` uses. `now()` is milliseconds since the Unix epoch;
            // `sleep(ms)` blocks and returns unit, with a non-positive value a
            // no-op.
            (
                "args",
                vec![],
                prim(TypeKind::Slice(Box::new(prim(TypeKind::Str)))),
            ),
            ("getcwd", vec![], result_of(prim(TypeKind::Str), io_error())),
            ("now", vec![], prim(TypeKind::I64)),
            ("sleep", vec![prim(TypeKind::I64)], prim(TypeKind::Unit)),
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
            (
                "str_from_i64",
                vec![prim(TypeKind::I64)],
                prim(TypeKind::Str),
            ),
            (
                "str_from_f64",
                vec![prim(TypeKind::F64)],
                prim(TypeKind::Str),
            ),
            (
                "str_from_bool",
                vec![prim(TypeKind::Bool)],
                prim(TypeKind::Str),
            ),
            // The way out of a situation the program cannot continue from.
            //
            // Its result type is `Never`, not `void`, and that is the whole point:
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
            //     main.ya:3:5: panic: assertion failed: n > 0
            //
            // Like `panic`, the interesting arguments are added by the checker
            // (`Checker::assert_call`): the source text and the position are
            // facts only it holds.
            //
            // The type is `void`, not `Never`: an assertion that *passes* returns
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
            // The declared pair is an `int` one, but the checker never consults
            // it: every call is built generically in `Checker::assert_cmp_call`,
            // so the two operands may be any pair of `int`, `float`, `bool` or
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
            // `int` rather than an `i32` is because `int` is the only integer
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
                    visibility: Visibility::Private,
                },
            );
        }
        Self {
            scope,
            structs: HashMap::new(),
            enums: builtin_enums(),
            fns: HashMap::new(),
            generics: HashMap::new(),
            type_subst: HashMap::new(),
            bound_context: Vec::new(),
            pending_generics: Vec::new(),
            instantiated: HashSet::new(),
            fn_instantiations: HashMap::new(),
            fn_instantiation_site: HashMap::new(),
            active_generics: Vec::new(),
            borrowed_bindings: Vec::new(),
            methods: HashMap::new(),
            struct_schemes: HashMap::new(),
            enum_schemes: HashMap::new(),
            type_monomorphs: HashMap::new(),
            monomorph_schemes: HashMap::new(),
            instantiating: Vec::new(),
            instantiated_types: Vec::new(),
            traits: BTreeMap::new(),
            trait_impls: BTreeMap::new(),
            self_type: None,
            fn_ret: None,
            externs: HashSet::new(),
            variadic_externs: HashSet::new(),
            errors: Vec::new(),
            file,
            aliases: Vec::new(),
            release: options.release,
            uses_threads: false,
            uses_async: false,
            async_ctx: Vec::new(),
            in_const_init: false,
            lambda_ret_infer: Vec::new(),
            extern_symbols: HashMap::new(),
            defined_fns: HashSet::new(),
            iteration_borrows: Vec::new(),
            closures: Vec::new(),
            debug_printers: Vec::new(),
            pending_comps: Vec::new(),
            debug_printer_names: HashSet::new(),
            const_fns: ConstFns::new(),
            item_origins: HashMap::new(),
            sm,
            loop_labels: Vec::new(),
            labeled_loops: Vec::new(),
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push((self.file, TypeError::new(span, message)));
    }

    /// Whether the body being checked is an `async fn` (the innermost function).
    fn in_async(&self) -> bool {
        self.async_ctx.last().copied().unwrap_or(false)
    }

    /// Wrap the declared return type of an `async fn` in `Task[_]`.
    fn wrap_async_ret(ret: Type, span: Span) -> Type {
        Type {
            kind: TypeKind::Task(Box::new(ret)),
            span,
        }
    }

    /// The declared return type of an `async fn` whose call returns
    /// `Task[T]`, or `T` itself for a synchronous function.
    fn unwrap_async_ret(ret: &Type) -> Type {
        match &ret.kind {
            TypeKind::Task(inner) => (**inner).clone(),
            _ => ret.clone(),
        }
    }

    /// Whether an item is wrapped in an `async` effect declaration, looking
    /// through `pub` and doc wrappers so `pub async fn` and friends count.
    fn item_is_async(item: &Item) -> bool {
        match item {
            Item::Effect { effect, .. } => *effect == yashy_common::DeclaredEffect::Async,
            Item::Public { item, .. } | Item::Documented { item, .. } => Self::item_is_async(item),
            _ => false,
        }
    }

    /// Push an error that carries a catalog code.
    fn error_code(&mut self, span: Span, code: &'static str, message: impl Into<String>) {
        self.errors
            .push((self.file, TypeError::new(span, message).with_code(code)));
    }

    /// Record an `extern` declaration's linker symbol.
    ///
    /// The symbol is `link_name`'s value when the declaration wrote one and the
    /// declaration's own name otherwise. Two declarations that resolve to one
    /// symbol are one C function: agreeing signatures are the same function
    /// described twice, and the emitter declares it once; disagreeing ones are
    /// `E0900`, because neither signature can be trusted for both call sites.
    fn check_extern_symbol(&mut self, decl: &ExternDecl, sig: &FnSig) {
        let symbol = decl.symbol.as_deref().unwrap_or(&decl.name).to_string();
        match self.extern_symbols.get(&symbol) {
            None => {
                self.extern_symbols.insert(
                    symbol,
                    ExternSymbol {
                        name: decl.name.clone(),
                        sig: sig.clone(),
                        variadic: decl.variadic,
                        span: decl.span,
                        file: self.file,
                    },
                );
            }
            Some(existing)
                if same_signature(&existing.sig, sig) && existing.variadic == decl.variadic => {}
            Some(existing) => {
                let message = format!(
                    "`{}` and `{}` both resolve to symbol `{}`, but their signatures differ: `{}` and `{}`",
                    decl.name,
                    existing.name,
                    symbol,
                    signature_text(&existing.sig, existing.variadic),
                    signature_text(sig, decl.variadic),
                );
                self.error_code(decl.span, codes::DUPLICATE_EXTERN_SYMBOL, message);
            }
        }
    }

    /// Check one argument past a variadic declaration's fixed parameters.
    ///
    /// C passes these bytes with no type information: the callee's format string
    /// decides how to read them. Yashy therefore does not compare the argument
    /// with any parameter type, but it does require a type whose machine
    /// representation C has -- a fixed-width integer, a float, a pointer, or a
    /// `string` (which codegen converts). A Yashy `int` is refused because its
    /// width is the compiler's, not C's: the program must write the C width with
    /// a cast, and there is no default promotion to hide the mistake.
    fn check_variadic_argument(&mut self, arg: &HirExpr) {
        let ty = expr_ty(arg);
        match &ty.kind {
            TypeKind::CInt(_)
            | TypeKind::CUInt(_)
            | TypeKind::CFloat(_)
            | TypeKind::CSize
            | TypeKind::CPtr(_)
            | TypeKind::Str
            | TypeKind::F64 => {}
            TypeKind::I64 => self.error(
                expr_span(arg),
                "a variadic argument of type `int` needs an explicit C width; write \
                 `42 as i32` (C `int`) or another fixed-width integer so the call \
                 site says which bytes C reads",
            ),
            _ => self.error(
                expr_span(arg),
                format!(
                    "a variadic argument cannot have type `{ty}`: C has no fixed \
                     representation for it"
                ),
            ),
        }
    }

    /// Refuse a declaration whose symbol is a function this program defines.
    ///
    /// The program's functions and the C functions it declares share one flat
    /// linker namespace, so a symbol cannot be both. If it were allowed, the
    /// emitter would write a `declare` and a `define` for one name -- which
    /// `clang` rejects -- and, were it accepted, the declaration's call would
    /// reach the Yashy body: a wrapper around a C function of its own name
    /// would call itself. Reported once, after every item is declared, because
    /// the declaration and the definition may appear in either order.
    fn check_symbol_collisions(&mut self) {
        let mut collisions = Vec::new();
        for (symbol, declared) in &self.extern_symbols {
            if self.defined_fns.contains(symbol) {
                collisions.push((
                    declared.file,
                    declared.span,
                    format!(
                        "`{}` resolves to symbol `{}`, which this program also defines: \
                         a call would reach the Yashy function instead of the C one",
                        declared.name, symbol
                    ),
                ));
            }
        }
        for (file, span, message) in collisions {
            self.errors.push((
                file,
                TypeError::new(span, message).with_code(codes::DUPLICATE_EXTERN_SYMBOL),
            ));
        }
    }

    /// Report a type mismatch from the expected and found types.
    ///
    /// A `Result` used where its success value is expected is the missing-`?`
    /// case: the code says so and the note names both fixes. Every other
    /// mismatch gets the general code.
    fn mismatch(&mut self, span: Span, expected: &Type, found: &Type) {
        let mut error = TypeError::new(span, format!("expected `{expected}`, found `{found}`"));
        match &found.kind {
            TypeKind::Result(ok, _) if ok.eq_ignoring_span(expected) => {
                error = error
                    .with_code(codes::MISSING_QUESTION)
                    .with_note("use `?` to propagate the error, or match the `Result`");
            }
            TypeKind::Option(inner) if inner.eq_ignoring_span(expected) => {
                error = error
                    .with_code(codes::MISSING_QUESTION)
                    .with_note("use `?` to propagate `None`, or match the `Option`");
            }
            _ => error = error.with_code(codes::TYPE_MISMATCH),
        }
        self.report(error);
    }

    /// Report a type mismatch whose message is already written.
    fn mismatch_message(&mut self, span: Span, message: impl Into<String>) {
        self.report(TypeError::new(span, message).with_code(codes::TYPE_MISMATCH));
    }

    /// Push an error that was built elsewhere, keeping any label it carries.
    ///
    /// Resolving a type or a pattern returns a [`TypeError`] that may already
    /// have a "did you mean" label; rebuilding it from `span` and `message`
    /// here would silently drop that help.
    fn report(&mut self, error: TypeError) {
        self.errors.push((self.file, error));
    }

    /// The nearest visible name, for a "did you mean" label.
    ///
    /// Every visible binding is a candidate, including the builtins and the
    /// top-level functions, because they are all names a read can find.
    fn value_suggestion(&self, name: &str) -> Option<String> {
        yashy_common::closest(name, self.scope.names())
    }

    /// The nearest visible *function* name.
    ///
    /// A call says "cannot find function", so suggesting a local that is not
    /// callable would be a worse answer than none. Builtins and top-level
    /// functions both live in the root scope with a `Func` type; the print
    /// family and the value constructors are resolved before the scope lookup
    /// and therefore have to be offered explicitly.
    fn function_suggestion(&self, name: &str) -> Option<String> {
        let candidates = self
            .scope
            .names()
            .into_iter()
            .filter(|candidate| {
                self.scope
                    .resolve(candidate)
                    .map(|binding| matches!(binding.ty.kind, TypeKind::Func(_, _)))
                    .unwrap_or(false)
            })
            .chain(CALLABLE_EXTRAS.iter().copied());
        yashy_common::closest(name, candidates)
    }

    /// Report a name the scope does not have, with the nearest spelling when
    /// there is one.
    fn error_missing_value(&mut self, name: &str, span: Span) {
        let mut error = TypeError::new(span, format!("cannot find value `{name}` in this scope"))
            .with_code(codes::UNKNOWN_NAME);
        if let Some(suggestion) = self.value_suggestion(name) {
            error = error.with_label(format!("did you mean `{suggestion}`?"));
        }
        self.errors.push((self.file, error));
    }

    /// Report a callee the scope does not have, with the nearest function when
    /// there is one.
    fn error_missing_function(&mut self, name: &str, span: Span) {
        let mut error = TypeError::new(span, format!("cannot find function `{name}`"))
            .with_code(codes::UNKNOWN_NAME);
        if let Some(suggestion) = self.function_suggestion(name) {
            error = error.with_label(format!("did you mean `{suggestion}`?"));
        }
        self.errors.push((self.file, error));
    }

    /// The nearest declared or built-in type name, for a "did you mean" label.
    fn type_suggestion(&self, name: &str) -> Option<String> {
        let candidates = self
            .structs
            .keys()
            .map(String::as_str)
            .chain(self.enums.keys().map(String::as_str))
            .chain(BUILTIN_TYPE_NAMES.iter().copied());
        yashy_common::closest(name, candidates)
    }

    /// The nearest field of `struct_name`, for a "did you mean" label.
    fn struct_field_suggestion(&self, struct_name: &str, field: &str) -> Option<String> {
        let fields = self.structs.get(struct_name)?;
        yashy_common::closest(field, fields.fields.iter().map(|(name, _)| name.as_str()))
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
    /// value the program still owns, and Yashy has no way to say which of the
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
    fn capture_in(&mut self, frame_index: usize, name: &str, depth: usize, ty: &Type) -> usize {
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
    /// A view is a pointer into the binding's storage, and Yashy has no way to
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
        if self
            .iteration_borrows
            .iter()
            .any(|borrowed| borrowed == name)
        {
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

    /// Spread-move `name`: like [`Self::move_out`], but a later use is `E1002`.
    fn move_out_spread(&mut self, name: &str, span: Span) -> Result<(), TypeError> {
        if self
            .iteration_borrows
            .iter()
            .any(|borrowed| borrowed == name)
        {
            return Err(TypeError::new(
                span,
                format!("cannot move `{name}` while it is being iterated"),
            ));
        }
        self.scope.move_out_spread(name, span)
    }

    /// A linear value may leave its binding only through `move`. Handing a live
    /// linear binding to a new binding (`let o = buf`) without `move` would
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
        ast: &yashy_parser::ast::Program,
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
        modules: &[(FileId, &yashy_parser::ast::Program)],
    ) -> Result<HirProgram, Vec<Diagnostic>> {
        for (file, ast) in modules {
            self.file = *file;
            self.declare_items(&ast.items);
            let mut module_aliases: HashMap<String, String> = HashMap::new();
            for item in &ast.items {
                if let Item::Import { path, alias, .. } = item {
                    if let Some(target) = path.last() {
                        let name = alias.clone().unwrap_or_else(|| target.clone());
                        module_aliases.insert(name, target.clone());
                    }
                }
            }
            self.aliases.push((*file, module_aliases));
        }
        self.check_symbol_collisions();
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
        // Comprehension builders, like generic instantiations: checking one
        // may queue another (a nested comprehension), so drain until empty.
        while let Some(job) = self.pending_comps.pop() {
            self.file = job.file;
            if let Some(hir) = self.check_fn_body(
                &job.name,
                &job.params,
                &job.param_tys,
                &job.ret_ty,
                &[],
                &[],
                &job.body,
                job.span,
                yashy_common::DeclaredEffect::Inferred,
            ) {
                items.push(HirItem::Fn(hir));
            }
        }
        // The monomorphised generic types, so code generation emits one
        // definition per monomorph: it reads named types from the HIR, and a
        // monomorph that only a pattern or a field mentions would otherwise
        // never be defined.
        items.extend(self.monomorph_items());
        // The printers are ordinary functions with synthesized bodies. They are
        // appended after every generic body has been checked, because a generic
        // instantiation is one of the places a `${value:?}` part can appear.
        items.extend(self.debug_printers.drain(..).map(HirItem::Fn));
        if self.errors.is_empty() {
            Ok(HirProgram {
                items,
                instantiations: self.instantiated.len() + self.type_monomorphs.len(),
                uses_threads: self.uses_threads,
                uses_async: self.uses_async,
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
        public: bool,
        span: Span,
        is_async: bool,
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
                        // `Self` as a pointee is the receiver type itself:
                        // `self: *mut Self` in a trait impl, where no concrete
                        // type is in scope yet.
                        let pointee_ty = match pointee.as_ref() {
                            TypeExpr::Name(written, _) if written == "Self" => receiver_ty.clone(),
                            _ => self.resolve_or_report(pointee, param.span),
                        };
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
                            format!("a receiver is `self` or `self: *mut {type_name}`"),
                        );
                        param_tys.push(receiver_ty.clone());
                    }
                }
                continue;
            }
            let param_ty = self.resolve_or_report(&param.ty, param.span);
            if let Some(default) = &param.default {
                let _ =
                    self.check_const_default(default, &param_ty, param.span, "a parameter default");
            }
            param_tys.push(param_ty);
        }
        let ret_inner = match ret {
            Some(ty) => self.resolve_or_report_return(ty, span),
            None => prim(TypeKind::Unit),
        };
        let ret_ty = if is_async {
            Self::wrap_async_ret(ret_inner, span)
        } else {
            ret_inner
        };
        if is_async {
            self.uses_async = true;
        }
        MethodSig {
            type_name: type_name.to_string(),
            name: name.to_string(),
            params: param_tys,
            ret: ret_ty,
            has_self,
            receiver_is_pointer,
            is_async,
            origin: ItemOrigin {
                file: self.file,
                public,
            },
        }
    }

    /// Whether an item (or its wrapper chain) is marked `pub`.
    fn is_public_item(item: &Item) -> bool {
        match item {
            Item::Public { .. } => true,
            Item::Effect { item, .. } | Item::Documented { item, .. } => Self::is_public_item(item),
            _ => false,
        }
    }

    /// Check one `impl Trait for Type` block: coherence, then completeness.
    ///
    /// Coherence is per (trait, type) pair and is decided before any body is
    /// checked (bodies are checked in pass 2): a second impl for a pair is
    /// `E1501` at the header, and a method set that does not match the trait
    /// is `E1502`. The impl's methods are registered under `Type.Trait.method`,
    /// beside the inherent `Type.method` spelling, so a call resolves to one
    /// concrete `HirFn` either way.
    fn declare_trait_impl(
        &mut self,
        trait_name: &str,
        type_name: &str,
        methods: &[Item],
        receiver_ty: &Type,
        span: Span,
    ) {
        let Some(trait_sig) = self.traits.get(trait_name).cloned() else {
            let mut err = TypeError::new(span, format!("no trait named `{trait_name}`"));
            if let Some(suggestion) =
                yashy_common::closest(trait_name, self.traits.keys().map(String::as_str))
            {
                err = err.with_label(format!("did you mean `{suggestion}`?"));
            }
            self.errors
                .push((self.file, err.with_code(codes::UNKNOWN_TRAIT)));
            return;
        };
        if self
            .trait_impls
            .contains_key(&(trait_name.to_string(), type_name.to_string()))
        {
            self.error_code(
                span,
                codes::CONFLICTING_IMPL,
                format!(
                    "`{type_name}` already implements `{trait_name}`: one trait has one impl per type"
                ),
            );
            return;
        }
        // The impl's own methods, resolved with `Self` the implementing type,
        // the same way an inherent impl's are.
        let outer_self_type = self.self_type.replace(receiver_ty.clone());
        let mut impl_sigs: Vec<(MethodSig, Span)> = Vec::new();
        for method in methods {
            let Item::Fn {
                name,
                type_params,
                params,
                ret,
                body: _,
                requires: _,
                ensures: _,
                span: method_span,
            } = unwrap_public(method)
            else {
                self.error(span, "an `impl` block holds only methods");
                continue;
            };
            if !type_params.is_empty() {
                self.error(*method_span, "a method cannot have type parameters yet");
                continue;
            }
            impl_sigs.push((
                self.method_signature(
                    type_name,
                    name,
                    params,
                    ret,
                    receiver_ty,
                    Self::is_public_item(method),
                    *method_span,
                    Self::item_is_async(method),
                ),
                *method_span,
            ));
        }
        self.self_type = outer_self_type;
        // `Iterator[T]` is the one generic trait: its `T` is the element
        // type, which each impl picks out with its own `Option[E]` return,
        // so the declared signature cannot be compared literally. What is
        // checked instead is the protocol shape: exactly one `next` with a
        // pointer receiver returning `Option[E]`.
        if trait_name == "Iterator" {
            let impl_name = "next";
            let missing = trait_sig.methods.iter().any(|e| e.name == impl_name)
                && !impl_sigs.iter().any(|(sig, _)| sig.name == impl_name);
            if missing {
                self.error_code(
                    span,
                    codes::INCOMPLETE_IMPL,
                    format!("`{type_name}` does not implement `Iterator`: missing `fn next(self: *mut Self) -> Option[T]`"),
                );
            }
            for (sig, method_span) in &impl_sigs {
                if sig.name != impl_name {
                    self.error_code(
                        *method_span,
                        codes::INCOMPLETE_IMPL,
                        format!(
                            "`{type_name}` implements `Iterator` but `Iterator` declares no `{}`",
                            sig.name,
                        ),
                    );
                    continue;
                }
                if !sig.has_self || !sig.receiver_is_pointer {
                    self.error_code(
                        *method_span,
                        codes::INCOMPLETE_IMPL,
                        format!(
                            "`{type_name}`'s `next` does not match `Iterator`: impl has `{}`, trait requires `fn next(self: *mut Self) -> Option[T]`",
                            describe_method(sig, type_name),
                        ),
                    );
                } else if !matches!(sig.ret.kind, TypeKind::Option(_)) {
                    self.error_code(
                        *method_span,
                        codes::INCOMPLETE_IMPL,
                        format!(
                            "`{type_name}`'s `next` does not match `Iterator`: impl has `{}`, trait requires `fn next(self: *mut Self) -> Option[T]`",
                            describe_method(sig, type_name),
                        ),
                    );
                }
            }
        } else {
            // Completeness: exactly the trait's methods, with exactly its
            // signatures.
            for expected in &trait_sig.methods {
                match impl_sigs.iter().find(|(sig, _)| sig.name == expected.name) {
                    Some((got, method_span)) => {
                        if !same_method_signature(expected, got, receiver_ty) {
                            self.error_code(
                            *method_span,
                            codes::INCOMPLETE_IMPL,
                            format!(
                                "`{type_name}`'s `{}` does not match `{trait_name}`: impl has `{}`, \
                                 trait requires `{}`",
                                expected.name,
                                describe_method(got, type_name),
                                describe_method(expected, type_name),
                            ),
                        );
                        }
                    }
                    None => {
                        self.error_code(
                            span,
                            codes::INCOMPLETE_IMPL,
                            format!(
                                "`{type_name}` does not implement `{trait_name}`: missing `{}`",
                                describe_method(expected, type_name),
                            ),
                        );
                    }
                }
            }
        } // end non-Iterator completeness
        let mut emitted = Vec::new();
        for (sig, method_span) in &impl_sigs {
            if !trait_sig.methods.iter().any(|e| e.name == sig.name) {
                self.error_code(
                    *method_span,
                    codes::INCOMPLETE_IMPL,
                    format!(
                        "`{type_name}` implements `{trait_name}` but `{trait_name}` declares no `{}`",
                        sig.name,
                    ),
                );
            }
            let symbol = format!("{type_name}.{trait_name}.{}", sig.name);
            if let Some(previous) = self.methods.insert(symbol.clone(), sig.clone()) {
                if previous.name == sig.name {
                    self.error(
                        *method_span,
                        format!("`{type_name}` already has a method named `{}`", sig.name),
                    );
                }
            }
            emitted.push((sig.name.clone(), symbol));
        }
        self.trait_impls.insert(
            (trait_name.to_string(), type_name.to_string()),
            TraitImpl { methods: emitted },
        );
    }

    /// Check every bound of every type parameter against the argument types.
    ///
    /// A miss is `E1503` naming the type, the trait and where the impl
    /// belongs; a bound on an undeclared trait was already reported where the
    /// parameter was declared. `required_by` is what the message names as
    /// the thing demanding the bound: the function for a call, the generic
    /// type for an instantiation.
    fn check_type_args_bounds(
        &mut self,
        params: &[TypeParam],
        arg_tys: &[Type],
        span: Span,
        required_by: &str,
    ) {
        for (param, concrete) in params.iter().zip(arg_tys.iter()) {
            for bound in &param.bounds {
                if !self.traits.contains_key(bound) {
                    continue;
                }
                let satisfies = match &concrete.kind {
                    TypeKind::Struct(type_name) | TypeKind::Enum(type_name) => self
                        .trait_impls
                        .contains_key(&(bound.clone(), type_name.clone())),
                    _ => false,
                };
                if !satisfies {
                    self.error_code(
                        span,
                        codes::UNSATISFIED_BOUND,
                        format!(
                            "`{concrete}` does not implement `{bound}` required by \
                             `{required_by}`{}",
                            self.impl_needs_line(concrete, bound),
                        ),
                    );
                }
            }
        }
    }

    /// Intern every valid generic type in a type expression.
    ///
    /// Resolution itself is pure (it cannot intern), so the reporting
    /// wrappers walk the tree first. Each `Name[args..]` whose name is a
    /// declared generic type and whose arity matches is bounds-checked and
    /// interned once per argument tuple. Anything else is left for the pure
    /// resolution to report once, at the use site.
    fn intern_generics_in(&mut self, ty: &TypeExpr) {
        match ty {
            TypeExpr::Generic { name, args, span } => {
                self.intern_generic(name, args, *span);
            }
            TypeExpr::Ptr { pointee, .. }
            | TypeExpr::FixedArray(pointee, _, _)
            | TypeExpr::View(pointee, _)
            | TypeExpr::Slice(pointee, _)
            | TypeExpr::Option(pointee, _)
            | TypeExpr::Channel(pointee, _) => self.intern_generics_in(pointee),
            TypeExpr::Result(ok, err, _) | TypeExpr::Map(ok, err, _) => {
                self.intern_generics_in(ok);
                self.intern_generics_in(err);
            }
            TypeExpr::Tuple(items, _) => {
                for item in items {
                    self.intern_generics_in(item);
                }
            }
            TypeExpr::Fn { params, ret, .. } => {
                for param in params {
                    self.intern_generics_in(param);
                }
                self.intern_generics_in(ret);
            }
            _ => {}
        }
    }

    /// Bounds-check and intern one generic use, `Name[args..]`.
    fn intern_generic(&mut self, name: &str, args: &[TypeExpr], span: Span) {
        let params: Vec<TypeParam> = if let Some(scheme) = self.struct_schemes.get(name) {
            scheme.type_params.clone()
        } else if let Some(scheme) = self.enum_schemes.get(name) {
            scheme.type_params.clone()
        } else {
            return;
        };
        if args.len() != params.len() {
            return;
        }
        let mut arg_tys = Vec::with_capacity(args.len());
        for arg in args {
            // Deeper generics first, so their monomorphs exist before the
            // fields of this one resolve through them.
            self.intern_generics_in(arg);
            match self.resolve_type(arg) {
                Ok(ty) => arg_tys.push(ty),
                // The pure resolution reports this once; nothing to intern.
                Err(_) => return,
            }
        }
        // Bounds fire at every use, whether or not the monomorph is new.
        self.check_type_args_bounds(&params, &arg_tys, span, name);
        self.intern_monomorph(name, &params, args, arg_tys, span);
    }

    /// Intern one monomorph: resolve the scheme's fields with the arguments
    /// substituted, and record the argument tuple.
    ///
    /// One monomorphic type per argument tuple, named by the same mangled
    /// name generic functions use, and emitted once (see
    /// `instantiated_types`). The recursion guard is what keeps a type
    /// naming itself in its own fields from instantiating forever.
    fn intern_monomorph(
        &mut self,
        name: &str,
        params: &[TypeParam],
        args: &[TypeExpr],
        arg_tys: Vec<Type>,
        span: Span,
    ) {
        let mangled = mangled_generic_name(name, &arg_tys);
        self.monomorph_schemes
            .insert(mangled.clone(), name.to_string());
        self.type_monomorphs
            .insert(mangled.clone(), arg_tys.clone());
        let is_struct = self.struct_schemes.contains_key(name);
        if (is_struct && self.structs.contains_key(&mangled))
            || (!is_struct && self.enums.contains_key(&mangled))
        {
            return;
        }
        if self.instantiating.contains(&mangled) {
            self.error(
                span,
                format!(
                    "`{name}` names itself in its own fields: a recursive type needs a pointer"
                ),
            );
            return;
        }
        self.instantiating.push(mangled.clone());
        let origin_file = self
            .item_origins
            .get(name)
            .map(|origin| origin.file)
            .unwrap_or(self.file);
        let outer_file = std::mem::replace(&mut self.file, origin_file);
        let outer_subst = std::mem::replace(
            &mut self.type_subst,
            params
                .iter()
                .zip(arg_tys.iter())
                .map(|(param, arg)| (param.name.clone(), arg.clone()))
                .collect(),
        );
        if is_struct {
            let scheme = self.struct_schemes[name].clone();
            let allow_c = scheme.repr_c;
            let mut field_tys = Vec::with_capacity(scheme.fields.len());
            for field in &scheme.fields {
                let ty = self.resolve_or_report_field(&field.ty, field.span, allow_c);
                if matches!(ty.kind, TypeKind::Func(_, _)) {
                    self.error(field.span, "a struct field cannot have a function type yet");
                }
                field_tys.push((field.name.clone(), ty));
            }
            let field_public: Vec<bool> = scheme.fields.iter().map(|f| f.public).collect();
            let defaults: Vec<Option<HirExpr>> = scheme
                .fields
                .iter()
                .zip(field_tys.iter())
                .map(|(field, (_, ty))| match &field.default {
                    Some(default) => self.check_field_default(default, ty, field.span),
                    None => None,
                })
                .collect();
            self.structs.insert(
                mangled.clone(),
                StructSig {
                    fields: field_tys,
                    field_public,
                    defaults,
                    repr_c: scheme.repr_c,
                    origin: scheme.origin,
                },
            );
        } else {
            let scheme = self.enum_schemes[name].clone();
            let allow_c = scheme.repr_c;
            let mut payloads = Vec::with_capacity(scheme.variants.len());
            for variant in &scheme.variants {
                if scheme.repr_c && !variant.fields.is_empty() {
                    self.error_code(
                        variant.span,
                        codes::REPR_NOT_ALLOWED,
                        format!(
                            "`repr(C)` needs a fieldless `enum`: variant `{}` of `{name}` has a payload",
                            variant.name
                        ),
                    );
                }
                let mut field_tys = Vec::with_capacity(variant.fields.len());
                for field in &variant.fields {
                    let ty = self.resolve_or_report_field(field.ty(), field.span(), allow_c);
                    if matches!(ty.kind, TypeKind::Func(_, _)) {
                        self.error(
                            field.span(),
                            "an enum field cannot have a function type yet",
                        );
                    }
                    field_tys.push(ty.kind);
                }
                payloads.push(field_tys);
            }
            // The variants stored on the monomorph have the arguments
            // substituted in, so patterns resolve their payloads against
            // the monomorph rather than the scheme.
            let subst_exprs: Vec<(String, TypeExpr)> = params
                .iter()
                .zip(args.iter())
                .map(|(param, arg)| (param.name.clone(), arg.clone()))
                .collect();
            let variants: Vec<VariantDef> = scheme
                .variants
                .iter()
                .map(|variant| VariantDef {
                    name: variant.name.clone(),
                    fields: variant
                        .fields
                        .iter()
                        .map(|field| match field {
                            VariantField::Positional { ty, .. } => VariantField::Positional {
                                ty: substitute_type_expr(ty, &subst_exprs),
                                span: field.span(),
                            },
                            VariantField::Named { name, ty, span } => VariantField::Named {
                                name: name.clone(),
                                ty: substitute_type_expr(ty, &subst_exprs),
                                span: *span,
                            },
                        })
                        .collect(),
                    span: variant.span,
                })
                .collect();
            self.enums.insert(
                mangled.clone(),
                EnumSig {
                    variants,
                    payloads,
                    repr_c: scheme.repr_c,
                },
            );
        }
        self.type_subst = outer_subst;
        self.file = outer_file;
        self.instantiating.pop();
        self.instantiated_types.push(mangled);
    }

    /// The monomorphs interned this check, after everything is checked.
    ///
    /// The checker emits them itself: code generation reads named types from
    /// the HIR, and a monomorph that only a pattern or a field mentions
    /// would otherwise never be defined.
    fn monomorph_items(&mut self) -> Vec<HirItem> {
        let mut out = Vec::new();
        let names = std::mem::take(&mut self.instantiated_types);
        for mangled in names {
            if let Some(sig) = self.structs.get(&mangled).cloned() {
                let scheme_name = self
                    .monomorph_schemes
                    .get(&mangled)
                    .cloned()
                    .unwrap_or_default();
                let scheme_span = self
                    .struct_schemes
                    .get(&scheme_name)
                    .map(|scheme| scheme.span)
                    .unwrap_or(Span::new(0, 0));
                let field_spans: Vec<Span> = self
                    .struct_schemes
                    .get(&scheme_name)
                    .map(|scheme| scheme.fields.iter().map(|f| f.span).collect())
                    .unwrap_or_else(|| sig.fields.iter().map(|_| scheme_span).collect());
                out.push(HirItem::Struct(HirStruct {
                    name: mangled.clone(),
                    fields: sig
                        .fields
                        .iter()
                        .zip(field_spans.iter())
                        .map(|((fname, fty), fspan)| HirFieldDef {
                            name: fname.clone(),
                            ty: fty.clone(),
                            span: *fspan,
                        })
                        .collect(),
                    repr: if sig.repr_c { Some(ReprKind::C) } else { None },
                    span: scheme_span,
                }));
            } else if let Some(sig) = self.enums.get(&mangled).cloned() {
                let scheme_name = self
                    .monomorph_schemes
                    .get(&mangled)
                    .cloned()
                    .unwrap_or_default();
                let scheme_span = self
                    .enum_schemes
                    .get(&scheme_name)
                    .map(|scheme| scheme.span)
                    .unwrap_or(Span::new(0, 0));
                out.push(HirItem::Enum(HirEnum {
                    name: mangled.clone(),
                    variants: sig
                        .variants
                        .iter()
                        .zip(sig.payloads.iter())
                        .map(|(variant, payload)| HirVariantDef {
                            name: variant.name.clone(),
                            fields: variant
                                .fields
                                .iter()
                                .zip(payload.iter())
                                .map(|(field, ty)| HirVariantField {
                                    name: match field {
                                        VariantField::Positional { .. } => None,
                                        VariantField::Named { name, .. } => Some(name.clone()),
                                    },
                                    ty: Type {
                                        kind: ty.clone(),
                                        span: field.span(),
                                    },
                                    span: field.span(),
                                })
                                .collect(),
                            span: variant.span,
                        })
                        .collect(),
                    repr: if sig.repr_c { Some(ReprKind::C) } else { None },
                    span: scheme_span,
                }));
            }
        }
        out
    }

    /// Where an impl of `bound` for `concrete` would belong, for a bound
    /// diagnostic. A named type's impl belongs beside the type's declaration;
    /// a primitive has no module to name.
    fn impl_needs_line(&self, concrete: &Type, bound: &str) -> String {
        if let TypeKind::Struct(name) | TypeKind::Enum(name) = &concrete.kind {
            if let Some(origin) = self.item_origins.get(name) {
                return format!(
                    "; write `impl {bound} for {name}` in `{}`",
                    self.module_name(origin.file)
                );
            }
        }
        String::new()
    }

    /// Resolve `method` on `type_name` by the fixed order: inherent impl,
    /// then the single trait impl in scope, then -- at the call site -- a
    /// bound on the receiver's type parameter.
    ///
    /// Candidates are filtered by arity and receiver before the order
    /// applies, so a method at the wrong arity does not shadow one at the
    /// right one. Two candidates at one level are `E1504`, reported, never
    /// resolved by import order. A trait that declares the method but has no
    /// impl for the type is `E1503`. The outer `Option` is `None` when the
    /// type has no method of that name at any level (the caller falls
    /// through to the builtin path); the inner is `None` when an error was
    /// reported here.
    fn resolve_method(
        &mut self,
        type_name: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<(String, MethodSig)>> {
        let own_args = args.len();
        // Level 1: the inherent impl.
        let inherent_key = format!("{type_name}.{method}");
        let inherent = self.methods.get(&inherent_key).cloned();
        if let Some(sig) = &inherent {
            if sig.has_self && sig.params.len().saturating_sub(1) == own_args {
                return Some(Some((inherent_key, sig.clone())));
            }
        }
        // Level 2: trait impls for the type providing `method`.
        let mut impls: Vec<(String, String, MethodSig)> = Vec::new();
        for ((trait_name, ty), impl_) in &self.trait_impls {
            if ty != type_name {
                continue;
            }
            for (mname, symbol) in &impl_.methods {
                if mname == method {
                    if let Some(sig) = self.methods.get(symbol) {
                        impls.push((trait_name.clone(), symbol.clone(), sig.clone()));
                    }
                }
            }
        }
        let fitting: Vec<&(String, String, MethodSig)> = impls
            .iter()
            .filter(|(_, _, sig)| sig.has_self && sig.params.len().saturating_sub(1) == own_args)
            .collect();
        if fitting.len() == 1 {
            let (_, symbol, sig) = fitting[0].clone();
            return Some(Some((symbol, sig)));
        }
        if fitting.len() > 1 {
            let names = fitting
                .iter()
                .map(|(trait_name, _, _)| format!("`{trait_name}`"))
                .collect::<Vec<_>>()
                .join(" and ");
            let (first, second) = (&fitting[0].0, &fitting[1].0);
            self.error_code(
                span,
                codes::AMBIGUOUS_METHOD,
                format!(
                    "`{method}` is ambiguous on `{type_name}`: both {names} provide it \
                     (qualify with `{first}::{method}(x)` or `{second}::{method}(x)` \
                     once `::` is available)"
                ),
            );
            return Some(None);
        }
        // Nothing compatible: report from the highest level that had any.
        if let Some(sig) = inherent {
            // An arity or receiver error in the call path that follows.
            return Some(Some((inherent_key, sig)));
        }
        if impls.len() == 1 {
            let (_, symbol, sig) = impls.into_iter().next().unwrap();
            return Some(Some((symbol, sig)));
        }
        if impls.len() > 1 {
            let names = impls
                .iter()
                .map(|(trait_name, _, _)| format!("`{trait_name}`"))
                .collect::<Vec<_>>()
                .join(" and ");
            let (first, second) = (&impls[0].0, &impls[1].0);
            self.error_code(
                span,
                codes::AMBIGUOUS_METHOD,
                format!(
                    "`{method}` is ambiguous on `{type_name}`: both {names} provide it \
                     (qualify with `{first}::{method}(x)` or `{second}::{method}(x)` \
                     once `::` is available)"
                ),
            );
            return Some(None);
        }
        // A trait declares the method but the type does not implement it.
        let declaring = self
            .traits
            .iter()
            .filter(|(_, sig)| sig.methods.iter().any(|m| m.name == method))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        if let Some(trait_name) = declaring.first() {
            let concrete = Type {
                kind: if self.enums.contains_key(type_name) {
                    TypeKind::Enum(type_name.to_string())
                } else {
                    TypeKind::Struct(type_name.to_string())
                },
                span,
            };
            self.error_code(
                span,
                codes::UNSATISFIED_BOUND,
                format!(
                    "`{method}` is provided by `{trait_name}` but `{type_name}` does not \
                     implement it{}",
                    self.impl_needs_line(&concrete, trait_name),
                ),
            );
            return Some(None);
        }
        None
    }

    /// The bound-context entry a method receiver is bound to, if any.
    ///
    /// Only a receiver that names a parameter binding whose declared type is
    /// the type parameter has one: the bounds apply to the parameter, not to
    /// every value of the substituted type.
    fn bound_entry(&self, receiver: &HirExpr) -> Option<&BoundBinding> {
        let HirExpr::Ident(path, _, _) = receiver else {
            return None;
        };
        let name = path.last()?;
        self.bound_context.iter().find(|entry| entry.name == *name)
    }

    /// Resolve a method call on a type-parameter binding through the bounds
    /// in force.
    ///
    /// Returns the `(symbol, signature)` to call, or `None` after reporting:
    /// `E1503` when no bound provides the method — naming the missing bound,
    /// which is the trait whose impl on the concrete type provides it when
    /// there is one — and an ambiguity error when two bounds provide it.
    fn bound_candidate(
        &mut self,
        entry: &BoundBinding,
        type_name: &str,
        method: &str,
        span: Span,
    ) -> Option<(String, MethodSig)> {
        let mut candidates = Vec::new();
        for bound in &entry.bounds {
            let symbol = format!("{type_name}.{bound}.{method}");
            if let Some(sig) = self.methods.get(&symbol) {
                if sig.has_self {
                    candidates.push((bound.clone(), symbol, sig.clone()));
                }
            }
        }
        match candidates.len() {
            1 => {
                let (_, symbol, sig) = candidates.into_iter().next().unwrap();
                Some((symbol, sig))
            }
            0 => {
                self.error_code(
                    span,
                    codes::UNSATISFIED_BOUND,
                    self.missing_bound_message(entry, type_name, method),
                );
                None
            }
            _ => {
                let names = candidates
                    .iter()
                    .map(|(bound, _, _)| format!("`{bound}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.error(
                    span,
                    format!(
                        "`{method}` is provided by both {names} on `{type_name}`: \
                         one call has one target"
                    ),
                );
                None
            }
        }
    }

    /// The `E1503` message for a method no bound in force provides.
    fn missing_bound_message(&self, entry: &BoundBinding, type_name: &str, method: &str) -> String {
        for ((trait_name, ty), impl_) in &self.trait_impls {
            if ty.as_str() != type_name {
                continue;
            }
            if impl_.methods.iter().any(|(name, _)| name == method) {
                return format!(
                    "`{method}` needs `{}`: `{trait_name}` is not one of its bounds",
                    entry.param,
                );
            }
        }
        let bounds = if entry.bounds.is_empty() {
            "no bounds".to_string()
        } else {
            entry
                .bounds
                .iter()
                .map(|bound| format!("`{bound}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "`{method}` is not provided by `{}`'s bounds ({bounds})",
            entry.param,
        )
    }

    /// Report a bound naming a trait the program does not declare.
    ///
    /// Checked where the parameter is written, not at each instantiation: the
    /// mistake is in the declaration, and repeating it at every call would
    /// bury it.
    fn check_generic_bounds(&mut self, param: &TypeParam) {
        for bound in &param.bounds {
            if !self.traits.contains_key(bound) {
                let mut err = TypeError::new(
                    param.span,
                    format!("no trait named `{bound}` to bound `{}` on", param.name),
                );
                if let Some(suggestion) =
                    yashy_common::closest(bound, self.traits.keys().map(String::as_str))
                {
                    err = err.with_label(format!("did you mean `{suggestion}`?"));
                }
                self.errors
                    .push((self.file, err.with_code(codes::UNKNOWN_TRAIT)));
            }
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
        type_params: &[TypeParam],
        params: &[Param],
        ret: &Option<TypeExpr>,
        requires: &[Contract],
        ensures: &[Contract],
        body: &Block,
        span: Span,
        is_async: bool,
    ) {
        if self.generics.contains_key(name) || self.scope.resolve(name).is_some() {
            self.error(span, format!("duplicate definition of `{name}`"));
        }
        for (index, param) in type_params.iter().enumerate() {
            if type_params[..index]
                .iter()
                .any(|previous| previous.name == param.name)
            {
                self.error(
                    param.span,
                    format!("duplicate type parameter `{}`", param.name),
                );
            }
            if param.name == "Self" {
                self.error(
                    param.span,
                    "`Self` is not a type parameter name: it means the method's own type",
                );
            }
            // A type parameter shadows the type it names inside the body, so a
            // parameter that names one turns every mention of that type in the
            // body into something else. That is never what the author meant.
            if self
                .resolve_type(&TypeExpr::Name(param.name.clone(), param.span))
                .is_ok()
            {
                self.error(
                    param.span,
                    format!(
                        "type parameter `{}` shadows a type of the same name",
                        param.name
                    ),
                );
            }
            // A bound names a trait: one the program does not declare cannot
            // be implemented, so it is `E1500` where it is written rather than
            // `E1503` at every call.
            self.check_generic_bounds(param);
        }
        self.generics.insert(
            name.to_string(),
            GenericFn {
                type_params: type_params.to_vec(),
                params: params.to_vec(),
                ret: ret.clone(),
                requires: requires.to_vec(),
                ensures: ensures.to_vec(),
                body: body.clone(),
                is_async,
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
        self.error_code(
            span,
            codes::PRIVATE_ITEM,
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
        self.error_code(
            span,
            codes::PRIVATE_ITEM,
            format!("field `{field}` of `{struct_name}` is private to module `{module}`"),
        );
    }

    /// The name a module is known by in a diagnostic.
    ///
    /// A module is one source file, so the name is that file's own name
    /// without its extension: `json_keys.ya` is the module `json_keys`.
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
                // A doc block may wrap outside `pub` (`/// x\npub fn`): the
                // `pub` test below has to see through it, so unwrap here and
                // declare the item inside with the same visibility.
                Item::Documented { item, .. } => self.declare_items(std::slice::from_ref(&**item)),
                // `pub` is a modifier in front of an item: the item inside is
                // declared as usual, and marked as one another module may use.
                Item::Public { item, .. } => {
                    self.declare_item(item, true, yashy_common::DeclaredEffect::Inferred)
                }
                // `pure`/`io`/`unsafe` are modifiers like `pub`: the item
                // inside is declared as usual. A `pure trait` keeps its
                // effect on the trait itself, so impls inherit it.
                Item::Effect { effect, item, .. } => {
                    self.declare_item(item, false, *effect);
                    if let Item::Trait { name, span, .. } = (**item).clone() {
                        if *effect == yashy_common::DeclaredEffect::Async {
                            self.error_code(
                                span,
                                codes::ASYNC_TRAIT,
                                format!(
                                    "`{name}` is declared `async`: traits have no async methods; declare a synchronous method instead"
                                ),
                            );
                        }
                        if let Some(sig) = self.traits.get_mut(&name) {
                            sig.declared = *effect;
                        }
                    }
                }
                other => self.declare_item(other, false, yashy_common::DeclaredEffect::Inferred),
            }
        }
    }

    /// Declare one item's name, and record where it came from.
    ///
    /// `public` says whether the item is marked `pub`, which is what decides
    /// whether another module may use it. `declared` is the effect wrapper
    /// in front of the item, if any: an `async fn`'s call type is `Task[T]`
    /// rather than `T`, which is a fact about its signature pass 1 has to
    /// record.
    fn declare_item(&mut self, item: &Item, public: bool, declared: yashy_common::DeclaredEffect) {
        match item {
            Item::Public { item, .. } => self.declare_item(item, public, declared),
            Item::Effect { item, .. } => self.declare_item(item, public, declared),
            // A doc block documents the item inside it; the item declares
            // as usual.
            Item::Documented { item, .. } => self.declare_item(item, public, declared),
            // The `cfg` filter runs before this pass, so a guarded item should
            // never arrive still wrapped. Looking through it keeps a caller
            // that skipped the filter (the one-file `compile_to_binary`
            // convenience) from silently declaring nothing.
            Item::Cfg { item, .. } => self.declare_item(item, public, declared),
            Item::Fn {
                name,
                type_params,
                params,
                ret,
                requires,
                ensures,
                body,
                span,
            } => {
                let is_async = declared == yashy_common::DeclaredEffect::Async;
                if is_async {
                    self.uses_async = true;
                }
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
                        requires,
                        ensures,
                        body,
                        *span,
                        is_async,
                    );
                    self.record_origin(name, public);
                    return;
                }
                let param_tys: Vec<Type> = params
                    .iter()
                    .map(|Param { ty, .. }| self.resolve_or_report(ty, *span))
                    .collect();
                let ret_inner = match ret {
                    Some(ty) => self.resolve_or_report_return(ty, *span),
                    None => prim(TypeKind::Unit),
                };
                // `async fn f() -> T` answers `Task[T]`: the call does not run
                // the body, it hands back the task that will.
                let ret_ty = if is_async {
                    Self::wrap_async_ret(ret_inner, *span)
                } else {
                    ret_inner
                };
                let sig = FnSig {
                    params: param_tys.clone(),
                    param_names: params.iter().map(|p| p.name.clone()).collect(),
                    param_defaults: params.iter().map(|p| p.default.clone()).collect(),
                    ret: ret_ty.clone(),
                    is_async,
                };
                // A parameter default is held to the same rule as a field
                // default (E0102), checked here, at the declaration, so a bad
                // default is reported even before any call. The evaluated
                // value is discarded: the call site re-evaluates the
                // (constant) expression when it fills the slot.
                for (param, ty) in params.iter().zip(param_tys.iter()) {
                    if let Some(default) = &param.default {
                        let _ = self.check_const_default(
                            default,
                            ty,
                            param.span,
                            "a parameter default",
                        );
                    }
                }
                let binding = Binding {
                    ty: Type {
                        kind: TypeKind::Func(param_tys, Box::new(ret_ty)),
                        span: *span,
                    },
                    mutability: Mutability::Const,
                    span: *span,
                    ownership: Ownership::Live,
                    visibility: Visibility::Private,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.report(e);
                }
                self.fns.insert(name.clone(), sig);
                self.defined_fns.insert(name.clone());
                self.record_origin(name, public);
            }
            Item::Impl {
                type_name,
                trait_name,
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
                if let Some(trait_name) = trait_name {
                    if trait_name == "Drop" {
                        self.error_code(
                            *span,
                            codes::DROP_TRAIT,
                            "there is no `Drop`: release with `defer` and `.free()` instead",
                        );
                        return;
                    }
                    self.declare_trait_impl(trait_name, type_name, methods, &receiver_ty, *span);
                    return;
                }
                for method in methods {
                    let Item::Fn {
                        name,
                        type_params,
                        params,
                        ret,
                        body: _,
                        requires: _,
                        ensures: _,
                        span: method_span,
                    } = unwrap_public(method)
                    else {
                        self.error(*span, "an `impl` block holds only methods");
                        continue;
                    };
                    if !type_params.is_empty() {
                        self.error(*method_span, "a method cannot have type parameters yet");
                        continue;
                    }
                    let sig = self.method_signature(
                        type_name,
                        name,
                        params,
                        ret,
                        &receiver_ty,
                        Self::is_public_item(method),
                        *method_span,
                        Self::item_is_async(method),
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
                repr,
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
                    visibility: Visibility::Private,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.report(e);
                }
                let allow_c = repr.is_some();
                let field_tys: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.resolve_or_report_field(&f.ty, f.span, allow_c);
                        (f.name.clone(), ty)
                    })
                    .collect();
                let field_public: Vec<bool> = fields.iter().map(|f| f.public).collect();
                let defaults: Vec<Option<HirExpr>> = fields
                    .iter()
                    .zip(field_tys.iter())
                    .map(|(f, (_, ty))| match &f.default {
                        Some(default) => self.check_field_default(default, ty, f.span),
                        None => None,
                    })
                    .collect();
                self.structs.insert(
                    name.clone(),
                    StructSig {
                        fields: field_tys,
                        field_public,
                        defaults,
                        repr_c: repr.is_some(),
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
                        requires: _,
                        ensures: _,
                        span: method_span,
                    } = unwrap_public(method)
                    else {
                        self.error(*span, "an `object` holds only methods");
                        continue;
                    };
                    if !type_params.is_empty() {
                        self.error(*method_span, "a method cannot have type parameters yet");
                        continue;
                    }
                    let sig = self.method_signature(
                        name,
                        method_name,
                        params,
                        ret,
                        &receiver_ty,
                        Self::is_public_item(method),
                        *method_span,
                        Self::item_is_async(method),
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
            Item::Struct {
                name,
                type_params,
                fields,
                repr,
                span,
            } => {
                let binding = Binding {
                    ty: Type {
                        kind: TypeKind::Struct(name.clone()),
                        span: *span,
                    },
                    mutability: Mutability::Const,
                    span: *span,
                    ownership: Ownership::Live,
                    visibility: Visibility::Private,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.report(e);
                }
                if !type_params.is_empty() {
                    // A generic struct is a scheme: its field types mention
                    // the parameters, so they resolve per instantiation, not
                    // once here. What is checked at the declaration is what
                    // does not depend on the arguments: every parameter used
                    // (E1510), and every bound naming a declared trait.
                    for param in type_params {
                        if !fields.iter().any(|f| {
                            let mut mentioned = Vec::new();
                            collect_type_names(&f.ty, &mut mentioned);
                            mentioned.iter().any(|n| n == &param.name)
                        }) {
                            self.error_code(
                                param.span,
                                codes::UNUSED_TYPE_PARAMETER,
                                format!(
                                    "`{name}` declares type parameter `{}`, but no field or variant payload uses it",
                                    param.name,
                                ),
                            );
                        }
                        self.check_generic_bounds(param);
                    }
                    self.struct_schemes.insert(
                        name.clone(),
                        StructScheme {
                            type_params: type_params.clone(),
                            fields: fields.clone(),
                            repr_c: repr.is_some(),
                            origin: self.origin(public),
                            span: *span,
                        },
                    );
                    self.record_origin(name, public);
                    return;
                }
                // `repr(C)` is a promise about memory, so its fields are the
                // bytes C will read: they may be spelled with C's type names,
                // and the query functions report the offsets C would.
                let allow_c = repr.is_some();
                let field_tys: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.resolve_or_report_field(&f.ty, f.span, allow_c);
                        // A function value in a struct would be copied into
                        // the aggregate without being counted, and the
                        // aggregate has no way to release it: the copy would
                        // outlive the environment it points at.
                        if matches!(ty.kind, TypeKind::Func(_, _)) {
                            self.error(f.span, "a struct field cannot have a function type yet");
                        }
                        (f.name.clone(), ty)
                    })
                    .collect();
                let field_public: Vec<bool> = fields.iter().map(|f| f.public).collect();
                let defaults: Vec<Option<HirExpr>> = fields
                    .iter()
                    .zip(field_tys.iter())
                    .map(|(f, (_, ty))| match &f.default {
                        Some(default) => self.check_field_default(default, ty, f.span),
                        None => None,
                    })
                    .collect();
                self.structs.insert(
                    name.clone(),
                    StructSig {
                        fields: field_tys,
                        field_public,
                        defaults,
                        repr_c: repr.is_some(),
                        origin: self.origin(public),
                    },
                );
                self.record_origin(name, public);
            }
            Item::Enum {
                name,
                type_params,
                variants,
                repr,
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
                    visibility: Visibility::Private,
                };
                if let Err(e) = self.scope.declare(name.clone(), binding) {
                    self.report(e);
                }
                if !type_params.is_empty() {
                    // A generic enum is a scheme like a generic struct.
                    for param in type_params {
                        if !variants.iter().any(|variant| {
                            variant.fields.iter().any(|field| {
                                let mut mentioned = Vec::new();
                                collect_type_names(field.ty(), &mut mentioned);
                                mentioned.iter().any(|n| n == &param.name)
                            })
                        }) {
                            self.error_code(
                                param.span,
                                codes::UNUSED_TYPE_PARAMETER,
                                format!(
                                    "`{name}` declares type parameter `{}`, but no field or variant payload uses it",
                                    param.name,
                                ),
                            );
                        }
                        self.check_generic_bounds(param);
                    }
                    self.enum_schemes.insert(
                        name.clone(),
                        EnumScheme {
                            type_params: type_params.clone(),
                            variants: variants.clone(),
                            repr_c: repr.is_some(),
                            span: *span,
                        },
                    );
                    self.record_origin(name, public);
                    return;
                }
                let allow_c = repr.is_some();
                let mut payloads = Vec::with_capacity(variants.len());
                for variant in variants {
                    // A C `enum` is a name for an integer, so there is no
                    // payload for C to see and no offset for `offsetof` to
                    // report. Refused here rather than in the parser because
                    // the payload's own types still have to be checked.
                    if repr.is_some() && !variant.fields.is_empty() {
                        self.error_code(
                            variant.span,
                            codes::REPR_NOT_ALLOWED,
                            format!(
                                "`repr(C)` needs a fieldless `enum`: variant `{}` of `{name}` has a payload",
                                variant.name
                            ),
                        );
                    }
                    let mut field_tys = Vec::with_capacity(variant.fields.len());
                    for field in &variant.fields {
                        let ty = self.resolve_or_report_field(field.ty(), field.span(), allow_c);
                        // The same reason a struct field cannot have one:
                        // an enum value is an aggregate, and an aggregate
                        // does not count the references it holds.
                        if matches!(ty.kind, TypeKind::Func(_, _)) {
                            self.error(
                                field.span(),
                                "an enum field cannot have a function type yet",
                            );
                        }
                        field_tys.push(ty.kind);
                    }
                    payloads.push(field_tys);
                }
                self.enums.insert(
                    name.clone(),
                    EnumSig {
                        variants: variants.clone(),
                        payloads,
                        repr_c: repr.is_some(),
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
                        param_names: decl.params.iter().map(|p| p.name.clone()).collect(),
                        param_defaults: decl.params.iter().map(|p| p.default.clone()).collect(),
                        ret: ret_ty.clone(),
                        is_async: false,
                    };
                    let binding = Binding {
                        ty: Type {
                            kind: TypeKind::Func(param_tys, Box::new(ret_ty)),
                            span: decl.span,
                        },
                        mutability: Mutability::Const,
                        span: decl.span,
                        ownership: Ownership::Live,
                        visibility: Visibility::Private,
                    };
                    if let Err(e) = self.scope.declare(decl.name.clone(), binding) {
                        self.report(e);
                    }
                    self.fns.insert(decl.name.clone(), sig.clone());
                    self.externs.insert(decl.name.clone());
                    if decl.variadic {
                        self.variadic_externs.insert(decl.name.clone());
                    }
                    self.check_extern_symbol(decl, &sig);
                }
            }
            Item::Import { .. } => {}
            Item::Trait {
                name,
                type_params,
                methods,
                span,
            } => {
                if !type_params.is_empty() && self.module_name(self.file) != "iter" {
                    self.error_code(
                        *span,
                        codes::GENERIC_TRAIT,
                        format!(
                            "`{name}` is a generic trait: type parameters on traits are reserved \
                             for the standard library's iterator protocol (`iter.ya`)"
                        ),
                    );
                }
                if self.traits.contains_key(name) {
                    self.error(*span, format!("duplicate trait `{name}`"));
                    return;
                }
                // The methods are resolved where the trait is declared, with
                // `Self` the placeholder a trait has no implementing type for
                // yet, so errors about a signature point at the trait's own
                // file rather than at some later impl's.
                let placeholder = Type {
                    kind: TypeKind::Struct("Self".to_string()),
                    span: *span,
                };
                let outer_self_type = self.self_type.replace(placeholder.clone());
                // A generic trait's own parameters are in scope while its
                // methods resolve: `T` in `Option[T]` stands for the element
                // type, the same way `Self` stands for the implementing type.
                let outer_subst = std::mem::replace(
                    &mut self.type_subst,
                    type_params
                        .iter()
                        .map(|param| {
                            (
                                param.name.clone(),
                                Type {
                                    kind: TypeKind::Struct(param.name.clone()),
                                    span: *span,
                                },
                            )
                        })
                        .collect(),
                );
                let mut sigs = Vec::with_capacity(methods.len());
                for method in methods {
                    if method.is_async {
                        self.error_code(
                            method.span,
                            codes::ASYNC_TRAIT,
                            format!(
                                "`{}` is an async method of trait `{name}`: traits have no async methods; declare a synchronous method instead",
                                method.name
                            ),
                        );
                    }
                    let sig = self.method_signature(
                        name,
                        &method.name,
                        &method.params,
                        &method.ret,
                        &placeholder,
                        false, // a trait method takes the trait's visibility
                        method.span,
                        false, // async trait methods were rejected above
                    );
                    if sigs
                        .iter()
                        .any(|existing: &MethodSig| existing.name == sig.name)
                    {
                        self.error(
                            method.span,
                            format!(
                                "trait `{name}` declares a method named `{}` twice",
                                sig.name
                            ),
                        );
                        continue;
                    }
                    sigs.push(sig);
                }
                self.self_type = outer_self_type;
                self.type_subst = outer_subst;
                self.traits.insert(
                    name.clone(),
                    TraitSig {
                        methods: sigs,
                        type_params: type_params.iter().map(|p| p.name.clone()).collect(),
                        declared: yashy_common::DeclaredEffect::Inferred,
                    },
                );
                self.record_origin(name, public);
            }
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
                    Some(annotation) => self.resolve_or_report_return(annotation, *span),
                    None => Type {
                        kind: TypeKind::Unit,
                        span: *span,
                    },
                };
                // A `const fn` has no run-time existence, so its name is
                // not in the root scope: there is no function of that
                // name for a call to reach.
                if self.const_fns.contains_key(name) {
                    self.error(*span, format!("duplicate definition of `{name}`"));
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
            Item::Macro { name, span, .. } => {
                // A macro is declared so its name is reserved in the module:
                // expansion needs the declaration to see it, and the checker
                // must never see a call to it.
                let _binding = name;
                let _ = span;
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
        self.intern_generics_in(ty);
        match self.resolve_type_in(ty, allow_c) {
            Ok(resolved) => {
                if is_return || !self.refuse_void_value(ty, &resolved) {
                    resolved
                } else {
                    never(fallback_span)
                }
            }
            Err(e) => {
                self.report(e);
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
    ///
    /// A `void` value is refused: it is only legal as a function's return
    /// type, and a parameter, field, binding or element type is a value
    /// position. Use [`CheckCtx::resolve_or_report_return`] for a return
    /// annotation.
    fn resolve_or_report(&mut self, ty: &TypeExpr, fallback_span: Span) -> Type {
        self.intern_generics_in(ty);
        match self.resolve_type(ty) {
            Ok(resolved) => {
                // A named type is an item too, so it is private to the module
                // that declares it unless it is `pub`.
                if let TypeExpr::Name(name, span) = ty {
                    self.check_visibility(name, *span);
                }
                if self.refuse_void_value(ty, &resolved) {
                    never(fallback_span)
                } else {
                    resolved
                }
            }
            Err(e) => {
                self.report(e);
                never(fallback_span)
            }
        }
    }

    /// Resolve a field's type, allowing the C spellings under `repr(C)`.
    ///
    /// A `repr(C)` struct is a promise about bytes, so its fields are the bytes
    /// C reads and may be named with C's words: `a: u8, b: i64`. Everything
    /// else -- the visibility rule, the refusal of a `void` value -- is the
    /// same as [`CheckCtx::resolve_or_report`], which is what this delegates to
    /// when the type is an ordinary Yashy type.
    fn resolve_or_report_field(
        &mut self,
        ty: &TypeExpr,
        fallback_span: Span,
        allow_c: bool,
    ) -> Type {
        self.intern_generics_in(ty);
        if !allow_c {
            return self.resolve_or_report(ty, fallback_span);
        }
        match self.resolve_type_in(ty, true) {
            Ok(resolved) => {
                if let TypeExpr::Name(name, span) = ty {
                    self.check_visibility(name, *span);
                }
                if self.refuse_void_value(ty, &resolved) {
                    never(fallback_span)
                } else {
                    resolved
                }
            }
            Err(e) => {
                self.report(e);
                never(fallback_span)
            }
        }
    }

    /// Report `resolved` when it is a `void` value, and say whether it did.
    ///
    /// `void` is legal only as a function's return type, so every value
    /// position -- a parameter, field, binding, element or payload -- refuses
    /// it with the same message.
    fn refuse_void_value(&mut self, ty: &TypeExpr, resolved: &Type) -> bool {
        if is_void_value(&resolved.kind) {
            self.error(
                type_expr_span(ty),
                "`void` is only allowed as a function's return type".to_string(),
            );
            true
        } else {
            false
        }
    }

    /// Resolve a function's return annotation.
    ///
    /// The return position is the one place `void` is legal, so this is
    /// [`CheckCtx::resolve_or_report`] without the void refusal.
    fn resolve_or_report_return(&mut self, ty: &TypeExpr, fallback_span: Span) -> Type {
        self.intern_generics_in(ty);
        match self.resolve_type(ty) {
            Ok(resolved) => {
                if let TypeExpr::Name(name, span) = ty {
                    self.check_visibility(name, *span);
                }
                resolved
            }
            Err(e) => {
                self.report(e);
                never(fallback_span)
            }
        }
    }

    /// Resolve a type expression to a concrete type. `Option[T]` and
    /// `Result[T, E]` are resolved structurally, because their shape is
    /// self-describing and they need no scope entry.
    ///
    /// C-only spellings are rejected here: ordinary Yashy code writes `int`,
    /// never `i32`. An `extern` declaration resolves through
    /// [`CheckCtx::resolve_type_in`] with `allow_c` set.
    fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, TypeError> {
        self.resolve_type_in(ty, false)
    }

    /// The monomorph kind and type parameters for `name[args..]`, checking
    /// scheme and arity and resolving the arguments.
    ///
    /// Pure: interning, bounds and field resolution happen in
    /// [`Self::intern_generics_in`], which the reporting wrappers run before
    /// delegating here, so a shape error surfaces exactly once.
    fn generic_kind(
        &self,
        name: &str,
        args: &[TypeExpr],
        span: Span,
    ) -> Result<(TypeKind, Vec<TypeParam>), TypeError> {
        let (params, is_enum) = if let Some(scheme) = self.struct_schemes.get(name) {
            (scheme.type_params.clone(), false)
        } else if let Some(scheme) = self.enum_schemes.get(name) {
            (scheme.type_params.clone(), true)
        } else if self.structs.contains_key(name) || self.enums.contains_key(name) {
            return Err(TypeError::new(
                span,
                format!("`{name}` is not generic: it takes no type arguments"),
            )
            .with_code(codes::WRONG_TYPE_ARG_COUNT));
        } else {
            return Err(TypeError::new(span, format!("cannot find type `{name}`"))
                .with_code(codes::UNKNOWN_TYPE));
        };
        if args.len() != params.len() {
            return Err(TypeError::new(
                span,
                format!(
                    "`{name}` declares {} type parameter{}, given {}",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len(),
                ),
            )
            .with_code(codes::WRONG_TYPE_ARG_COUNT));
        }
        let mut arg_tys = Vec::with_capacity(args.len());
        for arg in args {
            arg_tys.push(self.resolve_type_in(arg, false)?);
        }
        let mangled = mangled_generic_name(name, &arg_tys);
        let kind = if is_enum {
            TypeKind::Enum(mangled)
        } else {
            TypeKind::Struct(mangled)
        };
        Ok((kind, params))
    }

    /// Resolve a type expression, optionally allowing the C spellings.
    ///
    /// `allow_c` is true inside an `extern` declaration and in an `as` target,
    /// where the user is naming a C type for an explicit conversion. It changes two
    /// things: the FFI-only names (`i32`, `usize`, `void`, ...) resolve instead
    /// of being rejected, and `*T` / `*mut T` / `*const T` all resolve to
    /// [`TypeKind::CPtr`] instead of an owning Yashy pointer.
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
                             declaration; ordinary Yashy code uses `int`, `float`, `bool`, \
                             or `string`"
                        ),
                    ));
                }
                // `i64` and `f64` are the old spellings. The FFI edge still
                // needs them (`int64_t` is `i64` in a C signature), so they
                // are rejected only where the program is speaking Yashy, and
                // the message names the word to write instead.
                if !allow_c {
                    let replacement = match name.as_str() {
                        "i64" => Some("int"),
                        "f64" => Some("float"),
                        _ => None,
                    };
                    if let Some(replacement) = replacement {
                        return Err(TypeError::new(
                            *span,
                            format!("`{name}` is spelled `{replacement}` in Yashy"),
                        ));
                    }
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
                    "int" => Some(TypeKind::I64),
                    "float" => Some(TypeKind::F64),
                    "bool" => Some(TypeKind::Bool),
                    "char" => Some(TypeKind::Char),
                    "string" => Some(TypeKind::Str),
                    // A return position spells the no-value type `void`. In an
                    // `extern` declaration the same word is the C `void` and
                    // resolves through `from_c_spelling` below.
                    "void" if !allow_c => Some(TypeKind::Unit),
                    "Mutex" => Some(TypeKind::Mutex),
                    "Channel" => {
                        return Err(TypeError::new(
                            *span,
                            "`Channel` takes an element type, as in `Channel[int]`".to_string(),
                        ))
                    }
                    "Task" => {
                        return Err(TypeError::new(
                            *span,
                            "`Task` takes a result type, as in `Task[int]`".to_string(),
                        ))
                    }
                    _ => {
                        // A generic type used bare is an arity error, not an
                        // unknown type: the name is known, the count is wrong.
                        let generic_size = self
                            .struct_schemes
                            .get(name)
                            .map(|scheme| scheme.type_params.len())
                            .or_else(|| {
                                self.enum_schemes
                                    .get(name)
                                    .map(|scheme| scheme.type_params.len())
                            });
                        if let Some(declared) = generic_size {
                            return Err(TypeError::new(
                                *span,
                                format!(
                                    "`{name}` declares {declared} type parameter{}, given 0",
                                    if declared == 1 { "" } else { "s" },
                                ),
                            )
                            .with_code(codes::WRONG_TYPE_ARG_COUNT));
                        }
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
                    None => {
                        let mut error = TypeError::new(*span, format!("cannot find type `{name}`"))
                            .with_code(codes::UNKNOWN_TYPE);
                        if let Some(suggestion) = self.type_suggestion(name) {
                            error = error.with_label(format!("did you mean `{suggestion}`?"));
                        }
                        Err(error)
                    }
                }
            }
            TypeExpr::Generic { name, args, span } => {
                // `Task[T]`: the opaque handle an `async fn` call returns.
                // It is a built-in like `Option[T]`, not a declared generic
                // type, so it resolves here before the scheme lookup.
                if name == "Task" {
                    if args.len() != 1 {
                        return Err(TypeError::new(
                            *span,
                            format!("`Task` takes one result type, given {}", args.len()),
                        )
                        .with_code(codes::WRONG_TYPE_ARG_COUNT));
                    }
                    let inner = self.resolve_type_in(&args[0], allow_c)?;
                    return Ok(Type {
                        kind: TypeKind::Task(Box::new(inner)),
                        span: *span,
                    });
                }
                let (kind, _params) = self.generic_kind(name, args, *span)?;
                Ok(Type { kind, span: *span })
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
                        format!("a channel cannot carry `{element}`: its slots would have no size"),
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
                         an `extern` declaration; ordinary Yashy code writes `*mut T`"
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
                // borrows storage in the current frame, and Yashy has no
                // lifetime syntax to say which storage that is, so a written
                // `[T]` could not be checked: a parameter or a return type
                // would let the view outlive its source. The type exists
                // internally and in messages; it is not a spelling.
                Err(TypeError::new(
                    *span,
                    "a view cannot be named: bind it with `let` and use it in place".to_string(),
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
                        format!("an array cannot hold `{inner}`: its elements would have no size"),
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
                            "a map key must be `int`, `bool`, or `string`, found `{key}`: \
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
            TypeExpr::Unit(span) => Err(TypeError::new(
                *span,
                "`()` is spelled `void` in Yashy".to_string(),
            )),
        }
    }

    /// Check one function body against concrete parameter and return types.
    ///
    /// Shared by ordinary functions and by each instantiation of a generic
    /// one: the two differ only in where the types come from. `ret_ty` is the
    /// *declared* return type `T`: for an `async fn` the body returns `T`
    /// while the call answers `Task[T]`, and the wrapping is applied when the
    /// `HirFn` is built, so `return`, `?` and the `missing return` check all
    /// read `T` here.
    fn check_fn_body(
        &mut self,
        name: &str,
        params: &[Param],
        param_tys: &[Type],
        ret_ty: &Type,
        requires: &[Contract],
        ensures: &[Contract],
        body: &Block,
        span: Span,
        declared: yashy_common::DeclaredEffect,
    ) -> Option<HirFn> {
        let is_async = declared == yashy_common::DeclaredEffect::Async;
        self.async_ctx.push(is_async);
        let out = self.check_fn_body_inner(
            name, params, param_tys, ret_ty, requires, ensures, body, span, declared,
        );
        self.async_ctx.pop();
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn check_fn_body_inner(
        &mut self,
        name: &str,
        params: &[Param],
        param_tys: &[Type],
        ret_ty: &Type,
        requires: &[Contract],
        ensures: &[Contract],
        body: &Block,
        span: Span,
        declared: yashy_common::DeclaredEffect,
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
                    visibility: Visibility::Private,
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
        // A precondition is checked with the parameters in scope and no
        // `result`: there is no return value yet. A postcondition sees
        // `result`, in a scope of its own so the body cannot name it.
        let requires = self.check_contracts(requires);
        let checked = self.check_block_with_ret(body, ret_ty);
        let ensures = {
            self.scope.push();
            let _ = self.scope.declare(
                "result".to_string(),
                Binding {
                    ty: ret_ty.clone(),
                    mutability: Mutability::Const,
                    span,
                    ownership: Ownership::Live,
                    visibility: Visibility::Private,
                },
            );
            let checked = self.check_contracts(ensures);
            self.scope.pop();
            checked
        };
        self.borrowed_bindings = outer_borrows;
        self.fn_ret = outer_ret;
        self.scope.pop();
        let body = checked?;
        let requires = requires?;
        let ensures = ensures?;
        // A function that returns a value has to return one on every path out
        // of its body.
        if !matches!(ret_ty.kind, TypeKind::Unit | TypeKind::Never) && !block_always_returns(&body)
        {
            self.error(span, "missing return");
        }
        let is_async = declared == yashy_common::DeclaredEffect::Async;
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
            ret: if is_async {
                Self::wrap_async_ret(ret_ty.clone(), span)
            } else {
                ret_ty.clone()
            },
            body,
            requires,
            ensures,
            captures: Vec::new(),
            effect: yashy_common::Effect::PURE,
            declared,
            is_async,
            docs: Vec::new(),
            file: self.file,
            span,
        })
    }

    /// Check a list of contract clauses as `bool`, keeping each expression for
    /// the verifier.
    ///
    /// A clause that is not a `bool` is an ordinary type mismatch: the language
    /// has one error for "this expression has the wrong type", and a contract
    /// is not a second kind of expression.
    fn check_contracts(&mut self, clauses: &[Contract]) -> Option<Vec<HirContract>> {
        let mut checked = Vec::with_capacity(clauses.len());
        for clause in clauses {
            checked.push(self.check_contract(clause)?);
        }
        Some(checked)
    }

    /// Check one contract clause as `bool`.
    fn check_contract(&mut self, clause: &Contract) -> Option<HirContract> {
        let expr = self.check_expr(&clause.expr)?;
        if !expr_ty(&expr).eq_ignoring_span(&prim(TypeKind::Bool)) {
            self.mismatch(expr_span(&expr), &prim(TypeKind::Bool), &expr_ty(&expr));
        }
        Some(HirContract {
            expr,
            span: clause.span,
        })
    }

    /// Check a call to a generic function and record its instantiation.
    ///
    /// The arguments are checked first, without an expected type: the expected
    /// type is what the call is trying to discover. Each argument's type is
    /// then unified with the parameter's declared type, which binds the type
    /// parameters; a parameter that stays unbound is an error at the call.
    fn check_generic_call(&mut self, name: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        let Some(generic) = self.generics.get(name).cloned() else {
            return None;
        };
        if generic.params.len() != args.len() {
            self.error_code(
                span,
                codes::WRONG_ARGUMENT_COUNT,
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
            // An inferred lambda parameter cannot be typed here: the type
            // parameters are what the arguments are still fixing. Name the
            // declared signature it could not resolve against.
            if let Expr::Lambda {
                params,
                span: lambda_span,
                ..
            } = arg
            {
                if params.iter().any(|param| param.ty.is_none()) {
                    self.error_code(
                        *lambda_span,
                        codes::TYPE_MISMATCH,
                        format!(
                            "cannot infer a lambda parameter for `{}`: write its type",
                            type_expr_name(&param.ty)
                        ),
                    );
                    return None;
                }
            }
            let checked = self.check_expr(arg)?;
            let actual = expr_ty(&checked);
            if !self.unify_type(&param.ty, &generic.type_params, &mut subst, &actual) {
                self.mismatch_message(
                    expr_span(&checked),
                    format!("expected `{}`, found `{actual}`", type_expr_name(&param.ty)),
                );
            }
            checked_args.push(checked);
        }
        // Every type parameter has to be pinned down by an argument. One that
        // appears only in the return type, or nowhere at all, has no answer.
        let mut ordered = Vec::with_capacity(generic.type_params.len());
        for param in &generic.type_params {
            match subst.get(&param.name) {
                Some(concrete) => ordered.push(concrete.clone()),
                None => {
                    self.error(
                        span,
                        format!("cannot infer type parameter `{}`", param.name),
                    );
                    return None;
                }
            }
        }
        // Bounds are checked here, at the call site, never inside the body:
        // the argument type must implement each bound, and a miss is `E1503`
        // naming the type, the trait and where the impl belongs.
        self.check_type_args_bounds(&generic.type_params, &ordered, span, name);
        // Resolve the signature at the inferred substitution. The map is
        // installed only for the duration of the lookup: nothing outside a
        // generic body should see `T`.
        let outer_subst = std::mem::take(&mut self.type_subst);
        self.type_subst = generic
            .type_params
            .iter()
            .zip(ordered.iter())
            .map(|(param, concrete)| (param.name.clone(), concrete.clone()))
            .collect();
        let param_tys: Vec<Type> = generic
            .params
            .iter()
            .map(|param| self.resolve_or_report(&param.ty, param.span))
            .collect();
        let ret_inner = match &generic.ret {
            Some(ty) => self.resolve_or_report_return(ty, span),
            None => prim(TypeKind::Unit),
        };
        let ret_ty = if generic.is_async {
            Self::wrap_async_ret(ret_inner, span)
        } else {
            ret_inner
        };
        self.type_subst = outer_subst;
        // A parameter of linear type takes ownership, exactly as it does in an
        // ordinary call: passing a bare name moves it out. A field access is a
        // header the struct does not own, so it is not a move and its last
        // segment is not a binding to look up.
        for (arg, param_ty) in args.iter().zip(param_tys.iter()) {
            if matches!(linearity_of(&param_ty.kind), Linearity::Linear) {
                if let Expr::Ident(path, arg_span) = arg {
                    if path.len() == 1 {
                        let arg_name = path.last().cloned().unwrap_or_default();
                        if let Err(e) = self.move_out(&arg_name, *arg_span) {
                            self.report(e);
                        }
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
        params: &[TypeParam],
        subst: &mut HashMap<String, Type>,
        actual: &Type,
    ) -> bool {
        match declared {
            TypeExpr::Name(name, _) if params.iter().any(|type_param| type_param.name == *name) => {
                match subst.get(name) {
                    Some(bound) => bound.eq_ignoring_span(actual),
                    None => {
                        subst.insert(name.clone(), actual.clone());
                        true
                    }
                }
            }
            // A generic use unifies against a monomorph by its recorded
            // arguments: `Pair[T]` against `Pair[i64]` binds `T` to `i64`.
            // Anything else -- a different scheme, a concrete type -- does
            // not unify.
            TypeExpr::Generic { name, args, .. } => match &actual.kind {
                TypeKind::Struct(mangled) | TypeKind::Enum(mangled) => {
                    match (
                        self.monomorph_schemes.get(mangled),
                        self.type_monomorphs.get(mangled),
                    ) {
                        (Some(scheme), Some(arg_tys))
                            if scheme == name && args.len() == arg_tys.len() =>
                        {
                            args.iter().zip(arg_tys.iter()).all(|(declared, concrete)| {
                                self.unify_type(declared, params, subst, concrete)
                            })
                        }
                        _ => false,
                    }
                }
                _ => false,
            },
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
            TypeExpr::Fn {
                params: declared_params,
                ret,
                ..
            } => match &actual.kind {
                TypeKind::Func(actual_params, actual_ret) => {
                    declared_params.len() == actual_params.len()
                        && declared_params.iter().zip(actual_params.iter()).all(
                            |(declared, actual)| self.unify_type(declared, params, subst, actual),
                        )
                        && self.unify_type(ret, params, subst, actual_ret)
                }
                _ => false,
            },
            TypeExpr::Unit(_) => matches!(actual.kind, TypeKind::Unit),
            TypeExpr::Tuple(items, _) => match &actual.kind {
                TypeKind::Tuple(elements) => {
                    items.len() == elements.len()
                        && items.iter().zip(elements.iter()).all(|(declared, actual)| {
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
        // The budget, checked on insert so the diagnostic fires at the call
        // site that exceeded it.
        let count = self.fn_instantiations.get(name).copied().unwrap_or(0) + 1;
        if count > MAX_INSTANTIATIONS_PER_FN {
            let first = self.fn_instantiation_site.get(name).copied();
            let mut error = TypeError::new(
                span,
                format!(
                    "`{name}` was instantiated {count} times: the budget is \
                     {MAX_INSTANTIATIONS_PER_FN} instantiations per function"
                ),
            )
            .with_code(codes::INSTANTIATION_BUDGET);
            if let Some(first) = first {
                let a = self.sm.locate(self.file, first);
                let b = self.sm.locate(self.file, span);
                error = error.with_label(format!(
                    "the first instantiation was at {}:{}, this one at {}:{}",
                    a.line, a.col, b.line, b.col,
                ));
            }
            self.errors.push((self.file, error));
            return mangled;
        }
        if self.instantiated.len() + 1 > MAX_INSTANTIATIONS_TOTAL {
            self.error_code(
                span,
                codes::INSTANTIATION_BUDGET,
                format!(
                    "the program was instantiated {} times: the budget is \
                     {MAX_INSTANTIATIONS_TOTAL} instantiations in total",
                    self.instantiated.len() + 1,
                ),
            );
            return mangled;
        }
        self.fn_instantiations.insert(name.to_string(), count);
        self.fn_instantiation_site.insert(name.to_string(), span);
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
                .map(|(param, concrete)| (param.name.clone(), concrete.clone()))
                .collect();
            // The parameter bindings whose declared type is a type parameter
            // resolve through the bounds in force, so method calls on them
            // may rely on nothing beyond those bounds.
            let bound_context = generic
                .params
                .iter()
                .filter_map(|param| {
                    let TypeExpr::Name(tp_name, _) = &param.ty else {
                        return None;
                    };
                    let index = generic
                        .type_params
                        .iter()
                        .position(|p| &p.name == tp_name)?;
                    Some(BoundBinding {
                        name: param.name.clone(),
                        param: generic.type_params[index].name.clone(),
                        bounds: generic.type_params[index].bounds.clone(),
                    })
                })
                .collect();
            let outer_bound_context = std::mem::replace(&mut self.bound_context, bound_context);
            let param_tys: Vec<Type> = generic
                .params
                .iter()
                .map(|param| self.resolve_or_report(&param.ty, param.span))
                .collect();
            let ret_ty = match &generic.ret {
                Some(ty) => self.resolve_or_report_return(ty, job.span),
                None => prim(TypeKind::Unit),
            };
            // An async generic answers `Task[T]` at the call (`check_generic_call`
            // wraps), while its body returns `T`: the inner type is what the
            // body is checked against.
            let declared = if generic.is_async {
                yashy_common::DeclaredEffect::Async
            } else {
                yashy_common::DeclaredEffect::Inferred
            };
            let mangled = mangled_generic_name(&job.name, &job.subst);
            self.active_generics
                .push((job.name.clone(), job.subst.clone()));
            let checked = self.check_fn_body(
                &mangled,
                &generic.params,
                &param_tys,
                &ret_ty,
                &generic.requires,
                &generic.ensures,
                &generic.body,
                generic.span,
                declared,
            );
            self.active_generics.pop();
            self.type_subst.clear();
            self.bound_context = outer_bound_context;
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
    ///
    /// `declared` is the effect wrapper in front of the item: an `async fn`
    /// body is checked in an async context and its call answers `Task[T]`,
    /// so the body checker has to know before it reads the body rather than
    /// having the answer stamped on afterwards.
    fn check_item(&mut self, item: &Item) -> Option<Vec<HirItem>> {
        self.check_item_with_declared(item, yashy_common::DeclaredEffect::Inferred, &[])
    }

    fn check_item_with_declared(
        &mut self,
        item: &Item,
        declared: yashy_common::DeclaredEffect,
        docs: &[String],
    ) -> Option<Vec<HirItem>> {
        match item {
            // `pub` is a modifier in front of an item: the item inside is
            // checked as usual. The visibility was settled in the declaration
            // pass, where the item's name was recorded.
            Item::Public { item, .. } => self.check_item_with_declared(item, declared, docs),
            // A doc block is stored, not checked: the item inside checks as
            // usual, and `yashy doc` reads the text later.
            Item::Documented {
                docs: text, item, ..
            } => self.check_item_with_declared(item, declared, text),
            // As in the declaration pass: the filter has already dropped a
            // disabled item, so this only looks through a wrapper a caller
            // that skipped the filter left in place.
            Item::Cfg { item, .. } => self.check_item_with_declared(item, declared, docs),
            Item::Effect { effect, item, .. } => {
                // Check the inner item with the wrapper's effect in hand, so
                // an `async fn` is checked as async from the first statement.
                // (The wrapper may itself be nested; the outer one wins.)
                let deep = if declared == yashy_common::DeclaredEffect::Inferred {
                    *effect
                } else {
                    declared
                };
                self.check_item_with_declared(item, deep, docs)
            }
            Item::Fn {
                name,
                type_params,
                params,
                ret: _,
                requires,
                ensures,
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
                let body_ret = Self::unwrap_async_ret(&sig.ret);
                let checked = self.check_fn_body(
                    name,
                    params,
                    &sig.params,
                    &body_ret,
                    requires,
                    ensures,
                    body,
                    *span,
                    declared,
                )?;
                let mut checked = checked;
                checked.docs = docs.to_vec();
                Some(vec![HirItem::Fn(checked)])
            }
            Item::Struct {
                name,
                type_params,
                fields,
                repr,
                span,
            } => {
                // A generic struct is a scheme: it emits nothing, and its
                // monomorphs are emitted where the instantiations are
                // collected at the end of the check.
                if !type_params.is_empty() {
                    return None;
                }
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
                    repr: *repr,
                    span: *span,
                })])
            }
            Item::Enum {
                name,
                type_params,
                variants,
                repr,
                span,
            } => {
                // A generic enum is a scheme like a generic struct.
                if !type_params.is_empty() {
                    return None;
                }
                // The payload types were resolved in pass 1 and kept in the
                // signature, so the lowering reads them rather than resolving
                // a second time: a second pass would report the same unknown
                // name twice.
                let sig = self.enums.get(name).cloned()?;
                Some(vec![HirItem::Enum(HirEnum {
                    name: name.clone(),
                    variants: variants
                        .iter()
                        .zip(sig.payloads.iter())
                        .map(|(variant, payload)| HirVariantDef {
                            name: variant.name.clone(),
                            fields: variant
                                .fields
                                .iter()
                                .zip(payload.iter())
                                .map(|(field, ty)| HirVariantField {
                                    name: match field {
                                        VariantField::Named { name, .. } => Some(name.clone()),
                                        VariantField::Positional { .. } => None,
                                    },
                                    ty: Type {
                                        kind: ty.clone(),
                                        span: field.span(),
                                    },
                                    span: field.span(),
                                })
                                .collect(),
                            span: variant.span,
                        })
                        .collect(),
                    repr: *repr,
                    span: *span,
                })])
            }
            Item::Import { path, alias, span } => Some(vec![HirItem::Import(HirImport {
                path: path.clone(),
                alias: alias.clone(),
                span: *span,
                file: self.file,
            })]),
            Item::Trait { .. } => {
                // A trait is a compile-time requirement set: it has no layout,
                // no value and no symbol, so nothing is emitted for it. Its
                // impls are lowered through their own `Item::Impl` blocks.
                None
            }
            Item::Macro { .. } => {
                // A macro is expanded before checking; if one reaches the
                // checker the expander never ran, and there is no HIR for it.
                None
            }
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
                        symbol: decl.symbol.clone(),
                        variadic: decl.variadic,
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
                            Some(ty) => self
                                .resolve_type_in(ty, true)
                                .unwrap_or_else(|_| never(decl.span)),
                            None => prim(TypeKind::Unit),
                        },
                        span: decl.span,
                    })
                    .collect(),
                file: self.file,
                span: *span,
            })]),
            Item::Impl {
                type_name,
                trait_name,
                methods,
                span: _,
            } => self.check_impl_methods(type_name, trait_name.as_deref(), methods),
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
                repr,
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
                    repr: *repr,
                    span: *span,
                })];
                if let Some(methods) = self.check_impl_methods(name, None, methods) {
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
    /// is what the call site resolves to. A trait impl emits
    /// `Type.Trait.method` instead, so the trait method a call resolves to is
    /// one concrete `HirFn` and the ownership and effect passes see that impl
    /// and nothing else.
    fn check_impl_methods(
        &mut self,
        type_name: &str,
        trait_name: Option<&str>,
        methods: &[Item],
    ) -> Option<Vec<HirItem>> {
        let Some(receiver_ty) = self.declared_type(type_name) else {
            return None;
        };
        let mut items = Vec::new();
        for method in methods {
            // The method's own effect wrapper (`pure fn` in the impl), else
            // the trait's (`pure trait`), else inferred.
            let method_declared = {
                let mut current = method;
                let mut found = yashy_common::DeclaredEffect::Inferred;
                loop {
                    match current {
                        Item::Effect { effect, item, .. } => {
                            found = *effect;
                            current = item;
                        }
                        Item::Public { item, .. } | Item::Documented { item, .. } => current = item,
                        _ => break,
                    }
                }
                if found == yashy_common::DeclaredEffect::Inferred {
                    if let Some(t) = trait_name {
                        if let Some(sig) = self.traits.get(t) {
                            found = sig.declared;
                        }
                    }
                }
                found
            };
            let Item::Fn {
                name,
                type_params,
                params,
                ret,
                requires,
                ensures,
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
            // A method's contract would have to be checked at every call site
            // through the receiver, which the contract pass does not do yet.
            // Refusing it here is the honest answer: silently dropping a clause
            // the user wrote is worse than saying it is not supported.
            if !requires.is_empty() || !ensures.is_empty() {
                self.error(
                    *span,
                    "a method cannot declare a contract yet: write it on a free function",
                );
            }
            let sig = self.method_signature(
                type_name,
                name,
                params,
                ret,
                &receiver_ty,
                Self::is_public_item(method),
                *span,
                Self::item_is_async(method),
            );
            // An async method's body returns `T` (`sig.ret` is `Task[T]`):
            // check against the inner type, wrap at the `HirFn`.
            let is_async = sig.is_async;
            let body_declared = if is_async {
                yashy_common::DeclaredEffect::Async
            } else {
                method_declared
            };
            let body_ret = Self::unwrap_async_ret(&sig.ret);
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
                        visibility: Visibility::Private,
                    },
                );
            }
            let outer_self_type = self.self_type.replace(receiver_ty.clone());
            let outer_ret = self.fn_ret.replace(body_ret.clone());
            self.async_ctx.push(is_async);
            let checked = self.check_block_with_ret(body, &body_ret);
            self.async_ctx.pop();
            self.fn_ret = outer_ret;
            self.self_type = outer_self_type;
            self.scope.pop();
            let body = checked?;
            // A method that returns a value has to return one on every path.
            if !matches!(body_ret.kind, TypeKind::Unit | TypeKind::Never)
                && !block_always_returns(&body)
            {
                self.error(*span, "missing return");
            }
            items.push(HirItem::Fn(HirFn {
                name: match trait_name {
                    Some(trait_name) => format!("{type_name}.{trait_name}.{name}"),
                    None => format!("{type_name}.{name}"),
                },
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
                requires: Vec::new(),
                ensures: Vec::new(),
                captures: Vec::new(),
                effect: yashy_common::Effect::PURE,
                // A `pure trait`'s impls are declared `pure`, so an `io`
                // impl body is `E1300` at the impl in the verifier.
                declared: body_declared,
                is_async,
                docs: Vec::new(),
                file: self.file,
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
        let mut seen_statement = false;
        for stmt in &block.stmts {
            // An invariant is the loop header's clause, which the parser puts
            // at the head of the body. One written after a statement would be
            // a promise made too late to hold at the loop's entry, and the
            // verifier reads clauses in order.
            if matches!(stmt, Stmt::Invariant(_)) {
                if seen_statement {
                    self.error(
                        stmt.span(),
                        "an `invariant` may only appear at the head of a loop body",
                    );
                }
            } else {
                seen_statement = true;
            }
            let checked = self.check_stmt(stmt, ret)?;
            let is_loop = matches!(
                stmt,
                Stmt::ForRange { .. }
                    | Stmt::ForIn { .. }
                    | Stmt::ForC { .. }
                    | Stmt::WhileLet { .. }
            );
            stmts.push(checked);
            // A labeled jump inside the loop just checked sets a flag and
            // breaks out of it. Propagate the flag immediately, here in the
            // enclosing body, so statements after the inner loop do not run
            // on the jump's iteration. The body-end checks in `wrap_loop`
            // stay as the backstop for jumps in trailing position.
            if is_loop && !self.labeled_loops.is_empty() {
                let span = stmt.span();
                stmts.extend(self.propagation_checks(span));
            }
        }
        self.scope.pop();
        Some(HirBlock {
            stmts,
            span: block.span,
            arena: false,
        })
    }

    /// Lower `break L` / `continue L` (Task 9): flags, not new HIR.
    ///
    /// A labeled jump is `__brk_L = true; break;` (or `__cont_L`), built from
    /// the same statements a hand-written program would have. The flags are
    /// mutable `let` bindings (`Var`), never `const`.
    fn flag_bool(_flag: &str, value: bool, span: Span) -> HirExpr {
        HirExpr::Literal(Literal::Bool(value), span, prim(TypeKind::Bool))
    }

    fn flag_decl(label: &str, is_break: bool, span: Span) -> HirStmt {
        let flag = if is_break {
            format!("__brk_{label}")
        } else {
            format!("__cont_{label}")
        };
        HirStmt::Var {
            name: flag,
            ty: None,
            init: Self::flag_bool("", false, span),
            mutability: Mutability::Var,
            span,
        }
    }

    /// The flag a labeled jump sets, named by its label.
    fn flag_name(label: &str, is_break: bool) -> String {
        if is_break {
            format!("__brk_{label}")
        } else {
            format!("__cont_{label}")
        }
    }

    /// `flag = true;` -- the write half of a labeled jump.
    fn flag_set(label: &str, is_break: bool, span: Span) -> HirStmt {
        HirStmt::Assign {
            name: Self::flag_name(label, is_break),
            value: Self::flag_bool("", true, span),
            ty: prim(TypeKind::Bool),
            release_previous: false,
            span,
        }
    }

    /// `if flag { break; }` -- an intervening loop's propagation check.
    fn flag_check_break(label: &str, is_break: bool, span: Span) -> HirStmt {
        HirStmt::If {
            init: None,
            cond: HirExpr::Ident(
                vec![Self::flag_name(label, is_break)],
                span,
                prim(TypeKind::Bool),
            ),
            then_block: HirBlock {
                stmts: vec![HirStmt::Break(span)],
                span,
                arena: false,
            },
            else_block: None,
            span,
        }
    }

    /// `if __cont_L { __cont_L = false; continue; }` -- the labeled loop's tail.
    fn labeled_continue_tail(label: &str, span: Span) -> HirStmt {
        let flag = Self::flag_name(label, false);
        HirStmt::If {
            init: None,
            cond: HirExpr::Ident(vec![flag.clone()], span, prim(TypeKind::Bool)),
            then_block: HirBlock {
                stmts: vec![
                    HirStmt::Assign {
                        name: flag,
                        value: Self::flag_bool("", false, span),
                        ty: prim(TypeKind::Bool),
                        release_previous: false,
                        span,
                    },
                    HirStmt::Continue(span),
                ],
                span,
                arena: false,
            },
            else_block: None,
            span,
        }
    }

    /// The checks emitted after a loop statement inside a labeled loop's
    /// body: for every enclosing label, `if __brk_L { break; }` and the
    /// `continue` propagation (`break` for an outer label, reset-and-`continue`
    /// for the label of the loop whose body this is).
    fn propagation_checks(&self, span: Span) -> Vec<HirStmt> {
        let mut checks = Vec::new();
        for label in &self.labeled_loops {
            checks.push(Self::flag_check_break(label, true, span));
            if self.loop_labels.last().and_then(|l| l.as_ref()) == Some(label) {
                checks.push(Self::labeled_continue_tail(label, span));
            } else {
                checks.push(Self::flag_check_break(label, false, span));
            }
        }
        checks
    }

    /// The flag a labeled loop statement is wrapped in: the loop itself
    /// carries the propagation checks, and when the loop is labeled the
    /// two flag bindings live in enclosing `if true` inits. Unlabeled
    /// loops (label `None`) produce just the loop with the outer labels'
    /// checks appended.
    fn wrap_loop(&mut self, label: &Option<String>, loop_stmt: HirStmt, span: Span) -> HirStmt {
        let mut with_checks = match &loop_stmt {
            HirStmt::ForRange { body, .. } => body.stmts.clone(),
            HirStmt::ForIn { body, .. } => body.stmts.clone(),
            HirStmt::ForC { body, .. } => body.stmts.clone(),
            _ => Vec::new(),
        };
        for outer in self.labeled_loops.iter() {
            if Some(outer) == label.as_ref() {
                continue;
            }
            with_checks.push(Self::flag_check_break(outer, true, span));
        }
        for outer in self.labeled_loops.iter() {
            if Some(outer) == label.as_ref() {
                continue;
            }
            with_checks.push(Self::flag_check_break(outer, false, span));
        }
        if let Some(label) = label {
            with_checks.push(Self::flag_check_break(label, true, span));
            with_checks.push(Self::labeled_continue_tail(label, span));
        }
        // Rebuild the loop with the augmented body.
        let loop_stmt = match loop_stmt {
            HirStmt::ForRange {
                name,
                start,
                end,
                body,
                span,
            } => HirStmt::ForRange {
                name,
                start,
                end,
                body: HirBlock {
                    stmts: with_checks,
                    span: body.span,
                    arena: body.arena,
                },
                span,
            },
            HirStmt::ForIn {
                pattern,
                iterable,
                body,
                span,
            } => HirStmt::ForIn {
                pattern,
                iterable,
                body: HirBlock {
                    stmts: with_checks,
                    span: body.span,
                    arena: body.arena,
                },
                span,
            },
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                span,
            } => HirStmt::ForC {
                init,
                cond,
                update,
                body: HirBlock {
                    stmts: with_checks,
                    span: body.span,
                    arena: body.arena,
                },
                span,
            },
            other => other,
        };
        match label {
            None => loop_stmt,
            Some(label) => {
                let inner = HirStmt::If {
                    init: Some(Box::new(Self::flag_decl(label, false, span))),
                    cond: HirExpr::Literal(Literal::Bool(true), span, prim(TypeKind::Bool)),
                    then_block: HirBlock {
                        stmts: vec![loop_stmt],
                        span,
                        arena: false,
                    },
                    else_block: None,
                    span,
                };
                HirStmt::If {
                    init: Some(Box::new(Self::flag_decl(label, true, span))),
                    cond: HirExpr::Literal(Literal::Bool(true), span, prim(TypeKind::Bool)),
                    then_block: HirBlock {
                        stmts: vec![inner],
                        span,
                        arena: false,
                    },
                    else_block: None,
                    span,
                }
            }
        }
    }

    /// Lower a labeled `break L` / `continue L` to
    /// `if (flag = true) { break; }` -- the write and the real break in
    /// one statement, so `check_stmt` keeps its single-statement shape.
    /// Depth 0 (the label is on the innermost loop) is a plain jump.
    fn labeled_jump(&mut self, label: &str, is_break: bool, span: Span) -> Option<HirStmt> {
        let Some(depth) = self
            .loop_labels
            .iter()
            .rev()
            .position(|l| l.as_deref() == Some(label))
        else {
            let in_scope: Vec<String> = self
                .loop_labels
                .iter()
                .flatten()
                .map(|l| format!("`{l}`"))
                .collect();
            let message = if in_scope.is_empty() {
                format!("no enclosing loop is labeled `{label}`")
            } else {
                format!(
                    "no enclosing loop is labeled `{label}`; the labels in scope are: {}",
                    in_scope.join(", ")
                )
            };
            self.error_code(span, codes::UNKNOWN_LABEL, message);
            return None;
        };
        // Depth 0 (the label is on the innermost loop) is a plain jump: a
        // real `break` already leaves that loop, and a real `continue`
        // already re-tests it. The flag form would exit it instead.
        if depth == 0 {
            return Some(if is_break {
                HirStmt::Break(span)
            } else {
                HirStmt::Continue(span)
            });
        }
        let assign = Self::flag_set(label, is_break, span);
        let break_stmt = if is_break {
            HirStmt::Break(span)
        } else {
            HirStmt::Break(span) // continue lowers to a real break too
        };
        Some(HirStmt::If {
            init: Some(Box::new(assign)),
            cond: HirExpr::Literal(Literal::Bool(true), span, prim(TypeKind::Bool)),
            then_block: HirBlock {
                stmts: vec![break_stmt],
                span,
                arena: false,
            },
            else_block: None,
            span,
        })
    }

    /// Push a loop label, reporting `E0605` for a duplicate.
    fn push_loop_label(&mut self, label: &Option<String>, span: Span) {
        if let Some(label) = label {
            if self
                .loop_labels
                .iter()
                .any(|l| l.as_deref() == Some(label.as_str()))
            {
                self.error_code(
                    span,
                    codes::DUPLICATE_LABEL,
                    format!("the label `{label}` is already in scope"),
                );
            }
            self.loop_labels.push(Some(label.clone()));
        } else {
            self.loop_labels.push(None);
        }
    }

    fn pop_loop_label(&mut self) {
        self.loop_labels.pop();
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
        match const_eval::evaluate(init, &[], &self.const_fns, &mut work, Missing::NotConstant) {
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
                match error.code {
                    Some(code) => self.error_code(span, code, error.message),
                    None => self.error(span, error.message),
                }
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
            Err(error) => match error.code {
                Some(code) => self.error_code(error.span, code, error.message),
                None => self.error(error.span, error.message),
            },
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
                if let Some(annotation) = ty {
                    self.intern_generics_in(annotation);
                }
                let declared = match ty {
                    Some(annotation) => match self.resolve_type(annotation) {
                        Ok(declared) => {
                            if self.refuse_void_value(annotation, &declared) {
                                return None;
                            }
                            Some(declared)
                        }
                        Err(error) => {
                            self.report(error);
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
                    // The initializer is a constant expression, so a literal
                    // shift count outside its type's range is a compile-time
                    // fault here. The flag is restored before `?` so a later
                    // expression in the same function is not treated as a
                    // constant initializer.
                    let outer = self.in_const_init;
                    self.in_const_init = true;
                    let checked = match &declared {
                        Some(declared) => self.check_expr_with_expected(init, declared),
                        None => self.check_expr(init),
                    };
                    self.in_const_init = outer;
                    checked?
                };
                if let Some(declared) = &declared {
                    if !declared.eq_ignoring_span(&expr_ty(&init_expr)) {
                        self.mismatch(expr_span(&init_expr), declared, &expr_ty(&init_expr));
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
                        visibility: Visibility::Private,
                    },
                ) {
                    self.report(e);
                }
                Some(HirStmt::Const {
                    name: name.clone(),
                    ty: declared,
                    init: init_expr,
                    mutability: Mutability::Const,
                    span: *span,
                })
            }
            Stmt::Let {
                name,
                ty,
                init,
                span,
            } => {
                // Same ordering as `Stmt::Const` above: the annotation is the
                // initializer's expected type.
                if let Some(annotation) = ty {
                    self.intern_generics_in(annotation);
                }
                let declared = match ty {
                    Some(annotation) => match self.resolve_type(annotation) {
                        Ok(declared) => {
                            if self.refuse_void_value(annotation, &declared) {
                                return None;
                            }
                            Some(declared)
                        }
                        Err(error) => {
                            self.report(error);
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
                        self.mismatch(expr_span(&init_expr), declared, &expr_ty(&init_expr));
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
                        visibility: Visibility::Private,
                    },
                ) {
                    self.report(e);
                }
                Some(HirStmt::Var {
                    name: name.clone(),
                    ty: declared,
                    init: init_expr,
                    mutability: Mutability::Var,
                    span: *span,
                })
            }
            Stmt::LetPattern {
                kind,
                pattern,
                init,
                span,
            } => {
                let init_expr = self.check_expr(init)?;
                let init_ty = expr_ty(&init_expr);
                // A tuple is a copyable value, but the rule is still stated the
                // same way every other binding states it: a linear initializer
                // has to say `move`. Nothing a tuple can currently hold is
                // linear, so this is future-proofing rather than a live check.
                self.forbid_linear_copy(init, &init_expr);
                let mutability = match kind {
                    BindingKeyword::Const => Mutability::Const,
                    BindingKeyword::Let => Mutability::Var,
                };
                let pattern = self.check_binding_pattern(pattern, &init_ty, mutability)?;
                Some(HirStmt::Destructure {
                    init: init_expr,
                    pattern,
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
                // shapes are assignable: a name bound by `let` in a scope that
                // encloses this statement, an index into a mutable `*mut i64`
                // binding (`p[i] = v`), and a field of a `let` binding
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
                            format!(
                                "a pointer to nothing cannot be written through: `*mut {pointee}`"
                            ),
                        );
                        return None;
                    }
                    // The element type is whatever the pointer points at: a
                    // pointer is an address, and an address does not care
                    // whether it holds one `int` or one struct.
                    let element = (**pointee).clone();
                    let index = self.check_expr(index)?;
                    if !expr_ty(&index).eq_ignoring_span(&prim(TypeKind::I64)) {
                        let _ = self.check_expr(value);
                        self.error(expr_span(&index), "index must be `int`");
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
                        let span = yashy_parser::exprs::expr_span(other);
                        self.error(span, "only a `let` binding can be assigned");
                        return None;
                    }
                };

                let binding = match self.lookup(&name, yashy_parser::exprs::expr_span(target)) {
                    NameLookup::Found(binding) => binding,
                    // A capture holds a copy: assigning to it would change the
                    // copy and not the binding it came from.
                    NameLookup::Capture { .. } => {
                        let _ = self.check_expr(value);
                        self.error(
                            yashy_parser::exprs::expr_span(target),
                            format!("cannot assign to `{name}`, which is captured"),
                        );
                        return None;
                    }
                    NameLookup::Missing => {
                        let _ = self.check_expr(value);
                        let span = yashy_parser::exprs::expr_span(target);
                        self.error_missing_value(&name, span);
                        return None;
                    }
                };

                if !binding.mutability.is_mutable() {
                    let span = yashy_parser::exprs::expr_span(target);
                    self.report(
                        TypeError::new(
                            span,
                            format!("cannot assign to `{name}`, which is immutable"),
                        )
                        .with_code(codes::CONST_REASSIGNMENT)
                        .with_note("use `let` for a mutable binding"),
                    );
                }

                // A view points into this binding's storage. Rebinding would
                // free what the view covers and leave the pointer dangling, so
                // the binding stays put until the view is out of scope -- which
                // in Yashy means until the function returns.
                self.reject_borrowed_write(&name, yashy_parser::exprs::expr_span(target));

                let checked = self.check_expr_with_expected(value, &binding.ty)?;
                let found = expr_ty(&checked);
                if !binding.ty.eq_ignoring_span(&found) {
                    self.error(
                        expr_span(&checked),
                        format!("expected `{}`, found `{found}`", binding.ty),
                    );
                }
                // Re-initialising a `let` gives it a fresh owner, so whatever it
                // held before -- live or moved -- it owns something now. The
                // source may not be a bare linear name without `move`, for the
                // same reason as a declaration.
                //
                // Whether the old value still needs releasing is decided here,
                // where the ownership state is still known: `release_previous`
                // is true only when the slot holds a live allocation. A binding
                // that was already moved (by `buf.free()`, or by passing it to
                // a function that takes `*mut i64`) is not released again.
                // Re-read the binding's ownership *after* the value was
                // checked: `set = set_insert(set, 3)` moves `set` into the
                // call, so the slot no longer owns what it holds and the
                // backend must not release it again. `binding` above is the
                // snapshot from before the value was checked, which still says
                // `Live`.
                let still_live = self
                    .scope
                    .resolve(&name)
                    .is_some_and(|current| matches!(current.ownership, Ownership::Live));
                let release_previous =
                    matches!(linearity_of(&binding.ty.kind), Linearity::Linear) && still_live;
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
            Stmt::Return(expr, span) => {
                // Inside a lambda whose return type is being inferred, the
                // first `return` fixes it and later ones must agree.
                if let Some(current) = self.lambda_ret_infer.last().cloned() {
                    return match expr {
                        Some(e) => {
                            let checked = match &current {
                                Some(known) => self.check_expr_with_expected(e, known)?,
                                None => self.check_expr(e)?,
                            };
                            let found = expr_ty(&checked);
                            let prev = self.lambda_ret_infer.last().cloned().flatten();
                            if let Some(prev) = prev {
                                if !fits(&prev, &found) {
                                    self.mismatch(expr_span(&checked), &prev, &found);
                                }
                            } else if let Some(top) = self.lambda_ret_infer.last_mut() {
                                *top = Some(found.clone());
                            }
                            Some(HirStmt::Return(Some(checked), *span))
                        }
                        None => {
                            let prev = self.lambda_ret_infer.last().cloned().flatten();
                            match prev {
                                None => {
                                    if let Some(top) = self.lambda_ret_infer.last_mut() {
                                        *top = Some(prim(TypeKind::Unit));
                                    }
                                }
                                Some(prev) if !prev.eq_ignoring_span(&prim(TypeKind::Unit)) => {
                                    self.mismatch_message(
                                        *span,
                                        format!("expected `{prev}`, found `void`"),
                                    );
                                }
                                _ => {}
                            }
                            Some(HirStmt::Return(None, *span))
                        }
                    };
                }
                match expr {
                    Some(e) => {
                        let checked = self.check_expr_with_expected(e, ret)?;
                        if !fits(ret, &expr_ty(&checked)) {
                            self.mismatch(expr_span(&checked), ret, &expr_ty(&checked));
                        }
                        Some(HirStmt::Return(Some(checked), *span))
                    }
                    None => {
                        if !ret.eq_ignoring_span(&prim(TypeKind::Unit)) {
                            self.mismatch_message(*span, format!("expected `{ret}`, found `void`"));
                        }
                        Some(HirStmt::Return(None, *span))
                    }
                }
            }
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
                    self.mismatch(
                        expr_span(&cond_expr),
                        &prim(TypeKind::Bool),
                        &expr_ty(&cond_expr),
                    );
                }
                // Nested blocks belong to the same function, so `return` inside
                // them must still yield the *function's* return type. Checking
                // them against `void` rejected every `return` inside an `if` or a
                // loop body with a bogus "expected `void`, found `int`".
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
            Stmt::IfLet {
                pattern,
                value,
                then_block,
                else_block,
                span,
            } => self.check_if_let(pattern, value, then_block, else_block.as_ref(), *span, ret),
            Stmt::WhileLet {
                pattern,
                value,
                body,
                label,
                span,
            } => {
                self.push_loop_label(label, *span);
                if label.is_some() {
                    // Register for propagation even before the body is read.
                    self.labeled_loops.push(label.clone().unwrap());
                }
                let out = self.check_while_let(pattern, value, body, *span, ret);
                self.pop_loop_label();
                if label.is_some() {
                    self.labeled_loops.pop();
                }
                match out {
                    Some(loop_stmt) => Some(self.wrap_loop(label, loop_stmt, *span)),
                    None => None,
                }
            }
            Stmt::ForRange {
                name,
                start,
                end,
                body,
                label,
                span,
            } => {
                let start_expr = self.check_expr(start)?;
                let end_expr = self.check_expr(end)?;
                if !expr_ty(&start_expr).eq_ignoring_span(&prim(TypeKind::I64)) {
                    self.error(
                        expr_span(&start_expr),
                        format!("expected `int`, found `{}`", expr_ty(&start_expr)),
                    );
                }
                if !expr_ty(&end_expr).eq_ignoring_span(&prim(TypeKind::I64)) {
                    self.error(
                        expr_span(&end_expr),
                        format!("expected `int`, found `{}`", expr_ty(&end_expr)),
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
                        visibility: Visibility::Private,
                    },
                );
                self.push_loop_label(label, *span);
                if label.is_some() {
                    self.labeled_loops.push(label.clone().unwrap());
                }
                let body = self.check_block_with_ret(body, ret)?;
                self.pop_loop_label();
                if label.is_some() {
                    self.labeled_loops.pop();
                }
                self.scope.pop();
                let loop_stmt = HirStmt::ForRange {
                    name: name.clone(),
                    start: start_expr,
                    end: end_expr,
                    body,
                    span: *span,
                };
                Some(self.wrap_loop(label, loop_stmt, *span))
            }
            Stmt::ForIn {
                pattern,
                index,
                iterable,
                body,
                label,
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
                    TypeKind::Struct(name) | TypeKind::Enum(name) => {
                        return self.check_iterator_for_in(
                            match iterable {
                                Expr::Ident(path, _) if path.len() == 1 => {
                                    Some(path.last().cloned().unwrap_or_default())
                                }
                                _ => None,
                            },
                            iterable_expr,
                            &iterable_ty,
                            name,
                            pattern,
                            index.as_deref(),
                            body,
                            ret,
                            label,
                            *span,
                        );
                    }
                    _ => {
                        if index.is_some() {
                            self.error_code(
                                *span,
                                codes::TYPE_MISMATCH,
                                format!(
                                    "`for` over a value must iterate an array, found `{iterable_ty}`"
                                ),
                            );
                        } else {
                            self.error(
                                *span,
                                format!(
                                    "`for` over a value must iterate an array, found `{iterable_ty}`"
                                ),
                            );
                        }
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
                // `for i, pattern in xs` lowers to a range loop over `xs.len()`
                // whose body first reads element `i` and binds the pattern.
                if let Some(index) = index {
                    self.scope.push();
                    let _ = self.scope.declare(
                        index.clone(),
                        Binding {
                            ty: prim(TypeKind::I64),
                            mutability: Mutability::Const,
                            span: *span,
                            ownership: Ownership::Live,
                            visibility: Visibility::Private,
                        },
                    );
                    let element_pattern =
                        self.check_binding_pattern(pattern, &element, Mutability::Const);
                    self.iteration_borrows.push(binding);
                    self.push_loop_label(label, *span);
                    if label.is_some() {
                        self.labeled_loops.push(label.clone().unwrap());
                    }
                    let body = self.check_block_with_ret(body, ret);
                    self.pop_loop_label();
                    if label.is_some() {
                        self.labeled_loops.pop();
                    }
                    self.iteration_borrows.pop();
                    self.scope.pop();
                    let body = body?;
                    let element_pattern = element_pattern?;
                    let read = HirStmt::Destructure {
                        init: HirExpr::Index {
                            base: Box::new(iterable_expr.clone()),
                            index: Box::new(HirExpr::Ident(
                                vec![index.clone()],
                                *span,
                                prim(TypeKind::I64),
                            )),
                            span: *span,
                            ty: element.clone(),
                        },
                        pattern: element_pattern,
                        mutability: Mutability::Const,
                        span: *span,
                    };
                    let mut stmts = vec![read];
                    stmts.extend(body.stmts);
                    let loop_stmt = HirStmt::ForRange {
                        name: index.clone(),
                        start: HirExpr::Literal(
                            yashy_hir::hir::Literal::I64(0),
                            *span,
                            prim(TypeKind::I64),
                        ),
                        end: match &iterable_ty.kind {
                            TypeKind::FixedArray(_, length) => HirExpr::Literal(
                                yashy_hir::hir::Literal::I64(
                                    i64::try_from(*length).unwrap_or(i64::MAX),
                                ),
                                *span,
                                prim(TypeKind::I64),
                            ),
                            TypeKind::View(_) => HirExpr::Call {
                                callee: vec!["view_len".to_string()],
                                args: vec![iterable_expr.clone()],
                                unsafe_call: false,
                                span: *span,
                                ty: prim(TypeKind::I64),
                            },
                            _ => HirExpr::Call {
                                callee: vec!["len".to_string()],
                                args: vec![iterable_expr.clone()],
                                unsafe_call: false,
                                span: *span,
                                ty: prim(TypeKind::I64),
                            },
                        },
                        body: HirBlock {
                            stmts,
                            span: body.span,
                            arena: false,
                        },
                        span: *span,
                    };
                    return Some(self.wrap_loop(label, loop_stmt, *span));
                }
                self.scope.push();
                let element_pattern =
                    self.check_binding_pattern(pattern, &element, Mutability::Const);
                self.iteration_borrows.push(binding);
                self.push_loop_label(label, *span);
                if label.is_some() {
                    self.labeled_loops.push(label.clone().unwrap());
                }
                let body = self.check_block_with_ret(body, ret);
                self.pop_loop_label();
                if label.is_some() {
                    self.labeled_loops.pop();
                }
                self.iteration_borrows.pop();
                self.scope.pop();
                let body = body?;
                let loop_stmt = HirStmt::ForIn {
                    pattern: element_pattern?,
                    iterable: iterable_expr,
                    body,
                    span: *span,
                };
                Some(self.wrap_loop(label, loop_stmt, *span))
            }
            Stmt::ForC {
                init,
                cond,
                update,
                body,
                label,
                span,
            } => {
                let init = init
                    .as_ref()
                    .and_then(|s| self.check_stmt(s, ret))
                    .map(Box::new);
                let cond_expr = cond.as_ref().and_then(|c| self.check_expr(c));
                if let Some(c) = &cond_expr {
                    if !expr_ty(c).eq_ignoring_span(&prim(TypeKind::Bool)) {
                        self.mismatch_message(
                            expr_span(c),
                            format!("expected `bool`, found `{}`", expr_ty(c)),
                        );
                    }
                }
                let update_expr = update.as_ref().and_then(|u| self.check_expr(u));
                self.push_loop_label(label, *span);
                if label.is_some() {
                    self.labeled_loops.push(label.clone().unwrap());
                }
                let body = self.check_block_with_ret(body, ret)?;
                self.pop_loop_label();
                if label.is_some() {
                    self.labeled_loops.pop();
                }
                let loop_stmt = HirStmt::ForC {
                    init,
                    cond: cond_expr,
                    update: update_expr,
                    body,
                    span: *span,
                };
                Some(self.wrap_loop(label, loop_stmt, *span))
            }
            Stmt::Switch {
                subject,
                arms,
                span,
            } => self.check_switch_stmt(subject, arms, *span),
            Stmt::Break(label, span) => match label {
                None => Some(HirStmt::Break(*span)),
                Some(label) => self.labeled_jump(label, true, *span),
            },
            Stmt::Continue(label, span) => match label {
                None => Some(HirStmt::Continue(*span)),
                Some(label) => self.labeled_jump(label, false, *span),
            },
            Stmt::Defer { body, span } => {
                let body = self.check_block_with_ret(body, ret)?;
                Some(HirStmt::Defer { body, span: *span })
            }
            Stmt::Scope { body, span } => {
                self.uses_async = true;
                let body = self.check_block_with_ret(body, ret)?;
                Some(HirStmt::Scope { body, span: *span })
            }
            Stmt::Arena { body, span } => {
                // `arena { .. }` is an `if true` whose block carries the arena
                // boundary bit; the existing block machinery releases it on
                // every exit, including `break`, `continue` and `return`.
                let body = self.check_block_with_ret(body, ret)?;
                Some(HirStmt::If {
                    init: None,
                    cond: HirExpr::Literal(
                        yashy_hir::hir::Literal::Bool(true),
                        *span,
                        prim(TypeKind::Bool),
                    ),
                    then_block: HirBlock {
                        stmts: body.stmts,
                        span: body.span,
                        arena: true,
                    },
                    else_block: None,
                    span: *span,
                })
            }
            Stmt::Cfg { block, span, .. } => {
                // The pipeline's `cfg` filter runs before this pass and
                // splices an active block into its parent, so this arm is a
                // defensive one for a caller that never filtered: it checks
                // the block and keeps it, matching `Item::Cfg`'s "look through
                // the wrapper" rule above.
                let body = self.check_block_with_ret(block, ret)?;
                Some(HirStmt::If {
                    init: None,
                    cond: HirExpr::Literal(
                        yashy_hir::hir::Literal::Bool(true),
                        *span,
                        prim(TypeKind::Bool),
                    ),
                    then_block: body,
                    else_block: None,
                    span: *span,
                })
            }
            Stmt::Invariant(contract) => {
                let checked = self.check_contract(contract)?;
                Some(HirStmt::LoopInvariant(checked))
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

    /// Check `if let <pattern> = <value> { .. } else { .. }`.
    ///
    /// The pattern's names are declared in a scope that stays open for the then
    /// block and is popped before the `else` block is read: the else branch has
    /// no payload to name, so a use of the binding there is an unknown name.
    /// Ownership merges across the two branches exactly as it does for
    /// [`Stmt::If`]: a binding moved in either branch is moved after the
    /// statement, because which branch ran is a run-time question.
    fn check_if_let(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        then_block: &Block,
        else_block: Option<&Block>,
        span: Span,
        ret: &Type,
    ) -> Option<HirStmt> {
        let (pattern, subject) = self.check_let_head(pattern, value, "if let")?;
        let entry_moved = self.scope.moved_bindings();
        let then_block = match self.check_block_with_ret(then_block, ret) {
            Some(block) => block,
            None => {
                self.scope.pop();
                return None;
            }
        };
        let after_then = self.scope.moved_bindings();
        // Close the pattern's scope before the else branch: its bindings
        // belong to a value the else branch did not match.
        self.scope.pop();
        self.scope.apply_moved(&entry_moved);
        let else_block = match else_block {
            Some(block) => match self.check_block_with_ret(block, ret) {
                Some(checked) => Some(checked),
                None => return None,
            },
            None => None,
        };
        let after_else = if else_block.is_some() {
            self.scope.moved_bindings()
        } else {
            entry_moved.clone()
        };
        let merged: BTreeSet<(usize, String)> = after_then.union(&after_else).cloned().collect();
        self.scope.apply_moved(&merged);
        Some(HirStmt::IfLet {
            pattern,
            subject,
            then_block,
            else_block,
            span,
        })
    }

    /// Check `for <pattern> in <iterator> { .. }` over a user type.
    ///
    /// The iterable is the iterator: it is moved into a fresh mutable
    /// binding and the loop lowers to exactly the shape a hand-written
    /// `while let Some(x) = it.next() { body }` already takes -- a
    /// [`HirStmt::ForC`] whose body is one [`HirStmt::IfLet`] with an
    /// `else { break; }` -- so the verifier and the backend need no new
    /// loop node. A type with no `Iterator` impl is `E1503` at the `for`.
    #[allow(clippy::too_many_arguments)]
    fn check_iterator_for_in(
        &mut self,
        _binding: Option<String>,
        iterable_expr: HirExpr,
        iterable_ty: &Type,
        type_name: &str,
        pattern: &Pattern,
        index: Option<&str>,
        body: &Block,
        ret: &Type,
        label: &Option<String>,
        span: Span,
    ) -> Option<HirStmt> {
        if let Some(index) = index {
            self.error_code(
                span,
                codes::TYPE_MISMATCH,
                format!("`for {index}, x` needs an indexed iterable, found `{iterable_ty}`"),
            );
            return None;
        }
        let impl_ = match self
            .trait_impls
            .get(&("Iterator".to_string(), type_name.to_string()))
        {
            Some(impl_) => impl_.clone(),
            None => {
                self.error_code(
                    span,
                    codes::UNSATISFIED_BOUND,
                    format!(
                        "`{iterable_ty}` does not implement `Iterator` required by `for`{}",
                        self.impl_needs_line(iterable_ty, "Iterator"),
                    ),
                );
                return None;
            }
        };
        let symbol = impl_
            .methods
            .iter()
            .find(|(m, _)| m == "next")
            .map(|(_, s)| s.clone());
        let Some(symbol) = symbol else {
            self.error_code(
                span,
                codes::UNSATISFIED_BOUND,
                format!(
                    "`{iterable_ty}` does not implement `Iterator` required by `for`{}",
                    self.impl_needs_line(iterable_ty, "Iterator"),
                ),
            );
            return None;
        };
        let sig = self.methods.get(&symbol).cloned()?;
        self.check_visibility(type_name, span);
        self.check_method_visibility(&sig, "next", span);
        if !sig.has_self || !sig.receiver_is_pointer {
            self.error_code(
                span,
                codes::UNSATISFIED_BOUND,
                format!(
                    "`{iterable_ty}`'s `next` is not `fn next(self: *mut Self) -> Option[T]` as `Iterator` requires"
                ),
            );
            return None;
        }
        match &sig.ret.kind {
            TypeKind::Option(_) => {}
            _ => {
                self.error_code(
                    span,
                    codes::UNSATISFIED_BOUND,
                    format!(
                        "`{iterable_ty}`'s `next` is not `fn next(self: *mut Self) -> Option[T]` as `Iterator` requires"
                    ),
                );
                return None;
            }
        }
        // The iterator moves into a fresh mutable binding, the same shape
        // as the hand-written `let mut it = it;` the plan spells out.
        let fresh = format!("__iterator_{}", span.start);
        self.scope.push();
        let moved = HirExpr::Move(Box::new(iterable_expr), span, iterable_ty.clone());
        if let Err(e) = self.scope.declare(
            fresh.clone(),
            Binding {
                ty: iterable_ty.clone(),
                mutability: Mutability::Var,
                span,
                ownership: Ownership::Live,
                visibility: Visibility::Private,
            },
        ) {
            self.report(e);
        }
        // `while let Some(<pattern>) = it.next() { body }`, checked through
        // the same path as a hand-written one, labels included.
        self.push_loop_label(label, span);
        if label.is_some() {
            self.labeled_loops.push(label.clone().unwrap());
        }
        let some_pattern = Pattern::TupleVariant {
            path: vec!["Some".to_string()],
            args: vec![pattern.clone()],
            span,
        };
        let next_value = Expr::Call {
            callee: vec![fresh.clone(), "next".to_string()],
            args: Vec::new(),
            unsafe_call: false,
            span,
        };
        let loop_stmt = self.check_while_let(&some_pattern, &next_value, body, span, ret);
        self.pop_loop_label();
        if label.is_some() {
            self.labeled_loops.pop();
        }
        let loop_stmt = loop_stmt?;
        self.scope.pop();
        // Attach the moved iterator as the loop's init.
        let init = HirStmt::Var {
            name: fresh,
            ty: Some(iterable_ty.clone()),
            init: moved,
            mutability: Mutability::Var,
            span,
        };
        let loop_stmt = match loop_stmt {
            HirStmt::ForC {
                cond, update, body, ..
            } => HirStmt::ForC {
                init: Some(Box::new(init)),
                cond,
                update,
                body,
                span,
            },
            other => other,
        };
        Some(self.wrap_loop(label, loop_stmt, span))
    }

    /// Check `while let <pattern> = <value> { .. }`.
    ///
    /// Lowered to an endless [`HirStmt::ForC`] whose body is one
    /// [`HirStmt::IfLet`]: the value is re-evaluated every iteration, the
    /// pattern's names are in scope for the body, and a pattern that misses
    /// leaves the loop through the synthesized `else { break; }`. The backend
    /// already pushes a loop target for a `for`, so `break` and `continue` in
    /// the body keep working with no new statement.
    fn check_while_let(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        body: &Block,
        span: Span,
        ret: &Type,
    ) -> Option<HirStmt> {
        let (pattern, subject) = self.check_let_head(pattern, value, "while let")?;
        let body_block = match self.check_block_with_ret(body, ret) {
            Some(block) => block,
            None => {
                self.scope.pop();
                return None;
            }
        };
        self.scope.pop();
        let hit = HirStmt::IfLet {
            pattern,
            subject,
            then_block: body_block,
            else_block: Some(HirBlock {
                stmts: vec![HirStmt::Break(span)],
                span,
                arena: false,
            }),
            span,
        };
        Some(HirStmt::ForC {
            init: None,
            cond: None,
            update: None,
            body: HirBlock {
                stmts: vec![hit],
                span,
                arena: false,
            },
            span,
        })
    }

    /// The `pattern = value` head shared by `if let` and `while let`.
    ///
    /// On success the scope the bindings live in is still pushed and the
    /// caller owns popping it; on failure the scope is already popped and the
    /// error has been reported. The irrefutability check runs here because it
    /// is about the form, before any HIR is built.
    fn check_let_head(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        form: &str,
    ) -> Option<(HirPattern, HirExpr)> {
        let subject = self.check_expr(value)?;
        let subject_ty = expr_ty(&subject);
        self.scope.push();
        let checked = match self.check_arm_pattern(pattern, &subject_ty) {
            Ok(checked) => checked,
            Err(error) => {
                self.report(error);
                self.scope.pop();
                return None;
            }
        };
        let irrefutable = {
            let this = &*self;
            let variants = |name: &str| this.enums.get(name).map(|sig| sig.variants.clone());
            let struct_fields = |name: &str| {
                this.structs.get(name).map(|sig| {
                    sig.fields
                        .iter()
                        .map(|(field, ty)| (field.clone(), ty.clone()))
                        .collect()
                })
            };
            is_irrefutable(&checked, &subject_ty, &variants, &struct_fields, &|ty| {
                this.resolve_type(ty)
            })
        };
        if irrefutable {
            let subject_text = subject_ty.to_string();
            let message = if form == "if let" {
                format!(
                    "this pattern matches every `{subject_text}`: `if let` needs a pattern that can fail"
                )
            } else {
                format!("this pattern matches every `{subject_text}`: `while let` would never end")
            };
            self.report(
                TypeError::new(hir_pattern_span(&checked), message)
                    .with_code(codes::IRREFUTABLE_IF_LET),
            );
        }
        Some((checked, subject))
    }

    /// Check one arm against the subject type: pattern bindings are declared
    /// in a scope pushed for the arm and popped after the body, so two arms
    /// can bind the same name.
    fn check_arm_against(&mut self, subject_ty: &Type, arm: &SwitchArm) -> Option<HirSwitchArm> {
        self.scope.push();
        let pattern = match self.check_arm_pattern(&arm.pattern, subject_ty) {
            Ok(pattern) => pattern,
            Err(error) => {
                self.report(error);
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
        let struct_fields = |name: &str| {
            self.structs.get(name).map(|sig| {
                sig.fields
                    .iter()
                    .map(|(field, ty)| (field.clone(), ty.clone()))
                    .collect()
            })
        };
        // `resolve_type` borrows `self` immutably; wrap it so the closure
        // does not capture the whole checker mutably.
        let checked = {
            let this = &*self;
            check_pattern(
                pattern,
                subject_ty,
                &mut bindings,
                &variants,
                &struct_fields,
                &|ty| this.resolve_type(ty),
            )?
        };
        for (name, ty, mutability, span) in bindings {
            self.scope.declare(
                name,
                Binding {
                    ty,
                    mutability,
                    span,
                    ownership: Ownership::Live,
                    visibility: Visibility::Private,
                },
            )?;
        }
        Ok(checked)
    }

    /// Check a pattern in binding position: `let Point { x, y } = p;`,
    /// `const (a, b) = pair;` and `for Point { x, .. } in points`.
    ///
    /// Unlike [`Self::check_arm_pattern`], which binds with the pattern's
    /// [`Mutability::Const`], a binding-position pattern binds with
    /// `mutability` -- `let` names its `Var`, `const` and `for` name `Const`.
    /// The pattern must also be irrefutable: there is no branch to run when
    /// it misses, so `let Some(x) = opt;` is E0100, the same code the other
    /// type mismatches use.
    fn check_binding_pattern(
        &mut self,
        pattern: &Pattern,
        subject_ty: &Type,
        mutability: Mutability,
    ) -> Option<HirPattern> {
        let mut bindings = Vec::new();
        let variants = |name: &str| self.enums.get(name).map(|sig| sig.variants.clone());
        let struct_fields = |name: &str| {
            self.structs.get(name).map(|sig| {
                sig.fields
                    .iter()
                    .map(|(field, ty)| (field.clone(), ty.clone()))
                    .collect()
            })
        };
        let checked = {
            let this = &*self;
            match check_pattern(
                pattern,
                subject_ty,
                &mut bindings,
                &variants,
                &struct_fields,
                &|ty| this.resolve_type(ty),
            ) {
                Ok(checked) => checked,
                Err(error) => {
                    self.report(error);
                    return None;
                }
            }
        };
        let irrefutable = {
            let this = &*self;
            let variants = |name: &str| this.enums.get(name).map(|sig| sig.variants.clone());
            let struct_fields = |name: &str| {
                this.structs.get(name).map(|sig| {
                    sig.fields
                        .iter()
                        .map(|(field, ty)| (field.clone(), ty.clone()))
                        .collect()
                })
            };
            is_irrefutable(&checked, subject_ty, &variants, &struct_fields, &|ty| {
                this.resolve_type(ty)
            })
        };
        if !irrefutable {
            self.report(
                TypeError::new(
                    hir_pattern_span(&checked),
                    format!("this pattern can fail against `{subject_ty}`: it cannot appear in binding position"),
                )
                .with_code(codes::TYPE_MISMATCH)
                .with_label("use `switch`")
                .with_note("a binding has no branch to run when the pattern misses: use a `switch` to handle the case"),
            );
            return None;
        }
        for (name, ty, _, binding_span) in bindings {
            if let Err(error) = self.scope.declare(
                name,
                Binding {
                    ty,
                    mutability,
                    span: binding_span,
                    ownership: Ownership::Live,
                    visibility: Visibility::Private,
                },
            ) {
                self.report(error);
            }
        }
        Some(checked)
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
                    yashy_parser::ast::VariantDef {
                        name: "None".to_string(),
                        fields: Vec::new(),
                        span: Span::new(0, 0),
                    },
                    yashy_parser::ast::VariantDef {
                        name: "Some".to_string(),
                        fields: vec![yashy_parser::ast::VariantField::Positional {
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
                    yashy_parser::ast::VariantDef {
                        name: "Ok".to_string(),
                        fields: vec![yashy_parser::ast::VariantField::Positional {
                            ty: ok_ty,
                            span: Span::new(0, 0),
                        }],
                        span: Span::new(0, 0),
                    },
                    yashy_parser::ast::VariantDef {
                        name: "Err".to_string(),
                        fields: vec![yashy_parser::ast::VariantField::Positional {
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
                self.error_code(
                    span,
                    codes::NON_EXHAUSTIVE_MATCH,
                    "switch over a non-enum type is not exhaustive: add a `default` arm"
                        .to_string(),
                );
                None
            }
        }
    }

    /// Check a call of a private method from another module.
    ///
    /// Visibility is module-scoped, not type-scoped: the caller's module
    /// comparing equal with the method's own module always passes, and a
    /// `pub` method passes from anywhere. The message names the method, the
    /// type and the module that hides it, as the field rule does.
    fn check_method_visibility(&mut self, sig: &MethodSig, method: &str, span: Span) {
        if sig.origin.file == self.file || sig.origin.public {
            return;
        }
        let module = self.module_name(sig.origin.file);
        self.error_code(
            span,
            codes::PRIVATE_ITEM,
            format!(
                "method `{method}` on `{}` is private to module `{module}`",
                sig.type_name,
            ),
        );
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
        let missing = {
            let root = variants.to_vec();
            let lookup = |name: &str| {
                if name == enum_name {
                    Some(root.clone())
                } else {
                    self.enums.get(name).map(|sig| sig.variants.clone())
                }
            };
            missing_witnesses_with(enum_name, variants, &patterns, has_default, &lookup)
        };
        if missing.is_empty() {
            Some(())
        } else {
            self.error_code(
                span,
                codes::NON_EXHAUSTIVE_MATCH,
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
            self.error_code(
                span,
                codes::WRONG_ARGUMENT_COUNT,
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
    /// path whose head is not an enum falls through to the ordinary call path,
    /// which reports an unknown function as it always did.
    ///
    /// The enum name is every segment but the last, so `io.Error.NotFound`
    /// names the variant `NotFound` of `io.Error`. A two-segment path is the
    /// ordinary case; a longer one is what a builtin enum such as `io.Error`
    /// needs.
    fn check_enum_call(
        &mut self,
        callee: &[String],
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        let (enum_name, variant_name) = split_enum_path(callee)?;
        // A generic enum is a scheme, not a type: `Maybe.Just(2)` has to
        // infer the argument tuple from the payload before the concrete
        // variant exists. A name that is neither an enum nor a scheme falls
        // through to the ordinary call path.
        let Some(sig) = self.enums.get(&enum_name).cloned() else {
            return self.check_scheme_enum_call(args, span, &enum_name, &variant_name);
        };
        let variant = sig
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .cloned()?;
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
                    self.report(e);
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
            // site. `Shape.Box(buf)` and `buf.free()` agree about who owns it.
            if matches!(linearity_of(&expected.kind), Linearity::Linear) {
                if let Expr::Ident(path, arg_span) = arg {
                    let arg_name = path.last().cloned().unwrap_or_default();
                    if let Err(e) = self.move_out(&arg_name, *arg_span) {
                        self.report(e);
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

    /// `Maybe.Just(2)` for a generic enum: infer the type arguments from the
    /// payload values, intern the monomorph, and construct under its mangled
    /// name.
    ///
    /// The literal carries no type arguments of its own, so each variant
    /// field's declared type is unified against the value written. Every
    /// parameter must be inferred this way: a variant with no payload cannot
    /// be constructed without an expected type, which this call does not have.
    fn check_scheme_enum_call(
        &mut self,
        args: &[Expr],
        span: Span,
        enum_name: &str,
        variant_name: &str,
    ) -> Option<Option<HirExpr>> {
        let scheme = self.enum_schemes.get(enum_name)?.clone();
        // The enum is an item: private to its module unless `pub`.
        self.check_visibility(enum_name, span);
        let variant = scheme
            .variants
            .iter()
            .find(|variant| variant.name == variant_name)?
            .clone();
        if variant
            .fields
            .iter()
            .any(|field| matches!(field, VariantField::Named { .. }))
        {
            self.error(
                span,
                format!(
                    "cannot construct the named-field variant `{variant_name}` of the \
                     generic enum `{enum_name}` yet: its type arguments are inferred from \
                     positional payloads, or write a function that returns `{enum_name}[..]`"
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
        let mut subst: HashMap<String, Type> = HashMap::new();
        let mut checked = Vec::with_capacity(args.len());
        for (arg, field) in args.iter().zip(variant.fields.iter()) {
            let value = self.check_expr(arg)?;
            let _ = self.unify_type(
                field.ty(),
                &scheme.type_params,
                &mut subst,
                &expr_ty(&value),
            );
            // A payload of linear type takes ownership, exactly as it does in
            // the concrete construction path.
            if matches!(linearity_of(&expr_ty(&value).kind), Linearity::Linear) {
                if let Expr::Ident(path, arg_span) = arg {
                    let name = path.last().cloned().unwrap_or_default();
                    if let Err(error) = self.move_out(&name, *arg_span) {
                        self.report(error);
                    }
                }
            }
            checked.push(value);
        }
        let mut ordered = Vec::with_capacity(scheme.type_params.len());
        for param in &scheme.type_params {
            match subst.get(&param.name) {
                Some(concrete) => ordered.push(concrete.clone()),
                None => {
                    self.error(
                        span,
                        format!("cannot infer type argument for `{}`", param.name),
                    );
                    return Some(None);
                }
            }
        }
        self.check_type_args_bounds(&scheme.type_params, &ordered, span, enum_name);
        // `intern_monomorph` substitutes the arguments into the variant
        // definitions, so the inferred tuple is written back as syntax. The
        // resulting types are already concrete, which is what the monomorph
        // stores.
        let arg_exprs: Vec<TypeExpr> = ordered.iter().map(type_to_expr).collect();
        let mangled = mangled_generic_name(enum_name, &ordered);
        self.intern_monomorph(enum_name, &scheme.type_params, &arg_exprs, ordered, span);
        if !self.enums.contains_key(&mangled) {
            return Some(None);
        }
        Some(Some(HirExpr::Call {
            // Code generation matches the path's head against the value's
            // enum type, so the head is the monomorph's mangled name.
            callee: vec![mangled.clone(), variant_name.to_string()],
            args: checked,
            unsafe_call: false,
            span,
            ty: Type {
                kind: TypeKind::Enum(mangled),
                span,
            },
        }))
    }

    /// `Enum.Variant` for a variant with no payload.
    ///
    /// A dotted name is otherwise a field chain; an enum has no fields, so a
    /// name whose head is an enum can only be construction.
    fn check_enum_path(&mut self, path: &[String], span: Span) -> Option<Option<HirExpr>> {
        let (enum_name, variant_name) = split_enum_path(path)?;
        let sig = self.enums.get(&enum_name)?.clone();
        // The enum is an item, so it is private to the module that declares
        // it unless it is `pub`.
        self.check_visibility(&enum_name, span);
        let variant = sig
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .cloned()?;
        if !variant.fields.is_empty() {
            self.error(
                span,
                format!(
                    "variant `{variant_name}` of `{enum_name}` takes {} field{}; construct it with `{enum_name}.{variant_name}(...)`",
                    variant.fields.len(),
                    if variant.fields.len() == 1 { "" } else { "s" },
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
                kind: TypeKind::Enum(enum_name),
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
        let (enum_name, variant_name) = split_enum_path(path)?;
        let sig = self.enums.get(&enum_name)?.clone();
        // The enum is an item, so it is private to the module that declares
        // it unless it is `pub`.
        self.check_visibility(&enum_name, span);
        let variant = sig
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .cloned()?;
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
                self.error_code(
                    *field_span,
                    codes::UNKNOWN_FIELD,
                    format!("unknown field `{field_name}` in variant `{variant_name}`"),
                );
                let _ = self.check_expr(value);
                continue;
            };
            let expected = match self.resolve_type(expected.ty()) {
                Ok(ty) => ty,
                Err(e) => {
                    self.report(e);
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
                self.error_code(
                    span,
                    codes::MISSING_FIELD,
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
        // A lambda is checked against the function type it is wanted as: that
        // is where an inferred `|x|` gets its type from.
        if let Expr::Lambda {
            params,
            ret,
            body,
            span,
        } = expr
        {
            if let TypeKind::Func(exp_params, exp_ret) = &expected.kind {
                return self.check_lambda(
                    params,
                    ret,
                    body,
                    *span,
                    Some((exp_params.as_slice(), Some(exp_ret.as_ref()))),
                );
            }
        }
        if let Expr::Call {
            callee, args, span, ..
        } = expr
        {
            if callee.len() == 1 {
                let name = callee.last().cloned().unwrap_or_default();
                let arg_vals: Vec<Expr> = args.iter().map(|a| a.value.clone()).collect();
                if matches!(name.as_str(), "None" | "Some" | "Ok" | "Err") {
                    return self.check_constructor(&name, &arg_vals, *span, Some(expected))?;
                }
                if name == "map_new" {
                    return self.check_map_new(&arg_vals, *span, Some(expected));
                }
                if name == "chan_new" {
                    return self.check_chan_new(&arg_vals, *span, Some(expected));
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
        // The scalar conversions are methods on a value, so they are resolved
        // before a user method of the same name could be: `to_int` on a `char`
        // is not something a program declares.
        if matches!(method.as_str(), "to_int" | "to_char" | "to_string")
            && matches!(receiver_ty.kind, TypeKind::Char | TypeKind::I64)
        {
            if let Some(receiver_checked) =
                self.check_expr(&Expr::Ident(vec![receiver.clone()], span))
            {
                if let Some(converted) =
                    self.check_scalar_method(receiver_checked, method, args, span)
                {
                    return Some(Some(converted));
                }
            }
        }
        // The receiver's type is an item, so it is private to the module that
        // declares it unless it is `pub`.
        if let TypeKind::Struct(name) = &receiver_ty.kind {
            self.check_visibility(name, span);
        }
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
            "push" | "pop" | "free_slice" => self
                .check_slice_builtin(builtin, &call_args, span)
                .flatten(),
            "map_get" | "map_insert" | "map_remove" | "map_len" | "map_free" | "map_keys"
            | "map_values" | "map_contains_key" => {
                self.check_map_builtin(builtin, &call_args, span).flatten()
            }
            "chan_send" | "chan_recv" | "chan_close" | "chan_free" | "mutex_lock"
            | "mutex_unlock" | "mutex_free" => self
                .check_thread_builtin(builtin, &call_args, span)
                .flatten(),
            "dealloc" => self.check_pointer_free(&call_args, span),
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
            let mut error = TypeError::new(
                span,
                format!("`{type_name}` has no associated function `{method}`"),
            )
            .with_code(codes::UNKNOWN_ASSOCIATED_FN);
            let candidates = self
                .methods
                .keys()
                .filter_map(|key| key.strip_prefix(&format!("{type_name}.")))
                .filter(|name| !name.contains('.'));
            if let Some(suggestion) = yashy_common::closest(method, candidates) {
                error = error.with_label(format!("did you mean `{suggestion}`?"));
            }
            self.errors.push((self.file, error));
            return None;
        };
        // The type is an item, so it is private to the module that declares
        // it unless it is `pub`.
        self.check_visibility(type_name, span);
        self.check_method_visibility(&sig, method, span);
        if sig.has_self {
            self.error_code(
                span,
                codes::METHOD_THROUGH_TYPE,
                format!("`{method}` is a method; call it on a value: `p.{method}()`"),
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

    /// Resolve `name::method(args)`: an associated call on a type, or a
    /// method call qualified by a trait.
    ///
    /// If `name` is a declared type, the call is an associated call on that
    /// type: the inherent impl and the trait impls in scope are candidates,
    /// filtered to those without a receiver, and ambiguity is `E1504` as
    /// everywhere else. A candidate that takes a receiver is `E1506`; no
    /// candidate is `E1508` with suggestions. If `name` is a declared trait,
    /// it qualifies a receiver call: the first argument is the receiver, and
    /// the impl is `TraitTable[(trait, receiver type)]`'s method.
    fn check_type_path_call(
        &mut self,
        base: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        // The type form first: `Point::new(args)`.
        if self.structs.contains_key(base) || self.enums.contains_key(base) {
            return self.check_associated_call(base, method, args, span);
        }
        // The trait form: `Show::show(x)` qualifies a receiver call.
        if self.traits.contains_key(base) {
            return self.check_qualified_trait_call(base, method, args, span);
        }
        let mut error = TypeError::new(
            span,
            format!("no associated function `{method}` on `{base}`"),
        )
        .with_code(codes::UNKNOWN_ASSOCIATED_FN);
        let candidates = self
            .structs
            .keys()
            .chain(self.enums.keys())
            .map(String::as_str);
        if let Some(suggestion) = yashy_common::closest(base, candidates) {
            error = error.with_label(format!("did you mean `{suggestion}`?"));
        }
        self.errors.push((self.file, error));
        None
    }

    /// Check `Type::method(args)`: an associated call on a named type.
    fn check_associated_call(
        &mut self,
        type_name: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let inherent_key = format!("{type_name}.{method}");
        let inherent = self.methods.get(&inherent_key).cloned();
        // Candidates are the associated functions: inherent or one trait's
        // impl, without a receiver.
        let mut candidates: Vec<(Option<String>, String, MethodSig)> = Vec::new();
        if let Some(sig) = &inherent {
            if !sig.has_self {
                candidates.push((None, inherent_key.clone(), sig.clone()));
            }
        }
        for ((trait_name, ty), impl_) in &self.trait_impls {
            if ty != type_name {
                continue;
            }
            for (mname, symbol) in &impl_.methods {
                if mname == method {
                    if let Some(sig) = self.methods.get(symbol) {
                        if !sig.has_self {
                            candidates.push((
                                Some(trait_name.clone()),
                                symbol.clone(),
                                sig.clone(),
                            ));
                        }
                    }
                }
            }
        }
        match candidates.len() {
            1 => {
                let (_, symbol, sig) = candidates.into_iter().next().unwrap();
                self.finish_static_call(type_name, symbol, &sig, args, span)
            }
            0 => {
                if let Some(sig) = &inherent {
                    if sig.has_self {
                        self.error_code(
                            span,
                            codes::METHOD_THROUGH_TYPE,
                            format!("`{method}` is a method; call it on a value: `p.{method}()`"),
                        );
                        return None;
                    }
                }
                let mut error = TypeError::new(
                    span,
                    format!("`{type_name}` has no associated function `{method}`"),
                )
                .with_code(codes::UNKNOWN_ASSOCIATED_FN);
                let assoc_names = self
                    .methods
                    .keys()
                    .filter_map(|key| key.strip_prefix(&format!("{type_name}.")))
                    .filter(|name| !name.contains('.'));
                if let Some(suggestion) = yashy_common::closest(method, assoc_names) {
                    error = error.with_label(format!("did you mean `{suggestion}`?"));
                }
                self.errors.push((self.file, error));
                None
            }
            _ => {
                let names = candidates
                    .iter()
                    .map(|(trait_name, _, _)| {
                        trait_name
                            .as_deref()
                            .map(|t| format!("`{t}`"))
                            .unwrap_or_else(|| "`Self`".to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(" and ");
                let hints = candidates
                    .iter()
                    .map(|(trait_name, _, _)| {
                        trait_name
                            .as_deref()
                            .map(|t| format!("`{t}::{method}(x)`"))
                            .unwrap_or_else(|| format!("`{type_name}::{method}(x)`"))
                    })
                    .collect::<Vec<_>>()
                    .join(" or ");
                self.error_code(
                    span,
                    codes::AMBIGUOUS_METHOD,
                    format!(
                        "`{method}` is ambiguous on `{type_name}`: both {names} provide it \
                         (qualify with {hints} once `::` is available)"
                    ),
                );
                None
            }
        }
    }

    /// Check `Trait::method(x, args)`: a method call restricted to a trait.
    fn check_qualified_trait_call(
        &mut self,
        trait_name: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        // The trait is an item too, and so is the receiver that provides it.
        self.check_visibility(trait_name, span);
        let Some((receiver_expr, rest)) = args.split_first() else {
            self.error(
                span,
                format!("`{trait_name}::{method}` needs a receiver value first"),
            );
            return None;
        };
        let receiver_checked = self.check_expr(receiver_expr)?;
        let receiver_ty = expr_ty(&receiver_checked);
        let receiver_type = match &receiver_ty.kind {
            TypeKind::Struct(name) | TypeKind::Enum(name) => name.clone(),
            _ => {
                self.error_code(
                    span,
                    codes::UNSATISFIED_BOUND,
                    format!("a receiver of `{receiver_ty}` does not implement `{trait_name}`"),
                );
                return None;
            }
        };
        let impl_ = match self
            .trait_impls
            .get(&(trait_name.to_string(), receiver_type.clone()))
        {
            Some(impl_) => impl_.clone(),
            None => {
                self.error_code(
                    span,
                    codes::UNSATISFIED_BOUND,
                    format!(
                        "`{receiver_type}` does not implement `{trait_name}` required by \
                         `{trait_name}::{method}`{}",
                        self.impl_needs_line(&receiver_ty, trait_name),
                    ),
                );
                return None;
            }
        };
        let symbol = impl_
            .methods
            .iter()
            .find(|(m, _)| m == method)
            .map(|(_, s)| s.clone());
        let Some(symbol) = symbol else {
            self.error_code(
                span,
                codes::UNKNOWN_ASSOCIATED_FN,
                format!("`{trait_name}` declares no `{method}`"),
            );
            return None;
        };
        let sig = self.methods.get(&symbol).cloned()?;
        self.check_visibility(&receiver_type, span);
        // The method's own visibility, as one rule with every call: a
        // private method of an impl in another module only reaches the
        // qualified path from inside that module.
        self.check_method_visibility(&sig, method, span);
        if !sig.has_self {
            self.error_code(
                span,
                codes::ASSOC_THROUGH_VALUE,
                format!(
                    "`{method}` is an associated function; call it as `{receiver_type}::{method}()`"
                ),
            );
            return None;
        }
        let own = sig.params.len().saturating_sub(1);
        if own != rest.len() {
            self.error(
                span,
                format!(
                    "`{method}` takes {own} argument{}",
                    if own == 1 { "" } else { "s" }
                ),
            );
            return None;
        }
        // The receiver is a method receiver: by value, or by address for a
        // pointer receiver.
        let receiver_checked = if sig.receiver_is_pointer {
            let HirExpr::Ident(path, _, _) = &receiver_checked else {
                self.error(
                    span,
                    format!("`{method}` needs a mutable object binding as the receiver"),
                );
                return None;
            };
            let name = path.last().cloned().unwrap_or_default();
            let mutable = self
                .scope
                .resolve(&name)
                .map(|binding| binding.mutability.is_mutable())
                .unwrap_or(false);
            if !mutable {
                self.error(span, format!("`{method}` needs a mutable object binding"));
                return None;
            }
            HirExpr::AddrOf {
                name,
                span,
                ty: Type {
                    kind: TypeKind::Ptr(Box::new(receiver_ty.clone())),
                    span,
                },
            }
        } else {
            receiver_checked
        };
        let mut arg_exprs = Vec::with_capacity(rest.len() + 1);
        arg_exprs.push(receiver_checked);
        for (arg, param) in rest.iter().zip(sig.params.iter().skip(1)) {
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
            callee: vec![symbol],
            args: arg_exprs,
            unsafe_call: false,
            span,
            ty: sig.ret.clone(),
        })
    }

    /// The finish shared by the associated and static paths: arguments
    /// against an ordinary function signature, and the emitted call.
    fn finish_static_call(
        &mut self,
        type_name: &str,
        symbol: String,
        sig: &MethodSig,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        self.check_visibility(type_name, span);
        self.check_method_visibility(sig, &sig.name, span);
        if sig.params.len() != args.len() {
            self.error(
                span,
                format!(
                    "`{}` takes {} argument{}",
                    sig.name,
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
            callee: vec![symbol],
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
            TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Str | TypeKind::Char => true,
            TypeKind::Struct(name) => self
                .structs
                .get(name)
                .map(|sig| {
                    sig.fields
                        .iter()
                        .all(|(_, field)| self.has_equality(&field.kind))
                })
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
        // The order is inherent, then the single trait impl in scope, then
        // the bound on a type parameter (handled around this call site).
        // One candidate wins; two at one level is E1504, never a coin toss;
        // nothing at all falls through to the builtin path.
        let (mangled, sig) = if let Some(entry) = self.bound_entry(&receiver_checked).cloned() {
            match self.bound_candidate(&entry, &type_name, method, span) {
                Some(found) => found,
                None => return Some(None),
            }
        } else {
            match self.resolve_method(&type_name, method, args, span) {
                Some(Some(found)) => found,
                Some(None) => return Some(None),
                None => return None,
            }
        };
        self.check_method_visibility(&sig, method, span);
        if !sig.has_self {
            // A method with no `self` is a static one: it belongs to the type,
            // not to a value of it.
            self.error_code(
                span,
                codes::ASSOC_THROUGH_VALUE,
                format!(
                    "`{method}` is an associated function; call it as `{type_name}::{method}()`"
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
        // `dealloc` names a runtime entry point rather than a method whose
        // receiver is already known, so the suggestion stays receiver-neutral:
        // the same text is right for every binding the caller might have.
        if name == "dealloc" {
            self.error(span, "`dealloc` was replaced: use `.free()`".to_string());
            return;
        }
        let Some(receiver) = args.first() else {
            self.error(
                span,
                format!("`{name}` has moved to method syntax: call `.{method}()` on the receiver"),
            );
            return;
        };
        let receiver_text = self.snippet_text(yashy_parser::exprs::expr_span(receiver));
        let rest: Vec<String> = args
            .iter()
            .skip(1)
            .map(|arg| self.snippet_text(yashy_parser::exprs::expr_span(arg)))
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
    /// The conversions between `char` and the types around it.
    ///
    /// Each has exactly one spelling, and none of them is `as`: a cast says
    /// "the same value at a different width", while these three say what
    /// actually changes. `to_int` is a zero-extension and needs no call;
    /// `to_char` checks the range at run time, because 300 is not a character;
    /// `to_string` produces the one-byte string the byte spells.
    ///
    /// Returns `None` when this is not one of the three, so the ordinary method
    /// paths keep their own error for a method that does not exist.
    fn check_scalar_method(
        &mut self,
        receiver: HirExpr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let known = match (expr_ty(&receiver).kind, method) {
            (TypeKind::Char, "to_int" | "to_string") | (TypeKind::I64, "to_char") => true,
            _ => false,
        };
        if !known {
            return None;
        }
        if !args.is_empty() {
            self.error(span, format!("`{method}` takes 0 arguments"));
            return None;
        }
        Some(match method {
            "to_int" => HirExpr::Cast {
                expr: Box::new(receiver),
                ty: prim(TypeKind::I64),
                span,
            },
            "to_char" => HirExpr::Call {
                callee: vec!["char_from_int".to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: prim(TypeKind::Char),
            },
            _ => HirExpr::Call {
                callee: vec!["str_from_char".to_string()],
                args: vec![receiver],
                unsafe_call: false,
                span,
                ty: prim(TypeKind::Str),
            },
        })
    }

    fn check_string_method(
        &mut self,
        receiver: &str,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Option<Option<HirExpr>> {
        if method == "free" {
            // A string owns its bytes, but the binding releases them itself;
            // there is no separate allocation for the program to hand back.
            self.error(
                span,
                "a `string` releases itself: there is nothing to free".to_string(),
            );
            return Some(None);
        }
        let arity = match method {
            "len" | "is_empty" | "trim" | "trim_start" | "trim_end" | "to_upper" | "to_lower"
            | "lines" | "parse_i64" | "parse_f64" | "graphemes" => 0,
            "byte_at" | "get" | "contains" | "starts_with" | "ends_with" | "index_of"
            | "repeat" | "split" | "char_at" => 1,
            "slice" | "replace" | "substring" => 2,
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
                    ty: prim(TypeKind::Char),
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
            "graphemes" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["str_graphemes".to_string()],
                    args: vec![receiver_checked],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::I64),
                }))
            }
            "char_at" => {
                let Some(receiver_checked) = self.check_expr(&receiver_expr()) else {
                    return Some(None);
                };
                let Some(index) = self.check_i64_argument(&args[0]) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Call {
                    callee: vec!["str_grapheme_at".to_string()],
                    args: vec![receiver_checked, index],
                    unsafe_call: false,
                    span,
                    ty: prim(TypeKind::Str),
                }))
            }
            "substring" => {
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
                    callee: vec!["str_grapheme_slice".to_string()],
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
                    ty: option(&prim(TypeKind::I64), span),
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
                    ty: option(&prim(TypeKind::F64), span),
                }))
            }
            "is_empty" => {
                let Some(len) = self.check_len_builtin(&[receiver_expr()], span) else {
                    return Some(None);
                };
                Some(Some(HirExpr::Binary {
                    op: BinOp::Eq,
                    lhs: Box::new(len),
                    rhs: Box::new(HirExpr::Literal(Literal::I64(0), span, prim(TypeKind::I64))),
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
                        (
                            "str_repeat",
                            prim(TypeKind::Str),
                            vec![receiver_checked, count],
                        )
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

    /// Check one argument that must be an `int`.
    fn check_i64_argument(&mut self, arg: &Expr) -> Option<HirExpr> {
        let checked = self.check_expr_with_expected(arg, &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&checked)) {
            self.error(
                expr_span(&checked),
                format!("expected `int`, found `{}`", expr_ty(&checked)),
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
                    rhs: Box::new(HirExpr::Literal(Literal::I64(0), span, prim(TypeKind::I64))),
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
                    rhs: Box::new(HirExpr::Literal(Literal::I64(1), span, prim(TypeKind::I64))),
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
        let receiver_expr =
            || HirExpr::Ident(vec![receiver.to_string()], span, receiver_ty.clone());
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
                rhs: Box::new(HirExpr::Literal(Literal::I64(0), span, prim(TypeKind::I64))),
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
                    rhs: Box::new(HirExpr::Literal(Literal::I64(1), span, prim(TypeKind::I64))),
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
    fn check_array_map(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        let (receiver_checked, receiver_ty) = self.check_array_receiver(receiver, span)?;
        let TypeKind::Slice(element) = receiver_ty.kind else {
            return None;
        };
        let Some(func_checked) = self.check_callback(
            &args[0],
            span,
            Some((std::slice::from_ref(element.as_ref()), None)),
        ) else {
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
    fn check_array_filter(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        let (receiver_checked, receiver_ty) = self.check_array_receiver(receiver, span)?;
        let TypeKind::Slice(element) = receiver_ty.kind else {
            return None;
        };
        let bool_ty = prim(TypeKind::Bool);
        let Some(func_checked) = self.check_callback(
            &args[0],
            span,
            Some((std::slice::from_ref(element.as_ref()), Some(&bool_ty))),
        ) else {
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
    fn check_array_fold(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        let (receiver_checked, receiver_ty) = self.check_array_receiver(receiver, span)?;
        let TypeKind::Slice(element) = receiver_ty.kind else {
            return None;
        };
        let Some(init_checked) = self.check_expr(&args[0]) else {
            return None;
        };
        let accumulator_ty = expr_ty(&init_checked);
        let fold_params = [accumulator_ty.clone(), (*element).clone()];
        let Some(func_checked) = self.check_callback(
            &args[1],
            span,
            Some((fold_params.as_slice(), Some(&accumulator_ty))),
        ) else {
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

    /// Check one field default at declaration time.
    ///
    /// A default is a literal or a constant expression, evaluated once here;
    /// the checked literal is cloned into every literal that omits the field.
    /// Anything the constant evaluator refuses is `E0102`, and a value of
    /// the wrong type is `E0100` at the default.
    fn check_field_default(
        &mut self,
        default: &Expr,
        field_ty: &Type,
        span: Span,
    ) -> Option<HirExpr> {
        self.check_const_default(default, field_ty, span, "a field default")
    }

    /// One rule, one implementation, for every default site: a field default
    /// and a parameter default share this check, so the two cannot drift
    /// apart. A default is a literal or a `const` name (E0102); anything the
    /// constant evaluator refuses is rejected, and a value of the wrong type
    /// is E0100 at the default.
    fn check_const_default(
        &mut self,
        default: &Expr,
        ty: &Type,
        span: Span,
        what: &str,
    ) -> Option<HirExpr> {
        let mut work = const_eval::Work::default();
        match const_eval::evaluate(
            default,
            &[],
            &self.const_fns,
            &mut work,
            const_eval::Missing::NotConstant,
        ) {
            Ok(value) => {
                let found = Type {
                    kind: value.kind(),
                    span,
                };
                if !fits(ty, &found) {
                    self.mismatch(span, ty, &found);
                    return None;
                }
                Some(value.to_expr(span))
            }
            Err(_) => {
                self.error_code(
                    span,
                    codes::FIELD_DEFAULT_NOT_CONST,
                    format!("{what} must be a literal or a `const` name"),
                );
                None
            }
        }
    }

    /// Resolve `name: value` and positional call arguments to positional
    /// expressions, in parameter order.
    ///
    /// Named arguments never reach the HIR: the call the backend sees is
    /// positional, exactly as if the caller had written it in declaration
    /// order. `E0701` is an unknown name or a positional argument after a
    /// named one; `E0702` is a parameter given twice, checked by slot, so
    /// `f(a: 1, a: 2)` and `f(1, a: 2)` both report it. A slot left empty
    /// takes the parameter's default, or is `E0700` when it has none.
    fn resolve_named_args(
        &mut self,
        args: &[CallArg],
        param_names: &[String],
        param_defaults: &[Option<Box<Expr>>],
        span: Span,
        variadic: bool,
    ) -> Option<Vec<Expr>> {
        let n = param_names.len();
        let mut slots: Vec<Option<Expr>> = vec![None; n];
        let mut extras: Vec<Expr> = Vec::new();
        let mut next_pos = 0usize;
        let mut seen_named = false;
        for arg in args {
            match &arg.name {
                None => {
                    if seen_named {
                        self.error_code(
                            arg.span,
                            codes::UNKNOWN_NAMED_ARGUMENT,
                            "a positional argument cannot follow a named argument".to_string(),
                        );
                        return None;
                    }
                    if next_pos < n {
                        slots[next_pos] = Some(arg.value.clone());
                        next_pos += 1;
                    } else if variadic {
                        extras.push(arg.value.clone());
                    } else {
                        self.error_code(
                            arg.span,
                            codes::WRONG_ARGUMENT_COUNT,
                            format!("expected {n} arguments, found {}", args.len()),
                        );
                        return None;
                    }
                }
                Some(name) => {
                    seen_named = true;
                    let Some(index) = param_names.iter().position(|p| p == name) else {
                        self.error_code(
                            arg.span,
                            codes::UNKNOWN_NAMED_ARGUMENT,
                            format!(
                                "unknown named argument `{name}`; the parameters are {}",
                                param_names
                                    .iter()
                                    .map(|p| format!("`{p}`"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        );
                        return None;
                    };
                    if slots[index].is_some() {
                        self.error_code(
                            arg.span,
                            codes::DUPLICATE_ARGUMENT,
                            format!("parameter `{name}` is given twice"),
                        );
                        return None;
                    }
                    slots[index] = Some(arg.value.clone());
                }
            }
        }
        let mut out = Vec::with_capacity(n + extras.len());
        for (index, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(value) => out.push(value),
                None => match param_defaults.get(index).and_then(|d| d.as_ref()) {
                    Some(default) => out.push((**default).clone()),
                    None if variadic => {
                        self.error_code(
                            span,
                            codes::WRONG_ARGUMENT_COUNT,
                            format!("expected at least {} arguments, found {}", n, args.len()),
                        );
                        return None;
                    }
                    None => {
                        self.error_code(
                            span,
                            codes::WRONG_ARGUMENT_COUNT,
                            format!(
                                "expected {n} arguments, found {}: parameter `{}` has no default",
                                args.len(),
                                param_names[index]
                            ),
                        );
                        return None;
                    }
                },
            }
        }
        out.extend(extras);
        Some(out)
    }

    /// Check a struct literal over a generic type: `Pair { first: 1, ... }`.
    ///
    /// The literal carries no type arguments of its own, so the arguments
    /// come from the written field values: each field's declared type is
    /// unified against the value written (or against the spread's
    /// monomorph, when there is one). From there the literal follows the
    /// concrete path against the interned monomorph.
    fn check_scheme_struct_lit(
        &mut self,
        name: &str,
        path: &[String],
        fields: &[(String, Expr, Span)],
        spread: &Option<Box<Expr>>,
        span: Span,
    ) -> Option<HirExpr> {
        // The scheme is an item: it is private to its module unless `pub`.
        self.check_visibility(name, span);
        // Every value the literal wrote is checked; the unification below
        // reads the checked types.
        let mut written: Vec<(String, HirExpr, Span)> = Vec::new();
        for (fname, value, fspan) in fields {
            let value = self.check_expr(value)?;
            written.push((fname.clone(), value, *fspan));
        }
        let spread_checked = match spread {
            Some(operand) => {
                let checked = self.check_expr(operand)?;
                let is_ours = matches!(&expr_ty(&checked).kind, TypeKind::Struct(mangled)
                    if self.monomorph_schemes.get(mangled).map(String::as_str) == Some(name));
                if !is_ours {
                    self.mismatch_message(
                        expr_span(&checked),
                        format!("a spread for `{name}` must be a `{name}[..]` value"),
                    );
                    return None;
                }
                Some(checked)
            }
            None => None,
        };
        let mut subst: HashMap<String, Type> = HashMap::new();
        if let Some(spread_value) = &spread_checked {
            // A spread of another instantiation gives the arguments whole.
            if let TypeKind::Struct(mangled) = &expr_ty(spread_value).kind {
                if let Some(arg_tys) = self.type_monomorphs.get(mangled) {
                    let arg_tys = arg_tys.clone();
                    let params = self.struct_schemes[name].type_params.clone();
                    for (param, ty) in params.iter().zip(arg_tys.iter()) {
                        subst.insert(param.name.clone(), ty.clone());
                    }
                }
            }
        }
        {
            let scheme = self.struct_schemes[name].clone();
            for (fname, value, _) in &written {
                let Some(field) = scheme.fields.iter().find(|f| &f.name == fname) else {
                    continue;
                };
                let _ =
                    self.unify_type(&field.ty, &scheme.type_params, &mut subst, &expr_ty(value));
            }
        }
        let mut ordered = Vec::with_capacity(self.struct_schemes[name].type_params.len());
        for param in &self.struct_schemes[name].type_params {
            match subst.get(&param.name) {
                Some(concrete) => ordered.push(concrete.clone()),
                None => {
                    self.error(
                        span,
                        format!("cannot infer type argument for `{}`", param.name),
                    );
                    return None;
                }
            }
        }
        let mangled = mangled_generic_name(name, &ordered);
        self.intern_monomorph(
            name,
            &self.struct_schemes[name].type_params.clone(),
            &[],
            ordered,
            span,
        );
        let sig = match self.structs.get(&mangled).cloned() {
            Some(sig) => sig,
            None => return None,
        };
        let provided: Vec<&str> = written.iter().map(|(n, _, _)| n.as_str()).collect();
        for (index, (fname, _)) in sig.fields.iter().enumerate() {
            if !provided.contains(&fname.as_str())
                && spread_checked.is_none()
                && sig.defaults.get(index).map(|d| d.is_none()).unwrap_or(true)
            {
                self.error_code(
                    span,
                    codes::MISSING_FIELD,
                    format!("missing field `{fname}` in struct `{name}`: it has no default"),
                );
            }
        }
        for (fname, _value, _) in &written {
            if !sig.fields.iter().any(|(n, _)| n == fname) {
                self.error_code(
                    span,
                    codes::UNKNOWN_FIELD,
                    format!("unknown field `{fname}` in struct `{name}`"),
                );
            }
        }
        // Every written field has the type the monomorph declares for it:
        // inference reads the fields, so a conflict here is the failure the
        // unification anticipated.
        for (fname, fty) in &sig.fields {
            if let Some((_, value, _)) = written.iter().find(|(n, _, _)| n == fname) {
                let value_ty = expr_ty(value);
                if !fty.eq_ignoring_span(&value_ty) {
                    self.mismatch_message(
                        expr_span(value),
                        format!("expected `{fty}`, found `{value_ty}`"),
                    );
                }
                if let Some(index) = sig.fields.iter().position(|(n, _)| n == fname) {
                    self.check_field_visibility(&mangled, fname, &sig, index, span);
                }
            }
        }
        let _ = path;
        let hir_fields =
            self.fill_struct_fields(&mangled, &sig, &written, spread_checked.as_ref(), span);
        Some(HirExpr::StructLit {
            path: vec![mangled.clone()],
            fields: hir_fields,
            span,
            ty: Type {
                kind: TypeKind::Struct(mangled),
                span,
            },
        })
    }

    /// Build a struct literal's fields in declaration order.
    ///
    /// Precedence has exactly one implementation: an explicit field wins,
    /// then the spread, then (Task 7) a default, then `E0201`. A field
    /// written twice is `E0202`.
    fn fill_struct_fields(
        &mut self,
        name: &str,
        sig: &StructSig,
        written: &[(String, HirExpr, Span)],
        spread: Option<&HirExpr>,
        span: Span,
    ) -> Vec<(String, HirExpr, Span)> {
        let mut seen: Vec<&str> = Vec::new();
        for (fname, _, fspan) in written {
            if seen.contains(&fname.as_str()) {
                self.error_code(
                    *fspan,
                    codes::DUPLICATE_FIELD,
                    format!("duplicate field `{fname}` in struct `{name}`"),
                );
            }
            seen.push(fname.as_str());
        }
        let mut hir_fields = Vec::new();
        for (index, (fname, fty)) in sig.fields.iter().enumerate() {
            if let Some((_, value, value_span)) = written.iter().find(|(n, _, _)| n == fname) {
                hir_fields.push((fname.clone(), value.clone(), *value_span));
            } else if let Some(spread) = spread {
                hir_fields.push((
                    fname.clone(),
                    HirExpr::FieldAccess {
                        base: Box::new(spread.clone()),
                        field: fname.clone(),
                        index,
                        span: expr_span(spread),
                        ty: fty.clone(),
                    },
                    span,
                ));
            } else if let Some(Some(default)) = sig.defaults.get(index) {
                hir_fields.push((fname.clone(), default.clone(), span));
            }
        }
        hir_fields
    }

    /// Check a lambda, against an expected signature when one is known.
    ///
    /// A parameter that wrote its type is resolved as written and must agree
    /// with the expected one; a parameter that wrote only its name takes the
    /// expected type. The return type works the same way, except that an
    /// expected signature with no return type (the `map` callback) leaves the
    /// return to be inferred from the body's first `return`.
    fn check_lambda(
        &mut self,
        params: &[LambdaParam],
        ret: &Option<TypeExpr>,
        body: &Block,
        span: Span,
        expected: Option<(&[Type], Option<&Type>)>,
    ) -> Option<HirExpr> {
        if let Some((exp_params, _)) = expected {
            if exp_params.len() != params.len() {
                self.error_code(
                    span,
                    codes::TYPE_MISMATCH,
                    format!(
                        "expected a function of {} argument{}, found {}",
                        exp_params.len(),
                        if exp_params.len() == 1 { "" } else { "s" },
                        params.len()
                    ),
                );
                return None;
            }
        }
        let mut param_tys = Vec::with_capacity(params.len());
        for (index, param) in params.iter().enumerate() {
            let ty = match &param.ty {
                Some(ty) => {
                    let resolved = self.resolve_or_report(ty, param.span);
                    if let Some((exp_params, _)) = expected {
                        if let Some(exp) = exp_params.get(index) {
                            if !resolved.eq_ignoring_span(exp) {
                                self.mismatch(param.span, exp, &resolved);
                            }
                        }
                    }
                    resolved
                }
                None => match expected.and_then(|(exp, _)| exp.get(index)) {
                    Some(exp) => exp.clone(),
                    None => {
                        self.error_code(
                            param.span,
                            codes::TYPE_MISMATCH,
                            format!(
                                "cannot infer the type of `{}`: write its type, for example \
                                 `int {}`",
                                param.name, param.name
                            ),
                        );
                        return None;
                    }
                },
            };
            param_tys.push(ty);
        }
        let infer_ret = ret.is_none() && matches!(expected, Some((_, None)));
        let ret_ty = match ret {
            Some(ty) => {
                let resolved = self.resolve_or_report_return(ty, span);
                if let Some((_, Some(exp))) = expected {
                    if !resolved.eq_ignoring_span(exp) {
                        self.mismatch(span, exp, &resolved);
                    }
                }
                resolved
            }
            None => match expected {
                Some((_, Some(exp))) => exp.clone(),
                _ => prim(TypeKind::Unit),
            },
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
                    visibility: Visibility::Private,
                },
            );
        }
        // A name that resolves above this depth belongs to an enclosing
        // scope: a capture. The frame's depth is the scope the parameters
        // live in, so the closure's own names are fine and a global is fine,
        // and only an enclosing local is a capture.
        self.closures.push(ClosureFrame {
            scope_depth: self.scope.depth() - 1,
            captures: Vec::new(),
        });
        if infer_ret {
            self.lambda_ret_infer.push(None);
        }
        // There is no async closure: a lambda body is a synchronous function
        // even when written inside an `async fn`.
        self.async_ctx.push(false);
        let checked_body = self.check_block_with_ret(body, &ret_ty);
        self.async_ctx.pop();
        let inferred = if infer_ret {
            self.lambda_ret_infer.pop().flatten()
        } else {
            None
        };
        let frame = self.closures.pop().unwrap_or(ClosureFrame {
            scope_depth: 0,
            captures: Vec::new(),
        });
        self.scope.pop();

        let checked_body = checked_body?;
        let ret_ty = inferred.unwrap_or(ret_ty);
        let ty = Type {
            kind: TypeKind::Func(param_tys.clone(), Box::new(ret_ty.clone())),
            span,
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
                requires: Vec::new(),
                ensures: Vec::new(),
                captures: frame.captures,
                effect: yashy_common::Effect::PURE,
                declared: yashy_common::DeclaredEffect::Inferred,
                is_async: false,
                docs: Vec::new(),
                file: self.file,
                span,
            }),
            span,
            ty,
        })
    }

    /// Check a callback argument and return its checked expression.
    ///
    /// The three higher-order methods all take a function, and all say so the
    /// same way: the argument has to be one, and a value that is not is
    /// reported here rather than deeper in.
    fn check_callback(
        &mut self,
        expr: &Expr,
        span: Span,
        expected: Option<(&[Type], Option<&Type>)>,
    ) -> Option<HirExpr> {
        let checked = match (expr, expected) {
            (
                Expr::Lambda {
                    params,
                    ret,
                    body,
                    span: lambda_span,
                },
                Some((exp_params, exp_ret)),
            ) if params.len() == exp_params.len() => {
                self.check_lambda(params, ret, body, *lambda_span, Some((exp_params, exp_ret)))?
            }
            // A wrong arity is the higher-order method's own diagnostic: let
            // it check the lambda on its own and report the shape.
            _ => self.check_expr(expr)?,
        };
        let ty = expr_ty(&checked);
        if !matches!(ty.kind, TypeKind::Func(_, _)) {
            self.error(span, format!("expected a function, found `{ty}`"));
            return None;
        }
        Some(checked)
    }

    /// Resolve the receiver of an array method to its binding and slice type.
    fn check_array_receiver(&mut self, receiver: &str, span: Span) -> Option<(HirExpr, Type)> {
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
    fn check_array_insert(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
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
                format!("expected `int`, found `{}`", expr_ty(&index)),
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
    fn check_array_remove(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
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
                format!("expected `int`, found `{}`", expr_ty(&index)),
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
    fn check_array_extend(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
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
                    self.report(error);
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
    fn check_array_slice(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
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
                format!("expected `int`, found `{}`", expr_ty(&start)),
            );
            return None;
        }
        let end = self.check_expr_with_expected(&args[1], &prim(TypeKind::I64))?;
        if !fits(&prim(TypeKind::I64), &expr_ty(&end)) {
            self.error(
                expr_span(&end),
                format!("expected `int`, found `{}`", expr_ty(&end)),
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
    fn check_array_set(&mut self, receiver: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
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
                format!("expected `int`, found `{}`", expr_ty(&index)),
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

    /// `p.free()` on a pointer: the one method that consumes its receiver.
    ///
    /// The runtime entry point is still `dealloc`, so the HIR is the call the
    /// free function used to produce. What changes is the spelling: the
    /// receiver must be a binding, because the call moves it out.
    fn check_pointer_free(&mut self, args: &[Expr], span: Span) -> Option<HirExpr> {
        if args.len() != 1 {
            self.error(span, "`free` takes no arguments".to_string());
            return None;
        }
        let Expr::Ident(path, ident_span) = &args[0] else {
            self.error(
                span,
                "`free` needs a pointer binding, not a temporary".to_string(),
            );
            return None;
        };
        if path.len() != 1 {
            self.error(
                *ident_span,
                "`free` needs a pointer binding, not a field access".to_string(),
            );
            return None;
        }
        let receiver = self.check_expr(&args[0])?;
        let binding = path.last().cloned().unwrap_or_default();
        if let Err(error) = self.move_out(&binding, *ident_span) {
            self.report(error);
            return None;
        }
        Some(HirExpr::Call {
            callee: vec!["dealloc".to_string()],
            args: vec![receiver],
            unsafe_call: false,
            span,
            ty: unit(span),
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
                self.report(error);
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
    /// `map[K]V`. That keeps `let m = map_new();` an error instead of guessing
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
                "cannot infer the map's key and value types: write `let map[K]V m = map_new()`",
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
                self.report(error);
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

    /// `block_on(task) -> T`.
    ///
    /// Drives a `Task[T]` to completion on the default executor and yields
    /// `T`. It is the one place a task runs without an enclosing `async fn`,
    /// which is why `main` can drive a task without being `async` itself.
    fn check_block_on(&mut self, args: &[Expr], span: Span) -> Option<HirExpr> {
        self.uses_async = true;
        if args.len() != 1 {
            self.error(
                span,
                format!("`block_on` takes 1 argument, found {}", args.len()),
            );
            return None;
        }
        let task = self.check_expr(&args[0])?;
        let found = expr_ty(&task);
        let TypeKind::Task(inner) = &found.kind else {
            self.error(
                expr_span(&task),
                format!(
                    "`block_on` needs a `Task[T]`, found `{found}`: `block_on` drives a task to its result"
                ),
            );
            return None;
        };
        let result = (**inner).clone();
        Some(HirExpr::Call {
            callee: vec!["block_on".to_string()],
            args: vec![task],
            unsafe_call: false,
            span,
            ty: result,
        })
    }

    /// `thread_spawn(body, args...)`.
    ///
    /// The first argument names an ordinary function, and the rest are checked
    /// against its parameters exactly as a direct call's arguments would be.
    /// The body must return `void`: a thread has no caller to hand a value back
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
        let Some(body_ty) = self
            .scope
            .resolve(&body_name)
            .map(|binding| binding.ty.clone())
        else {
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
                format!("a thread body must return `void`; `{body_name}` returns `{ret}`"),
            );
            return None;
        }
        if params.len() != rest.len() {
            self.error_code(
                span,
                codes::WRONG_ARGUMENT_COUNT,
                format!("expected {} arguments, found {}", params.len(), rest.len()),
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
    /// The handle is an `int` at the source level even though the runtime ABI
    /// is `i32`: ordinary Yashy code cannot spell a C width, and a handle is
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
                format!("expected `int`, found `{found}`"),
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
                self.report(error);
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
                format!("expected `int`, found `{found}`"),
            );
            return None;
        }
        let Some(expected) = expected else {
            self.error(
                span,
                "cannot infer the channel's element type: write `let Channel[i64] c = chan_new(4)`",
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
                self.report(error);
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
            Expr::Literal(Literal::Char(byte), span) => Some(HirExpr::Literal(
                Literal::Char(*byte),
                *span,
                prim(TypeKind::Char),
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
                // fields, so a dotted name whose head names one cannot be
                // anything else. The head may itself be qualified, as in
                // `io.Error.NotFound`.
                if path.len() >= 2 {
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
                    self.report(e);
                    return None;
                }
                match self.lookup(&name, *span) {
                    NameLookup::Found(binding) => {
                        // `main` is the entry point, and it is emitted with the
                        // operating system's `i32` signature rather than its
                        // Yashy one. It can be called; it cannot be a value.
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
                        self.error_missing_value(&name, *span);
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
                        // Arithmetic is checked on `int` only (the other widths
                        // are reached through a cast and keep their C
                        // behaviour); a shift is checked on every integer
                        // width, because the count has to be inside the
                        // storage type's bits.
                        let checked = if matches!(op, BinOp::Shl | BinOp::Shr) {
                            ty.kind.bit_width().is_some()
                        } else {
                            is_checked_arithmetic(*op) && matches!(ty.kind, TypeKind::I64)
                        };
                        // A literal count outside the type's range in a `const`
                        // initializer is known to fault, so it is rejected here
                        // rather than left to a run-time check that can never
                        // pass. Ordinary code keeps the run-time check, so both
                        // build modes agree about the same expression.
                        if matches!(op, BinOp::Shl | BinOp::Shr) && self.in_const_init {
                            if let (Some(count), Some(bits)) =
                                (const_int(&rhs), ty.kind.bit_width())
                            {
                                if count < 0 || count as i128 >= bits as i128 {
                                    self.error_code(
                                        *span,
                                        codes::SHIFT_OUT_OF_RANGE,
                                        format!(
                                            "shift amount {count} is out of range for {} (0..{})",
                                            ty.kind,
                                            bits - 1
                                        ),
                                    );
                                }
                            }
                        }
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
                        // Operators cannot be overloaded: `a + b` on a
                        // type with an `add` method is E1515, not the
                        // generic operand error.
                        let op_method = match op {
                            BinOp::Add => "add",
                            BinOp::Sub => "sub",
                            BinOp::Mul => "mul",
                            BinOp::Div => "div",
                            BinOp::Rem => "rem",
                            _ => "",
                        };
                        if !op_method.is_empty() {
                            if let TypeKind::Struct(name) | TypeKind::Enum(name) = &lhs_ty.kind {
                                let has_method = self.methods.keys().any(|symbol| {
                                    symbol.starts_with(&format!("{name}."))
                                        && symbol.ends_with(&format!(".{op_method}"))
                                });
                                if has_method {
                                    self.error_code(
                                        *span,
                                        codes::OPERATOR_OVERLOAD,
                                        format!(
                                            "`{}` is not defined for `{name}`: call the method `a.{op_method}(b)` instead",
                                            binop_symbol(*op)
                                        ),
                                    );
                                    return None;
                                }
                            }
                        }
                        match op {
                            BinOp::BitAnd
                            | BinOp::BitOr
                            | BinOp::BitXor
                            | BinOp::Shl
                            | BinOp::Shr => {
                                let code = if lhs_ty.kind != rhs_ty.kind {
                                    codes::BITWISE_MIXED_TYPES
                                } else {
                                    codes::BITWISE_NOT_INTEGER
                                };
                                self.error_code(*span, code, message);
                            }
                            _ => self.error(*span, message),
                        }
                        None
                    }
                }
            }
            Expr::CompareChain {
                operands,
                ops,
                span,
            } => {
                // `a < b <= c` is `a < b && b <= c` with each operand
                // evaluated once: nested binding switches bind every operand,
                // then the adjacent comparisons are joined with `&&`.
                for op in ops {
                    if matches!(op, BinOp::Eq | BinOp::Ne) {
                        self.error_code(
                            *span,
                            codes::CHAINED_EQUALITY,
                            format!(
                                "`{}` cannot appear in a comparison chain: write the two \
                                 comparisons separately with `&&`",
                                op.symbol()
                            ),
                        );
                        return None;
                    }
                }
                let mut checked: Vec<HirExpr> = Vec::with_capacity(operands.len());
                let mut types: Vec<Type> = Vec::with_capacity(operands.len());
                for operand in operands {
                    let value = self.check_expr(operand)?;
                    types.push(expr_ty(&value));
                    checked.push(value);
                }
                for (index, op) in ops.iter().enumerate() {
                    let operand_ty = &types[index];
                    if !matches!(
                        operand_ty.kind,
                        TypeKind::I64 | TypeKind::F64 | TypeKind::Char
                    ) {
                        self.error_code(
                            *span,
                            codes::TYPE_MISMATCH,
                            format!(
                                "cannot compare `{operand_ty}` in a chain: the operands must \
                                 all be `int`, `float` or `char`"
                            ),
                        );
                        return None;
                    }
                    if let Err(message) = binary_result_type(*op, &types[index], &types[index + 1])
                    {
                        self.error_code(*span, codes::TYPE_MISMATCH, message);
                        return None;
                    }
                }
                let bool_ty = prim(TypeKind::Bool);
                // The innermost body: adjacent comparisons over the bindings.
                let mut body: Option<HirExpr> = None;
                for (index, op) in ops.iter().enumerate() {
                    let comparison = HirExpr::Binary {
                        op: *op,
                        lhs: Box::new(HirExpr::Ident(
                            vec![format!("__c{index}")],
                            *span,
                            types[index].clone(),
                        )),
                        rhs: Box::new(HirExpr::Ident(
                            vec![format!("__c{}", index + 1)],
                            *span,
                            types[index + 1].clone(),
                        )),
                        span: *span,
                        ty: bool_ty.clone(),
                        fault: None,
                    };
                    body = Some(match body {
                        None => comparison,
                        Some(previous) => HirExpr::Binary {
                            op: BinOp::And,
                            lhs: Box::new(previous),
                            rhs: Box::new(comparison),
                            span: *span,
                            ty: bool_ty.clone(),
                            fault: None,
                        },
                    });
                }
                let mut result = body.expect("a chain has at least two operands");
                for index in (0..checked.len()).rev() {
                    let subject = checked.pop().expect("operand count matches ops");
                    result = HirExpr::SwitchExpr {
                        subject: Box::new(subject),
                        arms: vec![HirSwitchArm {
                            pattern: HirPattern::Binding {
                                name: format!("__c{index}"),
                                ty: types[index].clone(),
                                mutability: Mutability::Const,
                                span: *span,
                            },
                            guard: None,
                            body: result,
                            is_default: true,
                            span: *span,
                        }],
                        span: *span,
                        ty: bool_ty.clone(),
                    };
                }
                Some(result)
            }
            Expr::Unary { op, operand, span } => {
                let operand = self.check_expr(operand)?;
                let operand_ty = expr_ty(&operand);
                let result = match op {
                    UnOp::Neg if is_numeric(&operand_ty) => Ok(operand_ty),
                    UnOp::Not if operand_ty.eq_ignoring_span(&prim(TypeKind::Bool)) => {
                        Ok(prim(TypeKind::Bool))
                    }
                    UnOp::BitNot if operand_ty.kind.is_integer() => Ok(operand_ty),
                    UnOp::Neg => Err(format!("cannot apply `-` to `{operand_ty}`")),
                    UnOp::Not => Err(format!("cannot apply `!` to `{operand_ty}`")),
                    UnOp::BitNot => Err(format!("cannot apply `~` to `{operand_ty}`")),
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
                        let code = match op {
                            UnOp::BitNot => codes::BITWISE_NOT_INTEGER,
                            _ => codes::TYPE_MISMATCH,
                        };
                        self.error_code(*span, code, message);
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
                            self.report(e);
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
                if let Some(converted) =
                    self.check_scalar_method(receiver_checked.clone(), method, args, *span)
                {
                    return Some(converted);
                }
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
            Expr::TypePathCall {
                type_name,
                name,
                args,
                span,
            } => {
                // `Point::new(1, 2)` or `Show::show(p)` (`p`, `Show`):
                // resolved like an associated call, or a qualified receiver
                // call, through the one impl table the method call uses.
                let arg_vals: Vec<Expr> = args.iter().map(|a| a.value.clone()).collect();
                self.check_type_path_call(type_name, name, &arg_vals, *span)
            }
            Expr::TypePath {
                type_name,
                name,
                span,
            } => {
                // An associated function as a value is not formed in v1:
                // only the call through it is supported.
                let _ = type_name;
                self.error(
                    *span,
                    format!(
                        "`{type_name}::{name}` is an associated function: call it in \
                         parentheses"
                    ),
                );
                None
            }
            Expr::Call {
                callee,
                args,
                unsafe_call,
                span,
            } => {
                let arg_vals: Vec<Expr> = args.iter().map(|a| a.value.clone()).collect();
                // `Enum.Variant(args)` constructs a user enum. Checked first:
                // the ordinary path resolves only the last segment, so `A` in
                // `Shape.Circle(...)` would otherwise be reported as an unknown
                // function and `Shape` as an unknown value.
                if callee.len() >= 2 {
                    if let Some(constructed) = self.check_enum_call(callee, &arg_vals, *span) {
                        return constructed;
                    }
                }
                // An aliased module path: `io.print(..)` where `io` is this
                // file's alias for the module named by the import path's last
                // segment. The head must not be a local binding (that would be
                // a receiver method, handled below); a *function* binding
                // does not count, because a module can share its name with a
                // builtin -- `args.all()` must reach `args.ya`'s `all`, not the
                // builtin `args()`'s (absent) `all` method. Resolve the tail as
                // an ordinary unqualified call after the visibility check for
                // the target module.
                let head_is_function = self
                    .scope
                    .resolve(&callee[0])
                    .is_some_and(|binding| matches!(binding.ty.kind, TypeKind::Func(_, _)));
                if callee.len() == 2
                    && (self.scope.resolve(&callee[0]).is_none() || head_is_function)
                {
                    let aliased = self
                        .aliases
                        .iter()
                        .find(|(file, _)| *file == self.file)
                        .and_then(|(_, table)| table.get(&callee[0]).cloned());
                    if let Some(target_module) = aliased {
                        let name = callee[1].clone();
                        if let Some(origin) = self.item_origins.get(&name).copied() {
                            if self.module_name(origin.file) != target_module {
                                self.error_code(
                                    *span,
                                    codes::UNKNOWN_NAME,
                                    format!("`{name}` is not declared in module `{target_module}`"),
                                );
                                return None;
                            }
                            self.check_visibility(&name, *span);
                        }
                        if self.errors.iter().any(|(_, e)| e.span == *span) {
                            return None;
                        }
                        let single = Expr::Call {
                            callee: vec![name],
                            args: args.clone(),
                            unsafe_call: *unsafe_call,
                            span: *span,
                        };
                        return self.check_expr(&single);
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
                if let Some(checked) = self.check_builtin_method(callee, &arg_vals, *span) {
                    return checked;
                }
                // `print` is the output form. It takes any number of values
                // of any printable type (or none), converts each to a string
                // internally, and writes one line. The old names are handled
                // by the migration table below.
                if callee.len() == 1 {
                    let call_name = callee.last().cloned().unwrap_or_default();
                    match call_name.as_str() {
                        "print" => {
                            // `print` is the main, Python-style output: any
                            // number of values, converted to text and joined
                            // with one space, then a newline. `println` was
                            // removed; the checker names the fix.
                            let mut parts: Vec<HirExpr> = Vec::with_capacity(arg_vals.len());
                            for arg in &arg_vals {
                                let checked = match self.check_expr(arg) {
                                    Some(c) => c,
                                    None => return None,
                                };
                                parts.push(self.as_string(checked, *span)?);
                            }
                            let mut joined: Option<HirExpr> = None;
                            for part in parts {
                                joined = Some(match joined {
                                    None => part,
                                    Some(previous) => {
                                        let separator = HirExpr::Literal(
                                            Literal::Str(str_lit(" ".to_string())),
                                            *span,
                                            prim(TypeKind::Str),
                                        );
                                        HirExpr::Binary {
                                            op: BinOp::Add,
                                            lhs: Box::new(previous),
                                            rhs: Box::new(HirExpr::Binary {
                                                op: BinOp::Add,
                                                lhs: Box::new(separator),
                                                rhs: Box::new(part),
                                                span: *span,
                                                ty: prim(TypeKind::Str),
                                                fault: None,
                                            }),
                                            span: *span,
                                            ty: prim(TypeKind::Str),
                                            fault: None,
                                        }
                                    }
                                });
                            }
                            let joined = joined.unwrap_or(HirExpr::Literal(
                                Literal::Str(str_lit(String::new())),
                                *span,
                                prim(TypeKind::Str),
                            ));
                            return Some(HirExpr::Call {
                                callee: vec!["print_line_str".to_string()],
                                args: vec![joined],
                                unsafe_call: false,
                                span: *span,
                                ty: prim(TypeKind::Unit),
                            });
                        }
                        "println" => {
                            self.error(
                                *span,
                                "`println` was removed: use `print`, which takes any number of \
                                 values and writes a newline"
                                    .to_string(),
                            );
                            return None;
                        }
                        _ => {}
                    }
                }
                let name = callee.last().cloned().unwrap_or_default();
                // `sizeof`, `alignof` and `offsetof` are compile-time queries
                // about a type's layout, not calls: the answer is a constant,
                // so a program may use one in a `const` initializer and the
                // backend never sees a function that does not exist. Their
                // argument is a type *name*, which is why they are read here
                // before the arguments are checked as expressions.
                if callee.len() == 1 && matches!(name.as_str(), "sizeof" | "alignof" | "offsetof") {
                    return self.check_layout_query(&name, &arg_vals, *span);
                }
                // A generic function is found by name like an ordinary one, but
                // its signature has type variables: the substitution comes from
                // the argument types, and the body is checked at that
                // substitution rather than here.
                if callee.len() == 1 && self.generics.contains_key(&name) {
                    // A generic function is an item, so it is private to the
                    // module that declares it unless it is `pub`.
                    self.check_visibility(&name, *span);
                    return self.check_generic_call(&name, &arg_vals, *span);
                }
                // The method surface replaced these free-function spellings.
                // They are not aliases: a call written this way points at the
                // method form with the receiver the caller already wrote. This
                // is checked before constructors so `push`/`len` cannot fall
                // through to a scope lookup that no longer has them.
                if callee.len() == 1 {
                    if let Some((method, arity)) = old_builtin_method_name(&name) {
                        if arg_vals.len() == arity {
                            self.report_old_builtin(&name, method, &arg_vals, *span);
                            return None;
                        }
                    }
                }
                // `None`, `Some(x)`, `Ok(v)` and `Err(e)` are value
                // constructors, not calls. They resolve by name and arity,
                // not by scope lookup — `None` takes no arguments and the
                // others take exactly one.
                if callee.len() == 1 {
                    if let Some(constructed) =
                        self.check_constructor(&name, &arg_vals, *span, None)?
                    {
                        return Some(constructed);
                    }
                    if name == "map_new" {
                        return self.check_map_new(&arg_vals, *span, None);
                    }
                    if name == "block_on" {
                        return self.check_block_on(&arg_vals, *span);
                    }
                    if matches!(
                        name.as_str(),
                        "thread_spawn" | "thread_join" | "mutex_new" | "chan_new"
                    ) {
                        return self.check_thread_builtin(&name, &arg_vals, *span).flatten();
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
                        return self.check_capture_call(index, ty, &arg_vals, *span);
                    }
                    NameLookup::Missing => None,
                };
                match resolved {
                    Some(ty) => match ty.kind {
                        TypeKind::Func(params, ret) => {
                            // A variadic `extern` accepts any number of
                            // arguments beyond its fixed parameters; C checks
                            // the fixed ones and leaves the rest to the
                            // caller. Everything else is exact.
                            let variadic = self.variadic_externs.contains(&name);
                            // Named and default arguments are resolved here,
                            // at the call site, to positional expressions in
                            // parameter order. The HIR call below is positional
                            // either way, so no later pass learns about names.
                            // A callee with no signature on file (a first-class
                            // function value, an `extern` reached by type) has
                            // no names to resolve against: a named argument
                            // there is E0701.
                            let arg_vals: Vec<Expr> = match self.fns.get(&name).cloned() {
                                Some(sig) => match self.resolve_named_args(
                                    args,
                                    &sig.param_names,
                                    &sig.param_defaults,
                                    *span,
                                    variadic,
                                ) {
                                    Some(resolved_args) => resolved_args,
                                    None => return None,
                                },
                                None => {
                                    if let Some(named) = args.iter().find(|a| a.name.is_some()) {
                                        self.error_code(
                                            named.span,
                                            codes::UNKNOWN_NAMED_ARGUMENT,
                                            format!(
                                                "unknown named argument `{}`: `{name}` declares no named parameters",
                                                named.name.as_deref().unwrap_or_default()
                                            ),
                                        );
                                        return None;
                                    }
                                    arg_vals.clone()
                                }
                            };
                            if variadic {
                                if arg_vals.len() < params.len() {
                                    self.error_code(
                                        *span,
                                        codes::WRONG_ARGUMENT_COUNT,
                                        format!(
                                            "expected at least {} arguments, found {}",
                                            params.len(),
                                            arg_vals.len()
                                        ),
                                    );
                                    return None;
                                }
                            } else if params.len() != arg_vals.len() {
                                self.error_code(
                                    *span,
                                    codes::WRONG_ARGUMENT_COUNT,
                                    format!(
                                        "expected {} arguments, found {}",
                                        params.len(),
                                        arg_vals.len()
                                    ),
                                );
                                return None;
                            }
                            // `assert_eq` and `assert_ne` compare two values of
                            // *the same* type, which is any of the four with an
                            // equality, so they are built before the declared
                            // parameter types are applied and the table's `int`
                            // pair is never consulted. See
                            // `Checker::assert_cmp_call`.
                            if callee.len() == 1 && (name == "assert_eq" || name == "assert_ne") {
                                return self.assert_cmp_call(&name, &arg_vals, *span);
                            }
                            let mut arg_exprs = Vec::with_capacity(arg_vals.len());
                            for (i, (arg, param)) in arg_vals.iter().zip(params.iter()).enumerate()
                            {
                                let checked = self.check_expr_with_expected(arg, param)?;
                                if !fits(param, &expr_ty(&checked)) {
                                    self.error(
                                        expr_span(&checked),
                                        format!(
                                            "expected `{param}`, found `{}`",
                                            expr_ty(&checked)
                                        ),
                                    );
                                }
                                // A parameter of linear type takes ownership, so
                                // passing a bare name moves it out --
                                // `buf.free()`, and a user function taking
                                // `*mut T`, both consume without a `move` at the
                                // call site. An argument that is not a bare name
                                // (a fresh `alloc(...)`, say) owns nothing yet and
                                // is left alone.
                                //
                                // Two builtins are the exception, and the fact
                                // lives in `yashy-hir` because the verifier's
                                // leak pass needs the same answer:
                                // `arena_alloc` reuses its handle across many
                                // allocations, and `write_bytes` only reads the
                                // bytes it is handed.
                                let consumes = !builtin_borrows_argument(callee, i);
                                if consumes
                                    && matches!(linearity_of(&param.kind), Linearity::Linear)
                                {
                                    // Only a bare name is a binding that can be
                                    // moved. A field access is a header that
                                    // points at storage the struct does not own
                                    // (tutorial 04), so it is not a move and
                                    // there is no binding named by its last
                                    // segment to look up.
                                    if let Expr::Ident(path, arg_span) = arg {
                                        if path.len() == 1 {
                                            let arg_name = path.last().cloned().unwrap_or_default();
                                            if let Err(e) = self.move_out(&arg_name, *arg_span) {
                                                self.report(e);
                                            }
                                        }
                                    }
                                }
                                arg_exprs.push(checked);
                            }
                            // The arguments past the fixed parameters are the
                            // variadic tail. C does not check their types, so
                            // Yashy does not either -- but it does insist the
                            // program wrote a type C can pass unchanged, and it
                            // makes the call say `unsafe`, because the callee's
                            // format string decides how these bytes are read.
                            if variadic {
                                for arg in arg_vals.iter().skip(params.len()) {
                                    let checked = self.check_expr(arg)?;
                                    self.check_variadic_argument(&checked);
                                    arg_exprs.push(checked);
                                }
                                if arg_vals.len() > params.len() && !*unsafe_call {
                                    self.error(
                                        *span,
                                        format!(
                                            "calling `{name}` with variadic arguments requires an \
                                             `unsafe` call: C does not check their types"
                                        ),
                                    );
                                }
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
                        // Migration table for retired builtins.
                        let last_name = callee.last().cloned().unwrap_or_default();
                        let migration_msg = match last_name.as_str() {
                            "print_str" => Some("`print_str` was replaced: use `print`"),
                            "print_i64" => Some("`print_i64` was replaced"),
                            "print_f64" => Some("`print_f64` was replaced"),
                            _ => None,
                        };
                        if let Some(msg) = migration_msg {
                            self.error(*span, msg.to_string());
                            return None;
                        }
                        self.error_missing_function(&last_name, *span);
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
                        for arg in &arg_vals {
                            let _ = self.check_expr(arg);
                        }
                        None
                    }
                }
            }
            Expr::StructLit {
                path,
                fields,
                spread,
                span,
            } => {
                // `Shape.Rect { w, h }` constructs a variant with named fields.
                if path.len() >= 2 {
                    if spread.is_some() {
                        self.mismatch_message(
                            *span,
                            "a spread is only allowed in a struct literal".to_string(),
                        );
                        return None;
                    }
                    if let Some(constructed) = self.check_enum_struct_lit(path, fields, *span) {
                        return constructed;
                    }
                }
                let name = path.last().cloned().unwrap_or_default();
                // A struct literal naming a generic type infers its type
                // arguments from the written field values: `Pair { first:
                // 1, second: 2 }` is `Pair[i64]`.
                if !self.structs.contains_key(&name) && self.struct_schemes.contains_key(&name) {
                    return self.check_scheme_struct_lit(&name, path, fields, spread, *span);
                }
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
                        let mut written: Vec<(String, HirExpr, Span)> = Vec::new();
                        for (fname, value, fspan) in fields {
                            let value = self.check_expr(value)?;
                            written.push((fname.clone(), value, *fspan));
                        }
                        let spread_checked = match spread {
                            Some(operand) => {
                                let checked = self.check_expr(operand)?;
                                let expected = Type {
                                    kind: TypeKind::Struct(name.clone()),
                                    span: *span,
                                };
                                if !expr_ty(&checked).eq_ignoring_span(&expected) {
                                    self.mismatch(
                                        expr_span(&checked),
                                        &expected,
                                        &expr_ty(&checked),
                                    );
                                }
                                if let Expr::Ident(spread_path, spread_span) = operand.as_ref() {
                                    if spread_path.len() == 1 {
                                        if let Err(error) =
                                            self.move_out_spread(&spread_path[0], *spread_span)
                                        {
                                            self.report(error);
                                        }
                                    }
                                }
                                Some(checked)
                            }
                            None => None,
                        };
                        let provided: Vec<&str> =
                            written.iter().map(|(n, _, _)| n.as_str()).collect();
                        for (index, (fname, _)) in sig.fields.iter().enumerate() {
                            if !provided.contains(&fname.as_str())
                                && spread_checked.is_none()
                                && !sig.defaults[index].is_some()
                            {
                                self.error_code(
                                    *span,
                                    codes::MISSING_FIELD,
                                    format!(
                                        "missing field `{fname}` in struct `{name}`: it has no default"
                                    ),
                                );
                            }
                        }
                        for (fname, fty) in &sig.fields {
                            if let Some((_, value, _)) = written.iter().find(|(n, _, _)| n == fname)
                            {
                                let value_ty = expr_ty(value);
                                if !fty.eq_ignoring_span(&value_ty) {
                                    self.mismatch_message(
                                        expr_span(value),
                                        format!("expected `{fty}`, found `{value_ty}`"),
                                    );
                                }
                            }
                        }
                        for (fname, _, _) in &written {
                            if !sig.fields.iter().any(|(n, _)| n == fname) {
                                let mut error = TypeError::new(
                                    *span,
                                    format!("unknown field `{fname}` in struct `{name}`"),
                                )
                                .with_code(codes::UNKNOWN_FIELD);
                                if let Some(suggestion) = self.struct_field_suggestion(&name, fname)
                                {
                                    error =
                                        error.with_label(format!("did you mean `{suggestion}`?"));
                                }
                                self.errors.push((self.file, error));
                                continue;
                            }
                            // A field that is not `pub` may not be written from
                            // another module.
                            if let Some(index) = sig.fields.iter().position(|(n, _)| n == fname) {
                                self.check_field_visibility(&name, fname, &sig, index, *span);
                            }
                        }
                        // Laid out in **declaration** order, not the order the
                        // literal spelled the fields in. Two literals of one
                        // struct written differently would otherwise define two
                        // different IR layouts for the same named type, and a
                        // field's index would depend on which literal happened to
                        // be lowered first.
                        let hir_fields = self.fill_struct_fields(
                            &name,
                            &sig,
                            &written,
                            spread_checked.as_ref(),
                            *span,
                        );
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
                        let mut error =
                            TypeError::new(*span, format!("cannot find struct `{name}`"))
                                .with_code(codes::UNKNOWN_TYPE);
                        if let Some(suggestion) = self.type_suggestion(&name) {
                            error = error.with_label(format!("did you mean `{suggestion}`?"));
                        }
                        self.errors.push((self.file, error));
                        None
                    }
                }
            }
            Expr::SliceLit {
                elem,
                elements,
                span,
            } => {
                // The element type is written (`[]T {}`) or inferred from the
                // first value or spread (`[0, ..xs, 4]`).
                if let Some(ty) = elem {
                    self.intern_generics_in(ty);
                }
                let written_elem = match elem {
                    Some(ty) => match self.resolve_type_in(ty, false) {
                        Ok(ty) => Some(ty),
                        Err(error) => {
                            self.report(error);
                            return None;
                        }
                    },
                    None => None,
                };
                if let Some(elem_ty) = &written_elem {
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
                        self.error(*span, "an array cannot hold a function value yet");
                        return None;
                    }
                }
                // First pass: check spreads (their slice fixes the element
                // type when nothing else does) and values in order.
                let mut checked: Vec<HirExpr> = Vec::with_capacity(elements.len());
                let mut elem_ty = written_elem;
                for element in elements {
                    match element {
                        SliceElement::Value(value_expr) => {
                            let value = match &elem_ty {
                                Some(known) => self.check_expr_with_expected(value_expr, known)?,
                                None => self.check_expr(value_expr)?,
                            };
                            let found = expr_ty(&value);
                            match &elem_ty {
                                None => elem_ty = Some(found),
                                Some(known) if !fits(known, &found) => {
                                    self.error(
                                        expr_span(&value),
                                        format!("expected `{known}`, found `{found}`"),
                                    );
                                }
                                _ => {}
                            }
                            checked.push(value);
                        }
                        SliceElement::Spread(operand) => {
                            let source = self.check_expr(operand)?;
                            let source_span = expr_span(&source);
                            let source_ty = expr_ty(&source);
                            let TypeKind::Slice(source_elem) = &source_ty.kind else {
                                self.error(
                                    source_span,
                                    format!("cannot spread `{source_ty}`: expected an array"),
                                );
                                return None;
                            };
                            let source_elem = (**source_elem).clone();
                            match &elem_ty {
                                None => elem_ty = Some(source_elem.clone()),
                                Some(known) if !known.eq_ignoring_span(&source_elem) => {
                                    self.mismatch(source_span, known, &source_elem);
                                }
                                _ => {}
                            }
                            if let Expr::Ident(spread_path, spread_span) = operand.as_ref() {
                                if spread_path.len() == 1 {
                                    if let Err(error) =
                                        self.move_out_spread(&spread_path[0], *spread_span)
                                    {
                                        self.report(error);
                                    }
                                }
                            }
                            // The marker the backend's literal builder
                            // recognises as a spread copy.
                            let marker_ty = Type {
                                kind: TypeKind::Slice(Box::new(source_elem)),
                                span: source_span,
                            };
                            checked.push(HirExpr::Call {
                                callee: vec!["__spread".to_string()],
                                args: vec![source],
                                unsafe_call: false,
                                span: source_span,
                                ty: marker_ty,
                            });
                        }
                    }
                }
                let Some(elem_ty) = elem_ty else {
                    self.error(
                        *span,
                        "cannot infer the element type of an empty array literal: write `[]T {}`",
                    );
                    return None;
                };
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
                // element of a pointer, and always an `int`. `xs[i]` reads an
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
                    self.error(expr_span(&index), "index must be `int`");
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
                            format!(
                                "a pointer to nothing cannot be read through: `*mut {element}`"
                            ),
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
            Expr::IndexRange {
                base,
                start,
                end,
                span,
            } => {
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
                                let mut error = TypeError::new(
                                    *span,
                                    format!("struct `{sname}` has no field `{field}`"),
                                )
                                .with_code(codes::UNKNOWN_FIELD);
                                if let Some(suggestion) = self.struct_field_suggestion(sname, field)
                                {
                                    error =
                                        error.with_label(format!("did you mean `{suggestion}`?"));
                                }
                                self.errors.push((self.file, error));
                                None
                            }
                        }
                    }
                    TypeKind::Enum(ename) => {
                        self.error_code(
                            *span,
                            codes::UNKNOWN_FIELD,
                            format!("enum `{ename}` has no field `{field}`"),
                        );
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
                // and the only way a Yashy program can produce one is to say
                // so. This is also the one place a C spelling is legal outside
                // an `extern` block, because the conversion is explicit.
                //
                // The old wide spellings are the exception: they are not C
                // types the program should reach for, and `allow_c` would
                // otherwise keep `x as i64` working as a second spelling.
                if let TypeExpr::Name(name, name_span) = ty {
                    let replacement = match name.as_str() {
                        "i64" => Some("int"),
                        "f64" => Some("float"),
                        _ => None,
                    };
                    if let Some(replacement) = replacement {
                        self.error(
                            *name_span,
                            format!("`{name}` is spelled `{replacement}` in Yashy"),
                        );
                        return None;
                    }
                }
                let target = self.resolve_or_report_in(ty, *span, true, false);
                let source = expr_ty(&inner);
                // A char is a byte, not a number, and the conversion has a name
                // that says what it does. `as` would be a second spelling.
                if source.kind == TypeKind::Char {
                    self.error(
                        *span,
                        "a `char` is not cast with `as`: use `to_int()` to read its byte",
                    );
                    return None;
                }
                if target.kind == TypeKind::Char {
                    self.error(
                        *span,
                        "`as` cannot produce a `char`: use `to_char()` to convert an `int`",
                    );
                    return None;
                }
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
                // `?` unwraps whichever of the two failure-carrying types the
                // subject is. `Result` propagates its error, so the function
                // around it must return a `Result` that can carry the same
                // error; `Option` propagates `None`, so the function must
                // return an `Option`.
                let ok = match &inner_ty.kind {
                    TypeKind::Result(ok, err) => {
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
                                    let error = TypeError::new(
                                        *span,
                                        "`?` on a `Result` needs a function returning `Result`",
                                    )
                                    .with_code(codes::MIXED_TRY)
                                    .with_note(format!(
                                        "this function returns `{ret}`: use `switch` to handle \
                                         the `Result`, or change the return type"
                                    ));
                                    self.report(error);
                                    return None;
                                }
                            },
                            None => {
                                let error = TypeError::new(
                                    *span,
                                    "`?` on a `Result` needs a function returning `Result`",
                                )
                                .with_code(codes::MIXED_TRY)
                                .with_note(
                                    "use `switch` to handle the `Result`, or change the return type",
                                );
                                self.report(error);
                                return None;
                            }
                        }
                        (**ok).clone()
                    }
                    TypeKind::Option(ok) => {
                        match self.fn_ret.clone() {
                            Some(ret) if matches!(ret.kind, TypeKind::Option(_)) => {}
                            other => {
                                let message =
                                    "`?` on an `Option` needs a function returning `Option`";
                                let error = match other {
                                    Some(ret) => TypeError::new(*span, message)
                                        .with_code(codes::MIXED_TRY)
                                        .with_note(format!(
                                            "this function returns `{ret}`: use `switch` to handle \
                                             the `Option`, or change the return type"
                                        )),
                                    None => TypeError::new(*span, message)
                                        .with_code(codes::MIXED_TRY)
                                        .with_note(
                                            "use `switch` to handle the `Option`, or change the \
                                             return type",
                                        ),
                                };
                                self.report(error);
                                return None;
                            }
                        }
                        (**ok).clone()
                    }
                    _ => {
                        self.error(
                            *span,
                            format!("`?` can only be used on a `Result` or an `Option`, not `{inner_ty}`"),
                        );
                        return None;
                    }
                };
                Some(HirExpr::Try {
                    expr: Box::new(inner),
                    span: *span,
                    ty: ok,
                })
            }
            Expr::Await { expr, span } => {
                self.uses_async = true;
                // `await` suspends the enclosing task, so an enclosing async
                // frame is required. The check runs before the operand so a
                // bare `x.await` in a sync function reports `E1700` rather
                // than a secondary error about `x`.
                if !self.in_async() {
                    self.error_code(
                        *span,
                        codes::AWAIT_OUTSIDE_ASYNC,
                        "`await` outside an `async fn`: make the enclosing function `async`, or drive the task where one already runs",
                    );
                    return None;
                }
                let inner = self.check_expr(expr)?;
                let inner_ty = expr_ty(&inner);
                let ok = match &inner_ty.kind {
                    TypeKind::Task(inner) => (**inner).clone(),
                    _ => {
                        self.error_code(
                            *span,
                            codes::TYPE_MISMATCH,
                            format!(
                                "`await` needs a `Task[T]`, found `{inner_ty}`: `await` drives a task to its result"
                            ),
                        );
                        return None;
                    }
                };
                Some(HirExpr::Await {
                    expr: Box::new(inner),
                    span: *span,
                    ty: ok,
                })
            }
            Expr::Spawn { callee, args, span } => {
                self.uses_async = true;
                if !self.in_async() {
                    self.error_code(
                        *span,
                        codes::AWAIT_OUTSIDE_ASYNC,
                        "`spawn` outside an `async fn`: make the enclosing function `async`, or spawn where one already runs",
                    );
                    return None;
                }
                // The callee names a function value (`spawn(worker, ...)`):
                // keep the checked worker expression for codegen so it can
                // name what to run, check arguments against its parameters,
                // and wrap the worker's own answer as the task's result.
                let checked_callee = self.check_expr(callee)?;
                let callee_ty = expr_ty(&checked_callee);
                let (params, worker_ret) = match &callee_ty.kind {
                    TypeKind::Func(params, ret) => (params.clone(), (**ret).clone()),
                    _ => {
                        self.error_code(
                            *span,
                            codes::TYPE_MISMATCH,
                            format!("`spawn` needs a function, found `{callee_ty}`"),
                        );
                        return None;
                    }
                };
                if params.len() != args.len() {
                    // Named/default arguments resolve like a call's, using the
                    // function's own names when there is a signature on file.
                    let callee_name = match callee.as_ref() {
                        Expr::Ident(path, _) => path.last().cloned().unwrap_or_default(),
                        _ => String::new(),
                    };
                    let resolved = if callee_name.is_empty() {
                        None
                    } else {
                        self.fns.get(&callee_name).cloned().and_then(|sig| {
                            self.resolve_named_args(
                                args,
                                &sig.param_names,
                                &sig.param_defaults,
                                *span,
                                false,
                            )
                        })
                    };
                    let Some(resolved_args) = resolved else {
                        self.error_code(
                            *span,
                            codes::WRONG_ARGUMENT_COUNT,
                            format!("expected {} arguments, found {}", params.len(), args.len()),
                        );
                        return None;
                    };
                    let mut checked_args = Vec::with_capacity(resolved_args.len());
                    for (arg, param) in resolved_args.iter().zip(params.iter()) {
                        let value = self.check_expr_with_expected(arg, param)?;
                        if !fits(param, &expr_ty(&value)) {
                            self.error(
                                expr_span(&value),
                                format!("expected `{param}`, found `{}`", expr_ty(&value)),
                            );
                        }
                        checked_args.push(value);
                    }
                    // The worker's answer is the task's result. An async
                    // worker already answers `Task[T]`, so its inner type is
                    // the result; a synchronous worker's answer is it as-is.
                    let inner = Self::unwrap_async_ret(&worker_ret);
                    let task_ty = Type {
                        kind: TypeKind::Task(Box::new(inner)),
                        span: *span,
                    };
                    return Some(HirExpr::Spawn {
                        callee: Box::new(checked_callee),
                        args: checked_args,
                        span: *span,
                        ty: task_ty,
                    });
                }
                let mut checked_args = Vec::with_capacity(args.len());
                for (arg, param) in args.iter().zip(params.iter()) {
                    let value = self.check_expr_with_expected(&arg.value, param)?;
                    if !fits(param, &expr_ty(&value)) {
                        self.error(
                            expr_span(&value),
                            format!("expected `{param}`, found `{}`", expr_ty(&value)),
                        );
                    }
                    checked_args.push(value);
                }
                let inner = Self::unwrap_async_ret(&worker_ret);
                Some(HirExpr::Spawn {
                    callee: Box::new(checked_callee),
                    args: checked_args,
                    span: *span,
                    ty: Type {
                        kind: TypeKind::Task(Box::new(inner)),
                        span: *span,
                    },
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
            } => self.check_lambda(params, ret, body, *span, None),
            Expr::ArrayComp {
                expr,
                pattern,
                iterable,
                filter,
                span,
            } => {
                let (checked_iterable, iter_ty, element) =
                    self.check_comp_iterable(iterable, *span)?;
                self.check_comp_body(
                    true,
                    None,
                    expr,
                    pattern,
                    checked_iterable,
                    iter_ty,
                    filter.as_deref(),
                    element,
                    *span,
                )
            }
            Expr::MapComp {
                key,
                value,
                pattern,
                iterable,
                filter,
                span,
            } => {
                let (checked_iterable, iter_ty, element) =
                    self.check_comp_iterable(iterable, *span)?;
                self.check_comp_body(
                    false,
                    Some((key, value)),
                    value,
                    pattern,
                    checked_iterable,
                    iter_ty,
                    filter.as_deref(),
                    element,
                    *span,
                )
            }
        }
    }

    /// Check a comprehension's iterable and return its checked form, its
    /// type, and the element type it yields.
    fn check_comp_iterable(
        &mut self,
        iterable: &Expr,
        span: Span,
    ) -> Option<(HirExpr, Type, Type)> {
        let checked = self.check_expr(iterable)?;
        let ty = expr_ty(&checked);
        let element = match &ty.kind {
            TypeKind::Slice(element)
            | TypeKind::FixedArray(element, _)
            | TypeKind::View(element) => Some((**element).clone()),
            TypeKind::Struct(name) | TypeKind::Enum(name) => {
                match self
                    .trait_impls
                    .get(&("Iterator".to_string(), name.clone()))
                    .and_then(|impl_| {
                        impl_
                            .methods
                            .iter()
                            .find(|(m, _)| m == "next")
                            .map(|(_, s)| s.clone())
                    })
                    .and_then(|symbol| self.methods.get(&symbol).cloned())
                {
                    Some(sig) => match &sig.ret.kind {
                        TypeKind::Option(inner) => Some((**inner).clone()),
                        _ => {
                            self.error_code(
                                span,
                                codes::UNSATISFIED_BOUND,
                                format!(
                                    "`{ty}`'s `next` is not `fn next(self: *mut Self) -> Option[T]` as `Iterator` requires"
                                ),
                            );
                            None
                        }
                    },
                    None => {
                        self.error_code(
                            span,
                            codes::UNSATISFIED_BOUND,
                            format!(
                                "`{ty}` does not implement `Iterator` required by comprehension{}",
                                self.impl_needs_line(&ty, "Iterator"),
                            ),
                        );
                        None
                    }
                }
            }
            _ => {
                self.error_code(
                    span,
                    codes::TYPE_MISMATCH,
                    format!("a comprehension over `{ty}` needs an array-like iterable"),
                );
                None
            }
        }?;
        Some((checked, ty, element))
    }

    /// Check a comprehension's element expressions in the pattern's scope.
    fn check_comp_body(
        &mut self,
        array: bool,
        pair: Option<(&Expr, &Expr)>,
        expr: &Expr,
        pattern: &Pattern,
        checked_iterable: HirExpr,
        iter_ty: Type,
        filter: Option<&Expr>,
        element: Type,
        span: Span,
    ) -> Option<HirExpr> {
        // Check the produced expressions with the pattern's names in scope,
        // the way a loop body would see them.
        self.scope.push();
        let _ = self.check_binding_pattern(pattern, &element, Mutability::Const);
        let out_ty = if array {
            let checked = self.check_expr(expr)?;
            let elem_ty = expr_ty(&checked);
            if let Some(filter) = filter {
                let checked_filter = self.check_expr(filter)?;
                if !expr_ty(&checked_filter).eq_ignoring_span(&prim(TypeKind::Bool)) {
                    self.mismatch(
                        expr_span(&checked_filter),
                        &prim(TypeKind::Bool),
                        &expr_ty(&checked_filter),
                    );
                }
            }
            Type {
                kind: TypeKind::Slice(Box::new(elem_ty)),
                span,
            }
        } else {
            let (key, value) = pair.expect("a map comprehension carries key and value");
            let checked_key = self.check_expr(key)?;
            let checked_value = self.check_expr(value)?;
            if let Some(filter) = filter {
                let checked_filter = self.check_expr(filter)?;
                if !expr_ty(&checked_filter).eq_ignoring_span(&prim(TypeKind::Bool)) {
                    self.mismatch(
                        expr_span(&checked_filter),
                        &prim(TypeKind::Bool),
                        &expr_ty(&checked_filter),
                    );
                }
            }
            Type {
                kind: TypeKind::Map(
                    Box::new(expr_ty(&checked_key)),
                    Box::new(expr_ty(&checked_value)),
                ),
                span,
            }
        };
        self.scope.pop();
        let elem_ty = match &out_ty.kind {
            TypeKind::Slice(element) => (**element).clone(),
            _ => element.clone(),
        };
        // Build the builder function: `__comp_N(__xs: IterTy) -> OutTy`.
        let name = format!("__comp_{}_{}", self.file.index(), span.start);
        let param_name = "__xs".to_string();
        let out_name = "__out".to_string();
        let param_ty_expr = type_to_expr(&iter_ty);
        let out_ty_expr = type_to_expr(&out_ty);
        let out_decl = if array {
            Stmt::Let {
                name: out_name.clone(),
                ty: Some(out_ty_expr.clone()),
                init: Expr::SliceLit {
                    elem: Some(Box::new(type_to_expr(&elem_ty))),
                    elements: Vec::new(),
                    span,
                },
                span,
            }
        } else {
            Stmt::Let {
                name: out_name.clone(),
                ty: Some(out_ty_expr.clone()),
                init: Expr::Call {
                    callee: vec!["map_new".to_string()],
                    args: Vec::new(),
                    unsafe_call: false,
                    span,
                },
                span,
            }
        };
        let push = if array {
            Expr::Call {
                callee: vec![out_name.clone(), "push".to_string()],
                args: vec![CallArg {
                    name: None,
                    value: expr.clone(),
                    span,
                }],
                unsafe_call: false,
                span,
            }
        } else {
            let (key, value) = pair.expect("a map comprehension carries key and value");
            Expr::Call {
                callee: vec![out_name.clone(), "insert".to_string()],
                args: vec![
                    CallArg {
                        name: None,
                        value: key.clone(),
                        span,
                    },
                    CallArg {
                        name: None,
                        value: value.clone(),
                        span,
                    },
                ],
                unsafe_call: false,
                span,
            }
        };
        let loop_body = match filter {
            Some(cond) => Block {
                stmts: vec![Stmt::If {
                    init: None,
                    cond: cond.clone(),
                    then_block: Block {
                        stmts: vec![Stmt::Expr(push, span)],
                        span,
                    },
                    else_block: None,
                    span,
                }],
                span,
            },
            None => Block {
                stmts: vec![Stmt::Expr(push, span)],
                span,
            },
        };
        let body = Block {
            stmts: vec![
                out_decl,
                Stmt::ForIn {
                    pattern: pattern.clone(),
                    index: None,
                    iterable: Expr::Ident(vec![param_name.clone()], span),
                    body: loop_body,
                    label: None,
                    span,
                },
                Stmt::Return(Some(Expr::Ident(vec![out_name], span)), span),
            ],
            span,
        };
        self.pending_comps.push(CompJob {
            name: name.clone(),
            params: vec![Param {
                name: param_name,
                ty: param_ty_expr,
                span,
                default: None,
            }],
            param_tys: vec![iter_ty],
            ret_ty: out_ty.clone(),
            body,
            span,
            file: self.file,
        });
        Some(HirExpr::Call {
            callee: vec![name],
            args: vec![checked_iterable],
            unsafe_call: false,
            span,
            ty: out_ty,
        })
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
                format!("expected {} arguments, found {}", params.len(), args.len()),
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
                self.error_code(
                    span,
                    codes::UNKNOWN_FIELD,
                    format!("struct `{struct_name}` has no field `{field}`"),
                );
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
                    let mut error = TypeError::new(
                        span,
                        format!("struct `{struct_name}` has no field `{field}`"),
                    )
                    .with_code(codes::UNKNOWN_FIELD);
                    if let Some(suggestion) = self.struct_field_suggestion(&struct_name, field) {
                        error = error.with_label(format!("did you mean `{suggestion}`?"));
                    }
                    self.errors.push((self.file, error));
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
    /// down: the bottom of the chain has to be a `let` binding (a field of a
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
            self.error_missing_value(&root, yashy_parser::exprs::expr_span(target));
            return None;
        };
        if !binding.mutability.is_mutable() {
            self.error(
                yashy_parser::exprs::expr_span(target),
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
                format!("a pointer index must be an `int`, found `{index_ty}`"),
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
                    let mut error =
                        TypeError::new(*fspan, format!("struct `{sname}` has no field `{field}`"))
                            .with_code(codes::UNKNOWN_FIELD);
                    if let Some(suggestion) = self.struct_field_suggestion(sname, field) {
                        error = error.with_label(format!("did you mean `{suggestion}`?"));
                    }
                    self.errors.push((self.file, error));
                    return None;
                }
            };
            // A field is private to the module that declares it unless it is
            // `pub`, so writing one from another module is the mistake.
            if let Some(sig) = self.structs.get(sname).cloned() {
                self.check_field_visibility(sname, field, &sig, index, *fspan);
            }
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
                let span = yashy_parser::exprs::expr_span(target);
                let names = segments.map(|field| (field.clone(), span)).collect();
                Some((root, names))
            }
            // A one-segment name: the bottom of a `FieldAccess` chain, reached
            // when the chain was built by `move` or a postfix `.`.
            Expr::Ident(path, _) => Some((path.last()?.clone(), Vec::new())),
            other => {
                self.error(
                    yashy_parser::exprs::expr_span(other),
                    "only a `let` binding or one of its fields can be assigned",
                );
                None
            }
        }
    }

    /// Fold `sizeof(T)`, `alignof(T)` or `offsetof(T, f)` to a constant.
    ///
    /// The argument is a **type name**, not an expression: `sizeof(P)` asks
    /// about the type `P`, and there is nothing inside the parentheses to
    /// evaluate. That is why these are read before the arguments are checked
    /// as expressions -- `P` names a type, and checking it as a value would
    /// report that no such value exists.
    ///
    /// The result is `int`: it is a count of bytes, and `int` is the
    /// language's number. It is also the type a `print` accepts, which a
    /// `usize` is not yet.
    fn check_layout_query(&mut self, name: &str, args: &[Expr], span: Span) -> Option<HirExpr> {
        let ty = match name {
            "sizeof" | "alignof" => {
                if args.len() != 1 {
                    self.error_code(
                        span,
                        codes::SIZE_OF_UNKNOWN,
                        format!("`{name}` takes one type name"),
                    );
                    return None;
                }
                self.layout_query_type(name, &args[0], span)?
            }
            _ => {
                if args.len() != 2 {
                    self.error_code(
                        span,
                        codes::OFFSET_OF_NOT_REPR_C,
                        "`offsetof` takes a type name and a field name",
                    );
                    return None;
                }
                self.layout_query_type(name, &args[0], span)?
            }
        };
        if name == "offsetof" {
            return self.check_offsetof(&ty, &args[1], span);
        }
        let Some(layout) = self.layout_of(&ty) else {
            self.error_code(
                span,
                codes::SIZE_OF_UNKNOWN,
                format!(
                    "`{name}` of `{ty}` has no layout: a field names a type this module cannot see"
                ),
            );
            return None;
        };
        let value = if name == "sizeof" {
            layout.size
        } else {
            layout.align
        };
        Some(HirExpr::Literal(
            Literal::I64(i64::try_from(value).unwrap_or(i64::MAX)),
            span,
            prim(TypeKind::I64),
        ))
    }

    /// The type a layout query names, or `None` after reporting `E0107`.
    ///
    /// Only a bare name is a type. A dotted path names no type -- there is no
    /// `module.Type` spelling in the language -- and anything that is not a
    /// name is a value, which these queries do not take.
    fn layout_query_type(&mut self, query: &str, arg: &Expr, call_span: Span) -> Option<Type> {
        let Expr::Ident(path, span) = arg else {
            self.error_code(
                call_span,
                codes::SIZE_OF_UNKNOWN,
                format!("`{query}` takes the name of a type, not a value"),
            );
            return None;
        };
        if path.len() != 1 {
            self.error_code(
                *span,
                codes::SIZE_OF_UNKNOWN,
                format!(
                    "`{query}` names no known type: `{}` is not a type",
                    path.join(".")
                ),
            );
            return None;
        }
        let type_name = &path[0];
        self.check_visibility(type_name, *span);
        match self.resolve_type_in(&TypeExpr::Name(type_name.clone(), *span), true) {
            Ok(ty) => Some(ty),
            Err(_) => {
                // The resolution failure is not reported as its own diagnostic:
                // `sizeof(Point)` where `Point` is unknown is one mistake, and
                // the query's code names the question that could not be
                // answered rather than the type lookup that failed under it.
                self.error_code(
                    *span,
                    codes::SIZE_OF_UNKNOWN,
                    format!("`{query}` names no known type: `{type_name}` is not declared"),
                );
                None
            }
        }
    }

    /// `offsetof(T, f)`: the byte offset of field `f` in a `repr(C)` struct.
    fn check_offsetof(&mut self, ty: &Type, field: &Expr, span: Span) -> Option<HirExpr> {
        let TypeKind::Struct(struct_name) = &ty.kind else {
            self.error_code(
                span,
                codes::OFFSET_OF_NOT_REPR_C,
                format!("`offsetof` needs a `repr(C)` struct: `{ty}` is not a struct"),
            );
            return None;
        };
        let Some(sig) = self.structs.get(struct_name).cloned() else {
            self.error_code(
                span,
                codes::SIZE_OF_UNKNOWN,
                format!("`offsetof` names no known type: `{struct_name}` is not declared"),
            );
            return None;
        };
        if !sig.repr_c {
            self.errors.push((
                self.file,
                TypeError::new(
                    span,
                    format!(
                        "`offsetof` needs a `repr(C)` struct: `{struct_name}` has Yashy's own layout"
                    ),
                )
                .with_code(codes::OFFSET_OF_NOT_REPR_C)
                .with_note(format!(
                    "add `repr(C)` in front of `struct {struct_name}` to fix its layout"
                )),
            ));
            return None;
        }
        let Expr::Ident(path, field_span) = field else {
            self.error_code(
                span,
                codes::OFFSET_OF_NOT_REPR_C,
                "`offsetof` takes a field name, not a value",
            );
            return None;
        };
        let Some(field_name) = path.last() else {
            self.error_code(
                *field_span,
                codes::OFFSET_OF_NOT_REPR_C,
                "`offsetof` takes a field name",
            );
            return None;
        };
        let Some(index) = sig.fields.iter().position(|(name, _)| name == field_name) else {
            let fields: Vec<String> = sig.fields.iter().map(|(name, _)| name.clone()).collect();
            self.error_code(
                *field_span,
                codes::OFFSET_OF_NOT_REPR_C,
                format!(
                    "`{struct_name}` has no field `{field_name}`; its fields are: {}",
                    fields.join(", ")
                ),
            );
            return None;
        };
        let Some(layout) = self.layout_of(ty) else {
            self.error_code(
                span,
                codes::SIZE_OF_UNKNOWN,
                format!("`offsetof` of `{ty}` has no layout: a field names a type this module cannot see"),
            );
            return None;
        };
        let Some(offset) = layout.offsets.get(index).copied() else {
            self.error_code(
                span,
                codes::SIZE_OF_UNKNOWN,
                format!("`offsetof` of `{ty}` has no layout"),
            );
            return None;
        };
        Some(HirExpr::Literal(
            Literal::I64(i64::try_from(offset).unwrap_or(i64::MAX)),
            span,
            prim(TypeKind::I64),
        ))
    }

    /// The layout of `ty`, answered from this checker's declaration table.
    fn layout_of(&self, ty: &Type) -> Option<Layout> {
        let decls = CheckerDecls {
            structs: &self.structs,
            enums: &self.enums,
        };
        type_layout::layout(&ty.kind, &decls)
    }

    /// The `string` an interpolated expression contributes, converting with the
    /// `str_from_*` builtin that matches its type.
    ///
    /// A `string` needs no conversion and is returned as it is. `int`, `float`,
    /// `bool` and `char` each have exactly one conversion, named here so the
    /// piece and the builtin table cannot drift. Everything else that has a
    /// debug form goes through it; a map, channel, mutex, pointer, function or
    /// `void` has no text form, and formatting its bytes or its address would be
    /// a wrong answer that looks like a right one, so it is reported instead.
    ///
    /// Returns `None` after reporting when there is no conversion.
    fn as_string(&mut self, expr: HirExpr, span: Span) -> Option<HirExpr> {
        let ty = expr_ty(&expr);
        // Aggregates and other types with a debug form print through the same
        // machinery `${x:?}` uses. Only types with no form (maps, channels,
        // mutexes, pointers, functions, void) are reported.
        if !matches!(
            ty.kind,
            TypeKind::Str | TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Char
        ) {
            if self.has_debug_form(&ty.kind) {
                return self.debug_string(expr, span);
            }
            let why = match &ty.kind {
                TypeKind::Map(_, _) => "a map's order is not stable",
                TypeKind::Channel(_) => "a channel has no readable state",
                TypeKind::Mutex => "a mutex has no readable state",
                TypeKind::Ptr(_) | TypeKind::CPtr(_) => "a pointer has no value to show",
                TypeKind::Func(_, _) => "a function has no text form",
                TypeKind::Unit | TypeKind::Never => "void has no text form",
                TypeKind::Task(_) => "a task must be awaited before it has a value",
                _ => "this type has no text form",
            };
            if matches!(&ty.kind, TypeKind::Task(_)) {
                self.error_code(
                    expr_span(&expr),
                    codes::TYPE_MISMATCH,
                    format!("cannot print a `{ty}`: {why}: use `.await` to get its result"),
                );
            } else {
                self.error(expr_span(&expr), format!("cannot print a `{ty}`: {why}"));
            }
            return None;
        }
        let callee = match &ty.kind {
            TypeKind::Str => return Some(expr),
            TypeKind::I64 => "str_from_i64",
            TypeKind::F64 => "str_from_f64",
            TypeKind::Bool => "str_from_bool",
            TypeKind::Char => "str_from_char",
            // The guard above returns for every other kind. A kind added there
            // without a conversion lands here and is reported, not panicked on.
            _ => {
                self.error(
                    expr_span(&expr),
                    format!("cannot print a `{ty}`: this type has no text form"),
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
    /// wrong number, and so does negating the smallest `int`. Which operations
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
    /// two `int`s, which the runtime prints.
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
    /// The result type is `void` rather than `Never`: a condition that holds is
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
                    "`{name}` cannot compare `{left_ty}` values: it compares `int`, \
                     `float`, `bool`, `char` and `string`"
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
        "dealloc" => Some(("free", 1)),
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
        TypeKind::Ptr(_) => match method {
            "free" => Some("dealloc"),
            _ => None,
        },
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
/// Yashy yet, and comparing one by its first field would be a wrong answer
/// rather than a missing one, so asking is reported.
fn has_equality(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Str | TypeKind::Char
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

/// Whether two declarations describe the same C function.
///
/// Spans are ignored: two declarations written at different lines are still the
/// same function when their parameter and return types agree.
fn same_signature(a: &FnSig, b: &FnSig) -> bool {
    a.params.len() == b.params.len()
        && a.params
            .iter()
            .zip(b.params.iter())
            .all(|(left, right)| left.kind == right.kind)
        && a.ret.kind == b.ret.kind
}

/// A signature as the `E0900` message writes it: `(int, f64) -> int`.
///
/// The variadic marker is part of the signature: `(i8*, ...) -> i32` and
/// `(i8*) -> i32` are different calling conventions.
fn signature_text(sig: &FnSig, variadic: bool) -> String {
    let mut params: Vec<String> = sig.params.iter().map(|p| p.to_string()).collect();
    if variadic {
        params.push("...".to_string());
    }
    format!("({}) -> {}", params.join(", "), sig.ret)
}

/// The value of a checked integer expression when it is a compile-time
/// constant, or `None` when it is not.
///
/// Only what the source can spell without running anything is folded: a
/// literal, a cast of one (which is how `1 as u8` reaches a narrow type), and a
/// negated literal (which is how `-1` is written). Anything else is a run-time
/// value the check has to handle.
fn const_int(expr: &HirExpr) -> Option<i64> {
    match expr {
        HirExpr::Literal(Literal::I64(value), _, _) => Some(*value),
        HirExpr::Cast { expr, ty, .. } if ty.kind.is_integer() => const_int(expr),
        HirExpr::Unary {
            op: UnOp::Neg,
            operand,
            ..
        } => const_int(operand).map(i64::wrapping_neg),
        _ => None,
    }
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
    ast: &yashy_parser::ast::Program,
    sm: &SourceMap,
    file: FileId,
) -> Result<HirProgram, Vec<Diagnostic>> {
    check_program_with(ast, sm, file, CheckOptions::default())
}

/// The public entry point for the type checker, in the build mode `options`
/// asks for.
pub fn check_program_with(
    ast: &yashy_parser::ast::Program,
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
    modules: &[(FileId, &yashy_parser::ast::Program)],
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

/// Split `Enum.Variant` into the enum's name and the variant's.
///
/// The split is at the **last** dot, because an enum's own name may be
/// qualified: `io.Error.NotFound` names the variant `NotFound` of `io.Error`.
/// A single-segment path cannot name a variant, so it answers `None` and the
/// caller falls through to the ordinary name lookup.
fn split_enum_path(path: &[String]) -> Option<(String, String)> {
    let (variant, head) = path.split_last()?;
    if head.is_empty() {
        return None;
    }
    Some((head.join("."), variant.clone()))
}

/// The enums the language declares rather than the program.
///
/// `io.Error` is the failure half of the byte-buffer builtins. Registering it
/// before any item is read means the name resolves in every module and a
/// `switch` over it can be proved exhaustive. The variant order is the one
/// [`crate::ty::IO_ERROR_VARIANTS`] fixes for the checker, the backend and the
/// runtime together.
fn builtin_enums() -> HashMap<String, EnumSig> {
    let variants: Vec<VariantDef> = crate::ty::IO_ERROR_VARIANTS
        .iter()
        .map(|name| VariantDef {
            name: (*name).to_string(),
            fields: Vec::new(),
            span: Span::new(0, 0),
        })
        .collect();
    let payloads = vec![Vec::new(); variants.len()];
    let mut enums = HashMap::new();
    enums.insert(
        "io.Error".to_string(),
        EnumSig {
            variants,
            payloads,
            repr_c: false,
        },
    );
    enums
}

/// A pointer to `pointee`, at a synthetic span. Used for the builtin
/// signatures, where there is no source type expression to point at.
fn ptr(pointee: Type) -> Type {
    Type {
        kind: TypeKind::Ptr(Box::new(pointee)),
        span: Span::new(0, 0),
    }
}

/// The two `int` parameters shared by all nine arithmetic builtins.
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

/// `Option[payload]` at a synthetic span, for a builtin's signature.
fn option_of(payload: Type) -> Type {
    prim(TypeKind::Option(Box::new(payload)))
}

/// `Result[ok, err]` at a synthetic span, for a builtin's signature.
fn result_of(ok: Type, err: Type) -> Type {
    prim(TypeKind::Result(Box::new(ok), Box::new(err)))
}

/// `[]element` at a synthetic span.
fn slice_of(element: Type) -> Type {
    prim(TypeKind::Slice(Box::new(element)))
}

/// The builtin `io.Error`, the failure half of a byte-buffer call.
fn io_error() -> Type {
    prim(TypeKind::Enum("io.Error".to_string()))
}

/// Whether a slice of `kind` can be pushed to, indexed, popped and iterated in
/// M6.
///
/// Numbers, `bool`, and `string` qualify. A compound element (`Option`,
/// `Result`, a struct, an enum) needs its real layout as a stride, which
/// `yashy_codegen::ir::size_of` does not have. A `string` element is a
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
        "push"
            | "pop"
            | "set"
            | "insert"
            | "remove"
            | "clear"
            | "reverse"
            | "sort"
            | "extend"
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
            | TypeKind::Char
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
/// it: `int` bytes, a `bool` byte, or a `string` handle it hashes by content.
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
/// describe a NUL-terminated C string, and Yashy's `string` is the value a
/// program has. Codegen copies the bytes before the call (Task 3), so no Yashy
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
        TypeKind::Unit | TypeKind::CVoid => true,
        TypeKind::Option(inner) => is_void_value(&inner.kind),
        TypeKind::Result(ok, err) => is_void_value(&ok.kind) || is_void_value(&err.kind),
        TypeKind::Slice(element) => is_void_value(&element.kind),
        TypeKind::FixedArray(element, _) => is_void_value(&element.kind),
        TypeKind::Ptr(pointee) => is_void_value(&pointee.kind),
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
/// Used for the assertions, which return normally when they hold: `void` is the
/// honest type, and it is what keeps `assert(cond)` out of value positions.
fn unit(span: Span) -> Type {
    Type {
        kind: TypeKind::Unit,
        span,
    }
}

/// The `T` of an `Option[T]`, as a [`TypeExpr`] for synthesized variant defs.
/// Falls back to `void` when the subject is not an `Option` (which cannot
/// happen: callers only call this under a `TypeKind::Option` match).
fn option_inner(subject_ty: &Type) -> TypeExpr {
    match &subject_ty.kind {
        TypeKind::Option(inner) => type_to_expr(inner),
        _ => TypeExpr::Name("void".to_string(), Span::new(0, 0)),
    }
}

/// The `(T, E)` of a `Result[T, E]`, as [`TypeExpr`]s. Same fallback rule as
/// [`option_inner`].
fn result_inners(subject_ty: &Type) -> (TypeExpr, TypeExpr) {
    match &subject_ty.kind {
        TypeKind::Result(ok, err) => (type_to_expr(ok), type_to_expr(err)),
        _ => (
            TypeExpr::Name("void".to_string(), Span::new(0, 0)),
            TypeExpr::Name("void".to_string(), Span::new(0, 0)),
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
        TypeKind::I64 => TypeExpr::Name("int".to_string(), span),
        TypeKind::F64 => TypeExpr::Name("float".to_string(), span),
        TypeKind::Bool => TypeExpr::Name("bool".to_string(), span),
        TypeKind::Char => TypeExpr::Name("char".to_string(), span),
        TypeKind::Str => TypeExpr::Name("string".to_string(), span),
        TypeKind::Unit => TypeExpr::Name("void".to_string(), span),
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
        TypeKind::Task(inner) => TypeExpr::Generic {
            name: "Task".to_string(),
            args: vec![type_to_expr(inner)],
            span,
        },
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
        TypeKind::Never => TypeExpr::Name("void".to_string(), span),
    }
}

/// True for the numeric primitives: `int` and `float`, plus the C numeric kinds.
///
/// The C kinds are numeric only in an `as` target and in values that came back
/// from an `extern` call, which is exactly where they are useful: `malloc`
/// wants a `usize`, `strlen` returns one, and narrowing to `u8` is how a
/// program reaches a `char` parameter. Equality still governs arithmetic, so a
/// `i32` and an `int` need an explicit cast rather than silently widening.
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
    let lhs_char = lhs.kind == TypeKind::Char;
    let rhs_char = rhs.kind == TypeKind::Char;
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
            // A `char` is a byte, not a number: `'A' + 1` has no answer that
            // stays a `char`, and one that silently became an `int` would make
            // arithmetic and comparison disagree about what a char is. The
            // conversion is the way to do arithmetic on the byte.
            if lhs_char || rhs_char {
                return Err("no arithmetic on `char`: convert with `to_int()`".to_string());
            }
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
            // Comparing a char with an int asks whether two different kinds of
            // thing are equal, and there is no answer that is not a guess about
            // which of the two conversions the author meant.
            if lhs_char != rhs_char {
                return Err(format!("cannot compare `{lhs}` with `{rhs}`"));
            }
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
            if lhs_char != rhs_char {
                return Err(format!("cannot compare `{lhs}` with `{rhs}`"));
            }
            if !(numeric || (lhs_char && rhs_char)) || !lhs.eq_ignoring_span(rhs) {
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
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            // bool is allowed for the three logical bitwise forms.
            if lhs.eq_ignoring_span(&prim(TypeKind::Bool))
                && rhs.eq_ignoring_span(&prim(TypeKind::Bool))
            {
                return Ok(prim(TypeKind::Bool));
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
        BinOp::Shl | BinOp::Shr => {
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
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
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
        TypeExpr::Generic { span, .. } => *span,
        TypeExpr::Unit(span) => *span,
    }
}
