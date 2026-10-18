#![no_main]
//! Fuzz target: the whole front end, through code generation.
//!
//! `lex_parse` stops at the AST; this one carries a program that *does* parse
//! through the type checker and the IR emitter, which is where the interesting
//! panics live -- an unhandled type combination, an empty string slot, a
//! pattern that reaches a lookup with nothing behind it. Those need input that
//! is syntactically valid and semantically pathological, which random bytes
//! almost never produce and a corpus of real programs plus mutations does.

use libfuzzer_sys::fuzz_target;
use otter_codegen::{emit_program, CodegenOptions};
use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::check_program;

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);

    let mut sources = SourceMap::new();
    let file = sources.add_file("fuzz.ot", source.to_string());

    let Ok(tokens) = Lexer::new(&source).tokenize() else {
        return;
    };
    let mut parser = Parser::new(tokens, &sources, file);
    let Ok(program) = parser.parse_program() else {
        return;
    };
    let Ok(hir) = check_program(&program, &sources, file) else {
        return;
    };
    // The emitter is infallible by contract: it must return a `String` for every
    // HIR the checker accepted, however odd.
    let ir = emit_program(&hir, "fuzz.ot", CodegenOptions::default());
    // A cheap shape check. A panic would already have been caught, so this is
    // here for the case where the emitter silently returns nothing at all.
    assert!(ir.contains("target triple"), "the emitter produced no module");
});
