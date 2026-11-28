//! Edition migration for `yashy.toml`.
//!
//! A migration is a textual rewrite of the `edition` key in `[package]`, not a
//! regeneration of the manifest: every other spelling the user wrote survives
//! byte for byte. A project with no key is treated as the default `2021` and
//! gets the target key; a project already on the target is left alone.

/// Rewrite `text` to `target`, or return a diagnostic when the shape is not a
/// package manifest or the target is not a supported edition.
pub fn rewrite(text: &str, target: &str) -> Result<String, String> {
    if target != "2027" {
        return Err(format!("unknown edition `{target}`; expected 2027"));
    }
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    let package = lines
        .iter()
        .position(|line| line.trim() == "[package]")
        .ok_or("a manifest needs a `[package]` table")?;
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate().skip(package + 1) {
        if line.trim_start().starts_with('[') {
            end = index;
            break;
        }
    }
    let edition = lines[package + 1..end]
        .iter()
        .position(|line| line.trim_start().starts_with("edition"));
    let wanted = format!("edition = \"{target}\"");
    match edition {
        Some(index) => {
            let at = package + 1 + index;
            if lines[at].trim() != wanted {
                lines[at] = format!("{wanted}\n");
            }
        }
        None => {
            let newline = if lines[package].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            lines.insert(package + 1, format!("{wanted}{newline}"));
        }
    }
    Ok(lines.concat())
}

/// A line-shaped diff for `--dry-run`: removed and added lines only.
pub fn diff(old: &str, new: &str) -> String {
    let mut out = String::new();
    let before: Vec<&str> = old.lines().collect();
    let after: Vec<&str> = new.lines().collect();
    for line in after.iter().filter(|line| !before.contains(line)) {
        out.push_str("+ ");
        out.push_str(line);
        out.push('\n');
    }
    for line in before.iter().filter(|line| !after.contains(line)) {
        out.push_str("- ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Rewrites the legacy, name-first spellings documented in the E0109 page:
/// `let x: int =` to `let int x =`, `fn f(a: int)` to `fn f(int a)` and
/// `|x: int|` to `|int x|`. A struct field keeps `name: type` because it is a
/// field list, not a binding, and a receiver keeps `self: *mut T`.
///
/// Returns the rewritten text and the number of lines that changed. Nothing
/// outside those spellings is touched — this is a textual migration aid, not
/// a reformatter, so comments, spacing and line endings survive byte for byte.
pub fn rewrite_source(text: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut changes = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            out.push_str(line);
            continue;
        }
        let rewritten = rewrite_groups(&rewrite_binding(line));
        if rewritten != line {
            changes += 1;
        }
        out.push_str(&rewritten);
    }
    (out, changes)
}

fn is_ident(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn rewrite_binding(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let trimmed = &line[indent..];
    let keyword = if trimmed.starts_with("let ") {
        "let"
    } else if trimmed.starts_with("const ") {
        "const"
    } else {
        return line.to_string();
    };
    let after = &trimmed[keyword.len() + 1..];
    let Some(equal) = after.find('=') else {
        return line.to_string();
    };
    let Some(colon) = after[..equal].find(':') else {
        return line.to_string();
    };
    let name = after[..colon].trim();
    let ty = after[colon + 1..equal].trim();
    if !is_ident(name) || ty.is_empty() || ty.chars().any(|ch| matches!(ch, '(' | ')' | '{' | '}'))
    {
        return line.to_string();
    }
    format!(
        "{}{} {} {} {}",
        &line[..indent],
        keyword,
        ty,
        name,
        &after[equal..]
    )
}

/// Rewrite every parenthesised parameter list and every `|a: T|` lambda list
/// on one line. Both walk the same top-level comma splitter, so `Result[int,
/// string]` is one parameter and not two.
fn rewrite_groups(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    while at < line.len() {
        let ch = line[at..]
            .chars()
            .next()
            .expect("a byte in a char boundary");
        if ch == '(' {
            if let Some(end) = matching(line, at, '(', ')') {
                out.push('(');
                out.push_str(&rewrite_parameters(&line[at + 1..end]));
                out.push(')');
                at = end + 1;
                continue;
            }
        } else if ch == '|' {
            if let Some(relative) = line[at + 1..].find('|') {
                let end = at + 1 + relative;
                let inner = &line[at + 1..end];
                if inner.contains(':') {
                    out.push('|');
                    out.push_str(&rewrite_parameters(inner));
                    out.push('|');
                    at = end + 1;
                    continue;
                }
            }
        }
        out.push(ch);
        at += ch.len_utf8();
    }
    out
}

fn matching(text: &str, open: usize, left: char, right: char) -> Option<usize> {
    let mut depth = 0i32;
    for (index, ch) in text[open..].char_indices() {
        if ch == left {
            depth += 1;
        } else if ch == right {
            depth -= 1;
            if depth == 0 {
                return Some(open + index);
            }
        }
    }
    None
}

fn rewrite_parameters(inner: &str) -> String {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, ch) in inner.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let leading = part.len() - part.trim_start().len();
        let trailing = part.len() - part.trim_end().len();
        let body = &part[leading..part.len() - trailing];
        let colon = body.find(':');
        if let Some(colon) = colon {
            let name = &body[..colon];
            let ty = &body[colon + 1..];
            if is_ident(name) && name != "self" && !ty.trim().is_empty() {
                out.push(format!(
                    "{}{} {}{}",
                    &part[..leading],
                    ty.trim(),
                    name,
                    &part[part.len() - trailing..]
                ));
                continue;
            }
        }
        out.push(part.to_string());
    }
    out.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_key_is_added_to_the_package_table() {
        let source = "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n\n[dependencies]\n";
        let migrated = rewrite(source, "2027").unwrap();
        assert!(migrated.contains("[package]\nedition = \"2027\"\nname"));
        assert!(migrated.contains("\n[dependencies]\n"));
        assert_eq!(diff(source, &migrated), "+ edition = \"2027\"\n");
    }

    #[test]
    fn an_existing_key_is_replaced_in_place() {
        let source = "[package]\nname = \"demo\"\nedition = \"2021\"\n";
        let migrated = rewrite(source, "2027").unwrap();
        assert_eq!(migrated, "[package]\nname = \"demo\"\nedition = \"2027\"\n");
    }

    #[test]
    fn a_manifest_without_package_is_refused() {
        assert!(rewrite("[dependencies]\n", "2027").is_err());
        assert!(rewrite("[package]\nname = \"x\"\n", "2026").is_err());
    }

    #[test]
    fn source_rewrites_bindings_parameters_and_lambdas() {
        let source = "\
struct Point { x: int }
fn add(a: int, b: Result[int, string]) -> int {
    let sum: int = a;
    const label: string = \"x\";
    let f = |value: int| -> int { return value; };
    return sum;
}
";
        let (migrated, changes) = rewrite_source(source);
        assert_eq!(changes, 4);
        assert!(migrated.contains("struct Point { x: int }"), "{migrated}");
        assert!(
            migrated.contains("fn add(int a, Result[int, string] b) -> int"),
            "{migrated}"
        );
        assert!(migrated.contains("let int sum = a;"), "{migrated}");
        assert!(migrated.contains("const string label ="), "{migrated}");
        assert!(migrated.contains("|int value|"), "{migrated}");
    }

    #[test]
    fn source_leaves_a_receiver_and_comments_alone() {
        let source = "\
// let old: int = 1;
fn touch(self: *mut Point, other: int) {
    let int keep = other;
}
";
        let (migrated, changes) = rewrite_source(source);
        assert_eq!(changes, 1, "{migrated}");
        assert!(migrated.contains("// let old: int = 1;"), "{migrated}");
        assert!(migrated.contains("self: *mut Point"), "{migrated}");
        assert!(migrated.contains("int other"), "{migrated}");
    }
}
