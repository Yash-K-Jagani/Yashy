//! A registry served over HTTP: `yashy add` fetches, verifies the hash and
//! builds.
//!
//! The registry a release publishes is a static index on a host, not a
//! directory on the reader's machine, so the read path has to work over a URL.
//! The test serves the directory over HTTP with a small server and checks the
//! two things that matter: a package that arrives intact is built, and a
//! package that was changed on the way is refused.

mod common;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A tiny HTTP server for the registry directory.
///
/// It serves files and nothing else: the point is the *client's* behaviour,
/// not the server's.
struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start(root: PathBuf) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the test server must bind");
        let port = listener.local_addr().expect("the bound address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else { break };
                let mut request = [0u8; 4096];
                let read = stream.read(&mut request).unwrap_or(0);
                let text = String::from_utf8_lossy(&request[..read]).into_owned();
                let path = text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let name = path.trim_start_matches('/');
                let body = std::fs::read(root.join(name)).unwrap_or_default();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        Server {
            port,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The listener's accept wakes up when the process ends; the thread is
        // detached, because a test must not hang on a socket.
        let _ = self.handle.take();
    }
}

/// A package with a manifest and one module.
fn write_package(dir: &Path) {
    fs::create_dir_all(dir.join("src")).expect("the package directory must be creatable");
    fs::write(
        dir.join("yashy.toml"),
        "[package]\nname = \"greeting\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n",
    )
    .expect("the manifest must be writable");
    fs::write(
        dir.join("src/greeting.ya"),
        "pub fn hello() -> string {\n    return \"hello\";\n}\n",
    )
    .expect("the module must be writable");
}

/// The registry's own index, built from the package's tree hash.
fn write_registry(registry: &Path, package: &Path) -> String {
    let hash = yashy_manifest::hash_tree(package).expect("the package must hash");
    let entries = vec![yashy_driver::registry::RegistryEntry {
        name: "greeting".to_string(),
        version: "1.0.0".to_string(),
        hash: hash.clone(),
        description: "a greeting".to_string(),
    }];
    let mut index = yashy_common::Json::object();
    let mut array = Vec::new();
    for entry in &entries {
        let mut item = yashy_common::Json::object();
        item.set("name", yashy_common::Json::string(&entry.name));
        item.set("version", yashy_common::Json::string(&entry.version));
        item.set("hash", yashy_common::Json::string(&entry.hash));
        item.set("description", yashy_common::Json::string(&entry.description));
        array.push(item);
    }
    index.set("packages", yashy_common::Json::array(array));
    fs::create_dir_all(registry.join("packages")).expect("the registry must be creatable");
    fs::write(registry.join("index.json"), index.render()).expect("the index must be writable");

    // The tarball is the registry's own format: the same bytes the publish
    // command writes.
    let tarball = yashy_driver::registry::write_tarball(package).expect("the tarball must be written");
    let name = format!("{}.tar", hash.trim_start_matches("sha256:"));
    fs::write(registry.join("packages").join(name), tarball).expect("the tarball must be written");
    hash
}

/// A project that depends on the registry package.
fn write_project(dir: &Path, version: &str) {
    fs::create_dir_all(dir).expect("the project directory must be creatable");
    fs::write(
        dir.join("yashy.toml"),
        format!(
            "[package]\nname = \"user\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n\n[dependencies.greeting]\nversion = \"{version}\"\n"
        ),
    )
    .expect("the manifest must be writable");
    fs::write(
        dir.join("main.ya"),
        "import greeting;\n\nfn main() {\n    print(greeting.hello());\n}\n",
    )
    .expect("the program must be writable");
}

#[test]
fn a_package_is_fetched_from_an_http_registry_and_built() {
    let dir = common::work_dir("registry-http");
    let package = dir.join("package");
    let registry = dir.join("registry");
    let project = dir.join("project");
    write_package(&package);
    write_registry(&registry, &package);
    write_project(&project, "1.0.0");

    let server = Server::start(registry);
    let store = dir.join("store");

    let output = Command::new(common::yashy())
        .current_dir(&project)
        .arg("build")
        .arg("main.ya")
        .env("YASHY_REGISTRY", server.url())
        .env("YASHY_PACKAGE_STORE", &store)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "the build must fetch the package and succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The package is in the store, under its tree hash: the fetch happened.
    let fetched: Vec<PathBuf> = fs::read_dir(store.join("registry"))
        .expect("the store must hold the package")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(fetched.len(), 1, "the package must be in the store");
}

#[test]
fn a_tampered_tarball_fails_the_hash_check() {
    let dir = common::work_dir("registry-tampered");
    let package = dir.join("package");
    let registry = dir.join("registry");
    let project = dir.join("project");
    write_package(&package);
    write_registry(&registry, &package);
    write_project(&project, "1.0.0");

    // Change one byte of the tarball: the index still names the hash of the
    // tree the package had when it was published. The first tar entry is the
    // `src/` directory header (bytes 0-511), the second is `src/greeting.ya`'s
    // header (bytes 512-1023), so its file content starts at byte 1024.
    // Flipping a content byte (not a header byte) keeps the tar valid while
    // changing the extracted tree's hash.
    let packages = registry.join("packages");
    let tarball = fs::read_dir(&packages)
        .expect("the packages directory must be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("tar"))
        .expect("the tarball must be there");
    let mut bytes = fs::read(&tarball).expect("the tarball must be readable");
    assert!(bytes.len() > 1024, "the tarball must contain file content");
    bytes[1024] ^= 0xff;
    fs::write(&tarball, &bytes).expect("the tarball must be writable");

    let server = Server::start(registry);
    let store = dir.join("store");

    let output = Command::new(common::yashy())
        .current_dir(&project)
        .arg("build")
        .arg("main.ya")
        .env("YASHY_REGISTRY", server.url())
        .env("YASHY_PACKAGE_STORE", &store)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        !output.status.success(),
        "a tampered tarball must not build:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("tree hash") || stderr.contains("does not match"),
        "the failure must name the hash:\n{stderr}"
    );
}

#[test]
fn a_registry_url_is_a_valid_registry_spec() {
    let source = yashy_driver::registry::registry_source("https://example.com/registry")
        .expect("a URL must be a registry");
    assert!(!source.is_local());
    assert_eq!(source.describe(), "https://example.com/registry");

    let source = yashy_driver::registry::registry_source("file:///tmp/registry")
        .expect("a file URL must be a registry");
    assert!(source.is_local());

    let error = yashy_driver::registry::registry_source("relative/path")
        .expect_err("a relative path must be refused");
    assert!(error.message.contains("absolute path"), "{}", error.message);
}
