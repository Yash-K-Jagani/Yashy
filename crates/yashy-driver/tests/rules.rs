//! The rule-id page is generated from the registry, not hand-maintained.
//!
//! `docs/traceability.md` pairs a rule with its codes and the tests that
//! mention them; this page is the flat rule-to-code list. Both have a golden
//! file so a registry edit cannot leave either page stale.

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the driver crate is inside the workspace")
        .parent()
        .expect("the workspace is two levels up")
        .to_path_buf()
}

#[test]
fn rules_page_is_generated_from_the_registry() {
    let text = yashy_driver::trace::rules_markdown();
    let page = root().join("docs").join("rules.md");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&page, &text)
            .unwrap_or_else(|error| panic!("writing {} failed: {error}", page.display()));
        return;
    }
    let pinned = std::fs::read_to_string(&page)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", page.display()));
    assert_eq!(
        text, pinned,
        "docs/rules.md is stale; rerun with UPDATE_GOLDEN=1"
    );
}

#[test]
fn every_catalog_code_names_a_rule() {
    // The generator skips a code without a rule; this test makes that skip a
    // failure rather than a quiet omission from the page.
    let text = yashy_driver::trace::rules_markdown();
    for code in yashy_common::codes::CODES {
        assert!(
            text.contains(&format!("[{}](errors/{}.md)", code.id, code.id)),
            "{} is missing from the generated rule page",
            code.id
        );
    }
}

#[test]
fn planned_rules_without_a_code_are_named() {
    let text = yashy_driver::trace::rules_markdown();
    assert!(text.contains("## Rules without a code"));
    let section = text
        .split("## Rules without a code")
        .nth(1)
        .unwrap_or_default();
    for rule in yashy_common::codes::PLANNED_RULES {
        assert!(
            section.contains(&format!("`{rule}`")),
            "{rule} is a planned rule with no code, so it must be named"
        );
    }
}
