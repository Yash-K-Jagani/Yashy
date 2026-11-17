//! Type layout for `yashyc inspect --layout`.
//!
//! The layout walk already lives in `yashy-hir`; this module only hands it the
//! declarations the checker produced, so an `inspect` answer and a `sizeof`
//! answer cannot disagree.

use yashy_hir::hir::{HirEnum, HirItem, HirProgram, HirStruct};
use yashy_hir::layout::{Decls, Layout, NamedType};
use yashy_hir::ty::TypeKind;

/// Declaration tables reduced to what the layout walk needs.
#[derive(Clone, Debug, Default)]
pub struct HirDecls {
    structs: Vec<HirStruct>,
    enums: Vec<HirEnum>,
}

impl HirDecls {
    pub fn from_program(program: &HirProgram) -> Self {
        let mut out = Self::default();
        for item in &program.items {
            match item {
                HirItem::Struct(structure) => out.structs.push(structure.clone()),
                HirItem::Enum(enumeration) => out.enums.push(enumeration.clone()),
                _ => {}
            }
        }
        out
    }

    /// The field names of a declared struct, in declaration order.
    pub fn field_names(&self, name: &str) -> Vec<String> {
        self.structs
            .iter()
            .find(|structure| structure.name == name)
            .map(|structure| {
                structure
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Decls for HirDecls {
    fn named(&self, name: &str) -> Option<NamedType> {
        if let Some(structure) = self.structs.iter().find(|entry| entry.name == name) {
            return Some(NamedType::Struct {
                fields: structure
                    .fields
                    .iter()
                    .map(|field| field.ty.kind.clone())
                    .collect(),
            });
        }
        let enumeration = self.enums.iter().find(|entry| entry.name == name)?;
        Some(NamedType::Enum {
            payloads: enumeration
                .variants
                .iter()
                .map(|variant| {
                    variant
                        .fields
                        .iter()
                        .map(|field| field.ty.kind.clone())
                        .collect()
                })
                .collect(),
            repr_c: matches!(enumeration.repr, Some(yashy_hir::hir::ReprKind::C)),
        })
    }
}

/// A primitive's spelling, or `None` for a name that must be declared.
pub fn primitive(name: &str) -> Option<TypeKind> {
    Some(match name {
        "int" => TypeKind::I64,
        "float" => TypeKind::F64,
        "bool" => TypeKind::Bool,
        "char" => TypeKind::Char,
        "string" => TypeKind::Str,
        "void" | "unit" => TypeKind::Unit,
        _ => return None,
    })
}

/// Look up a type by its source spelling. Named struct and enum types use the
/// checker's declaration table; primitives use the same spelling the checker
/// accepts. Generic forms need an instantiation the parser owns and are not
/// guessed here.
pub fn find(name: &str, decls: &HirDecls) -> Option<Layout> {
    let kind = match primitive(name) {
        Some(kind) => kind,
        None => TypeKind::Struct(name.to_string()),
    };
    let layout = yashy_hir::layout::layout(&kind, decls)?;
    if decls.named(name).is_none() && primitive(name).is_none() {
        return None;
    }
    Some(layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_have_a_layout_without_declarations() {
        let decls = HirDecls::default();
        assert!(find("int", &decls).is_some());
        assert!(find("bool", &decls).is_some());
        assert!(find("NoSuchType", &decls).is_none());
    }
}
