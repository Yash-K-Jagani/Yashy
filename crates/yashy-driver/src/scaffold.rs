//! Project scaffolding for `yashyc new`.
//!
//! A template is data: a list of files whose bodies can name the project. No
//! shell runs, no network fetch, and an unknown template is a diagnostic.

use std::fs;
use std::path::{Path, PathBuf};

/// The templates `yashyc new` can create.
pub const TEMPLATES: &[&str] = &["bin", "lib", "cli", "http", "workspace", "service", "native"];

/// What `new` creates: a directory, a manifest, and the template's files.
pub struct Created {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
}

pub fn manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2027\"\n")
}

/// A workspace template writes a workspace root instead of a package manifest.
pub fn root_manifest(template: &str, name: &str) -> String {
    if template == "workspace" {
        format!(
            "[workspace]\nmembers = [\"app\", \"lib\"]\ndefault-members = [\"app\"]\n"
        )
    } else {
        manifest(name)
    }
}

fn files(template: &str, name: &str) -> Result<Vec<(&'static str, String)>, String> {
    match template {
        "bin" => Ok(vec![
            (
                "main.ya",
                "fn main() {\n    print(\"Hello, {name}!\");\n}\n".replace("{name}", name),
            ),
            (
                "tests/smoke.ya",
                "fn main() {\n    assert(1 + 1 == 2);\n}\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nRun `yashyc test`.\n")),
        ]),
        "lib" => Ok(vec![
            (
                "src/lib.ya",
                "pub fn answer() -> int {\n    return 42;\n}\n".to_string(),
            ),
            (
                "tests/lib.ya",
                "fn main() {\n    assert(1 + 1 == 2);\n}\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nA Yashy library.\n")),
        ]),
        "cli" => Ok(vec![
            (
                "main.ya",
                "fn main() {\n    print(\"usage: {name}\");\n}\n".replace("{name}", name),
            ),
            (
                "tests/smoke.ya",
                "fn main() {\n    assert(1 + 1 == 2);\n}\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nA Yashy command.\n")),
        ]),
        "http" => Ok(vec![
            (
                "main.ya",
                "// Phase 6 adds the HTTP stack. Until then this is a loopback smoke test.\nfn main() {\n    print(\"{name} ready\");\n}\n".replace("{name}", name),
            ),
            (
                "tests/smoke.ya",
                "fn main() {\n    assert(1 + 1 == 2);\n}\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nAn HTTP service (stack lands in Phase 6).\n")),
        ]),
        "workspace" => Ok(vec![
            (
                "app/yashy.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2027\"\n\n\
                 [dependencies.lib]\npath = \"../lib\"\n".to_string(),
            ),
            (
                "app/main.ya",
                "fn main() {\n    print(\"Hello from {name}!\");\n}\n".replace("{name}", name),
            ),
            (
                "app/tests/smoke.ya",
                "fn main() { assert(1 + 1 == 2); }\n".to_string(),
            ),
            (
                "lib/yashy.toml",
                "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2027\"\n".to_string(),
            ),
            (
                "lib/src/lib.ya",
                "pub fn answer() -> int { return 42; }\n".to_string(),
            ),
            (
                "lib/tests/lib.ya",
                "fn main() { assert(1 + 1 == 2); }\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nRun `yashyc test --workspace`.\n")),
        ]),
        "service" => Ok(vec![
            (
                "main.ya",
                "// Phase 6 adds the net and http modules. This template prints a health\n\
                 // response now; the first `yashyc run` still proves the program works.\n\
                 fn main() {\n    print(\"HTTP/1.1 200 OK\\r\\nContent-Length: 2\\r\\n\\r\\nok\");\n}\n".to_string(),
            ),
            (
                "tests/smoke.ya",
                "fn main() { assert(1 + 1 == 2); }\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nA service skeleton; the socket arrives with Phase 6.\n")),
        ]),
        "native" => Ok(vec![
            (
                "main.ya",
                r#"// A `-sys` package: `bindings.ya` is the raw C ABI, `safe.ya` is
// the checked wrapper, and `fixture.c` is the vendored source the
// build compiles with this program.
import safe;

fn main() {
    print(show(safe_add(1, 2)));
    print(show(safe_add(9223372036854775807, 1)));
}

/// The text of a checked sum. `None` is the wrapper's refusal, not a
/// sentinel from C.
fn show(Option[int] value) -> string {
    return switch value {
        case Some(total) { "${total}" }
        case None { "overflow" }
    };
}
"#
                .to_string(),
            ),
            (
                "bindings.ya",
                r#"// The raw C ABI: one declaration per symbol `fixture.c` defines, and a
// `pub` forward so another module can reach it. An `extern` declaration is
// private to the module that writes it, so this file is the one place that
// names C.
extern "C" link "fixture.c" {
    unsafe fn add(int a, int b) -> int;
}

/// C's `add`, unchecked: the sum wraps on overflow exactly as C says.
/// A `-sys` module's surface is the raw ABI, so this forward adds no check.
pub fn raw_add(int a, int b) -> int {
    return add(a, b);
}
"#
                .to_string(),
            ),
            (
                "safe.ya",
                r#"// The safe surface of the package. `bindings.ya` is C's contract;
// this module is Yashy's.
import bindings;

/// `add(a, b)`, or `None` when the sum would not fit in `int`.
///
/// The C function is a `long long` addition, which wraps on overflow.
/// The guard is what makes the call safe: an overflowing pair never
/// reaches C, so the answer is a value and not undefined behaviour.
pub fn safe_add(int a, int b) -> Option[int] {
    if a > 0 && b > 9223372036854775807 - a {
        return None;
    }
    if a < 0 && b < -9223372036854775807 - 1 - a {
        return None;
    }
    return Some(raw_add(a, b));
}
"#
                .to_string(),
            ),
            (
                "fixture.c",
                r#"/* The vendored C source. `link "fixture.c"` is an input to the
 * build, so clang compiles it with the program: no separate build step
 * and no installed library. */
long long add(long long a, long long b) { return a + b; }
"#
                .to_string(),
            ),
            (
                "tests/smoke.ya",
                r#"// A test is an entry file of its own, so it declares the ABI it
// exercises and links the project's fixture by relative path. The
// wrapper is exercised by `main.ya`; this test proves the C library
// itself is reachable.
extern "C" link "../fixture.c" {
    unsafe fn add(int a, int b) -> int;
}

fn main() {
    assert(add(1, 2) == 3);
    assert(add(20, 22) == 42);
}
"#
                .to_string(),
            ),
            (
                "README.md",
                format!(
                    "# {name}\n\n\
                     A `-sys` package skeleton. `bindings.ya` declares the C ABI and\n\
                     forwards it as `raw_add`, `safe.ya` wraps that in a checked call,\n\
                     `fixture.c` is the vendored source, and `tests/smoke.ya` links the\n\
                     same fixture.\n\n\
                     `link \"fixture.c\"` names a file rather than a library: the build\n\
                     compiles the C source with the program, so `yashyc build` and\n\
                     `yashyc test` need no separate `cc` step. A system library keeps\n\
                     the library spelling, such as `link \"m\"`.\n\n\
                     The wrapper is the package's contract. `safe_add` refuses a\n\
                     pair whose sum would overflow, so `raw_add` is the only path to\n\
                     the unchecked ABI.\n"
                ),
            ),
        ]),
        other => Err(format!(
            "unknown template `{other}`; expected one of: {}",
            TEMPLATES.join(", ")
        )),
    }
}

/// Create a project under `parent/<name>`. An existing non-empty directory is
/// refused rather than merged.
pub fn create(parent: &Path, name: &str, template: &str) -> Result<Created, String> {
    let mut parts = name.split('/');
    let leaf = parts.next_back().unwrap_or(name);
    if leaf.is_empty()
        || !leaf
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("invalid project name `{name}`"));
    }
    let template_files = files(template, leaf)?;
    let root = parent.join(name);
    if root.exists() {
        let entries = fs::read_dir(&root).map_err(|error| error.to_string())?;
        if entries.count() > 0 {
            return Err(format!(
                "directory {} already exists and is not empty",
                root.display()
            ));
        }
    }
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let mut created = Vec::new();
    let mut all = template_files;
    all.push(("yashy.toml", root_manifest(template, leaf)));
    for (relative, body) in all {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, body).map_err(|error| error.to_string())?;
        created.push(path);
    }
    Ok(Created {
        root,
        files: created,
    })
}
