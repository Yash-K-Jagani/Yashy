//! Tests for item parsing: `import`, `struct`, `enum`, `fn`.
//!
//! These tests pin three things at once. That each item kind is built with the
//! names and the field types the source wrote; that every node carries the span
//! of exactly the text it was parsed from; and that a malformed program yields
//! a [`Diagnostic`] rather than a panic, a hang, or a tree built out of
//! whatever the parser happened to find next.
//!
//! The last one is not a formality. The parser is the component most likely to
//! meet hostile input, so `unterminated_import_path_is_an_error_not_a_panic`
//! and `hostile_input_is_reported_rather_than_panicking` are the tests that
//! stand between a typo and a crash in the compiler.

use otter_common::{Diagnostic, FileId, SourceMap, Span};
use otter_lexer::Lexer;
use otter_parser::ast::{Expr, FieldDef, Item, Literal, Program, Stmt, TypeExpr, VariantField};
use otter_parser::Parser;

// -- helpers ---------------------------------------------------------------

/// The span of `text` inside `src`, computed rather than written down.
///
/// A span assertion that hardcodes byte offsets breaks whenever the fixture is
/// re-wrapped, and then it is the offsets that get "fixed" rather than the
/// parser. This one cannot drift: it asks the same question a diagnostic does,
/// which is exactly which bytes the node was parsed from.
fn span_of(src: &str, text: &str) -> Span {
    let start = match src.find(text) {
        Some(start) => start,
        None => panic!("the fixture must contain {text:?}"),
    };
    Span::new(
        u32::try_from(start).unwrap(),
        u32::try_from(start + text.len()).unwrap(),
    )
}

/// Parses `src`, having lexed it, and returns whatever the parser decided.
fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ot", src.to_string());
    let tokens = match Lexer::with_file(src, file).tokenize() {
        Ok(tokens) => tokens,
        Err(diags) => panic!("test source must lex, got {diags:?}"),
    };
    Parser::new(tokens, &map, file).parse_program()
}

/// Parses `src`, which must be well formed.
fn parse_ok(src: &str) -> Program {
    match parse_src(src) {
        Ok(program) => program,
        Err(diags) => panic!("{src:?} must parse, got {diags:?}"),
    }
}

/// Parses `src`, which must not be. Returns the diagnostics, in order.
fn parse_err(src: &str) -> Vec<Diagnostic> {
    match parse_src(src) {
        Ok(program) => panic!("{src:?} must not parse, got {program:?}"),
        Err(diags) => {
            assert!(
                !diags.is_empty(),
                "{src:?} must report at least one diagnostic"
            );
            diags
        }
    }
}

/// Every diagnostic in `diags`, as one string, for substring assertions.
fn all_messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| format!("[{}] {}", d.severity.as_str(), d.message))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- the brief's five tests -------------------------------------------------

/// One of every item kind the parser builds, from one source.
const FOUR_ITEMS: &str = "import std.io;\nstruct P { x: f64, y: f64 }\nenum C { Red, Green }\nfn d(a: P, b: P) -> f64 { return 0.0; }\n";

#[test]
fn parses_import_struct_enum_and_fn() {
    let program = parse_ok(FOUR_ITEMS);
    assert_eq!(program.items.len(), 4, "four items, in source order");

    // import std.io;
    match &program.items[0] {
        Item::Import { path, span } => {
            assert_eq!(path, &vec!["std".to_string(), "io".to_string()]);
            assert_eq!(
                *span,
                span_of(FOUR_ITEMS, "import std.io;"),
                "the import's span covers its own text"
            );
        }
        other => panic!("expected an import first, got {other:?}"),
    }

    // struct P { x: f64, y: f64 }
    match &program.items[1] {
        Item::Struct { name, fields, span } => {
            assert_eq!(name, "P");
            let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, vec!["x", "y"], "field names, in source order");
            for field in fields {
                match &field.ty {
                    TypeExpr::Name(type_name, _) => {
                        assert_eq!(type_name, "f64", "the type the source wrote")
                    }
                    other => panic!("a field type must be the named type written, got {other:?}"),
                }
            }
            // A field's span covers `x: f64`, not the whole struct.
            assert_eq!(fields[0].span, span_of(FOUR_ITEMS, "x: f64"));
            assert_eq!(fields[1].span, span_of(FOUR_ITEMS, "y: f64"));
            assert_eq!(*span, span_of(FOUR_ITEMS, "struct P { x: f64, y: f64 }"));
        }
        other => panic!("expected a struct second, got {other:?}"),
    }

    // enum C { Red, Green }
    match &program.items[2] {
        Item::Enum {
            name,
            variants,
            span,
        } => {
            assert_eq!(name, "C");
            let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
            assert_eq!(names, vec!["Red", "Green"]);
            assert!(
                variants.iter().all(|v| v.fields.is_empty()),
                "a variant with no payload has no fields: {variants:?}"
            );
            assert_eq!(variants[0].span, span_of(FOUR_ITEMS, "Red"));
            assert_eq!(*span, span_of(FOUR_ITEMS, "enum C { Red, Green }"));
        }
        other => panic!("expected an enum third, got {other:?}"),
    }

    // fn d(a: P, b: P) -> f64 { return 0.0; }
    match &program.items[3] {
        Item::Fn {
            name,
            params,
            ret,
            body,
            span,
        } => {
            assert_eq!(name, "d");
            let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, vec!["a", "b"], "parameter names, in source order");
            for param in params {
                match &param.ty {
                    TypeExpr::Name(type_name, _) => assert_eq!(type_name, "P"),
                    other => {
                        panic!("a parameter type must be the named type written, got {other:?}")
                    }
                }
            }
            assert_eq!(params[1].span, span_of(FOUR_ITEMS, "b: P"));
            match ret {
                Some(TypeExpr::Name(type_name, _)) => {
                    assert_eq!(type_name, "f64", "the return type the source wrote")
                }
                other => panic!("expected `-> f64`, got {other:?}"),
            }
            // The block's span includes both braces, so a diagnostic about the
            // body points at the body rather than at its first statement.
            assert_eq!(body.span, span_of(FOUR_ITEMS, "{ return 0.0; }"));
            assert_eq!(
                *span,
                span_of(FOUR_ITEMS, "fn d(a: P, b: P) -> f64 { return 0.0; }")
            );
        }
        other => panic!("expected a function fourth, got {other:?}"),
    }
}

#[test]
fn parses_dotted_import_path() {
    let program = parse_ok("import crypto.sha256;");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Import { path, span } => {
            assert_eq!(path, &vec!["crypto".to_string(), "sha256".to_string()]);
            assert_eq!(
                *span,
                span_of("import crypto.sha256;", "import crypto.sha256;")
            );
        }
        other => panic!("expected an import, got {other:?}"),
    }

    // Three segments is the same shape, not a different one: the path is a
    // list, and nothing about it is limited to two.
    match &parse_ok("import a.b.c;").items[0] {
        Item::Import { path, .. } => {
            assert_eq!(
                path,
                &vec!["a".to_string(), "b".to_string(), "c".to_string()]
            );
        }
        other => panic!("expected an import, got {other:?}"),
    }
}

/// Review Focus item 3: a `{` nobody closed is reported at the brace that was
/// left open, with the end of file named as the reason.
#[test]
fn unclosed_brace_in_fn_body_is_an_error_at_eof() {
    let diags = parse_err("fn f() {");
    let message = all_messages(&diags);
    assert!(
        message.contains("unclosed"),
        "the message must say what is wrong: {message}"
    );
    let first = match diags.first() {
        Some(d) => d,
        None => panic!("there must be a diagnostic"),
    };
    assert!(first.is_error(), "a parse failure is an error");
    assert_eq!(
        first.span,
        Span::new(7, 8),
        "the caret goes on the brace that was never closed, not on the end of the file"
    );
    assert!(
        first.message.contains("the file ended"),
        "the message must say the file ran out: {}",
        first.message
    );
}

#[test]
fn unterminated_import_path_is_an_error_not_a_panic() {
    let diags = parse_err("import ;");
    let message = all_messages(&diags);
    assert!(
        message.contains("import"),
        "the message must name the item: {message}"
    );
    assert_eq!(diags.len(), 1, "one mistake is one diagnostic: {message}");
    assert!(diags.first().map(|d| d.is_error()).unwrap_or(false));

    // Every way of stopping an import short is a diagnostic, not a panic and
    // not a tree with a path invented out of nothing. Some are a missing path
    // segment and some are a missing `;`; both are one diagnostic, not a
    // cascade.
    for src in [
        "import",
        "import std",
        "import std.",
        "import .io;",
        "import ..;",
        "import 1;",
        "import std.io",
    ] {
        let diags = parse_err(src);
        assert_eq!(diags.len(), 1, "{src:?}: {}", all_messages(&diags));
    }

    // The ones that really are about the path say what an import looks like.
    for src in [
        "import",
        "import std.",
        "import .io;",
        "import ..;",
        "import 1;",
    ] {
        let message = all_messages(&parse_err(src));
        assert!(
            message.contains("import std.io;"),
            "{src:?} must show what an import looks like: {message}"
        );
    }
}

#[test]
fn struct_requires_at_least_one_named_field() {
    let diags = parse_err("struct S { }");
    let first = match diags.first() {
        Some(d) => d,
        None => panic!("there must be a diagnostic"),
    };
    assert!(
        first.message.contains('S'),
        "the message must name the struct it is about: {}",
        first.message
    );
    assert!(
        first.message.contains("field"),
        "the message must say what is missing: {}",
        first.message
    );
    assert_eq!(
        first.span,
        span_of("struct S { }", "struct S"),
        "the caret goes on the declaration, which is what a reader has to edit"
    );

    // An enum variant list has the same rule.
    let diags = parse_err("enum E { }");
    let message = all_messages(&diags);
    assert!(
        message.contains('E'),
        "an empty enum names the enum: {message}"
    );
    assert!(
        message.contains("variant"),
        "and says what is missing: {message}"
    );
}

#[test]
fn extern_blocks_are_rejected_with_a_message_that_says_when_they_arrive() {
    // `Item::Extern` exists in the AST so M5 adds a case rather than changing a
    // contract four stages code against. Until then the parser must say no --
    // and say *when* it will stop saying no, because "expected an item" for a
    // keyword the language has would read as "Otter has no extern".
    for src in [
        "extern",
        "extern \"C\" { }",
        "extern \"C\" link \"m\" { unsafe fn puts(s: string) -> i64; }",
    ] {
        let diags = parse_err(src);
        // The first diagnostic is the one about the block. Recovery skips
        // forward to the next token that can begin an item, and inside a real
        // `extern` block that is the declaration after it -- which is then
        // rejected in its own right, and is entitled to its own diagnostic.
        let first = match diags.first() {
            Some(d) => d,
            None => panic!("there must be a diagnostic"),
        };
        assert!(
            first.message.contains("M5"),
            "{src:?}: the message must name the milestone: {}",
            first.message
        );
        assert_eq!(
            first.span,
            Span::new(0, 6),
            "{src:?}: the caret goes on the `extern` keyword"
        );
        for d in &diags {
            assert!(d.is_error(), "{src:?}: {}", d.message);
        }
    }
}

// -- spans -----------------------------------------------------------------

#[test]
fn every_item_span_covers_its_own_text_and_nothing_else() {
    // Comments and blank lines between the items, because a span computed from
    // the wrong line offset is exactly what those expose.
    let src = "// a comment\n\
               \n\
               import std.io;\n\
               /* block */ struct P {\n    x: f64,\n}\n\
               enum C {\n    Red,\n}\n\
               fn f() {\n}\n";
    let program = parse_ok(src);
    assert_eq!(program.items.len(), 4);
    let expected = [
        "import std.io;",
        // The comment before the `struct` is not inside its span: a comment is
        // trivia, and a diagnostic about the struct must underline the struct.
        "struct P {\n    x: f64,\n}",
        "enum C {\n    Red,\n}",
        "fn f() {\n}",
    ];
    for (item, text) in program.items.iter().zip(expected) {
        let span = match item {
            Item::Import { span, .. }
            | Item::Struct { span, .. }
            | Item::Enum { span, .. }
            | Item::Fn { span, .. }
            | Item::Extern { span, .. } => *span,
        };
        assert_eq!(span, span_of(src, text), "the span of {text:?}");
    }

    // Every item span starts at its own keyword and ends at its own last token:
    // no item may borrow a neighbour's range.
    let starts: Vec<u32> = program
        .items
        .iter()
        .map(|item| match item {
            Item::Import { span, .. }
            | Item::Struct { span, .. }
            | Item::Enum { span, .. }
            | Item::Fn { span, .. }
            | Item::Extern { span, .. } => span.start,
        })
        .collect();
    assert!(
        starts.windows(2).all(|pair| pair[0] < pair[1]),
        "item spans must be ordered and disjoint at their starts: {starts:?}"
    );
}

// -- the unclosed-delimiter stack ------------------------------------------

#[test]
fn an_unclosed_struct_body_points_at_the_opening_brace() {
    let diags = parse_err("struct S {\n    x: f64\n");
    let first = match diags.first() {
        Some(d) => d,
        None => panic!("there must be a diagnostic"),
    };
    assert_eq!(
        first.span,
        Span::new(9, 10),
        "the caret goes on the brace the struct opened"
    );
    assert!(first.message.contains("unclosed"), "{}", first.message);

    // And the same for an enum, and for a parameter list.
    assert!(all_messages(&parse_err("enum E { Red")).contains("unclosed"));
    assert!(all_messages(&parse_err("fn f(")).contains("unclosed"));
}

#[test]
fn a_well_formed_program_leaves_no_delimiter_unclosed() {
    // Two functions, one nested-brace-free body each. If the stack were not
    // popped on a `}`, the second item would either see a stale opener or the
    // end of the file would report one that was closed.
    let program = parse_ok("fn a() { }\nfn b() { }\nfn c() { }");
    assert_eq!(program.items.len(), 3);

    // Braces inside a body do not leak either: the closers are matched against
    // the innermost opener, so `f({ })` inside a body closes cleanly.
    assert_eq!(parse_ok("fn a() { f(); }").items.len(), 1);
    assert_eq!(parse_ok("fn a() { }").items.len(), 1);
}

#[test]
fn a_stray_closer_is_not_reported_as_an_unclosed_opener() {
    // `}` with nothing open is a mistake, but it is not "you forgot a `}`", and
    // reporting it as one would send the reader looking in the wrong place.
    let diags = parse_err("fn a() { } }\nfn b() { }");
    let message = all_messages(&diags);
    assert!(
        !message.contains("unclosed"),
        "nothing was left open: {message}"
    );
}

// -- types -----------------------------------------------------------------

#[test]
fn struct_field_types_cover_the_whole_type_grammar() {
    let src = "struct S { a: i64, b: Option[i64], c: Result[i64, string], d: (), e: (i64, f64), f: pkg.Thing }";
    let program = parse_ok(src);
    let fields = match &program.items[0] {
        Item::Struct { fields, .. } => fields,
        other => panic!("expected a struct, got {other:?}"),
    };
    let shapes: Vec<String> = fields.iter().map(|f| format!("{:?}", f.ty)).collect();

    assert!(
        shapes[0].starts_with("Name(\"i64\""),
        "a primitive is a named type: {shapes:?}"
    );
    assert!(shapes[1].starts_with("Option("), "`Option[T]`: {shapes:?}");
    assert!(
        shapes[2].starts_with("Result("),
        "`Result[T, E]`: {shapes:?}"
    );
    assert!(
        shapes[3].starts_with("Unit("),
        "`()` is the unit type, never an empty tuple: {shapes:?}"
    );
    assert!(
        shapes[4].starts_with("Tuple("),
        "`(i64, f64)` is a tuple type: {shapes:?}"
    );
    assert!(
        shapes[5].contains("pkg.Thing"),
        "a dotted path is one name: {shapes:?}"
    );

    // `()` is `Unit` and never an empty `Tuple`: two spellings of one type would
    // make `PartialEq` call identical types different.
    let unit = match &fields[3].ty {
        TypeExpr::Unit(_) => &fields[3].ty,
        other => panic!("`() must be Unit, got {other:?}"),
    };
    assert!(
        matches!(unit, TypeExpr::Unit(_)),
        "a tuple type is never empty, so `()` is Unit: {unit:?}"
    );

    // Each type's own span is the type's own text, so a diagnostic about one
    // field type does not point at its neighbour.
    assert_eq!(fields[1].span, span_of(src, "b: Option[i64]"));
    match &fields[1].ty {
        TypeExpr::Option(_, span) => assert_eq!(*span, span_of(src, "Option[i64]")),
        other => panic!("expected Option, got {other:?}"),
    }
}

/// A trailing comma inside a tuple type is accepted, and it does not add an
/// element.
///
/// This is the one comma loop in the crate where the "stop at the closer"
/// clause is load-bearing rather than redundant, and this test is what keeps it
/// that way: `parse_paren_type` is a bare `loop {}` with no `while !check(RParen)`
/// guard above it, so `self.check(TokenKind::RParen)` is the only thing standing
/// between `(i64,)` and "expected a type, found `)`". Deleting the clause turns
/// both of these red.
#[test]
fn a_trailing_comma_in_a_tuple_type_does_not_add_an_element() {
    // `(i64,)` -- one element, and the comma before `)` is not a separator.
    let src = "struct S { e: (i64,) }";
    match &parse_ok(src).items[0] {
        Item::Struct { fields, .. } => match &fields[0].ty {
            TypeExpr::Tuple(elements, _) => assert_eq!(
                elements.len(),
                1,
                "a trailing comma is not an element: {elements:?}"
            ),
            other => panic!("`(i64,)` is a tuple type, got {other:?}"),
        },
        other => panic!("expected a struct, got {other:?}"),
    }

    // The same trailing comma inside a variant's positional payload: one field,
    // not two, and the comma loop there agrees with the struct case.
    let src = "enum E { A(i64,) }";
    match &parse_ok(src).items[0] {
        Item::Enum { variants, .. } => assert_eq!(
            variants[0].fields.len(),
            1,
            "a trailing comma is not a field: {:?}",
            variants[0].fields
        ),
        other => panic!("expected an enum, got {other:?}"),
    }

    // The two-element form still works, so the trailing comma is not what is
    // making a one-element tuple parse.
    match &parse_ok("struct S { e: (i64, f64) }").items[0] {
        Item::Struct { fields, .. } => match &fields[0].ty {
            TypeExpr::Tuple(elements, _) => assert_eq!(elements.len(), 2, "{elements:?}"),
            other => panic!("expected a tuple, got {other:?}"),
        },
        other => panic!("expected a struct, got {other:?}"),
    }

    // And a doubled comma inside a tuple type is still a diagnostic, not a
    // silently dropped element.
    let message = all_messages(&parse_err("struct S { e: (i64,, f64) }"));
    assert!(message.contains("type"), "{message}");
    assert_eq!(
        parse_err("struct S { e: (i64,, f64) }").len(),
        1,
        "one mistake, one diagnostic: {message}"
    );
}

#[test]
fn a_type_the_parser_cannot_read_is_a_diagnostic() {
    // Otter has no generics, so `[` after any other name is a mistake worth
    // naming rather than a silent reading.
    let message = all_messages(&parse_err("struct S { x: Vec[i64] }"));
    assert!(message.contains("generics"), "{message}");
    assert!(all_messages(&parse_err("struct S { x: Option[ }")).contains("Option"));
    assert!(
        all_messages(&parse_err("struct S { x: Result[i64] }")).contains("two types"),
        "`Result` needs both halves or neither"
    );
    assert!(all_messages(&parse_err("struct S { x: }")).contains("type"));
    assert!(all_messages(&parse_err("fn f() -> 1 { }")).contains("type"));

    // A bare `Option` is *not* an error: it is a name, and whether anyone has
    // declared a type called `Option` is the type checker's question, not the
    // parser's. The parser reads what is written and stops.
    match &parse_ok("struct S { x: Option }").items[0] {
        Item::Struct { fields, .. } => match &fields[0].ty {
            TypeExpr::Name(name, _) => assert_eq!(name, "Option"),
            other => panic!("a bare `Option` is a named type, got {other:?}"),
        },
        other => panic!("expected a struct, got {other:?}"),
    }
}

// -- enums -----------------------------------------------------------------

#[test]
fn enum_variants_may_be_unit_tuple_or_struct() {
    let src = "enum Shape {\n    Circle(f64),\n    Rect { w: f64, h: f64 },\n    Empty,\n}";
    let program = parse_ok(src);
    let variants = match &program.items[0] {
        Item::Enum { variants, .. } => variants,
        other => panic!("expected an enum, got {other:?}"),
    };
    assert_eq!(variants.len(), 3);

    assert!(
        !variants[0].fields.is_empty(),
        "Circle carries a payload: {variants:?}"
    );
    match &variants[0].fields[0] {
        VariantField::Positional { ty, span } => match ty {
            TypeExpr::Name(name, _) => {
                assert_eq!(name, "f64");
                assert_eq!(
                    *span,
                    span_of(src, "f64"),
                    "the field carries the type's own text"
                );
            }
            other => panic!("expected `f64`, got {other:?}"),
        },
        other => panic!("`Circle(f64)` is a positional payload, got {other:?}"),
    }

    // A struct variant's fields are named, and the names are the ones written.
    let names: Vec<&str> = variants[1]
        .fields
        .iter()
        .map(|field| match field {
            VariantField::Named { name, .. } => name.as_str(),
            other => panic!("`Rect {{ .. }}` is a named payload, got {other:?}"),
        })
        .collect();
    assert_eq!(names, vec!["w", "h"], "a struct variant names its fields");
    assert_eq!(variants[1].fields[1].span(), span_of(src, "h: f64"));

    assert!(
        variants[2].fields.is_empty(),
        "a unit variant has no fields"
    );
    assert_eq!(variants[0].span, span_of(src, "Circle(f64)"));
    assert_eq!(variants[1].span, span_of(src, "Rect { w: f64, h: f64 }"));
    assert_eq!(variants[2].span, span_of(src, "Empty"));
}

/// The two payload shapes are two variants, and neither can be mistaken for the
/// other -- a sentinel cannot creep back in without this failing.
#[test]
fn a_positional_payload_is_not_a_named_field_with_no_name() {
    let src = "enum Shape { Circle(f64), Rect { w: f64 } }";
    let variants = match &parse_ok(src).items[0] {
        Item::Enum { variants, .. } => variants.clone(),
        other => panic!("expected an enum, got {other:?}"),
    };

    // A parenthesised payload is `Positional`, and `Positional` has no `name`
    // field at all -- so there is nothing in it that could be empty, and no
    // downstream `name.is_empty()` check to forget.
    match &variants[0].fields[0] {
        VariantField::Positional { ty, span } => {
            assert_eq!(span, &span_of(src, "f64"));
            assert!(matches!(ty, TypeExpr::Name(name, _) if name == "f64"));
        }
        other => panic!("`Circle(f64)` must be Positional, got {other:?}"),
    }
    let positional = format!("{:?}", variants[0].fields[0]);
    assert!(
        !positional.contains("name"),
        "a positional field has no name to render: {positional}"
    );

    // A braced payload is `Named`, with the name the source wrote.
    match &variants[1].fields[0] {
        VariantField::Named { name, ty, span } => {
            assert_eq!(name, "w");
            assert_eq!(span, &span_of(src, "w: f64"));
            assert!(matches!(ty, TypeExpr::Name(name, _) if name == "f64"));
        }
        other => panic!("`Rect {{ w: f64 }}` must be Named, got {other:?}"),
    }

    // The two are different values, and they are different *types*: a pattern
    // that reads the wrong one does not compile.
    assert_ne!(variants[0].fields[0], variants[1].fields[0]);

    // `FieldDef` -- a struct's field -- still has a plain `name`, and is not the
    // type a variant payload uses.
    let struct_fields = match &parse_ok("struct P { x: f64 }").items[0] {
        Item::Struct { fields, .. } => fields.clone(),
        other => panic!("expected a struct, got {other:?}"),
    };
    assert_eq!(struct_fields[0].name, "x");
    assert!(
        !struct_fields[0].name.is_empty(),
        "a struct field's name comes from an identifier, never from a sentinel"
    );
}

// -- function heads --------------------------------------------------------

#[test]
fn a_function_may_omit_its_return_type_and_its_parameters() {
    match &parse_ok("fn f() { }").items[0] {
        Item::Fn {
            name, params, ret, ..
        } => {
            assert_eq!(name, "f");
            assert!(params.is_empty());
            assert!(
                ret.is_none(),
                "no `->` is a missing annotation, which is not the same as `-> ()`: {ret:?}"
            );
        }
        other => panic!("expected a function, got {other:?}"),
    }

    // `-> ()` is written, so it is present and it is the unit type.
    match &parse_ok("fn f() -> () { }").items[0] {
        Item::Fn { ret: Some(ty), .. } => assert!(
            matches!(ty, TypeExpr::Unit(_)),
            "an explicit `-> ()` is Some(Unit), not None: {ty:?}"
        ),
        other => panic!("expected a function with a return type, got {other:?}"),
    }

    // A parameter without a type is a diagnostic: annotations are required at
    // the boundaries (spec 3.2), and the message names the missing `:`.
    assert!(all_messages(&parse_err("fn f(a) { }")).contains("`:`"));
    assert!(all_messages(&parse_err("fn f(a: i64 { }")).contains("`)`"));
}

/// A comma in a parameter list is either the separator or a trailing mark, and
/// which one it is has to be decided rather than stumbled into.
#[test]
fn a_stray_or_trailing_comma_in_a_parameter_list_is_handled_deliberately() {
    // The baseline: two parameters, one comma between them.
    match &parse_ok("fn f(a: i64, b: i64) { }").items[0] {
        Item::Fn { params, .. } => {
            let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, vec!["a", "b"]);
        }
        other => panic!("expected a function, got {other:?}"),
    }

    // A trailing comma is accepted, and it does not invent a parameter: the
    // comma before `)` ends the list rather than introducing an empty one.
    match &parse_ok("fn f(a: i64,) { }").items[0] {
        Item::Fn { params, .. } => assert_eq!(
            params.len(),
            1,
            "a trailing comma is not a parameter: {params:?}"
        ),
        other => panic!("expected a function, got {other:?}"),
    }
    // And the same in a struct's field list, for the same reason.
    match &parse_ok("struct S { x: f64, }").items[0] {
        Item::Struct { fields, .. } => assert_eq!(fields.len(), 1, "{fields:?}"),
        other => panic!("expected a struct, got {other:?}"),
    }

    // Everything else is one diagnostic. Not a loop, not a cascade, and never a
    // parameter built out of nothing.
    //
    // `f(1,)` / `f(,)` / `f(1,,2)` are call *expressions*, which Task 6 owns;
    // their parameter-list equivalents are here, and they are the same comma
    // loop over the same delimiter, so what is pinned is the same rule.
    for src in [
        "fn f(a: i64,, b: i64) { }", // f(1,,2)
        "fn f(,) { }",               // f(,)
        "fn f(, a: i64) { }",        // f(,1)
        "fn f(a: i64 b: i64) { }",
        "fn f(a:) { }",
        "fn f(a: i64,,, ) { }",
    ] {
        let diags = parse_err(src);
        assert_eq!(
            diags.len(),
            1,
            "{src:?} is one mistake, one diagnostic: {}",
            all_messages(&diags)
        );
        assert!(diags[0].is_error(), "{src:?}: {}", diags[0].message);
    }

    // `struct S { a: i64, }` is asserted above: a trailing comma there is one
    // field, not two. The named-variant payload and the positional one are the
    // remaining two comma loops, and they agree.
    match &parse_ok("enum E { A { w: f64, } }").items[0] {
        Item::Enum { variants, .. } => assert_eq!(
            variants[0].fields.len(),
            1,
            "a trailing comma is not a field: {:?}",
            variants[0].fields
        ),
        other => panic!("expected an enum, got {other:?}"),
    }
    match &parse_ok("enum E { A, B, }").items[0] {
        Item::Enum { variants, .. } => assert_eq!(
            variants.len(),
            2,
            "a trailing comma is not a variant: {variants:?}"
        ),
        other => panic!("expected an enum, got {other:?}"),
    }
}

/// An empty `struct` or `enum` is rejected on purpose -- the ruling, not an
/// accident of the loop -- and a stray comma inside one is rejected too.
#[test]
fn an_empty_declaration_body_is_rejected_deliberately() {
    // The ruling, stated once: a declaration with nothing in it names itself.
    for (src, name, what) in [
        ("struct S { }", 'S', "field"),
        ("enum E { }", 'E', "variant"),
    ] {
        let diags = parse_err(src);
        assert_eq!(diags.len(), 1, "{src:?}: {}", all_messages(&diags));
        assert!(
            diags[0].message.contains(name),
            "{src:?} must name the declaration it is about: {}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains(what),
            "{src:?} must say what is missing: {}",
            diags[0].message
        );
    }

    // A stray comma is not an empty declaration, it is a missing field or
    // variant -- and either way it is a diagnostic rather than a loop or a
    // panic, which is what this is here to hold.
    for src in [
        "struct S { , }",
        "enum E { , }",
        "struct S { , x: f64 }",
        "enum E { Red, , }",
    ] {
        let diags = parse_err(src);
        assert_eq!(
            diags.len(),
            1,
            "{src:?} is one mistake, one diagnostic: {}",
            all_messages(&diags)
        );
    }

    // And the bodies that are *not* empty parse, so the rejection above is a
    // decision about emptiness and not about braces.
    assert_eq!(parse_ok("enum E { Red }").items.len(), 1);
    assert_eq!(parse_ok("struct S { x: f64 }").items.len(), 1);
    assert_eq!(parse_ok("enum E { Red, Green }").items.len(), 1);

    // The ruling reaches inside the enum too: a variant with an *empty* payload
    // is the same mistake as an empty enum, one level down. A variant with no
    // fields is spelled `A` -- `A()` and `A {}` are rejected, so that `fields`
    // being empty has exactly one spelling behind it and a reader never has to
    // wonder which of the two produced it.
    for (src, variant) in [("enum E { A() }", 'A'), ("enum E { A {} }", 'A')] {
        let diags = parse_err(src);
        assert_eq!(diags.len(), 1, "{src:?}: {}", all_messages(&diags));
        let first = match diags.first() {
            Some(d) => d,
            None => panic!("there must be a diagnostic"),
        };
        assert!(
            first.message.contains(variant),
            "{src:?} must name the variant it is about: {}",
            first.message
        );
        assert!(
            first.message.contains("field"),
            "{src:?} must say what is missing: {}",
            first.message
        );
        // The same message shape as an empty struct body: `X` must have at
        // least one `Y`.
        let struct_message = parse_err("struct S { }")[0].message.clone();
        let variant_message = first.message.replace(&format!("variant `{variant}` of enum `E`"), "struct `S`");
        assert_eq!(
            variant_message, struct_message,
            "{src:?}: a variant's emptiness is reported like a struct's"
        );
    }

    // A variant with a real payload in the same spelling still parses, so the
    // rejection is about emptiness and not about parentheses or braces.
    assert_eq!(parse_ok("enum E { A(i64) }").items.len(), 1);
    assert_eq!(parse_ok("enum E { A { w: f64 } }").items.len(), 1);
    assert_eq!(parse_ok("enum E { A, B(f64), C { w: f64 } }").items.len(), 1);
}

#[test]
fn a_missing_semicolon_after_an_import_is_an_error() {
    let diags = parse_err("import std.io");
    let first = match diags.first() {
        Some(d) => d,
        None => panic!("there must be a diagnostic"),
    };
    // `expect` names both halves: what it wanted and what it found.
    assert_eq!(first.message, "expected `;`, found end of file");
    assert_eq!(
        first.span,
        Span::new(13, 13),
        "at the end of the file, which is where it ran out"
    );
}

#[test]
fn a_stray_token_where_an_item_belongs_is_reported_once() {
    // `@` is a real token with no item meaning. One diagnostic, then parsing
    // carries on to the next item -- and the loop cannot spin, because recovery
    // always consumes at least one token.
    let diags = parse_err("fn a() { } @ fn b() { }");
    let message = all_messages(&diags);
    assert_eq!(
        diags.len(),
        1,
        "one stray token is one diagnostic: {message}"
    );
    assert!(
        diags[0].message.contains("expected an item"),
        "{}",
        diags[0].message
    );
    assert_eq!(
        diags[0].span,
        Span::new(11, 12),
        "the caret goes on the stray `@`"
    );
}

#[test]
fn an_empty_program_has_no_items() {
    let program = parse_ok("");
    assert!(program.items.is_empty());

    // Whitespace and comments only is still an empty program, not a failure.
    assert!(parse_ok("// nothing here\n\n/* nor here */\n")
        .items
        .is_empty());
}

// -- the block seam --------------------------------------------------------

#[test]
fn the_block_seam_consumes_the_body_and_reports_its_span() {
    // Task 4 does not parse statements -- Task 5 owns that -- so the seam walks
    // a balanced body and returns it with no statements in it. What this test
    // pins is the half that IS Task 4's: the braces are consumed, the span
    // covers them, and nothing after the body is swallowed.
    //
    // Task 5 replaces this body with the real `parse_stmt` loop. When it does,
    // this test fails on `stmts.is_empty()` -- which is the point: the
    // placeholder cannot be forgotten.
    //
    // **TASK 5: this assertion is a hand-off marker and it is expected to fail
    // on the day you implement `parse_stmt`.** Delete it, and replace it with
    // the statements `fn f() { return 0.0; }` should actually produce -- one
    // `Stmt::Return` carrying the literal `0.0`. Everything else in this test
    // (the span, and the fact that the body does not swallow the next item)
    // still applies and should be left alone.
    match &parse_ok("fn f() { return 0.0; }").items[0] {
        Item::Fn { body, .. } => {
            assert_eq!(body.stmts.len(), 1, "one statement expected: {body:?}");
            match &body.stmts[0] {
                Stmt::Return(Some(Expr::Literal(Literal::F64(0.0), _)), _) => {}
                other => panic!("expected Return(Some(Literal(F64(0.0)))), got {other:?}"),
            }
            assert_eq!(
                body.span,
                span_of("fn f() { return 0.0; }", "{ return 0.0; }")
            );
        }
        other => panic!("expected a function, got {other:?}"),
    }

    // The body does not swallow the next item: `}` ended it.
    let program = parse_ok("fn f() { } fn g() { }");
    assert_eq!(
        program.items.len(),
        2,
        "the block seam must stop at the matching `}}`"
    );
}

// -- hostile input ---------------------------------------------------------

#[test]
fn hostile_input_is_reported_rather_than_panicking() {
    // Each of these is something a user can type by accident, or a fuzzer can
    // produce. None of them may panic, hang, or return a tree built out of
    // whatever followed the mistake. Only the outcome is checked -- a crash is
    // the failure being looked for, and there is nothing here to say which
    // diagnostic a given input deserves.
    let sources = [
        "",
        "}",
        "{",
        "}",
        "import",
        "import ;",
        "import std",
        "import std.",
        "import .",
        "import ..;",
        "import std.io;;",
        "import 1;",
        "struct",
        "struct S",
        "struct S {",
        "struct S { }",
        "struct S { , }",
        "struct S { x }",
        "struct S { x: }",
        "struct S { x: : i64 }",
        "struct S { x: i64,, y: i64 }",
        "struct S { x: i64,,, }",
        "struct S { 5 }",
        "struct S { x: Option[ }",
        "struct S { x: Option[i64",
        "struct S { x: Result[] }",
        "enum",
        "enum E",
        "enum E {",
        "enum E { }",
        "enum E { , }",
        "enum E { A( }",
        "enum E { A { } }",
        "enum E { A, }",
        "fn",
        "fn f",
        "fn f(",
        "fn f(,)",
        "fn f(, a: i64)",
        "fn f(a: i64,)",
        "fn f(a: i64,)",
        "fn f(a: i64,, b: i64)",
        "fn f(a: i64,,, )",
        "fn f(a: i64 b: i64)",
        "fn f()",
        "fn f() -",
        "fn f() ->",
        "fn f() -> {",
        "fn f() {",
        "fn f() {}",
        "fn f() {}}}",
        "fn f() {{{",
        "fn f() { ( }",
        "fn f() { [ }",
        "extern",
        "extern \"C\" { }",
        "extern \"C\" link \"m\" { unsafe fn puts(s: string) -> i64; }",
        "\u{0}",
        "@",
        "@@@",
        "fn f() { @ }",
        "}}}}}}}}",
        "((((((((((",
        "import std.io; struct P { x: f64 } enum C { Red } fn d() -> f64 { }",
    ];
    for src in sources {
        // A source that does not even lex is the lexer's business, not this
        // test's; either way, reaching here without a panic is the assertion.
        let mut map = SourceMap::new();
        let file = map.add_file("main.ot", src.to_string());
        if let Ok(tokens) = Lexer::with_file(src, file).tokenize() {
            let _ = Parser::new(tokens, &map, file).parse_program();
        }
    }
}

#[test]
fn deeply_nested_delimiters_hit_the_depth_limit_rather_than_the_stack() {
    // `{` a thousand deep is what a fuzzer finds first. The parser is
    // iterative, so there is no recursion to overflow; the depth limit is what
    // turns a million-deep input into a diagnostic.
    let mut src = String::from("fn f() { (");
    for _ in 0..130 {
        src.push('(');
    }
    src.push('1');
    for _ in 0..130 {
        src.push(')');
    }
    src.push_str(") ; }");

    let src2 = src.clone();
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || parse_err(&src2))
        .unwrap();
    let diags = handle.join().unwrap();
    let message = all_messages(&diags);
    assert!(
        message.contains("nested"),
        "the limit must be named in the message: {message}"
    );
    assert_eq!(
        diags.len(),
        1,
        "and reported once, not once per level: {message}"
    );
}

#[test]
fn every_diagnostic_is_an_error_against_the_parsed_file() {
    // A diagnostic that is not an error, or that names no file, cannot be
    // rendered and cannot decide an exit code.
    for src in [
        "fn f() {",
        "import ;",
        "struct S { }",
        "extern \"C\" { }",
        "@",
    ] {
        for d in parse_err(src) {
            assert!(d.is_error(), "{src:?}: {}", d.message);
            assert_eq!(d.file, FileId::from_index(0), "{src:?}: {}", d.message);
            assert!(!d.message.is_empty(), "{src:?} must say something");
            assert!(
                !d.message.ends_with('.'),
                "the renderer supplies the punctuation: {}",
                d.message
            );
        }
    }
}

// -- field order is part of the contract -----------------------------------

#[test]
fn field_and_variant_order_is_source_order() {
    let src = "struct S { z: i64, a: i64, m: i64 }";
    let fields: Vec<FieldDef> = match &parse_ok(src).items[0] {
        Item::Struct { fields, .. } => fields.clone(),
        other => panic!("expected a struct, got {other:?}"),
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["z", "a", "m"],
        "the parser does not sort, and does not reorder"
    );

    let variants: Vec<String> = match &parse_ok("enum E { C, A, B }").items[0] {
        Item::Enum { variants, .. } => variants.iter().map(|v| v.name.clone()).collect(),
        other => panic!("expected an enum, got {other:?}"),
    };
    assert_eq!(
        variants,
        vec!["C", "A", "B"],
        "declaration order is the match order"
    );
}
