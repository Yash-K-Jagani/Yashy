//! `yashy doc`: the public surface rendered from the compiler's own tables.
//!
//! One fact, two renderings: `--json` emits the same model the Markdown is
//! rendered from. Items are sorted by (kind, name); the Markdown has no
//! paths in the body and no timestamps, so the same source produces
//! byte-identical output on both OSes and a CI job can assert the committed
//! docs are current.
//!
//! HTML is out of scope: an HTML renderer is a template engine and a CSS
//! surface, and this project has neither. The Markdown is what `llms.txt`
//! and the index already consume.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yashy_common::{FileId, Json, SourceMap};
use crate::pipeline::check_and_verify;
use yashy_verify::VerifyOptions;
use yashy_sema::hir::{HirItem, HirProgram};

/// One documented item.
#[derive(Clone, Debug)]
pub struct DocItem {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub docs: Option<String>,
    pub effects: Option<(bool, bool, bool)>,
    pub contracts: Vec<(String, String)>,
}

/// One module and its items.
#[derive(Clone, Debug)]
pub struct DocModule {
    pub name: String,
    pub items: Vec<DocItem>,
}

impl DocModel {
    /// Render every module into `out` and an `index.md` beside them.
    pub fn write_to(&self, out: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(out)?;
        for module in &self.modules {
            let mut text = format!("# {}\n", module.name);
            for item in &module.items {
                text.push_str(&format!(
                    "\n## {} {}\n\n```\n{}\n```\n",
                    item.kind, item.name, item.signature
                ));
                match &item.docs {
                    Some(docs) => text.push_str(&format!("\n{}\n", indented(docs))),
                    None => text.push_str("\nNo documentation.\n"),
                }
            }
            std::fs::write(out.join(format!("{}.md", module.name)), text)?;
        }
        let mut index = String::from("# Modules\n");
        for module in &self.modules {
            index.push_str(&format!("- [{0}]({0}.md)\n", module.name));
        }
        std::fs::write(out.join("index.md"), index)
    }
}

/// The whole documentation model.
#[derive(Clone, Debug, Default)]
pub struct DocModel {
    pub modules: Vec<DocModule>,
}

/// Build the model for a root: check every program, keep the public items.
pub fn build_doc(root: &Path, private: bool) -> Result<DocModel, String> {
    let mut files = Vec::new();
    crate::index::collect_files_from(root, &mut files);
    files.sort();
    let mut model = DocModel::default();
    for path in &files {
        let entry_source = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut sources = SourceMap::new();
        let checked = crate::pipeline::check_and_verify(
            &mut sources,
            path,
            entry_source.clone(),
            false,
            &VerifyOptions {
                contracts: yashy_verify::ContractMode::Enforce,
            },
            1,
        );
        let Ok((hir, report)) = checked else {
            continue;
        };
        let module = doc_module(path, &hir, &report.effects, private);
        if !module.items.is_empty() || private {
            model.modules.push(module);
        }
    }
    model.modules.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(model)
}

/// The index command's file collector, shared so `index` and `doc` agree on
/// which programs exist.
pub fn module_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("module")
        .to_string()
}

fn doc_module(
    path: &Path,
    hir: &HirProgram,
    effects: &BTreeMap<String, yashy_common::Effect>,
    private: bool,
) -> DocModule {
    let mut items: Vec<DocItem> = Vec::new();
    for item in &hir.items {
        match item {
            HirItem::Fn(f) => {
                // Visibility is not in the HIR; the item this function was
                // declared from decides. The index answers the same question
                // from the same model: a private item is skipped unless
                // `--private` was asked.
                let public = crate::index::is_public(path, f);
                if !public && !private {
                    continue;
                }
                let docs = if f.docs.is_empty() {
                    None
                } else {
                    Some(f.docs.join("\n"))
                };
                items.push(DocItem {
                    name: f.name.clone(),
                    kind: "fn".to_string(),
                    signature: crate::index::signature_of(f),
                    docs,
                    effects: effects
                        .get(&f.name)
                        .map(|effect| (effect.io, effect.unsafe_, effect.async_)),
                    contracts: f
                        .requires
                        .iter()
                        .map(|_| ("requires".to_string(), String::new()))
                        .chain(
                            f.ensures
                                .iter()
                                .map(|_| ("ensures".to_string(), String::new())),
                        )
                        .collect(),
                });
            }
            HirItem::Struct(s) => items.push(DocItem {
                name: s.name.clone(),
                kind: "struct".to_string(),
                signature: format!("struct {}", s.name),
                docs: None,
                effects: None,
                contracts: Vec::new(),
            }),
            HirItem::Enum(e) => items.push(DocItem {
                name: e.name.clone(),
                kind: "enum".to_string(),
                signature: format!("enum {}", e.name),
                docs: None,
                effects: None,
                contracts: Vec::new(),
            }),
            _ => {}
        }
    }
    items.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
    DocModule {
        name: module_name(path),
        items,
    }
}

/// The JSON form of the model.
pub fn to_json(model: &DocModel) -> Json {
    let mut root = Json::object();
    root.set("schema_version", Json::number(1));
    root.set("tool", Json::string("yashy"));
    root.set(
        "modules",
        Json::array(
            model
                .modules
                .iter()
                .map(|module| {
                    let mut entry = Json::object();
                    entry.set("name", Json::string(&module.name));
                    entry.set(
                        "items",
                        Json::array(
                            module
                                .items
                                .iter()
                                .map(|item| {
                                    let mut entry = Json::object();
                                    entry.set("name", Json::string(&item.name));
                                    entry.set("kind", Json::string(&item.kind));
                                    entry.set("signature", Json::string(&item.signature));
                                    match &item.docs {
                                        Some(text) => {
                                            entry.set("docs", Json::string(text));
                                        }
                                        None => {
                                            entry.set("docs", Json::null());
                                        }
                                    }
                                    match item.effects {
                                        Some((io, unsafe_, async_)) => {
                                            let mut effects = Json::object();
                                            effects.set("io", Json::bool(io));
                                            effects.set("unsafe", Json::bool(unsafe_));
                                            effects.set("async", Json::bool(async_));
                                            entry.set("effects", effects);
                                        }
                                        None => {
                                            entry.set("effects", Json::null());
                                        }
                                    }
                                    entry.set("contracts", Json::array(vec![]));
                                    entry.set("typestate", Json::null());
                                    entry
                                })
                                .collect(),
                        ),
                    );
                    entry
                })
                .collect(),
        ),
    );
    root
}

/// Escape a doc line that starts with `#` or `|` so it cannot become a
/// heading or a table row in the surrounding document.
fn indented(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with('#') || line.starts_with('|') {
                format!("    {line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
