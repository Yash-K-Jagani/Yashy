//! `yashyc tokens`: the lexer's view of a file, and the editor grammar
//! generator that reads the same token classes.
//!
//! A token dump is useful when a grammar or a diagnostic looks wrong: it shows
//! exactly what the lexer saw, with byte spans, before any parser made a guess.

use std::path::PathBuf;

use yashy_common::{Json, SourceMap};
use yashy_lexer::Lexer;

/// Dump one source file's tokens as a schema-v1 JSON envelope.
pub fn dump_json(source: &str, file: &str) -> Result<Json, String> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().map_err(|diagnostics| {
        let mut map = SourceMap::new();
        map.add_file(file.to_string(), source.to_string());
        diagnostics
            .iter()
            .map(|diagnostic| map.render(diagnostic).trim_end().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let entries: Vec<Json> = tokens
        .iter()
        .map(|token| {
            let mut object = Json::object();
            object.set("kind", Json::string(format!("{:?}", token.kind)));
            object.set(
                "text",
                Json::string(
                    source
                        .get(token.span.start as usize..token.span.end as usize)
                        .unwrap_or(""),
                ),
            );
            object.set("start", Json::number(i64::from(token.span.start)));
            object.set("end", Json::number(i64::from(token.span.end)));
            object
        })
        .collect();
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("tokens"));
    report.set("file", Json::string(file));
    report.set("tokens", Json::array(entries));
    Ok(report)
}

/// Measure the lexical single-token property with the compiler's own lexer.
///
/// This is *not* an o200k/cl100k/Llama measurement: those vocabularies are
/// not vendored in this repository, so no honest report can claim them. What
/// this answers is the language-side half of the design goal -- every keyword,
/// operator and delimiter is exactly one Yashy token -- and it needs no network
/// or data files.
pub fn bpe_json(paths: &[PathBuf]) -> Result<Json, String> {
    use std::collections::{BTreeMap, BTreeSet};

    let model = yashy_lexer::grammar::model();
    let vocabulary: BTreeSet<&str> = model
        .keywords
        .iter()
        .chain(model.types.iter())
        .chain(model.operators.iter())
        .chain(model.delimiters.iter())
        .copied()
        .collect();
    let mut files = Vec::new();
    let mut vocabulary_tokens = 0i64;
    let mut total_tokens = 0i64;
    for path in paths {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize().map_err(|diagnostics| {
            format!("{}: {} lex error(s)", path.display(), diagnostics.len())
        })?;
        let mut classes: BTreeMap<String, i64> = BTreeMap::new();
        for token in tokens {
            let text = source
                .get(token.span.start as usize..token.span.end as usize)
                .unwrap_or("");
            *classes.entry(format!("{:?}", token.kind)).or_default() += 1;
            total_tokens += 1;
            if vocabulary.contains(text) {
                vocabulary_tokens += 1;
            }
        }
        let mut entry = Json::object();
        entry.set(
            "file",
            Json::string(path.to_string_lossy().replace('\\', "/")),
        );
        let mut counts = Json::object();
        for (kind, count) in classes {
            counts.set(kind, Json::number(count));
        }
        entry.set("counts", counts);
        files.push(entry);
    }
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("tokens --bpe"));
    report.set("tokenizer", Json::string("yashy-lexer"));
    report.set(
        "note",
        Json::string(
            "the o200k, cl100k and Llama vocabularies are not vendored; this measures Yashy's own tokenizer only",
        ),
    );
    report.set("files", Json::array(files));
    report.set("total_tokens", Json::number(total_tokens));
    report.set("vocabulary_tokens", Json::number(vocabulary_tokens));
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_dump_names_kinds_and_byte_spans() {
        let json = dump_json("fn main() {}", "main.ya").expect("valid source");
        let text = json.render();
        assert!(text.contains("\"kind\": \"Fn\""), "{text}");
        assert!(text.contains("\"text\": \"fn\""), "{text}");
        assert!(text.contains("\"start\": 0"), "{text}");
        assert!(text.contains("\"end\": 2"), "{text}");
    }

    #[test]
    fn a_lex_error_is_a_message_not_a_panic() {
        let error = dump_json("let x = \"unterminated", "main.ya").unwrap_err();
        assert!(!error.is_empty());
    }
}
