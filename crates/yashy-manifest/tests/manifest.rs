//! The conformance corpus: 15 manifests that parse and 15 that do not.
//!
//! The corpus is the executable form of `docs/cli.md`'s TOML subset. Each
//! invalid file names the line it must point at, so a reader that reports the
//! wrong place fails just as loudly as one that accepts the file.

use std::path::{Path, PathBuf};

use yashy_manifest::Manifest;

fn corpus(kind: &str) -> Vec<(String, PathBuf)> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join(kind);
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| {
            let path = entry.expect("a corpus entry").path();
            (
                path.file_name()
                    .expect("a file name")
                    .to_string_lossy()
                    .into_owned(),
                path,
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn the_corpus_has_fifteen_manifests_of_each_kind() {
    assert_eq!(corpus("valid").len(), 15, "15 valid manifests");
    assert_eq!(corpus("invalid").len(), 15, "15 invalid manifests");
}

#[test]
fn every_valid_manifest_parses_to_the_struct() {
    for (name, path) in corpus("valid") {
        let text = std::fs::read_to_string(&path).expect("readable");
        let manifest = Manifest::parse(&text)
            .unwrap_or_else(|error| panic!("{name} must parse, got {error}"));
        assert!(!manifest.package.name.is_empty(), "{name}: a package name");
        assert!(
            !manifest.package.version.is_empty(),
            "{name}: a package version"
        );
    }
}

#[test]
fn every_invalid_manifest_is_e1800_at_the_named_line() {
    // file -> (line, a fragment the message must contain)
    let expected: &[(&str, usize, &str)] = &[
        ("01-missing-package.toml", 1, "package"),
        ("02-missing-name.toml", 1, "name"),
        ("03-missing-version.toml", 1, "version"),
        ("04-range-version.toml", 3, "exact version"),
        ("05-short-version.toml", 3, "exact version"),
        ("06-unknown-table.toml", 5, "dependecies"),
        ("07-unknown-package-key.toml", 4, "nmae"),
        ("08-unknown-dependency-field.toml", 6, "url"),
        ("09-git-without-rev.toml", 5, "pinned to a commit"),
        ("10-rev-without-git.toml", 6, "rev"),
        ("11-two-sources.toml", 5, "exactly one source"),
        ("12-inline-table.toml", 6, "not a value in the subset"),
        ("13-malformed-header.toml", 1, "]"),
        ("14-duplicate-key.toml", 4, "assigned twice"),
        ("15-bad-name.toml", 2, "bad/name"),
    ];
    let files = corpus("invalid");
    assert_eq!(files.len(), expected.len());
    for (name, path) in files {
        let text = std::fs::read_to_string(&path).expect("readable");
        let error = Manifest::parse(&text)
            .unwrap_err_or_else(|| panic!("{name} must be rejected"));
        assert_eq!(error.code(), yashy_common::codes::MANIFEST_SYNTAX);
        let (_, line, fragment) = expected
            .iter()
            .find(|(file, _, _)| *file == name)
            .unwrap_or_else(|| panic!("{name} has no expected row"));
        assert_eq!(
            error.line, *line,
            "{name}: expected line {line}, got {}: {error}",
            error.line
        );
        assert!(
            error.message.contains(fragment),
            "{name}: message `{}` must contain `{fragment}`",
            error.message
        );
    }
}

/// `Result::unwrap_err` needs a `Debug` success value; this keeps the test
/// readable without an extra import at the call site.
trait UnwrapErrOrElse<T, E> {
    fn unwrap_err_or_else(self, message: impl FnOnce() -> String) -> E;
}

impl<T, E> UnwrapErrOrElse<T, E> for Result<T, E> {
    fn unwrap_err_or_else(self, message: impl FnOnce() -> String) -> E {
        match self {
            Ok(_) => panic!("{}", message()),
            Err(error) => error,
        }
    }
}
