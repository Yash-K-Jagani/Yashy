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

/// The normative reference is a directory, not a fixed list: a page added
/// later must have links that resolve just like the pages here.
#[test]
fn reference_pages_resolve() {
    let directory = root().join("docs/reference");
    let mut pages: Vec<_> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", directory.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect();
    pages.sort();
    assert!(!pages.is_empty(), "docs/reference must not be empty");
    for page in pages {
        let relative = page
            .strip_prefix(root())
            .unwrap_or(&page)
            .to_string_lossy()
            .replace('\\', "/");
        assert_links_resolve(&relative);
    }
}

#[test]
fn quickstart_and_reference_links_resolve() {
    for document in [
        "README.md",
        "docs/quickstart.md",
        "docs/cli.md",
        "docs/packages.md",
        "docs/stdlib.md",
        "docs/optimization.md",
        "docs/bench-policy.md",
        "docs/ci.md",
        "docs/traceability.md",
        "docs/rules.md",
        "docs/const-eval.md",
        "docs/errors.md",
        "docs/release-notes-template.md",
        "docs/tutorial/00-intro.md",
        "docs/tutorial/11-functions.md",
        "docs/tutorial/12-first-cli.md",
    ] {
        assert_links_resolve(document);
    }
}

/// The governance documents are part of the repo, not a slide deck. The RFC
/// template must carry every field the design names, and every local link in
/// the new pages must resolve.
#[test]
fn governance_documents_are_present_and_complete() {
    for document in [
        "docs/rfcs/README.md",
        "docs/rfcs/0000-template.md",
        "docs/lts.md",
        "docs/deprecation.md",
        "docs/editions.md",
        "docs/governance.md",
    ] {
        assert_links_resolve(document);
    }
    let path = root().join("docs/rfcs/0000-template.md");
    let template = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    for field in [
        "## Motivation",
        "## Syntax",
        "## HIR and desugaring",
        "## Verifier rules",
        "## Diagnostics",
        "## Determinism",
        "## Migration",
        "## Conformance",
        "## Benchmark",
        "## Documentation impact",
        "## Decision",
    ] {
        assert!(
            template.contains(field),
            "the RFC template is missing `{field}`"
        );
    }
}

/// The traceability page is rendered from the same matrix `yashyc trace`
/// prints. `UPDATE_GOLDEN=1` rewrites the page instead of comparing it.
#[test]
fn traceability_page_is_generated_from_the_rule_matrix() {
    let matrix = yashy_driver::trace::matrix(&root())
        .expect("the rule matrix must build from the registry and tests");
    let text = yashy_driver::trace::to_markdown(&matrix.rows);
    let page = root().join("docs").join("traceability.md");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&page, &text)
            .unwrap_or_else(|error| panic!("writing {} failed: {error}", page.display()));
        return;
    }
    let pinned = std::fs::read_to_string(&page)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", page.display()));
    assert_eq!(
        text, pinned,
        "docs/traceability.md is stale; rerun with UPDATE_GOLDEN=1"
    );
}

/// `docs/stdlib.md` is generated from the same documentation model `yashyc doc`
/// renders, so the reference cannot drift from the doc blocks.
///
/// `UPDATE_GOLDEN=1` rewrites the page instead of comparing it.
#[test]
fn stdlib_reference_is_generated_from_doc_blocks() {
    let stdlib = root().join("crates").join("yashy-stdlib").join("yashy");
    let model = yashy_driver::doc::build_doc(&stdlib, false)
        .expect("the standard library must build a documentation model");
    // `build_doc` skips a module that does not check, so compare the model
    // against the files: a module that stops checking must fail this test
    // rather than quietly leave the reference.
    let mut expected: Vec<String> = std::fs::read_dir(&stdlib)
        .expect("the stdlib directory must be readable")
        .map(|entry| entry.expect("a stdlib entry must be readable").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("ya"))
        .map(|path| yashy_driver::doc::module_name(&path))
        .collect();
    expected.sort();
    let documented: Vec<String> = model
        .modules
        .iter()
        .map(|module| module.name.clone())
        .collect();
    assert_eq!(
        documented, expected,
        "every stdlib module must check; one that fails is absent from the reference"
    );

    let mut text = String::from(
        "# The standard library\n\n\
         Generated from the `///` and `/** */` doc blocks by `yashyc doc`; edit the\n\
         module, not this page. `cargo test -p yashy-driver --test docs` fails\n\
         when the page and the compiler's tables disagree.\n",
    );
    for module in &model.modules {
        text.push_str(&format!("\n## `{}`\n", module.name));
        for item in &module.items {
            text.push_str(&format!(
                "\n### `{}`\n\n```yashy\n{}\n```\n",
                item.name, item.signature
            ));
            match &item.docs {
                Some(docs) => text.push_str(&format!("\n{}\n", docs.trim())),
                // A type carries no doc block in the compiler's model; its
                // signature above is the whole entry. A public function without
                // one is caught by `stdlib_coverage`, not by filler here.
                None => {}
            }
        }
    }

    let page = root().join("docs").join("stdlib.md");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&page, &text)
            .unwrap_or_else(|error| panic!("writing {} failed: {error}", page.display()));
        return;
    }
    let pinned = std::fs::read_to_string(&page)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", page.display()));
    assert_eq!(
        text, pinned,
        "docs/stdlib.md is stale; rerun with UPDATE_GOLDEN=1"
    );
}
