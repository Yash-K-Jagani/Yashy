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
    /// The field types of a declared struct, in declaration order.
    pub fn field_types(&self, name: &str) -> Vec<TypeKind> {
        self.structs
            .iter()
            .find(|structure| structure.name == name)
            .map(|structure| {
                structure
                    .fields
                    .iter()
                    .map(|field| field.ty.kind.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The emission order of a declared struct's fields, as declaration
    /// indices. The backend reads the same order from the HIR item.
    pub fn field_order(&self, name: &str) -> Vec<usize> {
        self.structs
            .iter()
            .find(|structure| structure.name == name)
            .map(|structure| structure.field_order.clone())
            .unwrap_or_default()
    }

    /// Every declared type's name, in declaration order.
    pub fn type_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .structs
            .iter()
            .map(|structure| structure.name.clone())
            .chain(self.enums.iter().map(|entry| entry.name.clone()))
            .collect();
        names.sort();
        names
    }
}

/// One type's report as a JSON object: the shape `inspect --layout --json` and
/// `inspect --layout --all --json` both print, so the two cannot drift.
fn fill_entry(entry: &mut yashy_common::Json, name: &str, decls: &HirDecls) {
    use yashy_common::Json;

    let Some(found) = find(name, decls) else {
        return;
    };
    let names = decls.field_names(name);
    let sizes = field_sizes(name, decls);
    entry.set("type", Json::string(name));
    entry.set("size", Json::number(found.size as i64));
    entry.set("align", Json::number(found.align as i64));
    let fields = decls.field_types(name);
    let natural =
        yashy_hir::layout::layout_fields(&fields, decls, &(0..fields.len()).collect::<Vec<_>>())
            .map(|layout| layout.size)
            .unwrap_or(found.size);
    let order = decls.field_order(name);
    let saved = natural.saturating_sub(found.size);
    entry.set("padding", Json::number(padding_bytes(name, decls) as i64));
    entry.set("saved", Json::number(saved as i64));
    let ordered: Vec<Json> = order
        .iter()
        .map(|&index| {
            Json::string(
                names
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| format!("field{index}")),
            )
        })
        .collect();
    entry.set("order", Json::array(ordered));
    match niche(name, decls) {
        Some(niche) => {
            entry.set("niche", Json::string(niche));
        }
        None => {
            entry.set("niche", Json::null());
        }
    }
    let fields: Vec<Json> = found
        .offsets
        .iter()
        .enumerate()
        .map(|(index, offset)| {
            let mut field = Json::object();
            field.set("offset", Json::number(*offset as i64));
            if let Some(label) = names.get(index) {
                field.set("field", Json::string(label));
            }
            if let Some(size) = sizes.get(index) {
                field.set("size", Json::number(*size as i64));
            }
            field
        })
        .collect();
    entry.set("fields", Json::array(fields));
}

/// The report for one type, with the envelope `inspect --layout --json` prints.
pub fn json_report(name: &str, decls: &HirDecls) -> Option<yashy_common::Json> {
    use yashy_common::Json;

    find(name, decls)?;
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("inspect --layout"));
    fill_entry(&mut report, name, decls);
    Some(report)
}

/// The report for every declared type, with the envelope `--all --json` prints.
pub fn json_report_all(decls: &HirDecls) -> yashy_common::Json {
    use yashy_common::Json;

    let mut rows = Vec::new();
    for name in decls.type_names() {
        let mut entry = Json::object();
        fill_entry(&mut entry, &name, decls);
        rows.push(entry);
    }
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("inspect --layout"));
    report.set("types", Json::array(rows));
    report
}

/// One type's report as the human line: `Name size=.. align=.. fields=..`.
pub fn human_line(name: &str, decls: &HirDecls) -> Option<String> {
    let found = find(name, decls)?;
    let names = decls.field_names(name);
    let fields: Vec<String> = found
        .offsets
        .iter()
        .enumerate()
        .map(|(index, offset)| match names.get(index) {
            Some(label) => format!("{label}@{offset}"),
            None => format!("field{index}@{offset}"),
        })
        .collect();
    Some(format!(
        "{name} size={} align={} fields={}",
        found.size,
        found.align,
        fields.join(", ")
    ))
}

/// The layout of every declared type, sorted by name.
pub fn all(decls: &HirDecls) -> Vec<(String, Layout, Vec<String>)> {
    decls
        .type_names()
        .into_iter()
        .filter_map(|name| {
            let names = decls.field_names(&name);
            find(&name, decls).map(|layout| (name, layout, names))
        })
        .collect()
}

/// The size of each field of a declared struct, in declaration order.
pub fn field_sizes(name: &str, decls: &HirDecls) -> Vec<usize> {
    decls
        .structs
        .iter()
        .find(|structure| structure.name == name)
        .map(|structure| {
            structure
                .fields
                .iter()
                .map(|field| {
                    yashy_hir::layout::layout(&field.ty.kind, decls)
                        .map(|found| found.size)
                        .unwrap_or(0)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The bytes in a declared struct that no field occupies: the inter-field gaps
/// plus the trailing padding. A non-struct or an unknown name reports 0, since
/// only a struct has fields of its own to leave gaps between.
pub fn padding_bytes(name: &str, decls: &HirDecls) -> usize {
    let Some(found) = find(name, decls) else {
        return 0;
    };
    let used: usize = field_sizes(name, decls).iter().sum();
    if field_sizes(name, decls).is_empty() {
        return 0;
    }
    found.size.saturating_sub(used)
}

/// A spare representation the layout already provides, if any.
///
/// `Option` and `Result` carry an explicit tag, and a pointer keeps all of its
/// bit patterns meaningful, so Yashy 1.0 exploits no niche. The report says so
/// rather than inventing one the backend does not use.
pub fn niche(_name: &str, _decls: &HirDecls) -> Option<String> {
    None
}

/// A stable, human-readable form of the whole layout table.
///
/// One line per type, with fields in declaration order and their byte offsets.
/// The widths are fixed so a reader can scan the table and a golden comparison
/// does not depend on terminal width.
pub fn render_human(decls: &HirDecls) -> String {
    let rows = all(decls);
    let Some((_, _, _)) = rows.first() else {
        return "no declared types\n".to_string();
    };
    let width = rows
        .iter()
        .map(|(name, _, _)| name.len())
        .max()
        .unwrap_or(4);
    let mut out = format!(
        "{:<width$}  {:>4}  {:>5}  fields\n",
        "type",
        "size",
        "align",
        width = width
    );
    for (name, found, names) in &rows {
        let fields: Vec<String> = found
            .offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| match names.get(index) {
                Some(label) => format!("{label}@{offset}"),
                None => format!("field{index}@{offset}"),
            })
            .collect();
        out.push_str(&format!(
            "{:<width$}  {:>4}  {:>5}  {}\n",
            name,
            found.size,
            found.align,
            fields.join(", "),
            width = width
        ));
    }
    out
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
                repr: structure.repr,
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
            tag_bytes: enumeration.repr.and_then(|repr| match repr {
                yashy_hir::hir::ReprKind::Int(bits) => Some(bits as usize),
                _ => None,
            }),
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
        None => match decls.named(name)? {
            NamedType::Enum { .. } => TypeKind::Enum(name.to_string()),
            NamedType::Struct { .. } => TypeKind::Struct(name.to_string()),
        },
    };
    Some(yashy_hir::layout::layout(&kind, decls)?)
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
