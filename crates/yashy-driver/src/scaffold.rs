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
                "// The Phase 6 native linker work makes this a one-command build.\n\
                 // For now the extern declaration documents the C ABI the fixture\n\
                 // in fixture.c provides.\n\
                 extern \"C\" {\n    fn add(int a, int b) -> int;\n}\n\
                 fn main() {\n    print(add(1, 2));\n}\n".to_string(),
            ),
            (
                "fixture.c",
                "int add(int a, int b) { return a + b; }\n".to_string(),
            ),
            (
                "tests/smoke.ya",
                "fn main() { assert(1 + 1 == 2); }\n".to_string(),
            ),
            ("README.md", format!("# {name}\n\nA native binding skeleton; see fixture.c.\n")),
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
