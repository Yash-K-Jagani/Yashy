//! `yashy-lsp`: the language server over stdio.

use std::io;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    yashy_lsp::serve(stdin.lock(), &mut lock)
}
