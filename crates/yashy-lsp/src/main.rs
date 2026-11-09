//! `yashy-lsp`: the language server over stdio.

use std::io;

fn main() -> io::Result<()> {
    for argument in std::env::args().skip(1) {
        if argument == "--version" {
            println!("yashy-lsp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    yashy_lsp::serve(stdin.lock(), &mut lock)
}
