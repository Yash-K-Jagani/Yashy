//! The memoised database.
//!
//! A query re-runs when any input it *read* changed; the read set is recorded
//! on the first execution, the classic red-green algorithm hand-written on a
//! single-threaded database. `&mut self` is the only interior mutability:
//! task 2 shards per file and therefore per instance.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use yashy_common::{Diagnostic, FileId, Severity, SourceMap, Span};
use yashy_hir::hir;
use yashy_parser::ast::{Item, TypeExpr, VariantField};
use yashy_sema::check_modules_with;
use yashy_sema::CheckOptions;
use yashy_verify::VerifyOptions;

use crate::query::{combine, fnv1a, hash_str};
use crate::{CheckedFile, FnFacts, FnId, QueryResult};

/// Per-stage statistics, reported by `yashy check --timings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// Queries answered from the memo.
    pub hits: u64,
    /// Queries re-computed because a read input changed.
    pub misses: u64,
    /// Names of queries re-computed since the last snapshot, sorted.
    pub recomputed: Vec<String>,
}

/// One memoised entry: the read-set hash recorded the last time it ran, and the
/// value, returned unchanged as long as every recorded input still describes
/// the current world. A single hash folds the whole read set together, so the
/// equality test compares every recorded input at once.
struct Entry<T> {
    reads: u64,
    value: T,
}

/// One file, as it is registered with the database. The source text is kept
/// here separately from the source map so that `update_source` can rebuild the
/// map while the map keeps the canonical file ids (the entry point is id 0).
struct ModuleEntry {
    file: FileId,
    path: String,
    /// The name a program used to import this module, for interface-based
    /// invalidation of its importers.
    name: Option<String>,
    text: String,
}

/// The checked world: everything the verifier and the code generator need once
/// per program, cached so the unit queries do not re-run the whole pass.
struct World {
    program: Arc<hir::HirProgram>,
}

/// A stable, span-free hash of a function as it is written. The checked HIR
/// carries spans; a sibling edit shifts them without changing the function, so
/// the identity is the source text: byte-for-byte identical for the same
/// function, different whenever the function changed.
fn fn_hash(f: &hir::HirFn, file_text: &str) -> u64 {
    let range = f.span.start as usize..f.span.end as usize;
    let slice = if range.start > file_text.len() || range.end > file_text.len() {
        ""
    } else {
        &file_text[range]
    };
    hash_str(slice)
}

/// A stable, span-free key for a type expression.
fn type_expr_key(ty: &TypeExpr) -> u64 {
    use TypeExpr::*;
    match ty {
        Name(s, _) => hash_str(s),
        Ptr { kind, pointee, .. } => {
            combine(hash_str(&format!("{:?}", kind)), type_expr_key(pointee))
        }
        FixedArray(inner, n, _) => combine(type_expr_key(inner), hash_str(&n.to_string())),
        View(inner, _) => type_expr_key(inner),
        Slice(inner, _) => type_expr_key(inner),
        Map(k, v, _) => combine(type_expr_key(k), type_expr_key(v)),
        Option(inner, _) => type_expr_key(inner),
        Result(k, v, _) => combine(type_expr_key(k), type_expr_key(v)),
        Channel(inner, _) => type_expr_key(inner),
        Fn { params, ret, .. } => {
            let mut state = fnv1a(b"fn(");
            state = state.wrapping_mul(0x9e3779b97f4a7c15);
            for p in params {
                state = combine(state, type_expr_key(p));
            }
            combine(state, type_expr_key(ret))
        }
        Tuple(items, _) => {
            let mut state = fnv1a(b"(");
            state = state.wrapping_mul(0x9e3779b97f4a7c15);
            for item in items {
                state = combine(state, type_expr_key(item));
            }
            state
        }
        Generic { name, args, .. } => {
            let mut state = hash_str(&format!("{name}["));
            for arg in args {
                state = combine(state, type_expr_key(arg));
            }
            combine(state, hash_str("]"))
        }
        Unit(_) => hash_str("void"),
    }
}

/// A stable key for a boolean expression in a contract that ignores spans.
fn expr_key(expr: &yashy_parser::ast::Expr) -> u64 {
    use yashy_parser::ast::Expr::*;
    match expr {
        Literal(l, _) => hash_str(&format!("{:?}", l)),
        Ident(p, _) => hash_str(&p.join(".")),
        Binary { lhs, op, rhs, .. } => {
            let mut state = hash_str(&format!("{:?}", op));
            state = state.wrapping_mul(0x9e3779b97f4a7c15);
            state = combine(state, expr_key(lhs));
            combine(state, expr_key(rhs))
        }
        Unary { op, operand, .. } => combine(hash_str(&format!("{:?}", op)), expr_key(operand)),
        MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            let mut state = hash_str(method);
            state = state.wrapping_mul(0x9e3779b97f4a7c15);
            state = combine(state, expr_key(receiver));
            for a in args {
                state = combine(state, expr_key(a));
            }
            state
        }
        _ => hash_str(&format!("{:?}", expr)),
    }
}

/// The read set of a query. Any input in this set being different means the
/// cached value cannot be trusted and must be recomputed. A `Reads` is built
/// by folding the flags hash and each input into a single FNV-1a, so the
/// equality test compares every recorded input at once.
struct Reads {
    /// The whole-program flags: compiler, target, release, contract policy,
    /// FP contraction.
    flags: u64,
    /// The thing the query was asked to produce: for `check`, the file id;
    /// for `verify_fn` / `emit_fn`, the function's source-text hash; for
    /// `emit_fn`, that plus its callees' IR.
    key: u64,
}

impl Reads {
    fn new(flags: u64, key: u64) -> Self {
        Self { flags, key }
    }

    fn hash(&self) -> u64 {
        combine(self.flags, self.key)
    }
}

/// The query database. All analysis passes through here; nothing in the
/// compiler may run a check or a verify outside the database.
pub struct Db {
    sources: SourceMap,
    /// Registered modules, in dependency order (dependencies before dependents,
    /// entry last): this is also the order the checker needs.
    modules: Vec<ModuleEntry>,
    /// Which module is the entry point; it has no name and no importer.
    entry: Option<FileId>,
    /// The checked world, once check has run.
    world: Option<World>,
    /// Cached whole-program parse+check results, keyed by file.
    check: HashMap<u32, Entry<CheckedFile>>,
    /// Cached whole-program verify report.
    verify: Option<Arc<yashy_verify::VerifyReport>>,
    /// Per-function verify results.
    verify_fn: HashMap<FnId, Entry<FnFacts>>,
    /// Per-function emit results.
    emit_fn: HashMap<FnId, Entry<String>>,
    /// The parsed ASTs, for the `parse` query.
    parse_table: HashMap<u32, Entry<Arc<yashy_parser::ast::Program>>>,
    /// The FNV-1a hash of each module's text, computed when the text is
    /// registered so a read-set check does not re-hash every source.
    text_hashes: HashMap<u32, u64>,
    /// The order modules are checked in, which is dependency-first so the entry
    /// point is last in the emitted program. Empty means registration order.
    order: Vec<FileId>,
    /// Compiler version: any change means everything is stale.
    compiler: String,
    /// Target triple: any change means everything is stale.
    target: String,
    /// Compilation mode: `release` changes the lowered `debug_assert` tree.
    release: bool,
    /// The verifier's contract policy.
    verify_opts: VerifyOptions,
    /// FP contraction is a codegen choice.
    fp_contract: bool,
    /// The query statistics, updated on every lookup.
    stats: QueryStats,
    /// Query names recomputed since the last snapshot.
    recomputed: BTreeSet<String>,
}

impl Db {
    /// A fresh, empty database.
    pub fn new() -> Self {
        Self {
            sources: SourceMap::new(),
            modules: Vec::new(),
            entry: None,
            world: None,
            check: HashMap::new(),
            verify: None,
            verify_fn: HashMap::new(),
            emit_fn: HashMap::new(),
            parse_table: HashMap::new(),
            text_hashes: HashMap::new(),
            order: Vec::new(),
            compiler: env!("CARGO_PKG_VERSION").to_string(),
            target: yashy_codegen::host_target_triple().to_string(),
            release: false,
            verify_opts: VerifyOptions {
                contracts: yashy_verify::ContractMode::Warn,
            },
            fp_contract: false,
            stats: QueryStats::default(),
            recomputed: BTreeSet::new(),
        }
    }

    /// The source map the database owns. The driver renders every diagnostic
    /// against it, so it must be current: it is rebuilt whenever a source
    /// changes.
    pub fn sources(&self) -> &SourceMap {
        &self.sources
    }

    /// Register a module and ask the database to parse it. `file` is the id
    /// handed out by the driver's `SourceMap` so spans and diagnostics stay
    /// aligned.
    pub fn add_source(&mut self, path: impl Into<String>, text: impl Into<String>) -> FileId {
        let path = path.into();
        let text = text.into();
        let file = self.sources.add_file(path.clone(), text.clone());
        self.text_hashes.insert(file.index(), hash_str(&text));
        self.modules.push(ModuleEntry {
            file,
            path,
            name: None,
            text,
        });
        file
    }

    /// Register a module the caller already parsed, so the database does not
    /// parse it a second time. The caller owns the source map and hands out the
    /// file ids, so modules must arrive in id order: the database rebuilds its
    /// own map with the same ids, and a diagnostic rendered against either map
    /// names the same file.
    pub fn add_parsed(
        &mut self,
        file: FileId,
        path: impl Into<String>,
        name: Option<String>,
        text: impl Into<String>,
        program: yashy_parser::ast::Program,
    ) {
        let path = path.into();
        let text = text.into();
        let id = self.sources.add_file(path.clone(), text.clone());
        assert_eq!(
            id.index(),
            file.index(),
            "modules must be registered in file-id order"
        );
        self.text_hashes.insert(file.index(), hash_str(&text));
        self.modules.push(ModuleEntry {
            file,
            path,
            name,
            text,
        });
        // Seed the parse memo so the first `check` is a hit, not a re-parse.
        let reads = Reads::new(
            self.flags(),
            combine(self.text_hash(file), hash_str(&self.compiler)),
        );
        self.parse_table.insert(
            file.index(),
            Entry {
                reads: reads.hash(),
                value: Arc::new(program),
            },
        );
    }

    /// The name a program used to import this module. Used for interface-based
    /// invalidation of importers.
    pub fn set_module_name(&mut self, file: FileId, name: Option<String>) {
        if let Some(m) = self.modules.iter_mut().find(|m| m.file == file) {
            m.name = name;
        }
    }

    /// The entry point, the module the user asked to compile. Dependency-first
    /// registration puts it last; callers may override when the loader knows
    /// better.
    pub fn set_entry(&mut self, file: FileId) {
        self.entry = Some(file);
    }

    /// Whether `debug_assert` is lowered. A release build checks the same
    /// program and only declines to emit the assertion, so this is a flag on
    /// every query rather than a different pipeline.
    pub fn set_release(&mut self, release: bool) {
        self.release = release;
    }

    /// The verifier's contract policy, part of every verify read set.
    pub fn set_verify_options(&mut self, options: VerifyOptions) {
        self.verify_opts = options;
    }

    /// Whether floating-point contraction is allowed, part of every emit read
    /// set.
    pub fn set_fp_contract(&mut self, on: bool) {
        self.fp_contract = on;
    }

    /// The order modules are checked in: dependencies first, entry point last.
    /// Registration is by file id so the source map lines up; the checker needs
    /// the loader's order so the emitted program keeps the entry point last.
    pub fn set_module_order(&mut self, order: Vec<FileId>) {
        self.order = order;
    }

    /// The import graph reachable from `entry`: `(importer, imported)` for each
    /// `import` in a reachable module, in dependency order. The edge target is
    /// the module registered under the import path's last segment, which is
    /// the name the loader recorded for it. This is a query input: every
    /// per-file query key already folds in its imports' text hashes through
    /// the checker, so an edit to `b.ya` invalidates `a.ya`'s check when `a`
    /// imports `b` and nothing else.
    pub fn module_graph(&self, entry: FileId) -> Vec<(FileId, FileId)> {
        let mut edges = Vec::new();
        let mut seen: Vec<FileId> = Vec::new();
        let mut queue = vec![entry];
        while let Some(file) = queue.pop() {
            if seen.contains(&file) {
                continue;
            }
            seen.push(file);
            let Some(program) = self.parse_cached(file) else {
                continue;
            };
            for item in &program.items {
                let Item::Import { path, .. } = item else {
                    continue;
                };
                let Some(wanted) = path.last() else {
                    continue;
                };
                if let Some(target) = self
                    .modules
                    .iter()
                    .find(|m| m.name.as_deref() == Some(wanted.as_str()))
                {
                    edges.push((file, target.file));
                    queue.push(target.file);
                }
            }
        }
        edges
    }

    /// Replace a registered source with new text. The module keeps its id and
    /// its name. Only the whole-program caches are dropped: the per-function
    /// entries stay, and their read sets decide what actually re-runs. That is
    /// the whole point of hashing inputs instead of clearing on every edit --
    /// an untouched sibling must not be recomputed.
    pub fn update_source(&mut self, file: FileId, text: impl Into<String>) {
        let text = text.into();
        self.text_hashes.insert(file.index(), hash_str(&text));
        for m in &mut self.modules {
            if m.file == file {
                m.text = text;
                break;
            }
        }
        self.rebuild_sources();
        self.world = None;
        self.verify = None;
    }

    fn rebuild_sources(&mut self) {
        let mut map = SourceMap::new();
        for m in &self.modules {
            let id = map.add_file(m.path.clone(), m.text.clone());
            debug_assert_eq!(id.index(), m.file.index());
        }
        self.sources = map;
    }

    /// The FNV-1a hash of the source text, the same shape `yashy-cache` uses
    /// for program-level caching, so the query engine can match the driver's
    /// keys rather than invent a family of its own.
    fn text_hash(&self, file: FileId) -> u64 {
        match self.text_hashes.get(&file.index()) {
            Some(hash) => *hash,
            None => hash_str(self.sources.file(file)),
        }
    }

    /// The interface a module declares: names, types and signatures, no bodies.
    /// Two files that declare the same interface invalidate each other's
    /// importers on any of them changing.
    fn interface_hash(&self, file: FileId) -> u64 {
        let Some(program) = self.parse_cached(file) else {
            return 0;
        };
        let mut state = fnv1a(file.index().to_le_bytes().as_slice());
        state = state.wrapping_mul(0x9e3779b97f4a7c15);
        for item in &program.items {
            self.hash_item_signature(&mut state, item);
        }
        state
    }

    fn hash_item_signature(&self, state: &mut u64, item: &Item) {
        let item = match item {
            Item::Public { item, .. } => item.as_ref(),
            Item::Effect { item, .. } => item.as_ref(),
            // A doc block is not part of the signature: changing a comment
            // must not bust the cache.
            Item::Documented { item, .. } => item.as_ref(),
            item => item,
        };
        match item {
            // The wrappers were unwrapped above; a nested wrapper is not
            // legal source, so it contributes nothing.
            Item::Public { .. } | Item::Effect { .. } | Item::Documented { .. } => {}
            Item::Import { path, .. } => {
                *state = combine(*state, hash_str(&path.join(".")));
            }
            Item::Struct {
                name,
                type_params,
                fields,
                repr,
                ..
            } => {
                *state = combine(*state, hash_str(name));
                for p in type_params {
                    *state = combine(*state, hash_str(&p.name));
                    for bound in &p.bounds {
                        *state = combine(*state, hash_str(bound));
                    }
                }
                for f in fields {
                    *state = combine(*state, hash_str(&f.name));
                    *state = combine(*state, type_expr_key(&f.ty));
                }
                if let Some(r) = repr {
                    *state = combine(*state, hash_str(&format!("{:?}", r)));
                }
            }
            Item::Enum {
                name,
                type_params,
                variants,
                repr,
                ..
            } => {
                *state = combine(*state, hash_str(name));
                for p in type_params {
                    *state = combine(*state, hash_str(&p.name));
                    for bound in &p.bounds {
                        *state = combine(*state, hash_str(bound));
                    }
                }
                for v in variants {
                    *state = combine(*state, hash_str(&v.name));
                    for f in &v.fields {
                        let field_name = match f {
                            VariantField::Positional { .. } => "",
                            VariantField::Named { name, .. } => name,
                        };
                        *state = combine(*state, hash_str(field_name));
                        *state = combine(*state, type_expr_key(f.ty()));
                    }
                }
                if let Some(r) = repr {
                    *state = combine(*state, hash_str(&format!("{:?}", r)));
                }
            }
            Item::Fn {
                name,
                type_params,
                params,
                ret,
                requires,
                ensures,
                ..
            } => {
                *state = combine(*state, hash_str(name));
                for p in type_params {
                    *state = combine(*state, hash_str(&p.name));
                    for bound in &p.bounds {
                        *state = combine(*state, hash_str(bound));
                    }
                }
                for p in params {
                    *state = combine(*state, hash_str(&p.name));
                    *state = combine(*state, type_expr_key(&p.ty));
                }
                if let Some(r) = ret {
                    *state = combine(*state, type_expr_key(r));
                }
                for c in requires {
                    *state = combine(*state, expr_key(&c.expr));
                }
                for c in ensures {
                    *state = combine(*state, expr_key(&c.expr));
                }
            }
            Item::ConstFn {
                name, params, ret, ..
            } => {
                *state = combine(*state, hash_str(name));
                for p in params {
                    *state = combine(*state, hash_str(&p.name));
                    *state = combine(*state, type_expr_key(&p.ty));
                }
                if let Some(r) = ret {
                    *state = combine(*state, type_expr_key(r));
                }
            }
            Item::Impl {
                type_name,
                trait_name,
                methods,
                ..
            } => {
                *state = combine(*state, hash_str(type_name));
                if let Some(trait_name) = trait_name {
                    *state = combine(*state, hash_str(trait_name));
                }
                for m in methods {
                    self.hash_item_signature(state, m);
                }
            }
            Item::Trait {
                name,
                type_params,
                methods,
                ..
            } => {
                *state = combine(*state, hash_str(name));
                for param in type_params {
                    *state = combine(*state, hash_str(&param.name));
                }
                for method in methods {
                    *state = combine(*state, hash_str(&method.name));
                    for p in &method.params {
                        *state = combine(*state, hash_str(&p.name));
                        *state = combine(*state, type_expr_key(&p.ty));
                    }
                    if let Some(r) = &method.ret {
                        *state = combine(*state, type_expr_key(r));
                    }
                }
            }
            Item::Object {
                name,
                fields,
                methods,
                repr,
                ..
            } => {
                *state = combine(*state, hash_str(name));
                for f in fields {
                    *state = combine(*state, hash_str(&f.name));
                    *state = combine(*state, type_expr_key(&f.ty));
                }
                for m in methods {
                    self.hash_item_signature(state, m);
                }
                if let Some(r) = repr {
                    *state = combine(*state, hash_str(&format!("{:?}", r)));
                }
            }
            Item::Extern {
                abi, link, decls, ..
            } => {
                if let Some(a) = abi {
                    *state = combine(*state, hash_str(a));
                }
                if let Some(l) = link {
                    *state = combine(*state, hash_str(l));
                }
                for d in decls {
                    *state = combine(*state, hash_str(&d.name));
                    if let Some(s) = &d.symbol {
                        *state = combine(*state, hash_str(s));
                    }
                    for p in &d.params {
                        *state = combine(*state, hash_str(&p.name));
                        *state = combine(*state, type_expr_key(&p.ty));
                    }
                    if let Some(r) = &d.ret {
                        *state = combine(*state, type_expr_key(r));
                    }
                }
            }
        }
    }

    /// Parse a registered module, reusing the memo when its text has not moved.
    /// A fresh parse is not stored here because this takes `&self`; `parse`
    /// owns the store, and `add_parsed` seeds it for modules the driver already
    /// parsed.
    fn parse_cached(&self, file: FileId) -> Option<Arc<yashy_parser::ast::Program>> {
        let reads = Reads::new(
            self.flags(),
            combine(self.text_hash(file), hash_str(&self.compiler)),
        );
        if let Some(entry) = self.parse_table.get(&file.index()) {
            if entry.reads == reads.hash() {
                return Some(entry.value.clone());
            }
        }
        self.parse_fresh(file)
    }

    fn parse_fresh(&self, file: FileId) -> Option<Arc<yashy_parser::ast::Program>> {
        let text = self.sources.file(file);
        let mut map = SourceMap::new();
        // The parser borrows the map for diagnostics; it must hand back the
        // same file id, so pad the fresh map to the id the caller uses.
        for index in 0..file.index() {
            map.add_file(format!("<parse:{index}>"), String::new());
        }
        let id = map.add_file(format!("<parse:{}>", file.index()), text.to_string());
        debug_assert_eq!(id.index(), file.index());
        let tokens = yashy_lexer::Lexer::with_file(text, file).tokenize().ok()?;
        let program = Arc::new(
            yashy_parser::Parser::new(tokens, &map, file)
                .parse_program()
                .ok()?,
        );
        Some(program)
    }

    /// The world must be checked before any unit query. The whole program is
    /// cached once and reused; the per-file check results are slices of it.
    fn ensure_world(&mut self) -> QueryResult<Arc<hir::HirProgram>> {
        if let Some(w) = &self.world {
            return Ok(w.program.clone());
        }

        // Dependency order, entry last: the checker builds the HIR in the order
        // it is given, and code generation expects the entry point's items last.
        let order: Vec<FileId> = if self.order.is_empty() {
            self.modules.iter().map(|m| m.file).collect()
        } else {
            self.order.clone()
        };

        for file in &order {
            self.parse_cached(*file).ok_or_else(|| {
                vec![Diagnostic::new(
                    Severity::Error,
                    *file,
                    Span::new(0, 0),
                    "parse failed",
                )]
            })?;
        }

        let parsed: Vec<Arc<yashy_parser::ast::Program>> = order
            .iter()
            .map(|file| self.parse_cached(*file).expect("parse just ran above"))
            .collect();
        let modules: Vec<(FileId, &yashy_parser::ast::Program)> = order
            .iter()
            .zip(parsed.iter())
            .map(|(file, program)| (*file, &**program))
            .collect();

        let check_options = CheckOptions {
            release: self.release,
        };
        let hir = check_modules_with(&modules, &self.sources, check_options)?;
        let hir = Arc::new(hir);

        self.world = Some(World {
            program: hir.clone(),
        });

        Ok(hir)
    }

    /// The checked module graph: the driver needs the whole `hir::HirProgram` in
    /// dependency order for code generation.
    pub fn check(&mut self, file: FileId) -> QueryResult<CheckedFile> {
        let program = self.ensure_world()?;
        let reads = Reads::new(self.flags(), self.check_key(file));

        let key = file.index();
        if let Some(entry) = self.check.get(&key) {
            if entry.reads == reads.hash() {
                let mut value = entry.value.clone();
                value.program = program.clone();
                self.stats.hits += 1;
                return Ok(value);
            }
        }
        self.stats.misses += 1;
        self.recomputed
            .insert(format!("check:{}", self.path_of(file)));

        let functions = program
            .items
            .iter()
            .filter_map(|item| match item {
                hir::HirItem::Fn(f) if f.file == file => Some(FnId::new(file, f.name.clone())),
                _ => None,
            })
            .collect();

        let value = CheckedFile {
            file,
            functions,
            program: program.clone(),
        };
        self.check.insert(
            key,
            Entry {
                reads: reads.hash(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    /// The read set of a `check`: the file's own text plus the interface of
    /// every module it imports. An importer re-checks when a dependency's
    /// declarations change, and not when only its bodies do.
    fn check_key(&self, file: FileId) -> u64 {
        let mut state = combine(self.text_hash(file), hash_str(&self.compiler));
        let imports = match self.parse_cached(file) {
            Some(p) => p
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Import { path, .. } => Some(path.last().cloned().unwrap_or_default()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        for name in imports {
            let iface = self
                .modules
                .iter()
                .find(|m| m.name.as_deref() == Some(&name))
                .map(|m| self.interface_hash(m.file));
            if let Some(iface) = iface {
                state = combine(state, iface);
            } else {
                // Unknown import: invalidate on every module's interface. A
                // compiled program resolves every import, so this is a
                // fallback that can only recompute more, never less.
                for m in &self.modules {
                    state = combine(state, self.interface_hash(m.file));
                }
                break;
            }
        }
        state
    }

    /// The hashes of every struct/enum declaration whose name appears in the
    /// function's source text. A field edit changes the declaration's hash and
    /// invalidates exactly the functions that name the type; a function that
    /// never mentions it keeps its cached facts and IR.
    ///
    /// Name matching is a textual over-approximation: a comment or a local
    /// with the type's name counts as a mention, which recomputes more, never
    /// less.
    fn type_reads(&self, fhir: &hir::HirFn, file_text: &str) -> Vec<u64> {
        let range = fhir.span.start as usize..fhir.span.end as usize;
        if range.start > file_text.len() || range.end > file_text.len() {
            return Vec::new();
        }
        let slice = &file_text[range];
        let mut mentioned: BTreeSet<String> = BTreeSet::new();
        let mut word = String::new();
        for ch in slice.chars() {
            if ch.is_alphanumeric() || ch == '_' {
                word.push(ch);
            } else if !word.is_empty() {
                mentioned.insert(std::mem::take(&mut word));
            }
        }
        if !word.is_empty() {
            mentioned.insert(word);
        }

        let mut out = Vec::new();
        for m in &self.modules {
            let Some(program) = self.parse_cached(m.file) else {
                continue;
            };
            for item in &program.items {
                let item = match item {
                    Item::Public { item, .. } => item.as_ref(),
                    Item::Effect { item, .. } => item.as_ref(),
                    Item::Documented { item, .. } => item.as_ref(),
                    item => item,
                };
                match item {
                    Item::Struct {
                        name,
                        type_params,
                        fields,
                        repr,
                        ..
                    } if mentioned.contains(name.as_str()) => {
                        let mut state = hash_str(name);
                        for p in type_params {
                            state = combine(state, hash_str(&p.name));
                            for bound in &p.bounds {
                                state = combine(state, hash_str(bound));
                            }
                        }
                        for f in fields {
                            state = combine(state, hash_str(&f.name));
                            state = combine(state, type_expr_key(&f.ty));
                        }
                        if let Some(r) = repr {
                            state = combine(state, hash_str(&format!("{:?}", r)));
                        }
                        out.push(state);
                    }
                    Item::Enum {
                        name,
                        type_params,
                        variants,
                        repr,
                        ..
                    } if mentioned.contains(name.as_str()) => {
                        let mut state = hash_str(name);
                        for p in type_params {
                            state = combine(state, hash_str(&p.name));
                            for bound in &p.bounds {
                                state = combine(state, hash_str(bound));
                            }
                        }
                        for v in variants {
                            state = combine(state, hash_str(&v.name));
                            for f in &v.fields {
                                let field_name = match f {
                                    VariantField::Positional { .. } => "",
                                    VariantField::Named { name, .. } => name,
                                };
                                state = combine(state, hash_str(field_name));
                                state = combine(state, type_expr_key(f.ty()));
                            }
                        }
                        if let Some(r) = repr {
                            state = combine(state, hash_str(&format!("{:?}", r)));
                        }
                        out.push(state);
                    }
                    _ => {}
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Verified facts for a function. The read set is the function's own source
    /// text plus the flags: a contract edit changes the source, a contract
    /// rule change changes the flags, and neither is missed.
    pub fn verify_fn(&mut self, f: FnId) -> QueryResult<FnFacts> {
        let program = self.ensure_world()?;

        let Some(fhir) = program.items.iter().find_map(|item| match item {
            hir::HirItem::Fn(g) if g.name == f.name && g.file == f.file => Some(g),
            _ => None,
        }) else {
            return Err(vec![Diagnostic::new(
                Severity::Error,
                f.file,
                Span::new(0, 0),
                "function not found in the checked program",
            )]);
        };
        let text = self.sources.file(fhir.file);
        let mut key = fn_hash(fhir, text);
        // A field or variant edit changes the declaration, so every function
        // that names the type must re-verify even though its own text did not
        // move.
        for decl in self.type_reads(fhir, text) {
            key = combine(key, decl);
        }
        // A contract on a callee is an assumption the caller's proof rests on,
        // so the callee's text is part of the caller's read set too.
        for name in Self::callee_names(fhir, text) {
            if name == f.name {
                continue;
            }
            if let Some(callee) = program.items.iter().find_map(|item| match item {
                hir::HirItem::Fn(g) if g.name == name && g.file == fhir.file => Some(g),
                _ => None,
            }) {
                key = combine(key, fn_hash(callee, text));
            }
        }

        let reads = Reads::new(self.flags(), key);

        let entry_key = f.clone();
        if let Some(entry) = self.verify_fn.get(&entry_key) {
            if entry.reads == reads.hash() {
                let value = entry.value.clone();
                self.stats.hits += 1;
                return Ok(value);
            }
        }
        self.stats.misses += 1;
        self.recomputed
            .insert(format!("verify:{}:{}", self.path_of(f.file), f.name));

        // Recompute: the verifier is the single source of truth for these facts;
        // the database memoises only its answer.
        let report = self.verify_program()?;
        let facts = &report.facts;
        let rel_start = fhir.span.start;
        let proven: Vec<(u32, u32)> = facts
            .proven_spans()
            .filter(|s| s.start >= rel_start && s.end <= fhir.span.end)
            .map(|s| (s.start - rel_start, s.end - rel_start))
            .collect();
        let shift_proven: Vec<(u32, u32)> = facts
            .shift_spans()
            .filter(|s| s.start >= rel_start && s.end <= fhir.span.end)
            .map(|s| (s.start - rel_start, s.end - rel_start))
            .collect();

        let value = FnFacts {
            id: f,
            proven: proven.into_iter().collect(),
            shift_proven: shift_proven.into_iter().collect(),
        };
        self.verify_fn.insert(
            entry_key,
            Entry {
                reads: reads.hash(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    /// Emitted IR for a single function, keyed on the function.
    pub fn emit_fn(&mut self, f: FnId) -> QueryResult<String> {
        let program = self.ensure_world()?;

        let Some(fhir) = program.items.iter().find_map(|item| match item {
            hir::HirItem::Fn(g) if g.name == f.name && g.file == f.file => Some(g),
            _ => None,
        }) else {
            return Err(vec![Diagnostic::new(
                Severity::Error,
                f.file,
                Span::new(0, 0),
                "function not found in the checked program",
            )]);
        };
        let text = self.sources.file(fhir.file);
        let mut reads_state = self.flags();
        reads_state = combine(reads_state, fn_hash(fhir, text));
        for decl in self.type_reads(fhir, text) {
            reads_state = combine(reads_state, decl);
        }

        // The read set includes the IR of every callee, so a changed callee is
        // re-emitted too.
        let mut computing = BTreeSet::new();
        let callee_keys = Self::callee_names(fhir, text);
        for name in callee_keys {
            if name == f.name || computing.contains(&name) {
                continue;
            }
            computing.insert(name.clone());
            if let Some(_c) = program.items.iter().find_map(|item| match item {
                hir::HirItem::Fn(g) if g.name == name && g.file == fhir.file => Some(g),
                _ => None,
            }) {
                let cfn = FnId::new(fhir.file, name.clone());
                let ir = self.emit_fn(cfn)?;
                reads_state = combine(reads_state, hash_str(&ir));
            }
        }
        let reads = Reads::new(self.flags(), reads_state);

        let entry_key = f.clone();
        if let Some(entry) = self.emit_fn.get(&entry_key) {
            if entry.reads == reads.hash() {
                let value = entry.value.clone();
                self.stats.hits += 1;
                return Ok(value);
            }
        }
        self.stats.misses += 1;
        self.recomputed
            .insert(format!("emit:{}:{}", self.path_of(f.file), f.name));

        let value = self.recompute_emit_fn(&program, f)?;
        self.emit_fn.insert(
            entry_key,
            Entry {
                reads: reads.hash(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    fn recompute_emit_fn(
        &mut self,
        program: &Arc<hir::HirProgram>,
        f: FnId,
    ) -> QueryResult<String> {
        use yashy_codegen::{emit_program, CodegenOptions};
        let options = CodegenOptions {
            fp_contract: self.fp_contract,
            escape: true,
            ..CodegenOptions::default()
        };
        let entry = self.entry_path();
        let report = self.verify_program()?;
        let ir = emit_program(program, &entry, options, &report.facts);
        // The emitter spells a definition as `define <ret> @<name>(...) {`.
        // Find the `define` line that names this function; a call inside
        // another body also mentions `@<name>(`, so the line prefix is what
        // distinguishes a definition from a use.
        let marker = format!("@{}(", f.name);
        let mut start = None;
        let mut offset = 0usize;
        for line in ir.split_inclusive('\n') {
            if line.starts_with("define ") && line.contains(&marker) {
                start = Some(offset);
                break;
            }
            offset += line.len();
        }
        let start = start.ok_or_else(|| {
            vec![Diagnostic::new(
                Severity::Error,
                f.file,
                Span::new(0, 0),
                "function IR not found",
            )]
        })?;
        let mut end = start;
        let mut depth = 0usize;
        for (i, ch) in ir[start..].char_indices() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
        }
        if depth != 0 {
            return Err(vec![Diagnostic::new(
                Severity::Error,
                f.file,
                Span::new(0, 0),
                "function IR has unbalanced braces",
            )]);
        }
        Ok(ir[start..end].to_string())
    }

    /// Collect function names that look like callees inside a function's
    /// source text, using simple name co-occurrence: a name is a candidate
    /// callee if it is declared as `fn name` and appears as a call `name(...)`
    /// in this function. Over-approximates (false positives re-run more, never
    /// less).
    fn callee_names(fhir: &hir::HirFn, file_text: &str) -> Vec<String> {
        let range = fhir.span.start as usize..fhir.span.end as usize;
        let slice = if range.start > file_text.len() || range.end > file_text.len() {
            return Vec::new();
        } else {
            &file_text[range]
        };

        // Names declared as functions: `fn name(` or `const fn name(`.
        let mut declared: BTreeSet<String> = BTreeSet::new();
        for line in slice.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("fn ") || trimmed.starts_with("const fn ") {
                let rest = if trimmed.starts_with("const fn ") {
                    &trimmed[9..]
                } else {
                    &trimmed[3..]
                };
                let name = rest
                    .split(|c: char| c.is_whitespace() || c == '(')
                    .next()
                    .unwrap_or("");
                if !name.is_empty() && name != "self" && name != "Self" {
                    declared.insert(name.to_string());
                }
            }
        }
        // Names that look like calls: `name(`.
        let mut used: BTreeSet<String> = BTreeSet::new();
        for tok in slice.split_inclusive(|c: char| c.is_whitespace()) {
            let name = tok.trim_end_matches(|c: char| {
                c == '('
                    || c == ')'
                    || c == '{'
                    || c == '}'
                    || c == ','
                    || c == ';'
                    || c == ':'
                    || c == '<'
                    || c == '>'
            });
            if name.len() > 1 && declared.contains(name) {
                used.insert(name.to_string());
            }
        }
        // A callee is a declared name that is actually called here.
        used.difference(&declared).cloned().collect()
    }

    /// Parse a registered module, memoised. A parse failure is not cached: a
    /// fixed file must be able to re-parse on the next call.
    pub fn parse(&mut self, file: FileId) -> QueryResult<Arc<yashy_parser::ast::Program>> {
        let reads = Reads::new(
            self.flags(),
            combine(self.text_hash(file), hash_str(&self.compiler)),
        );

        if let Some(entry) = self.check_parse(&file.index()) {
            if entry.reads == reads.hash() {
                let value = entry.value.clone();
                self.stats.hits += 1;
                return Ok(value);
            }
        }
        self.stats.misses += 1;
        self.recomputed
            .insert(format!("parse:{}", self.path_of(file)));

        let text = self.sources.file(file);
        let mut map = SourceMap::new();
        // The parser borrows the map for diagnostics; it must hand back the
        // same file id, so pad the fresh map to the id the caller uses.
        for index in 0..file.index() {
            map.add_file(format!("<parse:{index}>"), String::new());
        }
        let id = map.add_file(format!("<parse:{}>", file.index()), text.to_string());
        debug_assert_eq!(id.index(), file.index());
        let tokens = yashy_lexer::Lexer::with_file(text, file).tokenize()?;
        let program = Arc::new(yashy_parser::Parser::new(tokens, &map, file).parse_program()?);

        self.store_parse(
            file.index(),
            Entry {
                reads: reads.hash(),
                value: program.clone(),
            },
        );
        Ok(program)
    }

    fn check_parse(&self, key: &u32) -> Option<&Entry<Arc<yashy_parser::ast::Program>>> {
        self.parse_table.get(key)
    }

    fn store_parse(&mut self, key: u32, entry: Entry<Arc<yashy_parser::ast::Program>>) {
        self.parse_table.insert(key, entry);
    }

    /// Emitted IR for the whole program, cached. Used by the driver for builds.
    pub fn emit_program(&mut self) -> QueryResult<String> {
        let program = self.ensure_world()?;
        use yashy_codegen::{emit_program, CodegenOptions};
        let options = CodegenOptions {
            fp_contract: self.fp_contract,
            escape: true,
            ..CodegenOptions::default()
        };
        let entry = self.entry_path();
        let report = self.verify_program()?;
        Ok(emit_program(&program, &entry, options, &report.facts))
    }

    /// Whole-program verification, cached. The world is checked first, so this
    /// never panics on a fresh database: a caller that asks for the proof gets
    /// the checker's diagnostics instead.
    pub fn verify_program(&mut self) -> QueryResult<Arc<yashy_verify::VerifyReport>> {
        if let Some(v) = &self.verify {
            return Ok(v.clone());
        }
        let program = self.ensure_world()?;
        let report = Arc::new(yashy_verify::verify(&program, &self.verify_opts));
        self.verify = Some(report.clone());
        Ok(report)
    }

    /// The FNV-1a hash of the compiler version, target and flags that shape
    /// every output.
    fn flags(&self) -> u64 {
        let mut state = hash_str(&self.compiler);
        state = combine(state, hash_str(&self.target));
        state = combine(state, hash_str(&format!("{}", self.release)));
        state = combine(state, hash_str(&self.fp_contract.to_string()));
        state = combine(state, hash_str(&format!("{:?}", self.verify_opts)));
        state
    }

    fn path_of(&self, id: FileId) -> &str {
        let index = id.index() as usize;
        self.modules
            .get(index)
            .map(|m| m.path.as_str())
            .unwrap_or("<unknown>")
    }

    fn entry_path(&self) -> String {
        self.entry
            .map(|e| self.modules[e.index() as usize].path.clone())
            .unwrap_or_default()
    }

    /// Query statistics since the last snapshot.
    pub fn stats(&self) -> QueryStats {
        QueryStats {
            hits: self.stats.hits,
            misses: self.stats.misses,
            recomputed: self.recomputed.iter().cloned().collect(),
        }
    }

    /// Clear the query counters; recomputed names are retained by the
    /// `QueryStats` value returned.
    pub fn reset_stats(&mut self) {
        self.stats.hits = 0;
        self.stats.misses = 0;
        self.recomputed.clear();
    }
}
