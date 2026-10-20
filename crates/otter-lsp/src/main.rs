//! `otter-lsp`: the language server over stdio.

use std::io;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    otter_lsp::serve(stdin.lock(), &mut lock)
}
