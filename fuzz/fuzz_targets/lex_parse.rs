#![no_main]
//! Fuzz target: lexing and parsing arbitrary bytes.
//!
//! The input is a *byte* string rather than UTF-8 on purpose. A source file is
//! bytes on disk, and `from_utf8_lossy` is the same conversion the driver
//! effectively performs when a file turns out not to be valid UTF-8, so the
//! target covers invalid-encoding inputs too rather than having the fuzzer
//! reject them before the compiler sees them.
//!
//! Nothing is asserted: the only failure mode this target looks for is a panic
//! or a stack overflow, both of which libFuzzer reports as a crash.

use libfuzzer_sys::fuzz_target;
use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);

    // File id zero, because `Lexer::new` reports against file zero and the map
    // has to have it -- the same arrangement the driver uses.
    let mut sources = SourceMap::new();
    let file = sources.add_file("fuzz.ya", source.to_string());

    if let Ok(tokens) = Lexer::new(&source).tokenize() {
        let mut parser = Parser::new(tokens, &sources, file);
        let _ = parser.parse_program();
    }
});
