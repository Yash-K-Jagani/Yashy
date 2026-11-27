//! Semantic review: compare the compiler's current index with a recorded one.
//!
//! A line-level diff cannot say whether a change breaks callers. The index can:
//! it carries signatures, effects, contracts, typestate, visibility, spans and
//! the capability declarations. `yashyc review --baseline <index.json>` reads
//! a snapshot made by `yashyc index --json` and reports what changed in the
//! language's terms, not in the editor's. A change to documentation alone is
//! not a semantic change.

use std::collections::BTreeMap;
use std::path::Path;

use yashy_common::json_rpc::{self, Value};
use yashy_common::Json;

use crate::index::{self, IndexModel};

/// The facts about one symbol in the baseline snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BaselineSymbol {
    signature: String,
    effects: Option<(bool, bool, bool)>,
    contracts: Vec<(String, String)>,
    typestate: Option<String>,
    visibility: String,
    file: String,
    start: i64,
    end: i64,
}

struct Baseline {
    symbols: BTreeMap<(String, String), BaselineSymbol>,
    tests: usize,
    capabilities: Vec<String>,
}

fn string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn effects(value: Option<&Value>) -> Option<(bool, bool, bool)> {
    let value = value?;
    if matches!(value, Value::Null) {
        return None;
    }
    let flag = |name: &str| {
        value
            .get(name)
            .is_some_and(|flag| matches!(flag, Value::Bool(true)))
    };
    Some((flag("io"), flag("unsafe"), flag("async")))
}

fn parse_baseline(text: &str) -> Result<Baseline, String> {
    let root = json_rpc::parse(text).map_err(|error| format!("baseline is not JSON: {error}"))?;
    let entries = root
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or("baseline has no `symbols` array; make one with `yashyc index --json`")?;
    let mut symbols = BTreeMap::new();
    for entry in entries {
        let name = string(entry.get("name")).ok_or("a baseline symbol has no name")?;
        let contracts = entry
            .get("contracts")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        (
                            string(item.get("kind")).unwrap_or_default(),
                            string(item.get("text")).unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let span = entry.get("span");
        let symbol = BaselineSymbol {
            signature: string(entry.get("signature")).unwrap_or_default(),
            effects: effects(entry.get("effects")),
            contracts,
            typestate: string(entry.get("typestate")),
            visibility: string(entry.get("visibility")).unwrap_or_else(|| "private".to_string()),
            file: string(entry.get("file")).unwrap_or_default(),
            start: span
                .and_then(|span| span.get("start"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
            end: span
                .and_then(|span| span.get("end"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
        };
        symbols.insert((symbol.file.clone(), name), symbol);
    }
    Ok(Baseline {
        symbols,
        tests: root
            .get("tests")
            .and_then(Value::as_array)
            .map_or(0, <[Value]>::len),
        capabilities: strings(root.get("capabilities")),
    })
}

/// One classified change. The span comes from the current index, so a reader
/// can open the file and see the exact declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticChange {
    pub kind: &'static str,
    pub symbol: String,
    pub file: String,
    pub start: i64,
    pub end: i64,
    pub public: bool,
    pub detail: String,
}

impl SemanticChange {
    fn to_json(&self) -> Json {
        let mut entry = Json::object();
        entry.set("kind", Json::string(self.kind));
        entry.set("symbol", Json::string(&self.symbol));
        entry.set("file", Json::string(&self.file));
        entry.set("start", Json::number(self.start));
        entry.set("end", Json::number(self.end));
        entry.set("public", Json::bool(self.public));
        entry.set("detail", Json::string(&self.detail));
        entry
    }
}

/// The review: changes, breakage, deltas and a coarse risk label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub changes: Vec<SemanticChange>,
    pub public_break: bool,
    pub capability_delta: Vec<String>,
    pub unsafe_delta: i64,
    pub test_delta: i64,
    pub risk: &'static str,
}

impl Review {
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set(
            "changes",
            Json::array(self.changes.iter().map(SemanticChange::to_json).collect()),
        );
        root.set("public_break", Json::bool(self.public_break));
        root.set(
            "capability_delta",
            Json::array(
                self.capability_delta
                    .iter()
                    .map(|name| Json::string(name))
                    .collect(),
            ),
        );
        root.set("unsafe_delta", Json::number(self.unsafe_delta));
        root.set("test_delta", Json::number(self.test_delta));
        root.set("risk", Json::string(self.risk));
        root
    }

    pub fn render_human(&self) -> String {
        let mut out = format!(
            "Review: {} semantic changes  public break: {}  risk: {}\n",
            self.changes.len(),
            if self.public_break { "yes" } else { "no" },
            self.risk
        );
        for change in &self.changes {
            out.push_str(&format!(
                "  {} {} {} ({}:{}..{}) {}\n",
                change.kind,
                if change.public { "pub" } else { "private" },
                change.symbol,
                change.file,
                change.start,
                change.end,
                change.detail
            ));
        }
        out.push_str(&format!(
            "capability delta: {}  unsafe delta: {:+}  test delta: {:+}\n",
            if self.capability_delta.is_empty() {
                "none".to_string()
            } else {
                self.capability_delta.join(", ")
            },
            self.unsafe_delta,
            self.test_delta
        ));
        out
    }
}

fn push_change(
    changes: &mut Vec<SemanticChange>,
    public_break: &mut bool,
    kind: &'static str,
    symbol: &index::Symbol,
    detail: String,
) {
    let public = symbol.is_public();
    if public && kind != "docs" {
        *public_break = true;
    }
    changes.push(SemanticChange {
        kind,
        symbol: symbol.name.clone(),
        file: symbol.file.clone(),
        start: symbol.span.start as i64,
        end: symbol.span.end as i64,
        public,
        detail,
    });
}

/// Compare a baseline snapshot with the workspace at `root`.
pub fn review(root: &Path, baseline_text: &str) -> Result<Review, String> {
    let baseline = parse_baseline(baseline_text)?;
    let current = index::build_index(root)?;
    let mut changes = Vec::new();
    let mut public_break = false;

    for symbol in &current.symbols {
        match baseline
            .symbols
            .get(&(symbol.file.clone(), symbol.name.clone()))
        {
            None => push_change(
                &mut changes,
                &mut public_break,
                "behaviour",
                symbol,
                "symbol added".to_string(),
            ),
            Some(old) => {
                if old.signature != symbol.signature {
                    push_change(
                        &mut changes,
                        &mut public_break,
                        "signature",
                        symbol,
                        format!("{} -> {}", old.signature, symbol.signature),
                    );
                }
                if old.effects != symbol.effects {
                    push_change(
                        &mut changes,
                        &mut public_break,
                        "effect",
                        symbol,
                        format!("{:?} -> {:?}", old.effects, symbol.effects),
                    );
                }
                let contracts: Vec<(String, String)> = symbol
                    .contracts
                    .iter()
                    .map(|(kind, text, _)| (kind.clone(), text.clone()))
                    .collect();
                if old.contracts != contracts {
                    push_change(
                        &mut changes,
                        &mut public_break,
                        "contract",
                        symbol,
                        format!("{} -> {} clauses", old.contracts.len(), contracts.len()),
                    );
                }
                if old.typestate != symbol.typestate {
                    push_change(
                        &mut changes,
                        &mut public_break,
                        "ownership",
                        symbol,
                        format!("{:?} -> {:?}", old.typestate, symbol.typestate),
                    );
                }
                if old.visibility != symbol.visibility {
                    push_change(
                        &mut changes,
                        &mut public_break,
                        "signature",
                        symbol,
                        format!("{} -> {}", old.visibility, symbol.visibility),
                    );
                }
                // Documentation changes are deliberately not reported: a
                // comment-only diff is no semantic change.
            }
        }
    }
    for ((file, name), old) in &baseline.symbols {
        if current
            .symbols
            .iter()
            .any(|symbol| symbol.name == *name && symbol.file == *file)
        {
            continue;
        }
        let symbol = index::Symbol {
            name: name.clone(),
            kind: "fn".to_string(),
            file: old.file.clone(),
            span: yashy_common::Span::new(old.start.max(0) as u32, old.end.max(0) as u32),
            signature: old.signature.clone(),
            effects: old.effects,
            contracts: Vec::new(),
            typestate: old.typestate.clone(),
            visibility: old.visibility.clone(),
            docs: None,
        };
        push_change(
            &mut changes,
            &mut public_break,
            "signature",
            &symbol,
            "symbol removed".to_string(),
        );
    }

    let capability_delta: Vec<String> = current
        .capabilities
        .iter()
        .filter(|name| !baseline.capabilities.contains(name))
        .cloned()
        .collect();
    let unsafe_count =
        |model: &IndexModel, names: &BTreeMap<(String, String), BaselineSymbol>, current: bool| {
            if current {
                model
                    .symbols
                    .iter()
                    .filter(|symbol| symbol.effects.is_some_and(|(_, unsafe_, _)| unsafe_))
                    .count() as i64
            } else {
                names
                    .values()
                    .filter(|symbol| symbol.effects.is_some_and(|(_, unsafe_, _)| unsafe_))
                    .count() as i64
            }
        };
    let unsafe_delta = unsafe_count(&current, &baseline.symbols, true)
        - unsafe_count(&current, &baseline.symbols, false);
    let test_delta = current.tests.len() as i64 - baseline.tests as i64;
    let risk = if public_break || !capability_delta.is_empty() || unsafe_delta > 0 || test_delta < 0
    {
        "high"
    } else if changes.is_empty() {
        "low"
    } else {
        "medium"
    };
    Ok(Review {
        changes,
        public_break,
        capability_delta,
        unsafe_delta,
        test_delta,
        risk,
    })
}

/// The changed symbol names, for `impact` to fan out from.
pub fn changed_symbols(review: &Review) -> Vec<String> {
    let mut names: Vec<String> = review
        .changes
        .iter()
        .map(|change| change.symbol.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_baseline_without_symbols_is_refused() {
        assert!(parse_baseline("{\"symbols\":[]}").is_ok());
        assert!(parse_baseline("{}").is_err());
        assert!(parse_baseline("not json").is_err());
    }
}
