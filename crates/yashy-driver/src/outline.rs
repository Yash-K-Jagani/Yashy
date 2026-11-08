//! `yashy outline`, `yashy graph` and `yashy search`: three views over the
//! index model of [`crate::index`].
//!
//! All three read the same model, so their answers cannot disagree: the
//! outline is the model's symbols in source order, the graph is its imports
//! plus the call edges the model carries, and search ranks the model's
//! symbols and doc text.

use yashy_common::Json;

use crate::index::{build_index, IndexModel};

/// The outline of one file: one line per symbol, in source order.
pub fn outline_of(model: &IndexModel, file: &str) -> Vec<String> {
    model
        .symbols
        .iter()
        .filter(|symbol| symbol.file == file)
        .map(|symbol| {
            let flags = match symbol.effects {
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
                        String::new()
                    } else {
                        format!("  [{}]", flags.join(", "))
                    }
                }
                None => "  [unknown]".to_string(),
            };
            format!("{}{}{}", symbol.signature, flags, "")
        })
        .collect()
}

/// The module graph: `{"nodes":[…],"edges":[{"from","to","kind":"import"|"call"}]}`.
pub fn graph_json(model: &IndexModel) -> Json {
    let mut nodes: Vec<String> = model.files.clone();
    nodes.sort();
    nodes.dedup();
    let mut edges = Vec::new();
    for import in &model.imports {
        let mut edge = Json::object();
        edge.set("from", Json::string(&import.from));
        edge.set("to", Json::string(import.path.join(".")));
        edge.set("kind", Json::string("import"));
        edges.push(edge);
    }
    for test in &model.tests {
        for call in &test.calls {
            let mut edge = Json::object();
            edge.set("from", Json::string(&test.file));
            edge.set("to", Json::string(call));
            edge.set("kind", Json::string("call"));
            edges.push(edge);
        }
    }
    let mut root = Json::object();
    root.set(
        "nodes",
        Json::array(nodes.into_iter().map(Json::string).collect()),
    );
    root.set("edges", Json::array(edges));
    root
}

/// Graphviz DOT for the same graph.
pub fn graph_dot(model: &IndexModel) -> String {
    let mut out = String::from("digraph yashy {\n");
    for import in &model.imports {
        out.push_str(&format!(
            "  {:?} -> {:?} [label=\"import\"];\n",
            import.from,
            import.path.join(".")
        ));
    }
    for test in &model.tests {
        for call in &test.calls {
            out.push_str(&format!(
                "  {:?} -> {:?} [label=\"call\"];\n",
                test.file, call
            ));
        }
    }
    out.push_str("}\n");
    out
}

/// Search the symbols: an exact name match ranks before a prefix match,
/// which ranks before a doc match; ties break by file then span.
pub fn search(model: &IndexModel, query: &str) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut exact: Vec<_> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.name == query)
        .collect();
    let mut prefix: Vec<_> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.name != query && symbol.name.starts_with(query))
        .collect();
    let mut doc: Vec<_> = model
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name != query
                && !symbol.name.starts_with(query)
                && symbol
                    .docs
                    .as_ref()
                    .map(|docs| docs.contains(query))
                    .unwrap_or(false)
        })
        .collect();
    let by_file_span = |a: &&crate::index::Symbol, b: &&crate::index::Symbol| {
        (&a.file, a.span.start).cmp(&(&b.file, b.span.start))
    };
    exact.sort_by(by_file_span);
    prefix.sort_by(by_file_span);
    doc.sort_by(by_file_span);
    exact
        .into_iter()
        .chain(prefix)
        .chain(doc)
        .map(|symbol| format!("{} {}", symbol.signature, symbol.file))
        .collect()
}

/// Load the model for a root, for the CLI.
pub fn load(root: &std::path::Path) -> Result<IndexModel, String> {
    build_index(root)
}
