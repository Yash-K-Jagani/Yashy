//! `yashyc tokens`: the lexer's view of a file, and the editor grammar
//! generator that reads the same token classes.
//!
//! A token dump is useful when a grammar or a diagnostic looks wrong: it shows
//! exactly what the lexer saw, with byte spans, before any parser made a guess.

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
