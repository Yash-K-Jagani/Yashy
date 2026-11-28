//! C header generation from the compiler's typed model.
//!
//! A header is not prose: it is rendered from the same checked functions the
//! backend emits, so a signature change moves both together. Only
//! C-representable signatures are exported; a function with an aggregate or
//! generic parameter stays internal and is named in the excluded list rather
//! than guessed at.
//!
//! The symbols match `yashy_codegen::ir::fn_symbol`: a plain name keeps its
//! spelling, and a method's qualified name cannot be a C identifier, so only
//! top-level functions are candidates.

use std::collections::BTreeSet;

use yashy_codegen::ir::fn_symbol;
use yashy_hir::hir::{HirFn, HirItem, HirProgram};
use yashy_sema::ty::TypeKind;

/// One exported function and its rendered declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub yashy_name: String,
    pub symbol: String,
    pub declaration: String,
    pub ownership: String,
}

/// Whether a type crosses a C boundary without a hidden layout promise.
///
/// A parameter and a result have opposite ownership: the caller keeps a string
/// it passes in, and owns a string it gets back. One table for both directions
/// would put the wrong sentence in the header, so the two are separate.
fn c_param(ty: &TypeKind) -> Option<(&'static str, &'static str)> {
    match ty {
        TypeKind::I64 => Some(("int64_t", "owned value")),
        TypeKind::Bool => Some(("int", "0 or 1")),
        TypeKind::F64 => Some(("double", "owned value")),
        // `const` is the machine-readable half of the note: a caller can see
        // that the callee borrows the string and must not free it.
        TypeKind::Str => Some(("const yashy_str *", "borrowed")),
        TypeKind::Ptr(_) | TypeKind::CPtr(_) => Some(("const void *", "borrowed")),
        TypeKind::Unit => Some(("void", "no value")),
        _ => None,
    }
}

/// A result's C spelling, its ownership marker and the note that explains it.
fn c_return(ty: &TypeKind) -> Option<(&'static str, &'static str, &'static str)> {
    match ty {
        TypeKind::I64 => Some(("int64_t", "", "owned value")),
        TypeKind::Bool => Some(("int", "", "0 or 1")),
        TypeKind::F64 => Some(("double", "", "owned value")),
        TypeKind::Str => Some((
            "yashy_str *",
            "YASHY_OWNED",
            "owned; release with yashy_str_release",
        )),
        TypeKind::Ptr(_) | TypeKind::CPtr(_) => Some((
            "void *",
            "YASHY_BORROWED",
            "borrowed; ownership stays in Yashy",
        )),
        TypeKind::Unit => Some(("void", "", "no result")),
        _ => None,
    }
}

/// Collect the export list from checked HIR.
pub fn exports(hir: &HirProgram, public: &BTreeSet<String>) -> (Vec<Export>, Vec<String>) {
    let mut out = Vec::new();
    let mut excluded = Vec::new();
    for item in &hir.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        if function.name.contains('.') || function.name.is_empty() {
            continue;
        }
        if !public.contains(&function.name) {
            continue;
        }
        match render(function) {
            Some((declaration, ownership)) => out.push(Export {
                yashy_name: function.name.clone(),
                symbol: fn_symbol(&function.name),
                declaration,
                ownership,
            }),
            None => excluded.push(function.name.clone()),
        }
    }
    out.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    excluded.sort();
    (out, excluded)
}

fn render(function: &HirFn) -> Option<(String, String)> {
    let mut params = Vec::new();
    let mut notes: Vec<&'static str> = Vec::new();
    for param in &function.params {
        let (c, note) = c_param(&param.ty.kind)?;
        if note == "borrowed"
            && !notes.contains(&"borrowed parameters; the callee does not free them")
        {
            notes.push("borrowed parameters; the callee does not free them");
        }
        params.push(if c.ends_with('*') {
            format!("{c}{}", param.name)
        } else {
            format!("{c} {}", param.name)
        });
    }
    let (ret, marker, ret_note) = c_return(&function.ret.kind)?;
    if !notes.contains(&ret_note) {
        notes.push(ret_note);
    }
    let marker = if marker.is_empty() {
        String::new()
    } else {
        format!("{marker} ")
    };
    // `yashy_str *name`, not `yashy_str * name`: a pointer declarator binds
    // the star to the name in every C style guide.
    let separator = if ret.ends_with('*') { "" } else { " " };
    let declaration = format!(
        "{marker}{ret}{separator}{}({});",
        fn_symbol(&function.name),
        params.join(", ")
    );
    Some((declaration, notes.join("; ")))
}

/// Render the header document.
pub fn header(hir: &HirProgram, public: &BTreeSet<String>) -> String {
    let (exports, excluded) = exports(hir, public);
    let mut out = String::from(concat!(
        "/* Generated by `yashyc build --header`. Do not edit. */\n",
        "#ifndef YASHY_GENERATED_H\n",
        "#define YASHY_GENERATED_H\n",
        "#include <stdint.h>\n\n",
        "/* The ABI this header describes. A change to a declaration, a struct\n",
        "   layout or an ownership rule is a new version. */\n",
        "#define YASHY_ABI_VERSION 1\n\n",
        "/* A result the caller releases with `yashy_str_release`. */\n",
        "#define YASHY_OWNED\n",
        "/* A result the caller must not free. */\n",
        "#define YASHY_BORROWED\n\n",
        "typedef struct {\n",
        "    int64_t refcount;\n",
        "    int64_t len;\n",
        "    int64_t cap;\n",
        "    char *data;\n",
        "} yashy_str;\n\n",
        "/* Releases one owned string returned by an exported function. */\n",
        "void yashy_str_release(yashy_str *s);\n\n",
    ));
    for export in &exports {
        out.push_str(&format!(
            "/* yashy: {} -- {} */\n{}\n",
            export.yashy_name, export.ownership, export.declaration
        ));
    }
    if !excluded.is_empty() {
        out.push_str(&format!(
            "\n/* Not exported (no C-compatible signature): {} */\n",
            excluded.join(", ")
        ));
    }
    out.push_str("#endif /* YASHY_GENERATED_H */\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_type_tables_are_closed_and_direction_aware() {
        assert!(c_param(&TypeKind::I64).is_some());
        assert!(c_param(&TypeKind::Str).is_some());
        assert!(c_param(&TypeKind::Unit).is_some());
        assert!(c_param(&TypeKind::Never).is_none());
        let int_type = || yashy_hir::ty::Type {
            kind: TypeKind::I64,
            span: yashy_common::Span::SYNTHETIC,
        };
        assert!(c_param(&TypeKind::Map(Box::new(int_type()), Box::new(int_type()))).is_none());
        assert!(c_return(&TypeKind::Map(Box::new(int_type()), Box::new(int_type()))).is_none());
        // A string is borrowed going in and owned coming back.
        assert_eq!(
            c_param(&TypeKind::Str).map(|(c, _)| c),
            Some("const yashy_str *")
        );
        assert_eq!(
            c_return(&TypeKind::Str).map(|(c, marker, _)| (c, marker)),
            Some(("yashy_str *", "YASHY_OWNED"))
        );
    }
}
