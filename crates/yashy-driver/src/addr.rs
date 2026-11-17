//! `inspect --addr`: map a binary address back to a source span.
//!
//! The binary is read with `llvm-nm` (a symbol table, not a debugger). The
//! source span comes from the compiler's own index, so no second parser is
//! involved. A debug build's line table is what makes the address meaningful;
//! the command says so when the symbol table has no entry at or below the
//! address.

use std::path::{Path, PathBuf};
use std::process::Command;

use yashy_common::Json;

/// One symbol from `llvm-nm`, sorted by address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub address: u64,
    pub name: String,
}

/// Parse `llvm-nm -n` output. Only numeric addresses count; `U` (undefined)
/// and other non-address lines are ignored.
pub fn parse_nm(text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(address) = fields.next() else {
            continue;
        };
        if let Ok(address) = u64::from_str_radix(address, 16) {
            if let Some(name) = fields.next() {
                out.push(Symbol {
                    address,
                    name: name.to_string(),
                });
            }
        }
    }
    out
}

/// The symbol at or immediately before `address`.
pub fn nearest(symbols: &[Symbol], address: u64) -> Option<&Symbol> {
    symbols
        .iter()
        .filter(|symbol| symbol.address <= address)
        .max_by_key(|symbol| symbol.address)
}

fn nm_program() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("YASHY_NM") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(toolchain) = crate::toolchain::detect() {
        let sibling = toolchain
            .llc
            .with_file_name(format!("llvm-nm{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Ok(PathBuf::from("llvm-nm"))
}

/// Ask `llvm-nm` for the binary's sorted symbol table.
pub fn symbols(binary: &Path) -> Result<Vec<Symbol>, String> {
    let program = nm_program()?;
    let output = Command::new(program)
        .arg("-n")
        .arg(binary)
        .output()
        .map_err(|error| format!("cannot run llvm-nm: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "llvm-nm could not read {}: {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(parse_nm(&String::from_utf8_lossy(&output.stdout)))
}

/// Render an address lookup as a schema-v1 JSON envelope.
pub fn json(
    address: u64,
    symbol: &Symbol,
    source: &str,
    line: u32,
    column: u32,
    span: yashy_common::Span,
) -> Json {
    let mut result = Json::object();
    result.set("address", Json::string(format!("0x{address:x}")));
    result.set("symbol", Json::string(&symbol.name));
    result.set("file", Json::string(source));
    result.set("line", Json::number(i64::from(line)));
    result.set("column", Json::number(i64::from(column)));
    result.set("start", Json::number(i64::from(span.start)));
    result.set("end", Json::number(i64::from(span.end)));
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("inspect --addr"));
    report.set("result", result);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nm_output_is_parsed_in_address_order() {
        let text = "        0: 0000000140001000 T main\n                 U printf\n0000000140002000 t yashy_fn_add\n";
        let parsed = parse_nm(text);
        assert_eq!(parsed.len(), 2, "{parsed:?}");
        assert_eq!(parsed[0].name, "main");
        assert_eq!(parsed[1].name, "yashy_fn_add");
        assert_eq!(
            nearest(&parsed, 0x140001100).map(|s| s.name.as_str()),
            Some("main")
        );
        assert!(nearest(&parsed, 0x140000f00).is_none());
    }
}
