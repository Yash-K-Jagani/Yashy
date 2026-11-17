//! The error catalog is a contract: every code has a page, a catalog entry,
//! an offline `explain --json` record, and a docs URL.

mod common;

use common::yashyc;
use std::process::Command;
use yashy_common::codes;

#[test]
fn every_catalog_code_has_a_page_a_heading_and_a_docs_url() {
    assert!(!codes::CODES.is_empty(), "the catalog must not be empty");
    for entry in codes::CODES {
        let first_line = entry.page.lines().next().unwrap_or("");
        assert!(
            first_line.contains(entry.id),
            "{}'s page must open with a heading naming the code: {first_line}",
            entry.id
        );
        assert!(!entry.title.is_empty(), "{} has no title", entry.id);
        assert!(!entry.summary.is_empty(), "{} has no summary", entry.id);
        assert_eq!(
            codes::docs_url(entry.id),
            format!("https://yashy.dev/errors/{}", entry.id),
            "{} has an unexpected docs URL",
            entry.id
        );
    }
}

#[test]
fn explain_json_returns_the_same_fact_for_every_code() {
    for entry in codes::CODES {
        let output = Command::new(yashyc())
            .args(["explain", "--json", entry.id])
            .output()
            .expect("the CLI must run");
        assert!(
            output.status.success(),
            "explain --json {} must succeed:\n{}",
            entry.id,
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        for field in [
            format!("\"code\": \"{}\"", entry.id),
            format!("\"title\": \"{}\"", entry.title),
            format!("\"rule\": \"{}\"", codes::rule_for(entry.id)),
            format!("\"docs_url\": \"{}\"", codes::docs_url(entry.id)),
        ] {
            assert!(
                text.contains(&field),
                "{}: missing {field} in {text}",
                entry.id
            );
        }
    }
}

#[test]
fn explain_all_json_lists_the_whole_catalog() {
    let output = Command::new(yashyc())
        .args(["explain", "--all", "--json"])
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "explain --all --json must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("\"schema_version\": 1"),
        "the all-codes envelope has a schema version: {text}"
    );
    assert!(
        text.contains("\"total\": "),
        "the all-codes envelope has a total: {text}"
    );
    for entry in codes::CODES {
        assert!(
            text.contains(&format!("\"id\": \"{}\"", entry.id)),
            "the all-codes envelope must name {}",
            entry.id
        );
    }
}
