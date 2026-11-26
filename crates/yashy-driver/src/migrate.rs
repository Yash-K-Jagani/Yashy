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
}
