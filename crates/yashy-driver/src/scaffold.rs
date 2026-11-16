//! Project scaffolding for `yashyc new`.
//!
//! A template is data: a list of files whose bodies can name the project. No
//! shell runs, no network fetch, and an unknown template is a diagnostic.

use std::fs;
use std::path::{Path, PathBuf};

/// The templates `yashyc new` can create.
pub const TEMPLATES: &[&str] = &["bin", "lib", "cli", "http"];

/// What `new` creates: a directory, a manifest, and the template's files.
pub struct Created {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
}

pub fn manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2027\"\n")
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
    all.push(("yashy.toml", manifest(leaf)));
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
