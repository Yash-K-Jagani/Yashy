//! Imports: turning `import a.b.c;` into files, and each file into one module.
//!
//! A module is one source file. The loader walks the import graph depth-first
//! from the entry point, resolving every dotted path against the search roots
//! and reading each file **once**: a module imported from three places is one
//! [`Module`], which is what makes its definitions single rather than tripled.
//!
//! ```text
//! a.b.c  ->  <root>/a/b/c.ya  and  <root>/a/b/c/mod.ya
//! ```
//!
//! The roots are `YASHY_PATH` when it is set and non-empty, and the entry
//! file's directory otherwise. The bundled standard library is appended after
//! those roots, so `import io;` works without any environment variable while a
//! project can still put its own module first. `YASHY_PATH` therefore replaces
//! the *entry directory* default, not the stdlib; the separators are `;` on
//! Windows and `:` everywhere else.
//!
//! An import cycle is refused with the cycle written out. Yashy has no
//! `forward` declarations or module-level initialization order, so there is
//! nothing a cycle could mean yet; reporting it now beats defining a meaning
//! that the first real program would have to live with.
//!
//! Every module gets its own [`FileId`] in the caller's [`SourceMap`], and its
//! items keep spans into its own text. That is why checking can attribute a
//! diagnostic to the module it came from instead of the entry point.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::{Diagnostic, FileId, Severity, SourceMap, Span};
use yashy_lexer::{Lexer, Token, TokenKind};
use yashy_parser::ast::{Item, Program};
use yashy_parser::Parser;

/// How deeply imports may nest before the loader refuses the program.
///
/// Loading is recursive, one stack frame per module, so an unbounded chain is
/// an unbounded stack. The limit is a stack-safety bound like the parser's
/// nesting limit, not a style rule: real programs nest a handful of levels, and
/// a generated graph thousands deep is pathological. Refusing it with a
/// diagnostic is the alternative to crashing on it.
const MAX_MODULE_DEPTH: usize = 64;

/// One successfully loaded module.
pub struct Module {
    /// The path the module was found at, as it will be named in diagnostics.
    pub path: PathBuf,
    /// The id its text got in the source map the loader was given.
    pub file: FileId,
    /// The last segment of the path the program imported it under, which is
    /// the name an `import` statement writes and the name the warning pass
    /// compares against. `None` for the entry file, which nothing imports.
    ///
    /// This is recorded rather than derived from `path` because the two
    /// layouts disagree: `import a.b.c;` finds `<root>/a/b/c/mod.ya` as well
    /// as `<root>/a/b/c.ya`, and only the loader knows which one it took. A
    /// module's name is what the program called it, not what its file is
    /// called.
    pub name: Option<String>,
    /// Its parsed items, with spans into this module's own text.
    pub program: Program,
}

/// The import graph of a loaded program: every discovered module, and every
/// edge one module's `import` statement names. The phase 4 query pipeline
/// takes this as an input (`Db::module_graph`), so a change to `b.ya`
/// invalidates `a.ya`'s check when `a` imports `b`, and a change to an
/// unrelated file invalidates nothing.
pub struct ModuleGraph {
    /// Every discovered module, dependencies first.
    pub modules: Vec<ModuleSource>,
    /// `(importer, imported)` for each `import` statement, in discovery order.
    pub edges: Vec<(FileId, FileId)>,
}

/// A module discovered by the loader but not yet parsed.
///
/// Discovery still lexes each file -- the import graph lives in its tokens --
/// but parsing is deferred so the driver can parse every file in parallel.
pub struct ModuleSource {
    /// The path the module was found at, as it will be named in diagnostics.
    pub path: PathBuf,
    /// The id its text got in the source map the loader was given.
    pub file: FileId,
    /// The last segment of the path the program imported it under. `None` for
    /// the entry file.
    pub name: Option<String>,
    /// Pre-order position in the discovery walk.
    ///
    /// When several files have parse errors, the earliest one wins, because
    /// that is the file the serial loader would have stopped at. Discovery is
    /// depth-first, so a file is numbered before the modules it imports.
    pub order: u32,
    /// The file's tokens, ready to parse.
    pub tokens: Vec<Token>,
}

impl ModuleSource {
    /// Parse this module into a [`Module`].
    ///
    /// The parser borrows a private [`SourceMap`] because it slices source text
    /// (string literals, numeric literals); the returned program's spans are
    /// self-contained and the map the loader filled is used only for rendering
    /// diagnostics later. The private map is padded so the file id handed back
    /// matches the id the caller renders against.
    pub fn parse(&self, sources: &SourceMap) -> Result<Module, Vec<Diagnostic>> {
        let text = sources.file(self.file);
        let mut map = SourceMap::new();
        for index in 0..self.file.index() {
            map.add_file(format!("<parse:{index}>"), String::new());
        }
        let id = map.add_file(format!("<parse:{}>", self.file.index()), text.to_string());
        debug_assert_eq!(id.index(), self.file.index());
        let program = Parser::new(self.tokens.clone(), &map, self.file).parse_program()?;
        Ok(Module {
            path: self.path.clone(),
            file: self.file,
            name: self.name.clone(),
            program,
        })
    }
}

/// Every name a loaded module declares at top level, keyed by the name the
/// program imports it under.
///
/// The warning pass needs this to answer "was this import used?" for a program
/// that uses a module's names without a prefix (`import wc_words;` then
/// `count_words(..)`): the import is used when any name the module declares is
/// read, whether or not the module's own name appears.
///
/// Every top-level name is listed, not only the `pub` ones, because the checker
/// puts every module's items in one namespace today, so a private name is
/// reachable too. Listing it keeps the warning honest about what the program can
/// actually reach, and the list errs towards silence.
pub type Exports = BTreeMap<String, BTreeSet<String>>;

/// The [`Exports`] table for `modules`.
pub fn exports(modules: &[Module]) -> Exports {
    let mut table = Exports::new();
    for module in modules {
        let Some(name) = &module.name else { continue };
        let names = table.entry(name.clone()).or_default();
        for item in &module.program.items {
            if let Some(declared) = declared_name(item) {
                names.insert(declared.to_string());
            }
        }
    }
    table
}

/// The name an item declares, looking through `pub` and effect modifiers.
///
/// An `impl` block and an `extern` block declare no name of their own: their
/// methods and functions belong to a type or to C, not to the module's
/// namespace.
fn declared_name(item: &Item) -> Option<&str> {
    match item {
        Item::Public { item, .. } | Item::Effect { item, .. } | Item::Documented { item, .. } => {
            declared_name(item)
        }
        Item::Fn { name, .. }
        | Item::ConstFn { name, .. }
        | Item::Struct { name, .. }
        | Item::Enum { name, .. }
        | Item::Object { name, .. }
        | Item::Trait { name, .. } => Some(name),
        _ => None,
    }
}

/// The standard-library directory shipped with this compiler, if one can be
/// found.
///
/// The explicit `YASHY_STDLIB` variable wins. Otherwise the search follows
/// the layouts an installer can produce, then the development checkout. A
/// candidate must be a directory; a stale path is ignored rather than turned
/// into a confusing "module not found" for every import.
pub fn default_stdlib_root() -> Option<PathBuf> {
    if let Some(value) = env::var_os("YASHY_STDLIB") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(root) = stdlib_root_near_exe(&exe) {
            return Some(root);
        }
    }

    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("yashy-stdlib")
        .join("yashy");
    if dev.is_dir() {
        return Some(dev);
    }

    None
}

/// The stdlib layouts an installed `yashy` binary can sit next to.
fn stdlib_root_near_exe(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let candidates = [
        dir.join("yashy-stdlib").join("yashy"),
        dir.join("stdlib"),
        // The Unix installer writes the archive's `yashy` directory under
        // share/yashy/stdlib, so the nested form is the one a package-manager
        // style prefix produces. The flat form is kept for older installs and
        // for a hand-built prefix.
        dir.join("..")
            .join("share")
            .join("yashy")
            .join("stdlib")
            .join("yashy"),
        dir.join("..").join("share").join("yashy").join("stdlib"),
    ];
    candidates.into_iter().find(|candidate| candidate.is_dir())
}

/// The directories an import is looked up in.
///
/// `YASHY_PATH` when it is set and has at least one non-empty entry, and the
/// entry file's directory otherwise. The bundled stdlib root is appended in
/// both cases, so a user program and the stdlib can be found together. See the
/// module documentation for the separator.
pub fn module_roots(entry: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Some(value) = env::var_os("YASHY_PATH") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        roots.extend(
            value
                .to_string_lossy()
                .split(separator)
                .filter(|root| !root.is_empty())
                .map(PathBuf::from),
        );
    }

    if roots.is_empty() {
        match entry.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => roots.push(parent.to_path_buf()),
            _ => roots.push(PathBuf::from(".")),
        }
    }

    if let Some(stdlib) = default_stdlib_root() {
        if !roots.contains(&stdlib) {
            roots.push(stdlib);
        }
    }

    roots
}

/// Resolve a dotted module path to a file.
///
/// Each root is tried in order, and each root has two shapes: a file
/// `<root>/a/b/c.ya`, then a directory module `<root>/a/b/c/mod.ya`. The first
/// hit wins, so an unambiguous layout needs no further rule.
///
/// `from` names the importing file in the diagnostic's label; the caller owns
/// the span (and the file id), because only it knows where the `import` sits.
pub fn resolve_import(
    spec: &str,
    from: Option<&Path>,
    roots: &[PathBuf],
) -> Result<PathBuf, Diagnostic> {
    let segments: Vec<&str> = spec.split('.').collect();
    if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return Err(import_error(
            format!("`{spec}` is not a module path: write dotted names, such as `std.io`"),
            from,
        ));
    }

    let mut tried: Vec<String> = Vec::new();
    for root in roots {
        let mut base = root.clone();
        for segment in &segments {
            base.push(segment);
        }

        let mut file = base.clone();
        file.set_extension("ya");
        if file.is_file() {
            return Ok(file);
        }
        tried.push(file.display().to_string());

        let directory = base.join("mod.ya");
        if directory.is_file() {
            return Ok(directory);
        }
        tried.push(directory.display().to_string());
    }

    Err(import_error(
        format!("cannot find module `{spec}`: tried {}", tried.join(", ")),
        from,
    ))
}

/// Discover every module in the import graph without parsing it.
///
/// The loader walks the import graph depth-first from the entry point,
/// registering each file in the caller's [`SourceMap`] once and lexing it to
/// find the `import` statements that name more files. Parsing is deliberately
/// deferred: after discovery, the driver can parse every file in parallel.
///
/// The returned list has dependencies before dependants and the entry point
/// last; the entry is still file id zero, because its text is the first one
/// added to `sources`.
///
/// `entry_source` is passed in rather than read here so the CLI can report an
/// unreadable entry point with the same message it uses for every other input
/// file. Imported files are read here, because only the loader knows which ones
/// are wanted.
pub fn load_sources(
    sources: &mut SourceMap,
    entry_path: &Path,
    entry_source: String,
) -> Result<Vec<ModuleSource>, Vec<Diagnostic>> {
    load_graph(sources, entry_path, entry_source).map(|graph| graph.modules)
}

/// Load the module graph for `entry_path`: every module and every import edge.
pub fn load_graph(
    sources: &mut SourceMap,
    entry_path: &Path,
    entry_source: String,
) -> Result<ModuleGraph, Vec<Diagnostic>> {
    let roots = module_roots(entry_path);
    let mut loader = SourceLoader {
        sources,
        roots,
        seen: HashMap::new(),
        stack: Vec::new(),
        next_order: 0,
        sources_out: Vec::new(),
        edges: Vec::new(),
    };
    loader.visit(
        identity_of(entry_path),
        entry_path,
        entry_source,
        None,
        None,
    )?;
    Ok(ModuleGraph {
        modules: loader.sources_out,
        edges: loader.edges,
    })
}

/// Load `entry_path` and everything it imports, parsing every module on the
/// calling thread.
///
/// This is the serial form used by callers that need the parsed modules and do
/// not run a timed pipeline (the language server, the loader's own tests). The
/// compiler's front end calls [`load_sources`] and parses in parallel instead;
/// both produce the same modules in the same order.
pub fn load(
    sources: &mut SourceMap,
    entry_path: &Path,
    entry_source: String,
) -> Result<Vec<Module>, Vec<Diagnostic>> {
    let discovered = load_sources(sources, entry_path, entry_source)?;
    let map: &SourceMap = sources;

    // Parse in discovery order (pre-order), so a file that fails is the first
    // one the old serial loader would have stopped at -- the entry point when
    // it is the broken one. The results go back into the loader's order:
    // dependencies first, the entry point last.
    let mut order: Vec<usize> = (0..discovered.len()).collect();
    order.sort_by_key(|index| discovered[*index].order);
    let mut parsed: Vec<Option<Module>> = (0..discovered.len()).map(|_| None).collect();
    for index in order {
        parsed[index] = Some(discovered[index].parse(map)?);
    }
    Ok(parsed.into_iter().flatten().collect())
}

/// Whether a module has been fully discovered or is still on the loader's stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Loading,
    Loaded,
}

/// Discovers modules in the import graph without parsing them.
///
/// Lexing stays here rather than in the parallel parse phase because the
/// import graph is read from the tokens: doing it serially keeps file ids in
/// source order and keeps a syntax error's diagnostic identical to the serial
/// loader's.
struct SourceLoader<'a> {
    sources: &'a mut SourceMap,
    roots: Vec<PathBuf>,
    /// The canonical path of every module seen, and how far its discovery got.
    seen: HashMap<PathBuf, Seen>,
    /// The modules currently being discovered, outermost first: `(identity, path)`.
    stack: Vec<(PathBuf, PathBuf)>,
    /// The next pre-order number to hand a newly discovered module.
    next_order: u32,
    /// Discovered modules, dependencies first.
    sources_out: Vec<ModuleSource>,
    /// Import edges in discovery order: `(importer, imported)`.
    edges: Vec<(FileId, FileId)>,
}

impl SourceLoader<'_> {
    fn visit(
        &mut self,
        identity: PathBuf,
        path: &Path,
        source: String,
        site: Option<(FileId, Span)>,
        name: Option<String>,
    ) -> Result<(), Vec<Diagnostic>> {
        match self.seen.get(&identity) {
            Some(Seen::Loaded) => return Ok(()),
            Some(Seen::Loading) => return Err(vec![self.cycle_diagnostic(&identity, path, site)]),
            None => {}
        }

        if self.stack.len() >= MAX_MODULE_DEPTH {
            let message = format!(
                "imports nest more than {MAX_MODULE_DEPTH} modules deep: refusing a stack overflow"
            );
            return Err(vec![self.at(site, message)]);
        }

        self.seen.insert(identity.clone(), Seen::Loading);
        self.stack.push((identity.clone(), path.to_path_buf()));

        let order = self.next_order;
        self.next_order += 1;

        let file = self
            .sources
            .add_file(path.to_string_lossy().into_owned(), source);

        let tokens = match Lexer::with_file(self.sources.file(file), file).tokenize() {
            Ok(tokens) => tokens,
            Err(diagnostics) => return Err(self.abandon(identity, diagnostics)),
        };

        // The import graph is read from the tokens, not from the raw text, so
        // a word in a comment or string can never be mistaken for an import.
        let imports = scan_imports(&tokens, self.sources.file(file));

        for (segments, alias, span) in imports {
            let spec = segments.join(".");
            let target = match resolve_import(&spec, Some(path), &self.roots) {
                Ok(target) => target,
                Err(mut diagnostic) => {
                    diagnostic.file = file;
                    diagnostic.span = span;
                    return Err(self.abandon(identity, vec![diagnostic]));
                }
            };

            let target_source = match fs::read_to_string(&target) {
                Ok(source) => source,
                Err(error) => {
                    let diagnostic = self.at(
                        Some((file, span)),
                        format!("cannot read module `{}`: {error}", target.display()),
                    );
                    return Err(self.abandon(identity, vec![diagnostic]));
                }
            };

            if let Err(diagnostics) = self.visit(
                identity_of(&target),
                &target,
                target_source,
                Some((file, span)),
                alias.or_else(|| segments.last().cloned()),
            ) {
                return Err(self.abandon(identity, diagnostics));
            }
            self.edges.push((file, self.file_of(&target)));
        }

        self.stack.pop();
        self.seen.insert(identity, Seen::Loaded);
        self.sources_out.push(ModuleSource {
            path: path.to_path_buf(),
            file,
            name,
            order,
            tokens,
        });
        Ok(())
    }

    fn file_of(&self, target: &Path) -> FileId {
        let identity = identity_of(target);
        self.sources_out
            .iter()
            .find(|module| identity_of(&module.path) == identity)
            .map(|module| module.file)
            .unwrap_or(FileId::from_index(0))
    }

    /// Undo the loading state for `identity` before returning its diagnostics.
    ///
    /// Nothing sees the state after `load` returns `Err`, but leaving a module
    /// marked `Loading` would make a second attempt at the same load report a
    /// cycle that does not exist, and a caller retrying is a caller that should
    /// get the first error again.
    fn abandon(&mut self, identity: PathBuf, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self
            .stack
            .last()
            .map(|(id, _)| id == &identity)
            .unwrap_or(false)
        {
            self.stack.pop();
        }
        self.seen.remove(&identity);
        diagnostics
    }

    fn cycle_diagnostic(
        &self,
        identity: &Path,
        path: &Path,
        site: Option<(FileId, Span)>,
    ) -> Diagnostic {
        let start = self
            .stack
            .iter()
            .position(|(id, _)| id == identity)
            .unwrap_or(0);
        let names: Vec<String> = self
            .stack
            .get(start..)
            .unwrap_or(&[])
            .iter()
            .map(|(_, display)| display.display().to_string())
            .chain(std::iter::once(path.display().to_string()))
            .collect();
        self.coded_at(
            site,
            yashy_common::codes::MODULE_CYCLE,
            format!("import cycle: {}", names.join(" -> ")),
        )
    }

    fn at(&self, site: Option<(FileId, Span)>, message: String) -> Diagnostic {
        let (file, span) = site.unwrap_or((FileId::from_index(0), Span::SYNTHETIC));
        Diagnostic::new(Severity::Error, file, span, message)
    }

    fn coded_at(
        &self,
        site: Option<(FileId, Span)>,
        code: &'static str,
        message: String,
    ) -> Diagnostic {
        self.at(site, message).with_code(code)
    }
}

fn import_error(message: String, from: Option<&Path>) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        Severity::Error,
        FileId::from_index(0),
        Span::SYNTHETIC,
        message,
    );
    if let Some(from) = from {
        diagnostic.label = Some(format!("imported from {}", from.display()));
    }
    diagnostic
}

fn identity_of(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .map(|dir| dir.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

/// Read the import statements out of a module's tokens.
///
/// Mirrors `Parser::parse_import`: `import seg (`.` seg)* `;`, where a segment
/// is an identifier or a keyword (`std.io` names a module, not a modifier). A
/// stream that does not match that shape is left alone, so the parser reports
/// the syntax error instead of discovery inventing a module that was never
/// named.
fn scan_imports(tokens: &[Token], source: &str) -> Vec<(Vec<String>, Option<String>, Span)> {
    let mut imports = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].kind != TokenKind::Import {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let Some(first) = import_segment(tokens, &mut index, source) else {
            continue;
        };
        let mut path = vec![first];
        let mut complete = true;
        while tokens
            .get(index)
            .map_or(false, |token| token.kind == TokenKind::Dot)
        {
            index += 1;
            match import_segment(tokens, &mut index, source) {
                Some(segment) => path.push(segment),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete
            && tokens
                .get(index)
                .map_or(false, |token| token.kind == TokenKind::Semi)
        {
            let span = Span::new(tokens[start].span.start, tokens[index].span.end);
            imports.push((path, None, span));
            index += 1;
        } else if complete
            && tokens
                .get(index)
                .map_or(false, |token| token.kind == TokenKind::As)
        {
            index += 1;
            if let Some(alias) = import_segment(tokens, &mut index, source) {
                if tokens
                    .get(index)
                    .map_or(false, |token| token.kind == TokenKind::Semi)
                {
                    let span = Span::new(tokens[start].span.start, tokens[index].span.end);
                    imports.push((path, Some(alias), span));
                    index += 1;
                }
            }
        }
    }
    imports
}

/// One import-path segment's text, advancing past it.
fn import_segment(tokens: &[Token], index: &mut usize, source: &str) -> Option<String> {
    let token = *tokens.get(*index)?;
    if token.kind != TokenKind::Ident && !token.kind.is_keyword() {
        return None;
    }
    *index += 1;
    Some(
        source
            .get(token.span.start as usize..token.span.end as usize)?
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_layout(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yashy-stdlib-layout-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("the test layout must be creatable");
        dir
    }

    #[test]
    fn an_installed_stdlib_next_to_the_binary_is_found() {
        let dir = temp_layout("next-to-binary");
        let stdlib = dir.join("yashy-stdlib").join("yashy");
        fs::create_dir_all(&stdlib).expect("the stdlib directory must be creatable");
        fs::write(stdlib.join("io.ya"), "").expect("the stdlib file must be writable");

        assert_eq!(stdlib_root_near_exe(&dir.join("yashy.exe")), Some(stdlib));
    }

    #[test]
    fn a_nested_shared_prefix_layout_is_found() {
        let dir = temp_layout("nested-shared-prefix");
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).expect("the bin directory must be creatable");
        // This is the layout the Unix installer writes:
        // <prefix>/share/yashy/stdlib/yashy/*.ya.
        let stdlib = dir.join("share").join("yashy").join("stdlib").join("yashy");
        fs::create_dir_all(&stdlib).expect("the stdlib directory must be creatable");
        fs::write(stdlib.join("io.ya"), "").expect("the stdlib file must be writable");

        let found = stdlib_root_near_exe(&bin.join("yashy"))
            .expect("the nested shared-prefix layout must be found");
        assert_eq!(
            found.canonicalize().expect("the found path must exist"),
            stdlib.canonicalize().expect("the fixture must exist")
        );
    }

    #[test]
    fn a_shared_prefix_layout_is_found() {
        let dir = temp_layout("shared-prefix");
        // The executable sits in `<prefix>/bin`, so that directory has to exist
        // for the `..` in the shared-prefix candidate to resolve: Unix checks
        // each component in order, unlike Windows, which normalizes the path.
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).expect("the bin directory must be creatable");
        let stdlib = dir.join("share").join("yashy").join("stdlib");
        fs::create_dir_all(&stdlib).expect("the stdlib directory must be creatable");
        fs::write(stdlib.join("io.ya"), "").expect("the stdlib file must be writable");

        let found = stdlib_root_near_exe(&bin.join("yashy"))
            .expect("the shared-prefix layout must be found");
        assert_eq!(
            found.canonicalize().expect("the found path must exist"),
            stdlib.canonicalize().expect("the fixture must exist")
        );
    }
}
