//! `cfg`: the target condition, evaluated before anything is resolved.
//!
//! A `cfg(..)` clause guards an item, a statement block or a struct field. The
//! filter in this module runs on the parsed AST and **removes** what it guards
//! when the condition is false, so the checker, the verifier and the backend
//! never see the disabled code at all. That is the whole difference between
//! `cfg` and `if false`: a call to a function that exists only on the other
//! platform is legal inside a disabled `cfg`, because resolving it is exactly
//! what the filter makes unnecessary.
//!
//! The key set is derived from the target triple plus the build profile, so
//! `--target aarch64-apple-darwin` answers `macos` and `arch_aarch64` without
//! the emitter's own `cfg!` facts being consulted. A key the compiler does not
//! define is `E2202`, never a silently false condition.

use std::collections::BTreeSet;

use yashy_common::{codes, Diagnostic, FileId, Severity, Span};
use yashy_parser::ast::{Block, CfgExpr, FieldDef, Item, Program, Stmt};

/// Every key this compiler defines.
///
/// The order groups them by kind -- platform, architecture, profile -- because
/// that is the order a diagnostic lists them in. `--print-cfg` prints the
/// *active* keys sorted, so two runs can be compared.
pub const KEYS: [&str; 8] = [
    "windows",
    "linux",
    "macos",
    "unix",
    "arch_x86_64",
    "arch_aarch64",
    "debug",
    "release",
];

/// The keys that are true for one build.
#[derive(Clone, Debug)]
pub struct CfgSet {
    target: String,
    active: BTreeSet<&'static str>,
}

impl CfgSet {
    /// The keys true for `triple` at the given profile.
    ///
    /// An unrecognised triple answers no OS key at all rather than guessing:
    /// a wrong `cfg` answer is worse than a missing one, and `--target`
    /// already refuses a triple it cannot emit for.
    pub fn for_target(triple: &str, release: bool) -> CfgSet {
        let mut active = BTreeSet::new();
        if triple.contains("windows") {
            active.insert("windows");
        } else if triple.contains("apple-darwin") {
            active.insert("macos");
            active.insert("unix");
        } else if triple.contains("linux") {
            active.insert("linux");
            active.insert("unix");
        }
        if triple.starts_with("x86_64") {
            active.insert("arch_x86_64");
        } else if triple.starts_with("aarch64") {
            active.insert("arch_aarch64");
        }
        active.insert(if release { "release" } else { "debug" });
        CfgSet {
            target: triple.to_string(),
            active,
        }
    }

    /// Whether `key` is true. An unknown key is not true; the filter reports it
    /// separately, so this stays a pure predicate.
    pub fn is_active(&self, key: &str) -> bool {
        self.active.contains(key)
    }

    /// The active keys, sorted.
    ///
    /// Sorted rather than grouped by kind, so two runs can be compared by
    /// `diff` and a build script can read the list without knowing the order
    /// the keys are declared in.
    pub fn active_keys(&self) -> Vec<&'static str> {
        self.active.iter().copied().collect()
    }

    /// The triple this set was derived from.
    pub fn target(&self) -> &str {
        &self.target
    }
}

/// Evaluate `expr` against `set`, reporting every unknown key as `E2202`.
///
/// A condition that names an unknown key is an error even when the other side
/// of an `any(..)` is true: the rule exists so that a typo cannot delete code,
/// and "it happened to be true anyway" is not a reason to keep the typo.
pub fn evaluate(
    expr: &CfgExpr,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    match expr {
        CfgExpr::Key(name, span) => {
            if !KEYS.contains(&name.as_str()) {
                diagnostics.push(unknown_key(file, name, *span));
                return false;
            }
            set.is_active(name)
        }
        CfgExpr::Not(inner, _) => !evaluate(inner, set, file, diagnostics),
        CfgExpr::Any(parts, _) => parts.iter().fold(false, |acc, part| {
            evaluate(part, set, file, diagnostics) || acc
        }),
        CfgExpr::All(parts, _) => parts.iter().fold(true, |acc, part| {
            evaluate(part, set, file, diagnostics) && acc
        }),
    }
}

fn unknown_key(file: FileId, key: &str, span: Span) -> Diagnostic {
    Diagnostic::new(
        Severity::Error,
        file,
        span,
        format!("unknown `cfg` key `{key}`"),
    )
    .with_code(codes::UNKNOWN_CFG_KEY)
    .with_note(format!("known keys: {}", KEYS.join(", ")))
}

/// Remove every disabled item, statement and struct field from `program`.
///
/// Returns the diagnostics it produced: every unknown key it met, in source
/// order. A program with a diagnostic is not checked, which is what keeps a
/// typo from deleting a branch silently.
pub fn filter(program: &mut Program, set: &CfgSet, file: FileId) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    filter_items(&mut program.items, set, file, &mut diagnostics);
    for item in &mut program.items {
        filter_item_stmts(item, set, file, &mut diagnostics);
    }
    diagnostics
}

fn filter_items(
    items: &mut Vec<Item>,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut kept = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        // A disabled `cfg` item is dropped whole: the item inside it is not
        // visited, so an unknown key inside it is not reported either. That is
        // deliberate for the guarded item, but a nested `cfg` inside an active
        // one must still be evaluated, which the recursion below does.
        if let Some(item) = filter_item(item, set, file, diagnostics) {
            kept.push(item);
        }
    }
    *items = kept;
}

/// Filter one item, or drop it.
///
/// `None` means the item is gone: a `cfg` whose condition is false takes the
/// item it guards with it, and a wrapper whose inner item is gone goes too --
/// a doc comment on a function that does not exist here documents nothing.
fn filter_item(
    item: Item,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Item> {
    match item {
        Item::Cfg { key, item, .. } => {
            if evaluate(&key, set, file, diagnostics) {
                filter_item(*item, set, file, diagnostics)
            } else {
                None
            }
        }
        Item::Public { item, span } => {
            filter_item(*item, set, file, diagnostics).map(|inner| Item::Public {
                item: Box::new(inner),
                span,
            })
        }
        Item::Effect { effect, item, span } => {
            filter_item(*item, set, file, diagnostics).map(|inner| Item::Effect {
                effect,
                item: Box::new(inner),
                span,
            })
        }
        Item::Documented { docs, item, span } => {
            filter_item(*item, set, file, diagnostics).map(|inner| Item::Documented {
                docs,
                item: Box::new(inner),
                span,
            })
        }
        Item::Struct {
            name,
            type_params,
            mut fields,
            repr,
            span,
        } => {
            filter_fields(&mut fields, set, file, diagnostics);
            Some(Item::Struct {
                name,
                type_params,
                fields,
                repr,
                span,
            })
        }
        Item::Object {
            name,
            mut fields,
            mut methods,
            repr,
            span,
        } => {
            filter_fields(&mut fields, set, file, diagnostics);
            methods = filter_child_items(methods, set, file, diagnostics);
            Some(Item::Object {
                name,
                fields,
                methods,
                repr,
                span,
            })
        }
        Item::Impl {
            type_name,
            trait_name,
            mut methods,
            span,
        } => {
            methods = filter_child_items(methods, set, file, diagnostics);
            Some(Item::Impl {
                type_name,
                trait_name,
                methods,
                span,
            })
        }
        other => Some(other),
    }
}

/// Filter the items a type or an `impl` block holds.
fn filter_child_items(
    items: Vec<Item>,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Item> {
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        if let Some(item) = filter_item(item, set, file, diagnostics) {
            kept.push(item);
        }
    }
    kept
}

fn filter_fields(
    fields: &mut Vec<FieldDef>,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) {
    fields.retain(|field| match &field.cfg {
        Some(key) => evaluate(key, set, file, diagnostics),
        None => true,
    });
}

/// Every block a statement can contain, filtered recursively.
fn filter_item_stmts(
    item: &mut Item,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match item {
        Item::Public { item, .. } | Item::Effect { item, .. } | Item::Documented { item, .. } => {
            filter_item_stmts(item, set, file, diagnostics)
        }
        Item::Cfg { item, .. } => filter_item_stmts(item, set, file, diagnostics),
        Item::Fn { body, .. } | Item::ConstFn { body, .. } => {
            filter_block(body, set, file, diagnostics);
        }
        Item::Impl { methods, .. } => {
            for method in methods {
                filter_item_stmts(method, set, file, diagnostics);
            }
        }
        _ => {}
    }
}

fn filter_block(block: &mut Block, set: &CfgSet, file: FileId, diagnostics: &mut Vec<Diagnostic>) {
    if block.stmts.is_empty() {
        return;
    }
    let stmts = std::mem::take(&mut block.stmts);
    let mut kept = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        push_statement(stmt, set, file, diagnostics, &mut kept);
    }
    block.stmts = kept;
}

fn push_statement(
    stmt: Stmt,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
    out: &mut Vec<Stmt>,
) {
    match stmt {
        Stmt::Cfg { key, block, .. } => {
            if evaluate(&key, set, file, diagnostics) {
                // The block is spliced in, not nested: the checker would
                // otherwise treat it as a new scope and a binding made inside
                // would not reach the statements after it.
                let mut block = block;
                filter_block(&mut block, set, file, diagnostics);
                out.extend(block.stmts);
            }
        }
        mut other => {
            filter_statement_body(&mut other, set, file, diagnostics);
            out.push(other);
        }
    }
}

/// The blocks a statement carries, so a nested `cfg` is filtered too.
fn filter_statement_body(
    stmt: &mut Stmt,
    set: &CfgSet,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::If {
            then_block,
            else_block,
            ..
        }
        | Stmt::IfLet {
            then_block,
            else_block,
            ..
        } => {
            filter_block(then_block, set, file, diagnostics);
            if let Some(otherwise) = else_block {
                filter_block(otherwise, set, file, diagnostics);
            }
        }
        Stmt::WhileLet { body, .. }
        | Stmt::ForRange { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::ForC { body, .. }
        | Stmt::Defer { body, .. }
        | Stmt::Arena { body, .. }
        | Stmt::Scope { body, .. } => filter_block(body, set, file, diagnostics),
        _ => {}
    }
}
