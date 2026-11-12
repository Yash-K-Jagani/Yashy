//! The differential test for the self-hosting spike.
//!
//! `selfhost/` is a Yashy lexer and parser for Yashy that parses a subset of
//! the grammar, renders a canonical digest of what it read, and compares that
//! digest with the one the Rust front end renders here. The spike is a
//! measurement, not a second implementation: every file below is classified as
//!
//! - **agree**: both front ends produced the same digest;
//! - **reproduced**: the Rust parser rejected the file and the spike did too;
//! - **unsupported**: the file is outside the spike's declared subset, so
//!   there is nothing to compare;
//! - **mismatch**: anything else, which is a bug in the spike.
//!
//! The lexer is held to the stricter standard: its token kinds and spans must
//! agree on *every* corpus file, because the token vocabulary is the part of
//! the front end a self-hosted compiler cannot do without.

mod common;

use common::{work_dir, yashy};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use yashy_common::{DeclaredEffect, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{BinOp, Expr, Item, Literal, Program, Stmt, UnOp};
use yashy_parser::exprs::expr_span;
use yashy_parser::Parser;

/// The repository root: `crates/yashy-driver` is two directories down.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the driver crate lives under crates/")
        .to_path_buf()
}

/// Build the spike once per test process and hand back its path.
fn spike() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = work_dir("selfhost-spike");
        let out = dir.join(format!("selfhost{}", std::env::consts::EXE_SUFFIX));
        let output = Command::new(yashy())
            .arg("build")
            .arg(root().join("selfhost").join("main.ya"))
            .arg("-o")
            .arg(&out)
            .output()
            .expect("the yashy CLI must be runnable");
        assert!(
            output.status.success(),
            "the spike must build:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        out
    })
    .as_path()
}

/// Every corpus file: the worked programs and the example projects.
fn corpus() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_yashy(&root().join("crates/yashy-driver/tests/programs"), &mut files);
    let examples = root().join("examples");
    let mut example_dirs: Vec<PathBuf> = fs::read_dir(&examples)
        .expect("the examples directory must be readable")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join("main.ya").is_file())
        .collect();
    example_dirs.sort();
    files.extend(example_dirs.into_iter().map(|dir| dir.join("main.ya")));
    files
}

/// Recursively collect `*.ya` under `dir`, in a stable order.
fn collect_yashy(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            collect_yashy(&entry, out);
        } else if entry.extension().is_some_and(|ext| ext == "ya") {
            out.push(entry);
        }
    }
}

/// Ask the spike one question about one file: its token stream, or its digest.
fn ask(command: &str, file: &Path) -> Result<String, String> {
    let output = Command::new(spike())
        .arg(command)
        .arg(file)
        .output()
        .expect("the spike must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim_end_matches(['\r', '\n']).to_string();
    if let Some(message) = text.strip_prefix("ERROR: ") {
        return Err(message.to_string());
    }
    assert!(
        output.status.success(),
        "the spike failed on {command} {}:\n{}",
        file.display(),
        text
    );
    Ok(text)
}

/// The Rust lexer's token stream in the spike's own record format.
fn rust_tokens(source: &str) -> Result<Vec<String>, String> {
    let tokens = Lexer::new(source)
        .tokenize()
        .map_err(|diags| format!("{} diagnostics", diags.len()))?;
    Ok(tokens
        .into_iter()
        .map(|token| {
            format!(
                "{:?},{},{}",
                token.kind, token.span.start, token.span.end
            )
        })
        .collect())
}

/// A whole-file parse by the Rust front end.
fn rust_parse(source: &str) -> Result<Program, Vec<String>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("corpus.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .map_err(|diags| diags.into_iter().map(|d| d.message).collect::<Vec<_>>())?;
    let mut parser = Parser::new(tokens, &sources, file);
    parser
        .parse_program()
        .map_err(|diags| diags.into_iter().map(|d| d.message).collect())
}

/// How one file came out.
#[derive(Default)]
struct Report {
    agree: Vec<String>,
    reproduced: Vec<String>,
    unsupported: Vec<String>,
    mismatch: Vec<String>,
    spike_parsed_outside: Vec<String>,
}

#[test]
fn the_yashy_lexer_agrees_with_the_rust_lexer_on_every_corpus_file() {
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for file in corpus() {
        let source = fs::read_to_string(&file).expect("the corpus file must be readable");
        let expected = match rust_tokens(&source) {
            Ok(tokens) => tokens,
            Err(_) => {
                let spike = ask("lex", &file);
                if spike.is_err() {
                    checked += 1;
                    continue;
                }
                failures.push(format!(
                    "{}: the Rust lexer rejected it, the spike accepted it",
                    file.display()
                ));
                continue;
            }
        };
        let spike = match ask("lex", &file) {
            Ok(text) => text,
            Err(message) => {
                failures.push(format!("{}: the spike rejected it: {message}", file.display()));
                continue;
            }
        };
        let actual: Vec<String> = spike
            .split(';')
            .filter(|record| !record.is_empty())
            .map(str::to_string)
            .collect();
        checked += 1;
        if actual != expected {
            let first = actual
                .iter()
                .zip(expected.iter())
                .position(|(left, right)| left != right)
                .unwrap_or(actual.len().min(expected.len()));
            failures.push(format!(
                "{}: token {first} is {:?} from the spike and {:?} from the Rust lexer",
                file.display(),
                actual.get(first),
                expected.get(first)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} files disagreed with the Rust lexer:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!("lexer: {checked} files agree, token for token");
}

#[test]
fn the_yashy_parser_agrees_with_the_rust_parser_inside_its_subset() {
    let mut report = Report::default();
    for file in corpus() {
        let source = fs::read_to_string(&file).expect("the corpus file must be readable");
        let name = file.display().to_string();
        let rust = rust_parse(&source);
        let spike = ask("parse", &file);
        match (rust, spike) {
            (Err(_), Err(_)) => report.reproduced.push(name),
            (Err(_), Ok(_)) => report.mismatch.push(format!(
                "{name}: the Rust parser rejected it, the spike accepted it"
            )),
            (Ok(program), spike) => {
                let rendered = render_program(&source, &program);
                match (rendered, spike) {
                    (Some(expected), Ok(actual)) if expected == actual => {
                        report.agree.push(name);
                    }
                    (Some(expected), Ok(actual)) => {
                        report.mismatch.push(format!(
                            "{name}: digests differ\n  Rust:  {expected}\n  spike: {actual}"
                        ));
                    }
                    (Some(_), Err(message)) => report.mismatch.push(format!(
                        "{name}: the spike rejected a program inside its own subset: {message}"
                    )),
                    (None, Ok(_)) => {
                        report.spike_parsed_outside.push(name.clone());
                        report.unsupported.push(name);
                    }
                    (None, Err(_)) => report.unsupported.push(name),
                }
            }
        }
    }
    println!(
        "parser: {} agree, {} reproduced, {} unsupported ({} of those the spike parsed anyway), {} mismatched",
        report.agree.len(),
        report.reproduced.len(),
        report.unsupported.len(),
        report.spike_parsed_outside.len(),
        report.mismatch.len()
    );
    assert!(
        report.mismatch.is_empty(),
        "{} files disagreed inside the subset:\n{}",
        report.mismatch.len(),
        report.mismatch.join("\n")
    );
    assert!(
        !report.agree.is_empty(),
        "the subset must cover at least one corpus file"
    );
}

/// The source text a span covers.
fn source_of<'a>(source: &'a str, span: yashy_common::Span) -> Option<&'a str> {
    source.get(span.start as usize..span.end as usize)
}

/// The digest of a whole program, or `None` when it uses a construct outside
/// the spike's subset.
fn render_program(source: &str, program: &Program) -> Option<String> {
    let mut out = String::from("(program");
    for item in &program.items {
        out.push(' ');
        out.push_str(&render_item(source, item)?);
    }
    out.push(')');
    Some(out)
}

fn render_item(source: &str, item: &Item) -> Option<String> {
    Some(match item {
        Item::Import { path, alias, .. } => {
            let path = path.join(" ");
            match alias {
                Some(alias) => format!("(import ({path}) {alias})"),
                None => format!("(import ({path}))"),
            }
        }
        Item::Public { item, .. } => format!("(pub {})", render_item(source, item)?),
        Item::Effect { effect, item, .. } => {
            let name = match effect {
                DeclaredEffect::Pure => "pure",
                DeclaredEffect::Io => "io",
                DeclaredEffect::Unsafe => "unsafe",
                DeclaredEffect::Async => "async",
                DeclaredEffect::Inferred => return None,
            };
            format!("(effect {name} {})", render_item(source, item)?)
        }
        Item::Documented { docs, item, .. } => {
            format!("(doc {} {})", docs.len(), render_item(source, item)?)
        }
        Item::Fn {
            name,
            type_params,
            params,
            ret,
            requires,
            ensures,
            body,
            ..
        } => {
            if !type_params.is_empty() || !requires.is_empty() || !ensures.is_empty() {
                return None;
            }
            let mut out = format!("(fn {name} (params");
            for param in params {
                if param.default.is_some() {
                    return None;
                }
                out.push_str(&format!(
                    " (param {} {})",
                    render_type(&param.ty)?,
                    param.name
                ));
            }
            out.push(')');
            match ret {
                Some(ty) => out.push_str(&format!(" (ret {})", render_type(ty)?)),
                None => out.push_str(" (ret)"),
            }
            out.push_str(&format!(" (block{})", render_block(source, body)?));
            out.push(')');
            out
        }
        _ => return None,
    })
}

fn render_type(ty: &yashy_parser::ast::TypeExpr) -> Option<String> {
    Some(ty.to_string())
}

fn render_block(source: &str, block: &yashy_parser::ast::Block) -> Option<String> {
    let mut out = String::new();
    for stmt in &block.stmts {
        out.push(' ');
        out.push_str(&render_stmt(source, stmt)?);
    }
    Some(out)
}

fn render_stmt(source: &str, stmt: &Stmt) -> Option<String> {
    Some(match stmt {
        Stmt::Expr(expr, _) => format!("(expr {})", render_expr(source, expr)?),
        Stmt::Let { name, ty, init, .. } => format!(
            "(let {name} {} {})",
            render_optional_type(ty)?,
            render_expr(source, init)?
        ),
        Stmt::Const { name, ty, init, .. } => format!(
            "(const {name} {} {})",
            render_optional_type(ty)?,
            render_expr(source, init)?
        ),
        Stmt::Assign { target, value, .. } => format!(
            "(assign {} {})",
            render_expr(source, target)?,
            render_expr(source, value)?
        ),
        Stmt::Return(None, _) => "(return)".to_string(),
        Stmt::Return(Some(expr), _) => format!("(return {})", render_expr(source, expr)?),
        Stmt::If {
            init: None,
            cond,
            then_block,
            else_block,
            ..
        } => {
            let mut out = format!("(if {} (block{})", render_expr(source, cond)?, render_block(source, then_block)?);
            match else_block {
                Some(block) => out.push_str(&format!(" (block{}))", render_block(source, block)?)),
                None => out.push(')'),
            }
            out
        }
        Stmt::ForC {
            init: None,
            cond: Some(cond),
            update: None,
            body,
            label: None,
            ..
        } => format!("(while {} (block{}))", render_expr(source, cond)?, render_block(source, body)?),
        Stmt::ForRange {
            name,
            start,
            end,
            body,
            label: None,
            ..
        } => format!(
            "(for {name} {} {} (block{}))",
            render_expr(source, start)?,
            render_expr(source, end)?,
            render_block(source, body)?
        ),
        Stmt::Break(None, _) => "(break)".to_string(),
        Stmt::Continue(None, _) => "(continue)".to_string(),
        Stmt::Defer { body, .. } => format!("(defer (block{}))", render_block(source, body)?),
        _ => return None,
    })
}

fn render_optional_type(ty: &Option<yashy_parser::ast::TypeExpr>) -> Option<String> {
    match ty {
        Some(ty) => render_type(ty),
        None => Some("-".to_string()),
    }
}

fn render_expr(source: &str, expr: &Expr) -> Option<String> {
    Some(match expr {
        Expr::Literal(literal, _) => match literal {
            Literal::Bool(value) => format!("(lit {value})"),
            _ => format!("(lit {})", source_of(source, expr_span(expr))?),
        },
        Expr::Interp { .. } => format!("(lit {})", source_of(source, expr_span(expr))?),
        Expr::Ident(path, _) => format!("(name {})", path.join(" ")),
        Expr::Binary { op, lhs, rhs, .. } => format!(
            "(bin {} {} {})",
            symbol(*op),
            render_expr(source, lhs)?,
            render_expr(source, rhs)?
        ),
        Expr::CompareChain { operands, ops, .. } => {
            let mut out = String::from("(chain");
            for (index, operand) in operands.iter().enumerate() {
                out.push(' ');
                out.push_str(&render_expr(source, operand)?);
                if let Some(op) = ops.get(index) {
                    out.push(' ');
                    out.push_str(symbol(*op));
                }
            }
            out.push(')');
            out
        }
        Expr::Unary { op, operand, .. } => format!(
            "(un {} {})",
            unary_symbol(*op),
            render_expr(source, operand)?
        ),
        Expr::Call {
            callee,
            args,
            unsafe_call,
            ..
        } => {
            if *unsafe_call {
                return None;
            }
            let mut out = format!("(call ({})", callee.join(" "));
            for arg in args {
                if arg.name.is_some() {
                    return None;
                }
                out.push(' ');
                out.push_str(&render_expr(source, &arg.value)?);
            }
            out.push(')');
            out
        }
        Expr::FieldAccess { base, field, .. } => {
            format!("(field {} {})", render_expr(source, base)?, field)
        }
        Expr::Index { base, index, .. } => format!(
            "(index {} {})",
            render_expr(source, base)?,
            render_expr(source, index)?
        ),
        Expr::FixedArrayLit { elements, .. } => {
            let mut out = String::from("(array");
            for element in elements {
                out.push(' ');
                out.push_str(&render_expr(source, element)?);
            }
            out.push(')');
            out
        }
        _ => return None,
    })
}

fn symbol(op: BinOp) -> &'static str {
    op.symbol()
}

fn unary_symbol(op: UnOp) -> &'static str {
    op.symbol()
}
