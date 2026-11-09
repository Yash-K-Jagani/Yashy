//! The symbol index: one fact, two renderings.
//!
//! `yashy index` walks a directory (or a single file), loads and checks every
//! program in it, and answers what the compiler knows about its symbols:
//! what they are, where they are, what they do, what they promise and what
//! they touch. The human table and the `--json` document are two renderings
//! of the same [`IndexModel`], so an agent and a person cannot drift.
//!
//! Every field the compiler does not know is `null`, never omitted and never
//! invented: a function in a file that does not check still gets a symbol,
//! whose effects and contracts are `null` because the verifier never ran.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::pipeline::check_and_verify;
use yashy_common::{FileId, Json, SourceMap, Span};
use yashy_sema::hir::{HirExpr, HirFn, HirItem, HirProgram, HirStmt};
use yashy_sema::ty::TypeKind;
use yashy_lexer::Lexer;
use yashy_parser::ast::Item;
use yashy_parser::Parser;
use yashy_verify::{ContractMode, VerifyOptions};

/// One symbol the compiler knows about.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub span: Span,
    pub signature: String,
    pub effects: Option<(bool, bool, bool)>,
    pub contracts: Vec<(String, String, Span)>,
    pub typestate: Option<String>,
    pub visibility: String,
    pub docs: Option<String>,
}

impl Symbol {
    pub fn is_public(&self) -> bool {
        self.visibility == "pub"
    }
}

/// One `import` edge.
#[derive(Clone, Debug)]
pub struct ImportEdge {
    pub from: String,
    pub path: Vec<String>,
    pub alias: Option<String>,
    pub span: Span,
}

/// A test program and the functions it calls.
#[derive(Clone, Debug)]
pub struct TestProgram {
    pub file: String,
    pub calls: Vec<String>,
}

/// The whole index, before either rendering.
#[derive(Clone, Debug, Default)]
pub struct IndexModel {
    pub files: Vec<String>,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<ImportEdge>,
    pub tests: Vec<TestProgram>,
}

/// The effects a `pure` check reads: nothing but this.
fn effect_of(effects: &BTreeMap<String, yashy_common::Effect>, name: &str) -> Option<(bool, bool, bool)> {
    effects
        .get(name)
        .map(|effect| (effect.io, effect.unsafe_, effect.async_))
}

/// `fn fetch(string url) -> Result[string, io.Error]` from the checked item.
pub fn signature_of(f: &HirFn) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|param| format!("{} {}", param.ty, param.name))
        .collect();
    let ret = match &f.ret.kind {
        yashy_sema::ty::TypeKind::Task(inner) => inner.to_string(),
        _ => f.ret.to_string(),
    };
    let prefix = if f.is_async { "async fn " } else { "fn " };
    let name = f
        .name
        .rsplit('.')
        .next()
        .unwrap_or(&f.name)
        .to_string();
    format!(
        "{prefix}{name}({}) -> {ret}",
        params.join(", ")
    )
}

/// Every name this function body calls, in first-use order, deduplicated.
fn calls_of(f: &HirFn, out: &mut Vec<String>) {
    fn walk(e: &HirExpr, out: &mut Vec<String>) {
        match e {
            HirExpr::Call { callee, args, .. } => {
                if let Some(name) = callee.last() {
                    if !out.iter().any(|seen| seen == name) {
                        out.push(name.clone());
                    }
                }
                for arg in args {
                    walk(arg, out);
                }
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                walk(lhs, out);
                walk(rhs, out);
            }
            HirExpr::Unary { operand, .. } => walk(operand, out),
            HirExpr::Move(inner, ..) => walk(inner, out),
            HirExpr::Try { expr, .. } | HirExpr::Await { expr, .. } => walk(expr, out),
            HirExpr::Cast { expr, .. } => walk(expr, out),
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    walk(value, out);
                }
            }
            HirExpr::SliceLit { elements, .. } => {
                for element in elements {
                    walk(element, out);
                }
            }
            HirExpr::FixedArrayLit { elements, .. } => {
                for element in elements {
                    walk(element, out);
                }
            }
            HirExpr::FieldAccess { base, .. } => walk(base, out),
            HirExpr::Index { base, index, .. } => {
                walk(base, out);
                walk(index, out);
            }
            HirExpr::Spawn { callee, args, .. } => {
                walk(callee, out);
                for arg in args {
                    walk(arg, out);
                }
            }
            _ => {}
        }
    }
    fn stmt(s: &HirStmt, out: &mut Vec<String>) {
        match s {
            HirStmt::Expr(e, ..) => walk(e, out),
            HirStmt::Var { init, .. } | HirStmt::Const { init, .. } => walk(init, out),
            HirStmt::Assign { value, .. } => walk(value, out),
            HirStmt::Return(Some(e), ..) => walk(e, out),
            HirStmt::Return(None, ..) => {}
            HirStmt::If {
                init, cond, then_block, else_block, ..
            } => {
                if let Some(init) = init {
                    stmt(init, out);
                }
                walk(cond, out);
                for inner in &then_block.stmts {
                    stmt(inner, out);
                }
                if let Some(block) = else_block {
                    for inner in &block.stmts {
                        stmt(inner, out);
                    }
                }
            }
            _ => {}
        }
    }
    for statement in &f.body.stmts {
        stmt(statement, out);
    }
}

/// One `.ya` file: the checked program's symbols plus the import edges.
/// Whether the HirFn `f` in `path`'s AST is `pub`.
pub fn is_public(path: &Path, f: &HirFn) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let mut sources = SourceMap::new();
    let file = sources.add_file(path.to_string_lossy().into_owned(), text.clone());
    let tokens = Lexer::with_file(&text, file).tokenize().unwrap_or_default();
    let Ok(program) = Parser::new(tokens, &sources, file).parse_program() else {
        return false;
    };
    fn find(item: &Item, pubbed: bool, name: &str, span_start: u32) -> Option<bool> {
        match item {
            Item::Public { item, .. } => find(item, true, name, span_start),
            Item::Documented { item, .. } => find(item, pubbed, name, span_start),
            Item::Effect { item, .. } => find(item, pubbed, name, span_start),
            Item::Fn { name: item_name, span, .. } => {
                if item_name == name && span.start == span_start {
                    Some(pubbed)
                } else {
                    None
                }
            }
            Item::Impl { methods, .. } => {
                for method in methods {
                    if let found @ Some(_) = find(method, pubbed, name, span_start) {
                        return found;
                    }
                }
                None
            }
            _ => None,
        }
    }
    for item in &program.items {
        if let Some(found) = find(item, false, &f.name.rsplit('.').next().unwrap_or(&f.name), f.span.start) {
            return found;
        }
    }
    false
}

fn collect_file(
    path: &Path,
    sources: &mut SourceMap,
    file: FileId,
    hir: &HirProgram,
    effects: &BTreeMap<String, yashy_common::Effect>,
    model: &mut IndexModel,
) {
    let file_text = sources.file(file).to_string();
    let display = path.to_string_lossy().replace('\\', "/");
    model.files.push(display.clone());
    // Visibility and doc blocks are the AST's facts; the HIR dropped the
    // `pub` wrapper, so read them from the parsed items.
    let tokens = Lexer::with_file(&file_text, file)
        .tokenize()
        .unwrap_or_default();
    let program = Parser::new(tokens, sources, file)
        .parse_program()
        .unwrap_or(yashy_parser::ast::Program {
            items: Vec::new(),
        });
    fn item_facts(item: &Item, pubbed: bool, docs: Vec<String>, out: &mut Vec<(String, Span, String, Option<String>)>) {
        match item {
            Item::Public { item, .. } => item_facts(item, true, docs, out),
            Item::Documented { docs, item, .. } => item_facts(item, pubbed, docs.clone(), out),
            Item::Effect { item, .. } => item_facts(item, pubbed, docs, out),
            Item::Fn { name, span, .. } => out.push((
                name.clone(),
                *span,
                if pubbed { "pub".to_string() } else { "private".to_string() },
                if docs.is_empty() { None } else { Some(docs.join("\n")) },
            )),
            Item::Struct { name, span, .. } | Item::Enum { name, span, .. } => out.push((
                name.clone(),
                *span,
                if pubbed { "pub".to_string() } else { "private".to_string() },
                if docs.is_empty() { None } else { Some(docs.join("\n")) },
            )),
            Item::Impl { methods, .. } => {
                for method in methods {
                    item_facts(method, pubbed, docs.clone(), out);
                }
            }
            _ => {}
        }
    }
    let mut facts = Vec::new();
    for item in &program.items {
        item_facts(item, false, Vec::new(), &mut facts);
    }
    for item in &hir.items {
        match item {
            HirItem::Fn(f) => {
                let doc = if f.docs.is_empty() {
                    None
                } else {
                    Some(f.docs.join("\n"))
                };
                let (visibility, _) = facts
                    .iter()
                    .find(|(name, span, ..)| name == &f.name && span.start == f.span.start)
                    .map(|(_, _, visibility, _)| (visibility.clone(), ()))
                    .unwrap_or_else(|| ("private".to_string(), ()));
                model.symbols.push(Symbol {
                    name: f.name.clone(),
                    kind: "fn".to_string(),
                    file: display.clone(),
                    span: f.span,
                    signature: signature_of(f),
                    effects: effect_of(effects, &f.name),
                    contracts: f
                        .requires
                        .iter()
                        .map(|c| {
                            (
                                "requires".to_string(),
                                contract_text(sources, file, c.span),
                                c.span,
                            )
                        })
                        .chain(f.ensures.iter().map(|c| {
                            (
                                "ensures".to_string(),
                                contract_text(sources, file, c.span),
                                c.span,
                            )
                        }))
                        .collect(),
                    typestate: None,
                    visibility,
                    docs: doc,
                });
            }
            HirItem::Struct(s) => {
                let (_, span, visibility, docs) = facts
                    .iter()
                    .find(|(name, span, ..)| name == &s.name && span.start == s.span.start)
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            s.name.clone(),
                            s.span,
                            "private".to_string(),
                            None,
                        )
                    });
                model.symbols.push(Symbol {
                    name: s.name.clone(),
                    kind: "struct".to_string(),
                    file: display.clone(),
                    span,
                    signature: format!("struct {}", s.name),
                    effects: None,
                    contracts: Vec::new(),
                    typestate: None,
                    visibility,
                    docs,
                });
            }
            HirItem::Enum(e) => {
                let (_, span, visibility, docs) = facts
                    .iter()
                    .find(|(name, span, ..)| name == &e.name && span.start == e.span.start)
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            e.name.clone(),
                            e.span,
                            "private".to_string(),
                            None,
                        )
                    });
                model.symbols.push(Symbol {
                    name: e.name.clone(),
                    kind: "enum".to_string(),
                    file: display.clone(),
                    span,
                    signature: format!("enum {}", e.name),
                    effects: None,
                    contracts: Vec::new(),
                    typestate: None,
                    visibility,
                    docs,
                });
            }
            HirItem::Import(_) => {}
            HirItem::Extern(_) => {}
        }
    }
    for item in &program.items {
        if let Item::Import { path, alias, span } = item {
            model.imports.push(ImportEdge {
                from: display.clone(),
                path: path.clone(),
                alias: alias.clone(),
                span: *span,
            });
        }
    }
}

fn contract_text(sources: &SourceMap, file: FileId, span: Span) -> String {
    sources
        .file(file)
        .get(span.start as usize..span.end as usize)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Walk `root` and index every `.ya` program in it.
pub fn build_index(root: &Path) -> Result<IndexModel, String> {
    let mut files = Vec::new();
    collect_ya_files(root, &mut files);
    files.sort();
    let mut model = IndexModel::default();
    for path in &files {
        // One source map per program: the query database requires module
        // file ids to be registered in order, so a map shared across
        // programs would hand it ids out of order.
        let mut sources = SourceMap::new();
        let entry_source = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        // Not added to `sources` here: the loader registers the entry
        // itself, and a second registration would put its file id out of
        // order for the query database.
        let file = FileId::from_index(0);
        let checked = check_and_verify(
            &mut sources,
            path,
            entry_source.clone(),
            false,
            &VerifyOptions {
                contracts: ContractMode::Enforce,
            },
            1,
        );
        match checked {
            Ok((hir, report)) => {
                // The entry file is always the last one added for this
                // program; later imports are appended after it.
                collect_file(path, &mut sources, file, &hir, &report.effects, &mut model);
                if path
                    .components()
                    .any(|component| component.as_os_str() == "tests")
                {
                    let mut calls = Vec::new();
                    for item in &hir.items {
                        if let HirItem::Fn(f) = item {
                            calls_of(f, &mut calls);
                        }
                    }
                    model.tests.push(TestProgram {
                        file: path.to_string_lossy().replace('\\', "/"),
                        calls,
                    });
                }
            }
            Err(_) => {
                // A file that does not check is still indexed from its AST:
                // symbols with null effects and contracts.
                model.files.push(path.to_string_lossy().replace('\\', "/"));
                let tokens = Lexer::with_file(&entry_source, file)
                    .tokenize()
                    .unwrap_or_default();
                if let Ok(program) = Parser::new(tokens, &sources, file).parse_program() {
                    for item in &program.items {
                        let (name, span, kind) = match item {
                            Item::Fn { name, span, .. } => (name.clone(), *span, "fn"),
                            Item::Struct { name, span, .. } => (name.clone(), *span, "struct"),
                            Item::Enum { name, span, .. } => (name.clone(), *span, "enum"),
                            _ => continue,
                        };
                        model.symbols.push(Symbol {
                            name: name.clone(),
                            kind: kind.to_string(),
                            file: path.to_string_lossy().replace('\\', "/"),
                            span,
                            signature: format!("{kind} {name}"),
                            effects: None,
                            contracts: Vec::new(),
                            typestate: None,
                            visibility: "private".to_string(),
                            docs: None,
                        });
                    }
                }
            }
        }
    }
    model.symbols.sort_by(|a, b| {
        (&a.file, a.span.start)
            .cmp(&(&b.file, b.span.start))
    });
    model.files.sort();
    Ok(model)
}

/// Walk `root` and collect every `.ya` file in it.
pub fn collect_files_from(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_ya_files(dir, out);
}

fn collect_ya_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        if dir.extension().and_then(|ext| ext.to_str()) == Some("ya") {
            out.push(dir.to_path_buf());
        }
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name != "target" && !name.starts_with('.') {
                collect_ya_files(&path, out);
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
            out.push(path);
        }
    }
}

/// The JSON envelope every command shares: `{"schema_version":1,"tool":"yashy",…}`.
pub fn to_json(model: &IndexModel) -> Json {
    let mut root = Json::object();
    root.set("schema_version", Json::number(1));
    root.set("tool", Json::string("yashy"));
    root.set(
        "files",
        Json::array(
            model
                .files
                .iter()
                .map(|file| Json::string(file))
                .collect(),
        ),
    );
    root.set(
        "symbols",
        Json::array(
            model
                .symbols
                .iter()
                .map(|symbol| symbol_json(symbol))
                .collect(),
        ),
    );
    root.set(
        "imports",
        Json::array(
            model
                .imports
                .iter()
                .map(|edge| {
                    let mut entry = Json::object();
                    entry.set("from", Json::string(&edge.from));
                    entry.set(
                        "path",
                        Json::array(
                            edge.path
                                .iter()
                                .map(|segment| Json::string(segment))
                                .collect(),
                        ),
                    );
                    match &edge.alias {
                        Some(alias) => {
                            entry.set("alias", Json::string(alias));
                        }
                        None => {
                            entry.set("alias", Json::null());
                        }
                    }
                    entry
                })
                .collect(),
        ),
    );
    root.set(
        "tests",
        Json::array(
            model
                .tests
                .iter()
                .map(|test| {
                    let mut entry = Json::object();
                    entry.set("file", Json::string(&test.file));
                    entry.set(
                        "calls",
                        Json::array(
                            test.calls
                                .iter()
                                .map(|call| Json::string(call))
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

fn span_json(span: Span) -> Json {
    let mut entry = Json::object();
    entry.set("start", Json::number(span.start as i64));
    entry.set("end", Json::number(span.end as i64));
    entry
}

fn symbol_json(symbol: &Symbol) -> Json {
    let mut entry = Json::object();
    entry.set("name", Json::string(&symbol.name));
    entry.set("kind", Json::string(&symbol.kind));
    entry.set("file", Json::string(&symbol.file));
    entry.set(
        "span",
        span_json(symbol.span),
    );
    entry.set("signature", Json::string(&symbol.signature));
    match symbol.effects {
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
    entry.set(
        "contracts",
        Json::array(
            symbol
                .contracts
                .iter()
                .map(|(kind, text, span)| {
                    let mut entry = Json::object();
                    entry.set("kind", Json::string(kind));
                    entry.set("text", Json::string(text));
                    entry.set(
                        "span",
                        span_json(*span),
                    );
                    entry
                })
                .collect(),
        ),
    );
    match &symbol.typestate {
        Some(text) => {
            entry.set("typestate", Json::string(text));
        }
        None => {
            entry.set("typestate", Json::null());
        }
    }
    entry.set("visibility", Json::string(&symbol.visibility));
    match &symbol.docs {
        Some(text) => {
            entry.set("docs", Json::string(text));
        }
        None => {
            entry.set("docs", Json::null());
        }
    }
    entry
}

/// The human table: the same model, one line per symbol.
pub fn render_human(model: &IndexModel) -> String {
    let mut out = String::new();
    out.push_str(&format!("{} symbols in {} files\n", model.symbols.len(), model.files.len()));
    for symbol in &model.symbols {
        let effects = match symbol.effects {
            Some((io, unsafe_, async_)) => {
                let mut flags = Vec::new();
                if io {
                    flags.push("io");
                }
                if unsafe_ {
                    flags.push("unsafe");
                }
                if async_ {
                    flags.push("async");
                }
                if flags.is_empty() {
                    "pure".to_string()
                } else {
                    flags.join(", ")
                }
            }
            None => "unknown".to_string(),
        };
        out.push_str(&format!(
            "{} {} {} [{}]\n",
            symbol.kind, symbol.name, symbol.signature, effects
        ));
    }
    out
}
