//! Phase 8 package harnesses: CLI, config, template, auth/session/cache and
//! jobs. Each program prints one marker and must report zero live allocations.

mod common;

use std::path::Path;
use std::process::Command;

fn path_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

fn run(root: &Path, source: &str, name: &str, expected: &str, packages: &[&str]) {
    let mut paths = root.join("crates/yashy-stdlib/yashy").display().to_string();
    for package in packages {
        paths.push_str(path_separator());
        paths.push_str(&root.join("packages").join(package).join("src").display().to_string());
    }
    let dir = common::work_dir(name);
    let binary = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join(source))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building {name} failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the program must run");
    assert!(
        run.status.success(),
        "{name} exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        expected,
        "unexpected output from {name}"
    );
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "{name} leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn cli_parsing_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(&root, "packages/cli/tests/parsing.ya", "p8-cli", "cli-parsing-ok", &["cli"]);
}

#[test]
fn config_layers_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(&root, "packages/config/tests/layers.ya", "p8-config", "config-layers-ok", &["config"]);
}

#[test]
fn config_typed_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/config/tests/typed.ya",
        "p8-config-typed",
        "config-typed-ok",
        &["config"],
    );
}

#[test]
fn template_render_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(&root, "packages/template/tests/render.ya", "p8-template", "template-ok", &["template"]);
}

#[test]
fn template_loops_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/template/tests/loops.ya",
        "p8-template-loops",
        "template-loops-ok",
        &["template"],
    );
}

#[test]
fn auth_session_cache_jobs_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/auth_stack.ya",
        "p8-auth-stack",
        "auth-stack-ok",
        &["auth", "session", "cache", "jobs"],
    );
}

#[test]
fn cli_subcommands_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/cli/tests/subcommands.ya",
        "p8-cli-subcommands",
        "cli-subcommands-ok",
        &["cli"],
    );
}

#[test]
fn cli_required_option_is_a_usage_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = root.join("crates/yashy-stdlib/yashy").display().to_string();
    paths.push_str(path_separator());
    paths.push_str(&root.join("packages/cli/src").display().to_string());
    let dir = common::work_dir("p8-cli-required");
    let binary = dir.join(format!("cli-required{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/cli/tests/required.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let output = Command::new(&binary).output().expect("the program must run");
    assert_eq!(output.status.code(), Some(2), "a missing required option must exit 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing required --path"), "unexpected stderr: {stderr}");
}

#[test]
fn auth_tokens_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/auth/tests/tokens.ya",
        "p8-auth-tokens",
        "auth-tokens-ok",
        &["auth", "session"],
    );
}

#[test]
fn mail_jobs_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/mail/tests/compose.ya",
        "p8-mail-jobs",
        "mail-jobs-ok",
        &["mail", "jobs", "template"],
    );
}

#[test]
fn service_lifecycle_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/http/tests/lifecycle_test.ya",
        "p8-lifecycle",
        "lifecycle-ok",
        &["http"],
    );
}

#[test]
fn api_service_example_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "examples/api-service/main.ya",
        "p8-example-api",
        "{\"status\":\"ok\"}\n{\"openapi\":\"3.1.0\",\"info\":{\"title\":\"items\",\"version\":\"1.0.0\"},\"paths\":{\"/items\":{\"get\":{\"responses\":{\"200\":{\"description\":\"ok\"}}}},\"/items\":{\"post\":{\"requestBody\":{\"content\":{\"application/json\":{}}},\"responses\":{\"200\":{\"description\":\"ok\"}}}}}}",
        &["http"],
    );
}

#[test]
fn cli_tool_example_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(&root, "examples/cli-tool/main.ya", "p8-example-cli", "name: config-user", &["cli", "config"]);
}

#[test]
fn web_app_example_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "examples/web-app/main.ya",
        "p8-example-web",
        "<h1>Hello &lt;b&gt;Ada&lt;/b&gt;</h1>",
        &["template", "auth", "session"],
    );
}

#[test]
fn browser_auth_and_otel_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/auth/tests/browser.ya",
        "p8-browser-otel",
        "browser-otel-ok",
        &["auth", "otel"],
    );
}

#[test]
fn service_packages_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/phase8_services.ya",
        "p8-services",
        "phase8-services-ok",
        &["mail", "jobs", "http", "auth", "otel"],
    );
}
