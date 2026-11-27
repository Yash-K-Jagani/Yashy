//! The SDK wraps the same driver functions the CLI calls. Every result is a
//! JSON value in the CLI's schema, so a library client renders the same facts
//! as `yashyc` prints.

use std::fs;
use std::path::{Path, PathBuf};

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-sdk-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("tests")).expect("a work directory must be creatable");
    fs::write(
        dir.join("yashy.toml"),
        "[package]\nname = \"sdk-demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.ya"),
        "/// Adds two numbers.\npub fn add(int a, int b) -> int {\n    return a + b;\n}\n\nfn main() {\n    print(add(1, 2));\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("tests/smoke.ya"),
        "fn add(int a, int b) -> int { return a + b; }\nfn main() { assert(add(1, 2) == 3); }\n",
    )
    .unwrap();
    dir
}

#[test]
fn check_and_verify_share_the_diagnostic_model() {
    let dir = work_dir("check");
    let source = "fn main() { let int x = \"three\"; }\n";
    let path = dir.join("main.ya");
    let library = yashy_sdk::check(&path, source, false);
    assert!(!library.success, "a type error must fail a check");
    assert!(
        library.diagnostics.iter().any(|d| d.code == Some("E0100")),
        "the code must be preserved: {:?}",
        library.diagnostics
    );
    let json = library.to_json().render();
    assert!(json.contains("E0100"), "{json}");
    assert!(json.contains("\"success\": false"), "{json}");

    let ok = yashy_sdk::check(&path, "fn main() { print(1); }\n", false);
    assert!(ok.success, "a clean program passes");
}

#[test]
fn index_review_and_impact_use_the_same_model() {
    let dir = work_dir("index");
    let baseline = yashy_sdk::index(&dir)
        .expect("the index must build")
        .render();
    assert!(baseline.contains("\"symbols\""), "{baseline}");
    assert!(baseline.contains("\"add\""), "{baseline}");

    // Change a public signature and review against the earlier snapshot.
    fs::write(
        dir.join("main.ya"),
        "pub fn add(int a, int b) -> string {\n    return \"changed\";\n}\n\nfn main() { print(add(1, 2)); }\n",
    )
    .unwrap();
    let review = yashy_sdk::review(&dir, &baseline)
        .expect("the review must build")
        .render();
    assert!(review.contains("\"public_break\": true"), "{review}");
    assert!(review.contains("\"risk\": \"high\""), "{review}");

    let impact = yashy_sdk::impact(&dir, &baseline)
        .expect("the impact must build")
        .render();
    assert!(impact.contains("tests/smoke.ya"), "{impact}");
    assert!(impact.contains("\"add\""), "{impact}");
}

#[test]
fn explain_returns_the_same_catalog_facts() {
    let entry = yashy_sdk::explain("E0100").expect("a catalog entry");
    let rendered = entry.render();
    assert!(rendered.contains("\"code\": \"E0100\""), "{rendered}");
    assert!(rendered.contains("\"rule\":"), "{rendered}");
    assert!(rendered.contains("\"docs_url\":"), "{rendered}");
    assert!(
        yashy_sdk::explain("E9999").is_none(),
        "unknown codes are none"
    );
}

#[test]
fn context_and_workspace_root_use_the_same_workspace() {
    let dir = work_dir("context");
    let nested = dir.join("tests");
    assert_eq!(yashy_sdk::workspace_root(&nested), dir);
    let pack = yashy_sdk::context(&dir, "fix add", 8000).expect("a context pack");
    let rendered = pack.render();
    assert!(rendered.contains("schema_version"), "{rendered}");
    assert!(rendered.contains("included"), "{rendered}");
}
