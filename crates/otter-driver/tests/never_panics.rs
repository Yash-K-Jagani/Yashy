//! The compiler must never take the process down on input it did not expect.
//!
//! Every stage of the front end is written to be total -- `peek` past the end of
//! the token stream yields `Eof` rather than indexing, type lookups return
//! `Option`, and every failure is a diagnostic -- but "written to be total" is a
//! claim, and this is the test that holds it to it. Each input below is fed
//! through the whole front end plus code generation, and the only acceptable
//! outcomes are "compiled" or "diagnostics".
//!
//! # Why this is not just a fuzzer
//!
//! A fuzzer finds what it finds; a table of the specific shapes that break
//! hand-written parsers -- truncated tokens, a lone closer, unbalanced
//! delimiters in both directions, an unterminated block comment in every
//! position, a NUL byte, a 100k-character identifier, absurd nesting -- runs in
//! milliseconds and fails with the name of the offender rather than with a
//! minimized blob. The fuzz targets in `fuzz/` cover the input space; this pins
//! the cases that are known to matter.
//!
//! # Stack overflow is the one panic `catch_unwind` cannot catch
//!
//! So the deeply nested cases below assert a *diagnostic* rather than merely
//! "no panic". Without the parser's depth limit they abort the process, which
//! no test can survive and no `catch_unwind` can report.

use otter_codegen::emit_program;
use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::check_program;

/// Run the whole front end over `src`. `true` when it compiled cleanly.
///
/// Deliberately does not return the diagnostics: the test asks only whether a
/// stage blew up, and an `Err` here is a legitimate outcome, not a failure.
fn run_pipeline(src: &str) -> bool {
    let mut sources = SourceMap::new();
    let file = sources.add_file("fuzz.ot", src.to_string());

    let Ok(tokens) = Lexer::new(src).tokenize() else {
        return false;
    };
    let mut parser = Parser::new(tokens, &sources, file);
    let Ok(program) = parser.parse_program() else {
        return false;
    };
    let Ok(hir) = check_program(&program, &sources, file) else {
        return false;
    };
    let _ = emit_program(&hir, "fuzz.ot");
    true
}

/// The malformed inputs, each with a name for the failure message.
fn malformed_inputs() -> Vec<(&'static str, String)> {
    let long_ident = format!("fn {}() {{ }}", "a".repeat(100_000));
    let many_delimiters = format!("fn main() {{ const x = {}; }}", "([{".repeat(200));
    let long_string = format!("fn main() {{ print_str(\"{}\"); }}", "x".repeat(100_000));

    vec![
        ("empty file", String::new()),
        ("whitespace only", "   \n\t\n  ".to_string()),
        ("comment only", "// nothing here".to_string()),
        ("block comment only", "/* nothing here */".to_string()),
        ("lone keyword", "fn".to_string()),
        ("lone struct keyword", "struct".to_string()),
        ("lone enum keyword", "enum".to_string()),
        ("lone import keyword", "import".to_string()),
        ("lone extern keyword", "extern".to_string()),
        ("lone switch keyword", "switch".to_string()),
        ("fn with no body", "fn main()".to_string()),
        ("truncated parameter list", "fn main(".to_string()),
        ("truncated body", "fn main() {".to_string()),
        ("stray closing brace", "fn main() }".to_string()),
        ("stray closing paren", ")))".to_string()),
        ("stray closers in every order", "}}]}[))(".to_string()),
        ("unbalanced opener", "(((((".to_string()),
        ("openers of every kind", "([{[(".to_string()),
        ("struct with no fields", "struct S { x:".to_string()),
        ("struct never closed", "struct S { x: f64".to_string()),
        ("enum never closed", "enum E {".to_string()),
        ("enum variant never closed", "enum E { A(".to_string()),
        ("import with no semicolon", "import std.io".to_string()),
        ("return outside a function", "return 1;".to_string()),
        ("nul bytes", "\0\0\0".to_string()),
        ("nul inside a string", "fn main() { print_str(\"\0\"); }".to_string()),
        ("unterminated string", "fn main() { const x = \"unterminated }".to_string()),
        (
            "unterminated block comment",
            "/* unterminated".to_string(),
        ),
        (
            "unterminated block comment inside a function",
            "fn main() { const x = 1; /* unterminated".to_string(),
        ),
        (
            "unterminated block comment inside a string",
            "fn main() { print_str(\"/*\"); }".to_string(),
        ),
        ("switch with no subject", "fn main() { switch }".to_string()),
        (
            "switch arm with no pattern",
            "fn main() { switch x { case } }".to_string(),
        ),
        ("for with no clauses", "fn main() { for }".to_string()),
        ("for range with no end", "fn main() { for i in 1 }".to_string()),
        ("for range with no bounds", "fn main() { for i in .. }".to_string()),
        ("defer with no body", "fn main() { defer }".to_string()),
        ("if with no condition", "fn main() { if }".to_string()),
        (
            "if with a dangling else",
            "fn main() { if true { } else".to_string(),
        ),
        ("binding with no name", "fn main() { const = 1; }".to_string()),
        ("binding with no value", "fn main() { const x = ; }".to_string()),
        ("trailing operator", "fn main() { const x = 1 + ; }".to_string()),
        ("cast with no target type", "fn main() { const x = 1 as; }".to_string()),
        ("field access with no field", "fn main() { const x = x.; }".to_string()),
        ("call with no callee", "fn main() { (1); }".to_string()),
        (
            "assignment, which the grammar does not have",
            "fn main() { var x = 1; x = 2; }".to_string(),
        ),
        (
            "let, which is not a keyword",
            "fn main() { let x = 1; }".to_string(),
        ),
        (
            "integer literal past i64",
            "fn main() { const x = 999999999999999999999999999; }".to_string(),
        ),
        (
            "float literal past f64",
            "fn main() { const x = 1e999; }".to_string(),
        ),
        (
            "binary literal with no digits",
            "fn main() { const x = 0x; }".to_string(),
        ),
        (
            "two decimal points",
            "fn main() { const x = 1.2.3; }".to_string(),
        ),
        (
            "invalid escape in a string",
            "fn main() { const x = \"a\\q\"; }".to_string(),
        ),
        ("char literal, which Otter has no syntax for", "fn main() { const x = 'a'; }".to_string()),
        ("at sign with no use", "fn main() { const x = @; }".to_string()),
        ("byte order mark", "\u{feff}fn main() { }".to_string()),
        ("carriage returns", "fn main() {\r\n    const x = 1;\r\n}\r\n".to_string()),
        ("non-ascii identifier", "fn na\u{ef}ve() { const caf\u{e9} = 1; }".to_string()),
        ("non-ascii identifier outside latin-1", "fn main() { const \u{65e5}\u{672c}\u{8a9e} = 1; }".to_string()),
        ("emoji in a string", "fn main() { print_str(\"\u{1f600}\"); }".to_string()),
        ("replacement character", "\u{fffd}".to_string()),
        ("very long identifier", long_ident),
        ("very long string", long_string),
        ("many interleaved delimiters", many_delimiters),
    ]
}

#[test]
fn no_input_makes_the_compiler_panic() {
    let cases = malformed_inputs();
    // A panic message per case would bury the interesting output; the failure
    // message below names the offender instead.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut panicked: Vec<&str> = Vec::new();
    let mut compiled = 0usize;
    let mut rejected = 0usize;
    for (name, src) in &cases {
        match std::panic::catch_unwind(|| run_pipeline(src)) {
            Ok(true) => compiled += 1,
            Ok(false) => rejected += 1,
            Err(_) => panicked.push(name),
        }
    }

    std::panic::set_hook(default_hook);

    assert!(
        panicked.is_empty(),
        "these inputs made the compiler panic: {panicked:?}"
    );
    assert_eq!(
        compiled + rejected,
        cases.len(),
        "every input must have been run"
    );
    // If everything were rejected, the table would prove nothing about the
    // stages past the lexer; if nothing were rejected, it would not be testing
    // malformed input at all.
    assert!(
        compiled > 0 && rejected > 0,
        "the table must exercise both outcomes, got {compiled} compiled and {rejected} rejected"
    );
}

/// Assert that `src` is rejected with a nesting diagnostic rather than aborting
/// the process.
fn assert_nesting_diagnostic(what: &str, src: &str) {
    let mut sources = SourceMap::new();
    let file = sources.add_file("deep.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .unwrap_or_else(|diags| panic!("{what}: must lex, got {diags:?}"));
    let mut parser = Parser::new(tokens, &sources, file);
    match parser.parse_program() {
        Ok(_) => panic!("{what}: {src:.80}… must not parse"),
        Err(diags) => {
            assert!(
                diags.iter().any(|d| d.message.contains("nested")),
                "{what}: expected a nesting diagnostic, got {:?}",
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn deeply_nested_input_is_a_diagnostic_not_a_stack_overflow() {
    // Each of these recurses once per delimiter in the parser. Run to the
    // machine's actual limit they overflow the stack, which aborts the process:
    // this test existing at all is the proof that the bound is low enough.
    let opens = "(".repeat(5_000);
    let closes = ")".repeat(5_000);
    assert_nesting_diagnostic(
        "nested parentheses",
        &format!("fn main() {{ const x = {opens}1{closes}; }}"),
    );
    assert_nesting_diagnostic(
        "nested calls",
        &format!("fn main() {{ {opens}){closes}; }}"),
    );

    // The same for blocks, where the recursion is block -> statement -> block.
    // A bare `{` is not a statement in Otter, so this has to nest real
    // constructs: `if true { if true { … } }` is what recurses.
    let nested_ifs = format!(
        "fn main() {{ {} {} }}",
        "if true { ".repeat(5_000),
        "}".repeat(5_000)
    );
    assert_nesting_diagnostic("nested if bodies", &nested_ifs);

    // And nested `for` bodies, which recurse through a different statement.
    let nested_fors = format!(
        "fn main() {{ {} {} }}",
        "for i in 0..1 { ".repeat(5_000),
        "}".repeat(5_000)
    );
    assert_nesting_diagnostic("nested for bodies", &nested_fors);
}

#[test]
fn nesting_to_the_limit_still_parses() {
    // The bound must not reject an expression a person could plausibly write.
    // Sixteen levels of explicit grouping in one arithmetic expression is
    // already unreasonable; it has to compile.
    let depth = 16;
    let opens = "(".repeat(depth);
    let closes = ")".repeat(depth);
    assert!(
        run_pipeline(&format!("fn main() {{ const x = {opens}1{closes}; }}")),
        "{depth} levels of grouping must parse"
    );
}

// -- a deterministic fuzzer ----------------------------------------------------

/// A tiny xorshift64\* generator.
///
/// Seeded and dependency-free on purpose: a failure has to be reproducible from
/// the seed alone, and a fuzzer that needs a crate the offline build cannot
/// fetch is a fuzzer that does not run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..n`. `0` for an empty range rather than a division by zero.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// The bytes random inputs are built from: the punctuation the grammar is made
/// of, a few letters, and the whitespace and control characters that break
/// hand-written scanners.
const ALPHABET: &[u8] = b"fn mainstruct enum import extern switch case default \
const var return if else for defer break continue true false Ok Err Some None \
print_str print_i64 print_f64 Option Result i64 f64 bool string \
{}()[]<>:;,.=+-*/%!&|?@#$_'\"\\ \t\n\r\0\x7f\x1b";

/// Programs the random mutator starts from, so mutations land inside the
/// grammar rather than beside it.
const TEMPLATES: &[&str] = &[
    "fn main() { print_str(\"hi\"); }\n",
    "fn f(n: i64) -> i64 { if n < 2 { return n; } return f(n - 1) + f(n - 2); }\n",
    "fn f(o: Option[i64]) -> i64 { return switch o { case None { 0 } case Some(v) { v } }; }\n",
    "fn f(r: Result[i64, string]) -> i64 { const v = r?; return v; }\n",
    "enum E { A, B(i64) } fn f(e: E) -> i64 { return switch e { case A { 0 } case B(v) { v } }; }\n",
    "struct S { x: i64, y: f64 } fn f() { const s = S { x: 1, y: 2.0 }; }\n",
    "fn f() { defer { print_i64(1); } for i in 0..3 { print_i64(i); } }\n",
    "fn f(x: bool) { if x { break; } }\n",
    "fn f() -> Result[i64, string] { return Err(\"no\"); }\n",
    "import std.io;\nextern \"C\" { fn puts(s: string) -> i64; }\n",
];

#[test]
fn pseudo_random_input_does_not_panic() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    // A fixed seed: the sequence is part of the test, so a failure reproduces.
    let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
    let mut compiled = 0usize;
    let mut rejected = 0usize;
    let mut first_panic: Option<String> = None;

    for round in 0..20_000u64 {
        let input = random_input(&mut rng, round);
        match std::panic::catch_unwind(|| run_pipeline(&input)) {
            Ok(true) => compiled += 1,
            Ok(false) => rejected += 1,
            Err(_) => {
                // Keep the exact bytes that broke it: a fuzzer finding is only
                // useful if you can paste it into a test.
                first_panic = Some(input);
                break;
            }
        }
    }

    std::panic::set_hook(default_hook);

    assert!(
        first_panic.is_none(),
        "a random input made the compiler panic:\n{:?}",
        first_panic.unwrap_or_default()
    );
    assert!(
        compiled > 0 && rejected > 0,
        "the generator must produce both outcomes, got {compiled} compiled and {rejected} rejected"
    );
}

/// One random input. Alternating between raw noise and a mutation of a real
/// program: raw noise mostly exercises the lexer, and a mutation of something
/// that nearly parses is what reaches the checker.
fn random_input(rng: &mut Rng, round: u64) -> String {
    if round % 3 == 0 {
        let len = rng.below(120);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
        return String::from_utf8_lossy(&bytes).into_owned();
    }

    let template = TEMPLATES[rng.below(TEMPLATES.len())];
    let mut text: Vec<u8> = template.as_bytes().to_vec();

    // Truncate, so the input is a prefix of something valid.
    if round % 3 == 1 {
        let cut = rng.below(text.len().saturating_add(1));
        text.truncate(cut);
    }

    // Splice in one to four random bytes, and possibly delete a run.
    let splices = 1 + rng.below(4);
    for _ in 0..splices {
        if text.is_empty() {
            break;
        }
        let at = rng.below(text.len());
        match rng.below(3) {
            0 => text.insert(at, ALPHABET[rng.below(ALPHABET.len())]),
            1 => {
                let end = (at + 1 + rng.below(8)).min(text.len());
                text.drain(at..end);
            }
            _ => {
                let byte = ALPHABET[rng.below(ALPHABET.len())];
                if let Some(slot) = text.get_mut(at) {
                    *slot = byte;
                }
            }
        }
    }

    String::from_utf8_lossy(&text).into_owned()
}
