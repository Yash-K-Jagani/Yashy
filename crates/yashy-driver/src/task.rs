//! Named project tasks: a list of compiler verbs, never a shell string.
//!
//! `yashy.toml` gains `[tasks] test = ["test"]`. `yashyc task test` runs the
//! same verbs in the same process as typing them; a task cannot execute an
//! arbitrary command, and an unknown name is a diagnostic.

use std::fs;
use std::path::Path;

use yashy_manifest::toml::{self, Value};

/// One task: a sequence of argument vectors, each starting with a verb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub steps: Vec<Vec<String>>,
}

/// Read `[tasks]` from a manifest. Missing table means no tasks.
pub fn load(root: &Path) -> Result<Vec<Task>, String> {
    let path = root.join("yashy.toml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let document = toml::parse_document(&text).map_err(|error| error.to_string())?;
    let Some(table) = document.root.get("tasks").and_then(Value::as_table) else {
        return Ok(Vec::new());
    };
    let mut tasks = Vec::new();
    for (name, value) in table {
        let steps = match value {
            Value::Array(items) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(|text| text.split_whitespace().map(str::to_string).collect())
                        .ok_or_else(|| format!("task `{name}` has a non-string step"))
                })
                .collect::<Result<Vec<Vec<String>>, String>>()?,
            Value::String(text) => vec![text.split_whitespace().map(str::to_string).collect()],
            _ => {
                return Err(format!(
                    "task `{name}` must be an array of verbs or a string"
                ))
            }
        };
        for step in &steps {
            if step.is_empty() {
                return Err(format!("task `{name}` has an empty step"));
            }
        }
        tasks.push(Task {
            name: name.clone(),
            steps,
        });
    }
    tasks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tasks)
}

/// A task is a list of compiler verbs; anything else is refused.
pub fn validate(tasks: &[Task], known: &[&str]) -> Result<(), String> {
    for task in tasks {
        for step in &task.steps {
            if !known.contains(&step[0].as_str()) {
                return Err(format!(
                    "task `{}` names `{}`, which is not a compiler verb",
                    task.name, step[0]
                ));
            }
        }
    }
    Ok(())
}
