//! Module visibility: what another module may use, and what it may not.
//!
//! A module's items belong to it. Another module may use a `pub` item and may
//! not use a private one, so these tests are about the *use* -- the place the
//! mistake is visible -- rather than about the declaration, which is fine
//! either way.

use yashy_common::{Diagnostic, FileId, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_modules_with, CheckOptions, HirProgram};

/// Check a program made of several modules.
///
/// The first module is the entry point; the rest are the ones it imports. A
/// module is one file, and the name it is given here is the name the
/// diagnostic uses.
fn check(modules: &[(&str, &str)]) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let mut parsed = Vec::new();
    for (name, text) in modules {
        let file = sources.add_file(*name, text.to_string());
        let tokens = Lexer::new(text)
            .tokenize()
            .expect("the lexer must not panic");
        let mut parser = Parser::new(tokens, &sources, file);
        let program = parser
            .parse_program()
            .expect("the parser must accept the program");
        parsed.push((file, program));
    }
    let modules: Vec<(FileId, &yashy_parser::ast::Program)> = parsed
        .iter()
        .map(|(file, program)| (*file, program))
        .collect();
    check_modules_with(&modules, &sources, CheckOptions::default())
}

/// Assert that some module is rejected with a diagnostic naming `needle`.
fn assert_rejected(modules: &[(&str, &str)], needle: &str) {
    match check(modules) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected an error containing `{needle}`, got {diagnostics:?}"
        ),
    }
}

/// Assert the rejection is `E0801`, the private-item code.
fn assert_e0801(modules: &[(&str, &str)]) {
    match check(modules) {
        Ok(_) => panic!("expected E0801"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.code.as_deref() == Some("E0801")),
            "expected E0801, got {diagnostics:?}"
        ),
    }
}

#[test]
fn a_public_function_is_usable_from_another_module() {
    check(&[
        ("main.ya", "import lib;\nfn main() { print(seven()); }\n"),
        ("lib.ya", "pub fn seven() -> int { return 7; }\n"),
    ])
    .expect("a `pub fn` is another module's to use");
}

#[test]
fn a_private_function_is_not_usable_from_another_module() {
    assert_rejected(
        &[
            ("main.ya", "import lib;\nfn main() { print(seven()); }\n"),
            ("lib.ya", "fn seven() -> int { return 7; }\n"),
        ],
        "`seven` is private to module `lib`",
    );
    assert_e0801(&[
        ("main.ya", "import lib;\nfn main() { print(seven()); }\n"),
        ("lib.ya", "fn seven() -> int { return 7; }\n"),
    ]);
}

#[test]
fn a_private_field_is_e0801_too() {
    assert_e0801(&[
        (
            "main.ya",
            "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.x); }\n",
        ),
        ("lib.ya", "pub struct Point { x: int, pub y: int }\n"),
    ]);
}

#[test]
fn a_module_may_use_its_own_private_items() {
    check(&[
        ("main.ya", "import lib;\nfn main() { print(seven()); }\n"),
        (
            "lib.ya",
            "pub fn seven() -> int { return helper(); }\nfn helper() -> int { return 7; }\n",
        ),
    ])
    .expect("a module's own items are its own");
}

#[test]
fn a_private_type_is_not_usable_from_another_module() {
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; }\n",
            ),
            ("lib.ya", "struct Point { x: int, y: int }\n"),
        ],
        "`Point` is private to module `lib`",
    );
}

#[test]
fn a_public_type_with_a_private_field_is_half_visible() {
    // The type is another module's to name, and the field is not: the field
    // needs its own `pub`.
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; }\n",
            ),
            ("lib.ya", "pub struct Point { x: int, y: int }\n"),
        ],
        "field `x` of `Point` is private to module `lib`",
    );
}

#[test]
fn a_public_type_with_a_public_field_is_fully_visible() {
    check(&[
        (
            "main.ya",
            "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.x); }\n",
        ),
        ("lib.ya", "pub struct Point { pub x: int, pub y: int }\n"),
    ])
    .expect("a `pub` field is another module's to read and write");
}

#[test]
fn a_private_field_may_not_be_read_from_another_module() {
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.x); }\n",
            ),
            ("lib.ya", "pub struct Point { x: int, pub y: int }\n"),
        ],
        "field `x` of `Point` is private to module `lib`",
    );
}

#[test]
fn a_const_fn_is_private_too() {
    // A `const fn` has no run-time existence, but it belongs to its module the
    // same way any other item does.
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { const n = square(9); print(n); }\n",
            ),
            (
                "lib.ya",
                "const fn square(int n) -> int { return n * n; }\n",
            ),
        ],
        "`square` is private to module `lib`",
    );
}

#[test]
fn a_private_generic_function_is_not_usable_from_another_module() {
    // A generic function is not a value and has no root-scope binding, so the
    // check is not the ordinary name lookup's: it is the call path's.
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { print(identity(3)); }\n",
            ),
            ("lib.ya", "fn identity[T](T x) -> T { return x; }\n"),
        ],
        "`identity` is private to module `lib`",
    );
}

#[test]
fn a_private_field_may_not_be_written_from_another_module() {
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; p.x = 3; }\n",
            ),
            ("lib.ya", "pub struct Point { x: int, pub y: int }\n"),
        ],
        "field `x` of `Point` is private to module `lib`",
    );
}

#[test]
fn a_public_type_is_usable_from_another_module() {
    check(&[
        (
            "main.ya",
            "import lib;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.y); }\n",
        ),
        ("lib.ya", "pub struct Point { pub x: int, pub y: int }\n"),
    ])
    .expect("a `pub` type is another module's to name");
}

#[test]
fn a_public_method_is_usable_from_another_module() {
    check(&[
        (
            "main.ya",
            "import lib;\nfn main() { let c = Counter { value: 1 }; print(c.get()); }\n",
        ),
        (
            "lib.ya",
            "pub object Counter { pub value: int, fn secret() -> int { return 1; }, \
             pub fn get(self) -> int { return self.value; }, }\n",
        ),
    ])
    .expect("a `pub fn` is another module's to call");
}

#[test]
fn a_private_method_is_e0801_across_modules() {
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { let c = Counter { value: 1 }; print(c.secret()); }\n",
            ),
            (
                "lib.ya",
                "pub object Counter { pub value: int, fn secret() -> int { return 1; } }\n",
            ),
        ],
        "method `secret`",
    );
}

/// The label shape every private-method error has, so a sibling readable
/// cross-module method and an unreadable one stay distinguishable.
#[test]
fn a_root_private_method_in_the_same_module_is_fine() {
    check(&[(
        "main.ya",
        "object Counter { value: int, fn secret(self) -> int { return 1; }, }\n\
         fn main() { let c = Counter { value: 1 }; print(c.secret()); }\n",
    )])
    .expect("a module may call its own private methods");
}

#[test]
fn a_sibling_type_in_the_same_module_may_use_private_members() {
    check(&[(
        "main.ya",
        "struct Point { secret: int }\n\
         struct Probe { p: Point }\n\
         impl Point { fn get(self) -> int { return self.secret; } }\n\
         impl Probe { fn use_it(self) -> int { let q = self.p; return q.get(); } }\n\
         fn main() { let pr = Probe { p: Point { secret: 3 } }; print(pr.use_it()); }\n",
    )])
    .expect("visibility is module-scoped, not type-scoped");
}

#[test]
fn a_private_associated_function_is_e0801_across_modules() {
    assert_rejected(
        &[
            (
                "main.ya",
                "import lib;\nfn main() { let c = Counter.new(3); print(c.value); }\n",
            ),
            (
                "lib.ya",
                "pub object Counter { value: int, \
                 fn new(int start) -> Counter { return Counter { value: start }; } }\n",
            ),
        ],
        "method `new`",
    );
}
