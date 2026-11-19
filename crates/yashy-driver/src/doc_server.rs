//! `yashyc doc --serve`: a read-only docs server on loopback.
//!
//! The model is built once, at startup, from the compiler's own tables. There
//! is no write path, no path traversal and no bind beyond `127.0.0.1`: a docs
//! server that can edit a project or reach the network is a different, much
//! more dangerous program. `--port 0` asks the OS for a free port and prints
//! the chosen one, which is how the test and a CI job start it safely.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

use crate::doc::{self, DocModel};
use yashy_common::Json;

/// The response a request produces. Kept separate from the socket so the
/// routing rules can be tested without binding a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

fn json_body(model: &DocModel) -> String {
    doc::to_json(model).render()
}

/// Render one module's Markdown page. The body is derived from the same model
/// `yashyc doc --out` writes, not read from disk at request time.
fn markdown(module: &doc::DocModule) -> String {
    let mut text = format!("# {}\n", module.name);
    for item in &module.items {
        text.push_str(&format!(
            "\n## {} {}\n\n```\n{}\n```\n",
            item.kind, item.name, item.signature
        ));
        match &item.docs {
            Some(docs) => text.push_str(&format!("\n{}\n", docs)),
            None => text.push_str("\nNo documentation.\n"),
        }
    }
    text
}

fn html_escape(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

fn index(model: &DocModel) -> String {
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Yashy docs</title></head><body>\n<h1>Yashy docs</h1>\n<ul>\n",
    );
    for module in &model.modules {
        out.push_str(&format!(
            "<li><a href=\"/{}.md\">{}</a></li>\n",
            html_escape(&module.name),
            html_escape(&module.name)
        ));
    }
    out.push_str("</ul>\n<p><a href=\"/json\">/json</a> &middot; <a href=\"/search\">/search</a></p>\n</body></html>\n");
    out
}

/// Decode the `q` query parameter. Only the form `?q=value` is used, and the
/// value is percent-decoded one byte at a time so a malformed escape is kept
/// literally instead of panicking.
fn query_value(uri: &str) -> String {
    let Some(query) = uri.split_once('?').map(|(_, query)| query) else {
        return String::new();
    };
    let raw = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("q="))
        .unwrap_or("");
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (
                (bytes[at + 1] as char).to_digit(16),
                (bytes[at + 2] as char).to_digit(16),
            ) {
                out.push((high * 16 + low) as u8);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A case-insensitive search over names, signatures and docs.
pub fn search(model: &DocModel, query: &str) -> Vec<String> {
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    for module in &model.modules {
        for item in &module.items {
            let haystack = format!(
                "{} {} {}",
                item.name,
                item.signature,
                item.docs.as_deref().unwrap_or("")
            )
            .to_lowercase();
            if needle.is_empty() || haystack.contains(&needle) {
                hits.push(format!("{}.{}", module.name, item.name));
            }
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

/// Route one request path. Everything not listed here is a `404`; no path is
/// ever used to open a file.
pub fn respond(model: &DocModel, uri: &str) -> Response {
    let path = uri.split('?').next().unwrap_or("/");
    let body = match path {
        "/" | "/index.html" => Some(("text/html; charset=utf-8", index(model))),
        "/json" => Some(("application/json", json_body(model))),
        "/search" => {
            let hits = search(model, &query_value(uri));
            let mut value = Json::object();
            value.set("schema_version", Json::number(1));
            value.set("tool", Json::string("doc-search"));
            value.set("query", Json::string(query_value(uri)));
            value.set(
                "hits",
                Json::array(hits.iter().map(|hit| Json::string(hit)).collect()),
            );
            Some(("application/json", value.render()))
        }
        other if other.ends_with(".md") => {
            let name = other.trim_end_matches(".md").trim_start_matches('/');
            model
                .modules
                .iter()
                .find(|module| module.name == name)
                .map(|module| ("text/markdown; charset=utf-8", markdown(module)))
        }
        _ => None,
    };
    match body {
        Some((content_type, body)) => Response {
            status: 200,
            content_type,
            body,
        },
        None => Response {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: "not found\n".to_string(),
        },
    }
}

fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let reason = if response.status == 200 {
        "OK"
    } else {
        "Not Found"
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(response.body.as_bytes())
}

fn serve_stream(model: &DocModel, stream: &mut TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    // A request line is the only thing read. A bad or non-GET request gets a
    // 404 rather than being forwarded anywhere.
    let _ = reader.read_line(&mut request)?;
    // Drain the remaining headers. Dropping a socket with unread bytes makes
    // Linux send RST, which can discard the response we just wrote.
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let uri = request.split_whitespace().nth(1).unwrap_or("/").to_string();
    let response = if request.starts_with("GET ") {
        respond(model, &uri)
    } else {
        Response {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: "not found\n".to_string(),
        }
    };
    write_response(stream, &response)
}

/// Bind loopback and build the model once. Returns the listener, the model and
/// the chosen port; `--port 0` asks the OS for a free port.
pub fn bind(root: &Path, private: bool, port: u16) -> Result<(TcpListener, DocModel, u16), String> {
    let model = doc::build_doc(root, private)?;
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("cannot bind 127.0.0.1:{port}: {error}"))?;
    let chosen = listener
        .local_addr()
        .map_err(|error| format!("cannot read the bound address: {error}"))?
        .port();
    Ok((listener, model, chosen))
}

/// Serve the bound listener until the process exits. Split from [`bind`] so a
/// test can start the server on a known port and talk to it.
pub fn serve_bound(listener: TcpListener, model: DocModel) {
    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                let _ = serve_stream(&model, &mut stream);
            }
            Err(_) => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> DocModel {
        let mut model = DocModel::default();
        model.modules.push(doc::DocModule {
            name: "io".to_string(),
            items: vec![doc::DocItem {
                name: "read_file".to_string(),
                kind: "fn".to_string(),
                signature: "fn read_file(string path) -> Result[string, io.Error]".to_string(),
                docs: Some("Reads a file and answers with a Result.".to_string()),
                effects: None,
                contracts: Vec::new(),
            }],
        });
        model
    }

    #[test]
    fn the_index_json_and_page_are_read_only_routes() {
        assert_eq!(respond(&model(), "/").status, 200);
        assert_eq!(respond(&model(), "/json").content_type, "application/json");
        let page = respond(&model(), "/io.md");
        assert_eq!(page.status, 200);
        assert!(page.body.contains("read_file"));
        assert_eq!(respond(&model(), "/../../etc/passwd").status, 404);
    }

    #[test]
    fn search_matches_names_signatures_and_docs() {
        let found = search(&model(), "result");
        assert_eq!(found, vec!["io.read_file"]);
        assert!(search(&model(), "no-such-symbol").is_empty());
    }

    #[test]
    fn a_search_request_answers_json() {
        let response = respond(&model(), "/search?q=read_file");
        assert_eq!(response.status, 200);
        assert!(response.body.contains("io.read_file"));
    }
}
