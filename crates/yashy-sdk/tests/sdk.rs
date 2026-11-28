//! The SDK wraps the same driver functions the CLI calls. Every result is a
//! JSON value in the CLI's schema, so a library client renders the same facts
//! as `yashyc` prints.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The CLI this workspace built. Cargo only sets `CARGO_BIN_EXE_*` for a
/// package's own binaries, so an SDK integration test finds it in the target
/// directory (or takes an explicit `YASHYC` from the caller).
fn yashyc() -> PathBuf {
    if let Ok(path) = std::env::var("YASHYC") {
        return PathBuf::from(path);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let debug = target.join("debug");
    let candidates = if cfg!(windows) {
        [debug.join("yashyc.exe"), debug.join("yashyc")]
    } else {
        [debug.join("yashyc"), debug.join("yashyc.exe")]
    };
    for candidate in candidates {
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("build the yashyc binary before running SDK parity tests");
}

fn cli_json(dir: &Path, args: &[&str]) -> yashy_common::json_rpc::Value {
    let output = Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    yashy_common::json_rpc::parse(&text).expect("the CLI printed JSON")
}

fn parsed(text: &str) -> yashy_common::json_rpc::Value {
    yashy_common::json_rpc::parse(text).expect("a JSON value")
}

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

#[test]
fn the_cli_and_sdk_render_the_same_index_explain_and_pack_answers() {
    let dir = work_dir("parity");
    let root = dir.to_str().unwrap();
    assert_eq!(
        cli_json(&dir, &["index", "--json", root]),
        parsed(&yashy_sdk::index(&dir).expect("an index").render()),
        "index is one model whatever the caller"
    );
    assert_eq!(
        cli_json(&dir, &["explain", "--json", "E0100"]),
        parsed(
            &yashy_sdk::explain("E0100")
                .expect("a catalog entry")
                .render()
        ),
        "explain is one catalog object"
    );
    let request = yashy_driver::pack::PackRequest {
        focus: "fix add".to_string(),
        mode: yashy_driver::pack::PackMode::Focus,
        budget: 8000,
        changed: Vec::new(),
        diagnostic: None,
    };
    assert_eq!(
        cli_json(
            &dir,
            &["context", "--json", "--task", "fix add", "--mode", "focus", root]
        ),
        parsed(&yashy_sdk::pack(&dir, request).expect("a pack").render()),
        "a context pack is one value"
    );
    assert_eq!(yashy_sdk::PARITY_VERBS.len(), 3);
}
