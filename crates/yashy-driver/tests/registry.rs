//! A local registry: publish, search, resolve, and corruption checks.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{exe, launch_output, work_dir, write, yashy};

fn cli(directory: &Path, store: &Path, registry: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(directory)
        .env("YASHY_PACKAGE_STORE", store)
        .env("YASHY_CACHE_DIR", store.join("cache"))
        .env("YASHY_REGISTRY", registry)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn write_lib(root: &Path, value: i64) {
    write(
        root,
        "yashy.toml",
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\ndescription = \"test library\"\n",
    );
    write(
        root,
        "src/lib.ya",
        &format!("pub fn value() -> int {{ return {value}; }}\n"),
    );
}

fn write_app(root: &Path) {
    write(
        root,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n",
    );
    write(root, "src/main.ya", "fn main() { }\n");
}

fn publish(dir: &Path, package: &Path, registry: &Path) {
    let output = cli(dir, &dir.join("store"), registry, &[
        "publish",
        package.to_str().expect("path text"),
        "--registry",
        registry.to_str().expect("registry text"),
    ]);
    assert!(
        output.status.success(),
        "publish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn publish_search_add_and_build() {
    let dir = work_dir("registry-roundtrip");
    let registry = dir.join("registry");
    let package = dir.join("lib");
    let app = dir.join("app");
    write_lib(&package, 41);
    write_app(&app);
    publish(&dir, &package, &registry);

    let search = cli(&dir, &dir.join("store"), &registry, &[
        "search",
        "lib",
        "--registry",
        registry.to_str().unwrap(),
    ]);
    assert!(search.status.success());
    let text = String::from_utf8_lossy(&search.stdout);
    assert!(text.contains("lib 1.0.0"), "{text}");

    let add = cli(&app, &dir.join("store"), &registry, &[
        "add",
        "lib@1.0.0",
        "--registry",
        registry.to_str().unwrap(),
    ]);
    assert!(add.status.success(), "add:\n{}", String::from_utf8_lossy(&add.stderr));
    let manifest = fs::read_to_string(app.join("yashy.toml")).expect("manifest");
    assert!(manifest.contains("version = \"1.0.0\""), "{manifest}");
    assert!(app.join("yashy.lock").is_file());

    write(
        &app,
        "src/main.ya",
        "import lib;\n\nfn main() { assert(lib.value() == 41); print(\"registry-ok\"); }\n",
    );
    let binary = exe(&app, "app");
    let build = cli(
        &app,
        &dir.join("store"),
        &registry,
        &["build", "src/main.ya", "-o", binary.to_str().unwrap()],
    );
    assert!(build.status.success(), "build:\n{}", String::from_utf8_lossy(&build.stderr));
    let run = launch_output(&binary).expect("run");
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "registry-ok");
}

#[test]
fn republishing_a_different_hash_is_refused() {
    let dir = work_dir("registry-duplicate-hash");
    let registry = dir.join("registry");
    let package = dir.join("lib");
    write_lib(&package, 41);
    publish(&dir, &package, &registry);
    write_lib(&package, 42);
    let output = cli(&dir, &dir.join("store"), &registry, &[
        "publish",
        package.to_str().unwrap(),
        "--registry",
        registry.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate registry entry"), "{stderr}");
}

#[test]
fn a_corrupted_tarball_fails_the_hash_check() {
    let dir = work_dir("registry-corrupt");
    let registry = dir.join("registry");
    let package = dir.join("lib");
    write_lib(&package, 41);
    publish(&dir, &package, &registry);
    let tarballs: Vec<PathBuf> = fs::read_dir(registry.join("packages"))
        .expect("tarballs")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(tarballs.len(), 1);
    let mut bytes = fs::read(&tarballs[0]).expect("tarball");
    let offset = bytes
        .windows(b"pub fn value()".len())
        .position(|window| window == b"pub fn value()")
        .expect("source text in tarball");
    bytes[offset] = b'X';
    fs::write(&tarballs[0], bytes).expect("corrupt tarball");

    let app = dir.join("app");
    write_app(&app);
    let add = cli(&app, &dir.join("store"), &registry, &[
        "add",
        "lib@1.0.0",
        "--registry",
        registry.to_str().unwrap(),
    ]);
    assert!(!add.status.success());
    let stderr = String::from_utf8_lossy(&add.stderr);
    assert!(stderr.contains("tree hash"), "{stderr}");
}

#[test]
fn a_duplicate_index_entry_names_both() {
    let dir = work_dir("registry-duplicate-index");
    let registry = dir.join("registry");
    write(
        &registry,
        "index.json",
        "{\"packages\":[{\"name\":\"lib\",\"version\":\"1.0.0\",\"hash\":\"sha256:a\",\"description\":\"\"},{\"name\":\"lib\",\"version\":\"1.0.0\",\"hash\":\"sha256:b\",\"description\":\"\"}]}",
    );
    let output = cli(&dir, &dir.join("store"), &registry, &[
        "search",
        "lib",
        "--registry",
        registry.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("lib@1.0.0"), "{stderr}");
    assert!(stderr.contains("duplicate registry entry"), "{stderr}");
}
