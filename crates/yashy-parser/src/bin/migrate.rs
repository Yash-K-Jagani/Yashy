//! A one-off migration from name-first to type-first declarations.
//!
//! This binary is temporary: it lands with the dual-accept grammar, rewrites
//! every tracked source, and is deleted at the cutover commit that makes the
//! old order `E0109`.
//!
//! It rewrites three kinds of file:
//!
//! - `.ya` — the whole file;
//! - `.md` — the body of every fenced ```yashy block;
//! - `.rs` — the body of every Rust string literal, because the compiler's
//!   tests hold their fixtures there. A normal Rust string's escapes are
//!   replaced with spaces **at the same byte length**, so a token span in the
//!   padded text is a byte offset in the file and no edit can land inside an
//!   escape.
//!
//! Within a region the rewrite is token-based, not textual: `let`/`const`
//! followed by `name :`, a parameter list after `fn name(`, and a lambda's
//! `| ... |` bars. A struct field or a struct literal's `field: value` is
//! never preceded by one of those, so neither is touched.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use yashy_lexer::{Lexer, Token, TokenKind};

#[derive(Clone)]
struct Region {
    /// Byte offset in the file where the region's text starts.
    start: usize,
    /// The region's text, padded to the same length as the file's slice.
    text: String,
}

#[derive(Clone)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: migrate <path>...");
        std::process::exit(2);
    }
    let mut files = BTreeSet::new();
    for arg in &args {
        collect(Path::new(arg), &mut files);
    }
    let mut edited_files = 0usize;
    let mut edits = 0usize;
    for file in files {
        match migrate_file(&file) {
            Ok(0) => {}
            Ok(n) => {
                edited_files += 1;
                edits += n;
                println!("{}: {n} declarations", file.display());
            }
            Err(message) => {
                eprintln!("{}: {message}", file.display());
                std::process::exit(1);
            }
        }
    }
    println!("{edits} edits in {edited_files} files");
}

fn collect(path: &Path, out: &mut BTreeSet<PathBuf>) {
    if path.is_dir() {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                return;
            }
        };
        for entry in entries.flatten() {
            let child = entry.path();
            if child.file_name().and_then(|name| name.to_str()) == Some(".git") {
                continue;
            }
            collect(&child, out);
        }
        return;
    }
    let extension = path.extension().and_then(|ext| ext.to_str());
    if matches!(extension, Some("ya" | "md" | "rs")) {
        out.insert(path.to_path_buf());
    }
}

fn migrate_file(path: &Path) -> Result<usize, String> {
    let src = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let regions = match path.extension().and_then(|ext| ext.to_str()) {
        Some("ya") => vec![Region {
            start: 0,
            text: src.clone(),
        }],
        Some("md") => markdown_regions(&src),
        Some("rs") => rust_regions(&src),
        _ => return Ok(0),
    };
    let mut all: Vec<Edit> = Vec::new();
    for region in regions {
        let tokens = match Lexer::new(&region.text).tokenize() {
            Ok(tokens) => tokens,
            // A fragment that does not lex is not a Yashy snippet we can
            // rewrite: leave it exactly as it is.
            Err(_) => continue,
        };
        for edit in rewrite(&region, &tokens) {
            all.push(Edit {
                start: edit.start + region.start,
                end: edit.end + region.start,
                replacement: edit.replacement,
            });
        }
    }
    if all.is_empty() {
        return Ok(0);
    }
    let mut out = src;
    all.sort_by_key(|edit| edit.start);
    for edit in all.iter().rev() {
        out.replace_range(edit.start..edit.end, &edit.replacement);
    }
    fs::write(path, out).map_err(|error| error.to_string())?;
    Ok(all.len())
}

// -- regions ---------------------------------------------------------------

fn markdown_regions(src: &str) -> Vec<Region> {
    let mut regions = Vec::new();
    let mut offset = 0usize;
    let mut inside = false;
    let mut region_start = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !inside && trimmed.starts_with("```yashy") {
            inside = true;
            region_start = offset + line.len();
            offset += line.len();
            continue;
        }
        if inside && trimmed.starts_with("```") {
            regions.push(Region {
                start: region_start,
                text: src[region_start..offset].to_string(),
            });
            inside = false;
        }
        offset += line.len();
    }
    regions
}

fn rust_regions(src: &str) -> Vec<Region> {
    let bytes = src.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b'"' => {
                let start = i + 1;
                let mut body = String::new();
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b'\\' => {
                            let escape_start = i;
                            i += 1;
                            if i < bytes.len()
                                && bytes[i] == b'u'
                                && bytes.get(i + 1) == Some(&b'{')
                            {
                                i += 2;
                                while i < bytes.len() && bytes[i] != b'}' {
                                    i += 1;
                                }
                                i = (i + 1).min(bytes.len());
                            } else if i < bytes.len() {
                                i += 1;
                            }
                            for _ in escape_start..i {
                                body.push(' ');
                            }
                        }
                        b'\n' => {
                            body.push('\n');
                            i += 1;
                        }
                        _ => {
                            let ch = src[i..].chars().next().unwrap_or(' ');
                            body.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                if i - 1 > start {
                    regions.push(Region { start, text: body });
                }
            }
            b'r' | b'b' if i == 0 || !is_ident_byte(bytes[i - 1]) => {
                let mut j = i;
                if bytes[j] == b'b' {
                    j += 1;
                }
                if bytes.get(j) != Some(&b'r') {
                    i += 1;
                    continue;
                }
                j += 1;
                let mut hashes = 0usize;
                while bytes.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if bytes.get(j) != Some(&b'"') {
                    i = j;
                    continue;
                }
                let start = j + 1;
                let closing = format!("\"{}", "#".repeat(hashes));
                let rest = &src[start..];
                match rest.find(&closing) {
                    Some(length) => {
                        regions.push(Region {
                            start,
                            text: rest[..length].to_string(),
                        });
                        i = start + length + closing.len();
                    }
                    None => i = bytes.len(),
                }
            }
            _ => {
                let ch = src[i..].chars().next().unwrap_or(' ');
                i += ch.len_utf8();
            }
        }
    }
    regions
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

// -- rewrite ---------------------------------------------------------------

fn rewrite(region: &Region, tokens: &[Token]) -> Vec<Edit> {
    let mut edits = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::Let => {
                if let Some(edit) = binding_edit(region, tokens, i) {
                    edits.push(edit);
                }
            }
            TokenKind::Const => {
                if tokens.get(i + 1).map(|t| t.kind) != Some(TokenKind::Fn) {
                    if let Some(edit) = binding_edit(region, tokens, i) {
                        edits.push(edit);
                    }
                }
            }
            TokenKind::Fn => {
                if let Some(end) = fn_params(region, tokens, i, &mut edits) {
                    i = end;
                }
            }
            TokenKind::Pipe => {
                if let Some(end) = lambda_params(region, tokens, i, &mut edits) {
                    i = end;
                }
            }
            _ => {}
        }
        i += 1;
    }
    edits
}

/// `let name: type =` (old order) → `let type name =`.
fn binding_edit(region: &Region, tokens: &[Token], keyword: usize) -> Option<Edit> {
    let name = tokens.get(keyword + 1)?;
    let colon = tokens.get(keyword + 2)?;
    if name.kind != TokenKind::Ident || colon.kind != TokenKind::Colon {
        return None;
    }
    let mut depth = 0isize;
    let mut j = keyword + 3;
    while let Some(token) = tokens.get(j) {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
            TokenKind::Eq if depth == 0 => break,
            TokenKind::Semi if depth == 0 => return None,
            _ => {}
        }
        j += 1;
    }
    let eq = tokens.get(j)?;
    if eq.kind != TokenKind::Eq {
        return None;
    }
    let ty_start = tokens[keyword + 3].span.start as usize;
    let ty_end = tokens[j - 1].span.end as usize;
    let ty = region.text[ty_start..ty_end].trim();
    let name_text = region.text[name.span.start as usize..name.span.end as usize].trim();
    Some(Edit {
        start: name.span.start as usize,
        end: ty_end,
        replacement: format!("{ty} {name_text}"),
    })
}

/// `fn name(a: T, b: U)` → `fn name(T a, U b)`.
///
/// Returns the index of the closing `)` so the caller does not scan the
/// parameter list a second time.
fn fn_params(
    region: &Region,
    tokens: &[Token],
    keyword: usize,
    edits: &mut Vec<Edit>,
) -> Option<usize> {
    let name = tokens.get(keyword + 1)?;
    let open = tokens.get(keyword + 2)?;
    if name.kind != TokenKind::Ident || open.kind != TokenKind::LParen {
        return None;
    }
    let (close, _) = param_list(region, tokens, keyword + 2, edits)?;
    Some(close)
}

/// A lambda's `| ... |` list, when it really is a parameter list.
fn lambda_params(
    region: &Region,
    tokens: &[Token],
    open: usize,
    edits: &mut Vec<Edit>,
) -> Option<usize> {
    // `||` is one token (OrOr), so a `Pipe` here always has at least one
    // token inside its bars.
    let mut depth = 0isize;
    let mut j = open + 1;
    while let Some(token) = tokens.get(j) {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
            TokenKind::Pipe if depth == 0 => break,
            _ => {}
        }
        j += 1;
    }
    if tokens.get(j)?.kind != TokenKind::Pipe {
        return None;
    }
    let (close, count) = param_list(region, tokens, open, edits)?;
    // A pair of bars around something that is not a parameter list is a
    // bitwise-or expression: `a | b | c`. `param_list` only edits segments
    // shaped `name: type`, so `count` is the number of those segments.
    if count == 0 && j != open + 1 {
        return None;
    }
    Some(close)
}

/// Split `( ... )` into top-level segments and rewrite each `name: type`.
///
/// Returns the closing token index and how many segments were rewritten.
fn param_list(
    region: &Region,
    tokens: &[Token],
    open: usize,
    edits: &mut Vec<Edit>,
) -> Option<(usize, usize)> {
    let opener = tokens.get(open)?.kind;
    let mut j = open + 1;
    let mut depth = 1isize;
    let mut close = None;
    let mut segment_start = open + 1;
    let mut count = 0usize;
    while let Some(token) = tokens.get(j) {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    rewrite_segment(region, tokens, segment_start, j, edits, &mut count);
                    close = Some(j);
                    break;
                }
            }
            TokenKind::Pipe if opener == TokenKind::Pipe && depth == 1 => {
                rewrite_segment(region, tokens, segment_start, j, edits, &mut count);
                close = Some(j);
                break;
            }
            TokenKind::Comma if depth == 1 => {
                rewrite_segment(region, tokens, segment_start, j, edits, &mut count);
                segment_start = j + 1;
            }
            _ => {}
        }
        j += 1;
    }
    Some((close?, count))
}

fn rewrite_segment(
    region: &Region,
    tokens: &[Token],
    start: usize,
    end: usize,
    edits: &mut Vec<Edit>,
    count: &mut usize,
) {
    if end <= start + 2 {
        return;
    }
    let name = &tokens[start];
    let colon = &tokens[start + 1];
    if name.kind != TokenKind::Ident || colon.kind != TokenKind::Colon {
        return;
    }
    let name_text = region.text[name.span.start as usize..name.span.end as usize].trim();
    // `self: *mut T` is a receiver, not a named parameter: its spelling is
    // fixed, and the type says how the object is passed.
    if name_text == "self" {
        return;
    }
    let ty_start = tokens[start + 2].span.start as usize;
    let ty_end = tokens[end - 1].span.end as usize;
    let ty = region.text[ty_start..ty_end].trim();
    edits.push(Edit {
        start: name.span.start as usize,
        end: ty_end,
        replacement: format!("{ty} {name_text}"),
    });
    *count += 1;
}
