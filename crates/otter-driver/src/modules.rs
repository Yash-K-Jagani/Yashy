//! Imports: turning `import a.b.c;` into files, and each file into one module.
//!
//! A module is one source file. The loader walks the import graph depth-first
//! from the entry point, resolving every dotted path against the search roots
//! and reading each file **once**: a module imported from three places is one
//! [`Module`], which is what makes its definitions single rather than tripled.
//!
//! ```text
//! a.b.c  ->  <root>/a/b/c.ot  and  <root>/a/b/c/mod.ot
//! ```
//!
//! The roots are `OTTER_PATH` when it is set and non-empty, and the entry
//! file's directory otherwise. `OTTER_PATH` *replaces* the default rather than
//! extending it, so a build can be pinned to a known set of directories; the
//! separators are `;` on Windows and `:` everywhere else.
//!
//! An import cycle is refused with the cycle written out. Otter has no
//! `forward` declarations or module-level initialization order, so there is
//! nothing a cycle could mean yet; reporting it now beats defining a meaning
//! that the first real program would have to live with.
//!
//! Every module gets its own [`FileId`] in the caller's [`SourceMap`], and its
//! items keep spans into its own text. That is why checking can attribute a
//! diagnostic to the module it came from instead of the entry point.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use otter_common::{Diagnostic, FileId, Severity, SourceMap, Span};
use otter_lexer::Lexer;
use otter_parser::ast::{Item, Program};
use otter_parser::Parser;

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
    /// Its parsed items, with spans into this module's own text.
    pub program: Program,
}

/// The directories an import is looked up in.
///
/// `OTTER_PATH` when it is set and has at least one non-empty entry, and the
/// entry file's directory otherwise. See the module documentation for the
/// separator and for why the environment *replaces* the default.
pub fn module_roots(entry: &Path) -> Vec<PathBuf> {
    if let Some(value) = env::var_os("OTTER_PATH") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let roots: Vec<PathBuf> = value
            .to_string_lossy()
            .split(separator)
            .filter(|root| !root.is_empty())
            .map(PathBuf::from)
            .collect();
        if !roots.is_empty() {
            return roots;
        }
    }
    match entry.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => vec![parent.to_path_buf()],
        _ => vec![PathBuf::from(".")],
    }
}

/// Resolve a dotted module path to a file.
///
/// Each root is tried in order, and each root has two shapes: a file
/// `<root>/a/b/c.ot`, then a directory module `<root>/a/b/c/mod.ot`. The first
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
        file.set_extension("ot");
        if file.is_file() {
            return Ok(file);
        }
        tried.push(file.display().to_string());

        let directory = base.join("mod.ot");
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

/// Load `entry_path` and everything it imports.
///
/// The returned list has dependencies before dependants and the entry point
/// last; the entry is still file id zero, because its text is the first one
/// added to `sources`. That keeps `check_main_exists`'s "no `fn main`" message
/// pointing at the entry file.
///
/// `entry_source` is passed in rather than read here so the CLI can report an
/// unreadable entry point with the same message it uses for every other input
/// file. Imported files are read here, because only the loader knows which ones
/// are wanted.
pub fn load(
    sources: &mut SourceMap,
    entry_path: &Path,
    entry_source: String,
) -> Result<Vec<Module>, Vec<Diagnostic>> {
    let roots = module_roots(entry_path);
    let mut loader = Loader {
        sources,
        roots,
        seen: HashMap::new(),
        stack: Vec::new(),
        modules: Vec::new(),
    };
    loader.visit(identity_of(entry_path), entry_path, entry_source, None)?;
    Ok(loader.modules)
}

/// Whether a module has been fully loaded or is still on the loader's stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Loading,
    Loaded,
}

struct Loader<'a> {
    sources: &'a mut SourceMap,
    roots: Vec<PathBuf>,
    /// The canonical path of every module seen, and how far its load got.
    seen: HashMap<PathBuf, Seen>,
    /// The modules currently being loaded, outermost first: `(identity, path)`.
    stack: Vec<(PathBuf, PathBuf)>,
    /// Finished modules, dependencies first.
    modules: Vec<Module>,
}

impl Loader<'_> {
    fn visit(
        &mut self,
        identity: PathBuf,
        path: &Path,
        source: String,
        site: Option<(FileId, Span)>,
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

        let file = self
            .sources
            .add_file(path.to_string_lossy().into_owned(), source);

        let tokens = match Lexer::with_file(self.sources.file(file), file).tokenize() {
            Ok(tokens) => tokens,
            Err(diagnostics) => return Err(self.abandon(identity, diagnostics)),
        };

        let program = {
            let mut parser = Parser::new(tokens, self.sources, file);
            match parser.parse_program() {
                Ok(program) => program,
                Err(diagnostics) => return Err(self.abandon(identity, diagnostics)),
            }
        };

        let imports: Vec<(Vec<String>, Span)> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Import { path, span } => Some((path.clone(), *span)),
                _ => None,
            })
            .collect();

        for (segments, span) in imports {
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
            ) {
                return Err(self.abandon(identity, diagnostics));
            }
        }

        self.stack.pop();
        self.seen.insert(identity, Seen::Loaded);
        self.modules.push(Module {
            path: path.to_path_buf(),
            file,
            program,
        });
        Ok(())
    }

    /// Undo the loading state for `identity` before returning its diagnostics.
    ///
    /// Nothing sees the state after `load` returns `Err`, but leaving a module
    /// marked `Loading` would make a second attempt at the same load report a
    /// cycle that does not exist, and a caller retrying is a caller that should
    /// get the first error again.
    fn abandon(
        &mut self,
        identity: PathBuf,
        diagnostics: Vec<Diagnostic>,
    ) -> Vec<Diagnostic> {
        if self.stack.last().map(|(id, _)| id == &identity).unwrap_or(false) {
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
        self.at(site, format!("import cycle: {}", names.join(" -> ")))
    }

    fn at(&self, site: Option<(FileId, Span)>, message: String) -> Diagnostic {
        let (file, span) = site.unwrap_or((FileId::from_index(0), Span::SYNTHETIC));
        Diagnostic::new(Severity::Error, file, span, message)
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
