//! Documentation links are part of the product: a quickstart that points at a
//! missing page is a broken install path. These tests keep the local links
//! honest without requiring a network.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Remove fenced and inline code before looking for links. A code sample such
/// as `identity[T](x: T)` contains `](` but is not a link.
fn prose_only(markdown: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut in_code = false;
        for character in line.chars() {
            if character == '`' {
                in_code = !in_code;
                continue;
            }
            if !in_code {
                out.push(character);
            }
        }
        out.push('\n');
    }
    out
}

fn local_links(markdown: &str) -> Vec<String> {
    let markdown = prose_only(markdown);
    let mut links = Vec::new();
    let mut rest = markdown.as_str();
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else {
            break;
        };
        let target = after[..end].trim();
        rest = &after[end + 1..];

        if target.is_empty()
            || target.starts_with('#')
            || target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("mailto:")
        {
            continue;
        }
        let path = target.split('#').next().unwrap_or(target);
        links.push(path.to_string());
    }
    links
}

fn assert_links_resolve(relative: &str) {
    let path = root().join(relative);
    let markdown = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let base = path.parent().expect("a document must have a parent");

    for link in local_links(&markdown) {
        let target = base.join(&link);
        assert!(
            target.exists(),
            "{relative} links to `{link}`, which does not exist at {}",
            target.display()
        );
    }
}

#[test]
fn llms_links_resolve() {
    assert_links_resolve("llms.txt");
}

#[test]
fn repo_agents_links_resolve() {
    assert_links_resolve("AGENTS.md");
}

#[test]
fn quickstart_and_reference_links_resolve() {
    for document in [
        "README.md",
        "docs/quickstart.md",
        "docs/cli.md",
        "docs/errors.md",
        "docs/release-notes-template.md",
        "docs/tutorial/00-intro.md",
        "docs/tutorial/11-functions.md",
        "docs/tutorial/12-first-cli.md",
    ] {
        assert_links_resolve(document);
    }
}
