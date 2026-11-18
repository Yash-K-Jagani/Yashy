//! Catalog coverage for codes whose emitting path is hard to reach in a unit
//! test (sandbox policy, macro caps, trait coherence). Every one of these must
//! resolve to a page with the same code in its heading; an orphan code is a
//! diagnostic a user can get but cannot read about.

use yashy_common::codes;

#[test]
fn every_hard_to_reach_code_has_a_catalog_entry_and_page() {
    for (code, rule) in [
        ("E0002", "NAME-2"),
        ("E0107", "TYPE-8"),
        ("E0300", "TRY-1"),
        ("E0603", "PAT-4"),
        ("E1900", "SANDBOX-1"),
        ("E1807", "PKG-8"),
        ("E1501", "TRAIT-2"),
        ("E1504", "TRAIT-5"),
        ("E1506", "TRAIT-7"),
        ("E1507", "TRAIT-8"),
        ("E1508", "TRAIT-9"),
        ("E1601", "MACRO-2"),
        ("E1602", "MACRO-3"),
        ("E1603", "MACRO-4"),
        ("E2002", "LINT-3"),
        ("E2003", "LINT-4"),
    ] {
        let entry = codes::find(code).unwrap_or_else(|| panic!("{code} must be in the catalog"));
        assert_eq!(codes::rule_for(code), rule, "{code} has the wrong rule");
        assert!(
            entry.page.starts_with(&format!("# {code}")),
            "{code}'s page must name the code in its heading"
        );
        assert!(
            entry.page.contains("## Cause") && entry.page.contains("## Fix"),
            "{code}'s page is missing a Cause or Fix section"
        );
    }
}
