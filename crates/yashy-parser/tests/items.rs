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

use yashy_common::{Diagnostic, FileId, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::ast::{
    BinOp, Expr, FieldDef, InterpFormat, InterpPart, Item, Literal, Pattern, Program, Stmt,
    TypeExpr, UnOp, VariantField,
};
use yashy_parser::{Parser, MAX_NESTING_DEPTH};

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

/// The span of an item, looking through a `pub` wrapper.
///
/// A `pub` is a modifier in front of an item, so the item's span is still the
/// item's own text: the modifier has its own span, and a diagnostic about the
/// item must underline the item.
fn item_span(item: &Item) -> Span {
    match item {
        Item::Public { item, .. } => item_span(item),
        Item::Effect { item, .. } => item_span(item),
        Item::Import { span, .. }
        | Item::Struct { span, .. }
        | Item::Enum { span, .. }
        | Item::Fn { span, .. }
        | Item::Extern { span, .. }
        | Item::Impl { span, .. }
        | Item::Object { span, .. }
        | Item::ConstFn { span, .. } => *span,
    }
}

/// Parses `src`, having lexed it, and returns whatever the parser decided.
fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
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
const FOUR_ITEMS: &str = "import std.io;\nstruct P { x: float, y: float }\nenum C { Red, Green }\nfn d(a: P, b: P) -> float { return 0.0; }\n";

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

    // struct P { x: float, y: float }
    match &program.items[1] {
        Item::Struct { name, fields, span } => {
            assert_eq!(name, "P");
            let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, vec!["x", "y"], "field names, in source order");
            for field in fields {
                match &field.ty {
                    TypeExpr::Name(type_name, _) => {
                        assert_eq!(type_name, "float", "the type the source wrote")
                    }
                    other => panic!("a field type must be the named type written, got {other:?}"),
                }
            }
            // A field's span covers `x: float`, not the whole struct.
            assert_eq!(fields[0].span, span_of(FOUR_ITEMS, "x: float"));
            assert_eq!(fields[1].span, span_of(FOUR_ITEMS, "y: float"));
            assert_eq!(*span, span_of(FOUR_ITEMS, "struct P { x: float, y: float }"));
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

    // fn d(a: P, b: P) -> float { return 0.0; }
    match &program.items[3] {
        Item::Fn {
            name,
            type_params: _,
            params,
            ret,
            requires: _,
            ensures: _,
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
                    assert_eq!(type_name, "float", "the return type the source wrote")
                }
                other => panic!("expected `-> float`, got {other:?}"),
            }
            // The block's span includes both braces, so a diagnostic about the
            // body points at the body rather than at its first statement.
            assert_eq!(body.span, span_of(FOUR_ITEMS, "{ return 0.0; }"));
            assert_eq!(
                *span,
                span_of(FOUR_ITEMS, "fn d(a: P, b: P) -> float { return 0.0; }")
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

/// A module path may use a word that is also a keyword.
///
/// `io` is a function modifier and `std.io` is the standard input/output
/// module; the same spelling has to be able to name both. The segment is
/// recorded as its own text, so the path is `["std", "io"]` and not something
/// the modifier rule rewrote.
#[test]
fn an_import_path_segment_may_be_a_keyword() {
    for (source, expected) in [
        ("import std.io;", vec!["std", "io"]),
        ("import pure.io.defer;", vec!["pure", "io", "defer"]),
    ] {
        match &parse_ok(source).items[0] {
            Item::Import { path, .. } => {
                assert_eq!(path, &expected.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            }
            other => panic!("expected an import, got {other:?}"),
        }
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
fn extern_blocks_are_parsed_in_both_their_spellings() {
    // A block with an ABI and a library, and the single-declaration form. The
    // AST has one shape for both: a list of declarations.
    for src in [
        r#"extern "C" { unsafe fn puts(s: string) -> int; }"#,
        r#"extern "C" link "m" { unsafe fn puts(s: string) -> int; }"#,
        r#"extern "C" unsafe fn puts(s: string) -> int;"#,
    ] {
        let program = parse_ok(src);
        match program.items.first() {
            Some(Item::Extern { decls, .. }) => {
                assert_eq!(decls.len(), 1, "{src:?}: one declaration");
            }
            other => panic!("{src:?}: expected an extern item, got {other:?}"),
        }
    }
}

#[test]
fn a_bare_extern_keyword_is_an_error_not_a_panic() {
    let diags = parse_err("extern");
    let first = diags.first().expect("there must be a diagnostic");
    assert!(
        first.is_error(),
        "the diagnostic must be an error: {}",
        first.message
    );
    assert!(
        first.message.contains("unsafe"),
        "the header is missing its declaration, which must be `unsafe`: {}",
        first.message
    );
}

// -- spans -----------------------------------------------------------------

#[test]
fn every_item_span_covers_its_own_text_and_nothing_else() {
    // Comments and blank lines between the items, because a span computed from
    // the wrong line offset is exactly what those expose.
    let src = "// a comment\n\
               \n\
               import std.io;\n\
               /* block */ struct P {\n    x: float,\n}\n\
               enum C {\n    Red,\n}\n\
               fn f() {\n}\n";
    let program = parse_ok(src);
    assert_eq!(program.items.len(), 4);
    let expected = [
        "import std.io;",
        // The comment before the `struct` is not inside its span: a comment is
        // trivia, and a diagnostic about the struct must underline the struct.
        "struct P {\n    x: float,\n}",
        "enum C {\n    Red,\n}",
        "fn f() {\n}",
    ];
    for (item, text) in program.items.iter().zip(expected) {
        let span = item_span(item);
        assert_eq!(span, span_of(src, text), "the span of {text:?}");
    }

    // Every item span starts at its own keyword and ends at its own last token:
    // no item may borrow a neighbour's range.
    let starts: Vec<u32> = program
        .items
        .iter()
        .map(|item| item_span(item).start)
        .collect();
    assert!(
        starts.windows(2).all(|pair| pair[0] < pair[1]),
        "item spans must be ordered and disjoint at their starts: {starts:?}"
    );
}

// -- the unclosed-delimiter stack ------------------------------------------

#[test]
fn an_unclosed_struct_body_points_at_the_opening_brace() {
    let diags = parse_err("struct S {\n    x: float\n");
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
    let src = "struct S { a: int, b: Option[int], c: Result[int, string], d: (), e: (int, float), f: pkg.Thing }";
    let program = parse_ok(src);
    let fields = match &program.items[0] {
        Item::Struct { fields, .. } => fields,
        other => panic!("expected a struct, got {other:?}"),
    };
    let shapes: Vec<String> = fields.iter().map(|f| format!("{:?}", f.ty)).collect();

    assert!(
        shapes[0].starts_with("Name(\"int\""),
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
        "`(int, float)` is a tuple type: {shapes:?}"
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
    assert_eq!(fields[1].span, span_of(src, "b: Option[int]"));
    match &fields[1].ty {
        TypeExpr::Option(_, span) => assert_eq!(*span, span_of(src, "Option[int]")),
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
/// between `(int,)` and "expected a type, found `)`". Deleting the clause turns
/// both of these red.
#[test]
fn a_trailing_comma_in_a_tuple_type_does_not_add_an_element() {
    // `(int,)` -- one element, and the comma before `)` is not a separator.
    let src = "struct S { e: (int,) }";
    match &parse_ok(src).items[0] {
        Item::Struct { fields, .. } => match &fields[0].ty {
            TypeExpr::Tuple(elements, _) => assert_eq!(
                elements.len(),
                1,
                "a trailing comma is not an element: {elements:?}"
            ),
            other => panic!("`(int,)` is a tuple type, got {other:?}"),
        },
        other => panic!("expected a struct, got {other:?}"),
    }

    // The same trailing comma inside a variant's positional payload: one field,
    // not two, and the comma loop there agrees with the struct case.
    let src = "enum E { A(int,) }";
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
    match &parse_ok("struct S { e: (int, float) }").items[0] {
        Item::Struct { fields, .. } => match &fields[0].ty {
            TypeExpr::Tuple(elements, _) => assert_eq!(elements.len(), 2, "{elements:?}"),
            other => panic!("expected a tuple, got {other:?}"),
        },
        other => panic!("expected a struct, got {other:?}"),
    }

    // And a doubled comma inside a tuple type is still a diagnostic, not a
    // silently dropped element.
    let message = all_messages(&parse_err("struct S { e: (int,, float) }"));
    assert!(message.contains("type"), "{message}");
    assert_eq!(
        parse_err("struct S { e: (int,, float) }").len(),
        1,
        "one mistake, one diagnostic: {message}"
    );
}

#[test]
fn a_type_the_parser_cannot_read_is_a_diagnostic() {
    // Yashy has no generics, so `[` after any other name is a mistake worth
    // naming rather than a silent reading.
    let message = all_messages(&parse_err("struct S { x: Vec[int] }"));
    assert!(message.contains("generics"), "{message}");
    assert!(all_messages(&parse_err("struct S { x: Option[ }")).contains("Option"));
    assert!(
        all_messages(&parse_err("struct S { x: Result[int] }")).contains("two types"),
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
    let src = "enum Shape {\n    Circle(float),\n    Rect { w: float, h: float },\n    Empty,\n}";
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
                assert_eq!(name, "float");
                assert_eq!(
                    *span,
                    span_of(src, "float"),
                    "the field carries the type's own text"
                );
            }
            other => panic!("expected `float`, got {other:?}"),
        },
        other => panic!("`Circle(float)` is a positional payload, got {other:?}"),
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
    assert_eq!(variants[1].fields[1].span(), span_of(src, "h: float"));

    assert!(
        variants[2].fields.is_empty(),
        "a unit variant has no fields"
    );
    assert_eq!(variants[0].span, span_of(src, "Circle(float)"));
    assert_eq!(variants[1].span, span_of(src, "Rect { w: float, h: float }"));
    assert_eq!(variants[2].span, span_of(src, "Empty"));
}

/// The two payload shapes are two variants, and neither can be mistaken for the
/// other -- a sentinel cannot creep back in without this failing.
#[test]
fn a_positional_payload_is_not_a_named_field_with_no_name() {
    let src = "enum Shape { Circle(float), Rect { w: float } }";
    let variants = match &parse_ok(src).items[0] {
        Item::Enum { variants, .. } => variants.clone(),
        other => panic!("expected an enum, got {other:?}"),
    };

    // A parenthesised payload is `Positional`, and `Positional` has no `name`
    // field at all -- so there is nothing in it that could be empty, and no
    // downstream `name.is_empty()` check to forget.
    match &variants[0].fields[0] {
        VariantField::Positional { ty, span } => {
            assert_eq!(span, &span_of(src, "float"));
            assert!(matches!(ty, TypeExpr::Name(name, _) if name == "float"));
        }
        other => panic!("`Circle(float)` must be Positional, got {other:?}"),
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
            assert_eq!(span, &span_of(src, "w: float"));
            assert!(matches!(ty, TypeExpr::Name(name, _) if name == "float"));
        }
        other => panic!("`Rect {{ w: float }}` must be Named, got {other:?}"),
    }

    // The two are different values, and they are different *types*: a pattern
    // that reads the wrong one does not compile.
    assert_ne!(variants[0].fields[0], variants[1].fields[0]);

    // `FieldDef` -- a struct's field -- still has a plain `name`, and is not the
    // type a variant payload uses.
    let struct_fields = match &parse_ok("struct P { x: float }").items[0] {
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
    assert!(all_messages(&parse_err("fn f(a: int { }")).contains("`)`"));
}

/// A comma in a parameter list is either the separator or a trailing mark, and
/// which one it is has to be decided rather than stumbled into.
#[test]
fn a_stray_or_trailing_comma_in_a_parameter_list_is_handled_deliberately() {
    // The baseline: two parameters, one comma between them.
    match &parse_ok("fn f(a: int, b: int) { }").items[0] {
        Item::Fn { params, .. } => {
            let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, vec!["a", "b"]);
        }
        other => panic!("expected a function, got {other:?}"),
    }

    // A trailing comma is accepted, and it does not invent a parameter: the
    // comma before `)` ends the list rather than introducing an empty one.
    match &parse_ok("fn f(a: int,) { }").items[0] {
        Item::Fn { params, .. } => assert_eq!(
            params.len(),
            1,
            "a trailing comma is not a parameter: {params:?}"
        ),
        other => panic!("expected a function, got {other:?}"),
    }
    // And the same in a struct's field list, for the same reason.
    match &parse_ok("struct S { x: float, }").items[0] {
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
        "fn f(a: int,, b: int) { }", // f(1,,2)
        "fn f(,) { }",               // f(,)
        "fn f(, a: int) { }",        // f(,1)
        "fn f(a: int b: int) { }",
        "fn f(a:) { }",
        "fn f(a: int,,, ) { }",
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

    // `struct S { a: int, }` is asserted above: a trailing comma there is one
    // field, not two. The named-variant payload and the positional one are the
    // remaining two comma loops, and they agree.
    match &parse_ok("enum E { A { w: float, } }").items[0] {
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
        "struct S { , x: float }",
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
    assert_eq!(parse_ok("struct S { x: float }").items.len(), 1);
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
    assert_eq!(parse_ok("enum E { A(int) }").items.len(), 1);
    assert_eq!(parse_ok("enum E { A { w: float } }").items.len(), 1);
    assert_eq!(parse_ok("enum E { A, B(float), C { w: float } }").items.len(), 1);
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
        "struct S { x: : int }",
        "struct S { x: int,, y: int }",
        "struct S { x: int,,, }",
        "struct S { 5 }",
        "struct S { x: Option[ }",
        "struct S { x: Option[int",
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
        "fn f(, a: int)",
        "fn f(a: int,)",
        "fn f(a: int,)",
        "fn f(a: int,, b: int)",
        "fn f(a: int,,, )",
        "fn f(a: int b: int)",
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
        "extern \"C\" link \"m\" { unsafe fn puts(s: string) -> int; }",
        "\u{0}",
        "@",
        "@@@",
        "fn f() { @ }",
        "}}}}}}}}",
        "((((((((((",
        "import std.io; struct P { x: float } enum C { Red } fn d() -> float { }",
    ];
    for src in sources {
        // A source that does not even lex is the lexer's business, not this
        // test's; either way, reaching here without a panic is the assertion.
        let mut map = SourceMap::new();
        let file = map.add_file("main.ya", src.to_string());
        if let Ok(tokens) = Lexer::with_file(src, file).tokenize() {
            let _ = Parser::new(tokens, &map, file).parse_program();
        }
    }
}

#[test]
fn deeply_nested_delimiters_hit_the_depth_limit_rather_than_the_stack() {
    // Nested delimiters are what a fuzzer finds first. The parser is recursive
    // descent -- one stack frame per open delimiter -- so without the limit this
    // input overflows the stack and takes the process down; `MAX_NESTING_DEPTH`
    // is what turns it into a diagnostic instead.
    //
    // This test used to run the parse on a thread with an 8 MiB stack, on the
    // stated grounds that the parser was iterative and had nothing to overflow.
    // Task 14 measured the opposite: `yashy check` on 130 nested parentheses
    // died with STATUS_STACK_OVERFLOW on the default 1 MiB main-thread stack.
    // The thread is gone deliberately. Running on the ordinary test stack is
    // what makes this test evidence that the bound protects the *machine* stack
    // rather than that a large-enough thread hides the problem.
    let mut src = String::from("fn f() { (");
    for _ in 0..200 {
        src.push('(');
    }
    src.push('1');
    for _ in 0..200 {
        src.push(')');
    }
    src.push_str(") ; }");

    let diags = parse_err(&src);
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
    let src = "struct S { z: int, a: int, m: int }";
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

#[test]
fn move_expr_parses_to_move_variant() {
    let src = "fn f() { const c = move x; }";
    let prog = parse_ok(src);
    let Item::Fn { body, .. } = &prog.items[0] else { panic!("expected fn") };
    let Stmt::Const { init, .. } = &body.stmts[0] else { panic!("expected const") };
    assert!(matches!(init, Expr::Move(bx, _) if matches!(bx.as_ref(), Expr::Ident(path, _) if path == &vec!["x".to_string()])));
}

#[test]
fn move_binds_looser_than_postfix_tighter_than_binary() {
    let src = "fn f() { const c = move a.b; const d = -move x; }";
    let prog = parse_ok(src);
    let Item::Fn { body, .. } = &prog.items[0] else { panic!("expected fn") };
    let Stmt::Const { init, .. } = &body.stmts[0] else { panic!("expected const") };
    assert!(matches!(init, Expr::Move(bx, _) if matches!(bx.as_ref(), Expr::FieldAccess { .. })));
    let Stmt::Const { init, .. } = &body.stmts[1] else { panic!("expected const") };
    assert!(matches!(init, Expr::Unary { op: UnOp::Neg, operand, .. } if matches!(operand.as_ref(), Expr::Move(_, _))));
}

#[test]
fn an_empty_body_after_a_bare_name_is_a_block_not_a_struct_literal() {
    // `{}` is not a struct literal -- every struct declares a field -- so a bare
    // name followed by `{ }` is a condition (or a range end) and an empty body.
    // This shape used to be read as `name {}`, after which the loop or `if` had
    // no body and the error pointed at the end of the enclosing function.
    for (src, expected) in [
        ("fn f(flag: bool) { if flag { } }", "if flag { }"),
        ("fn g(flag: bool) { while flag { } }", "while flag { }"),
        ("fn h(n: int) { for i in 0..n { } }", "for i in 0..n { }"),
    ] {
        let prog = parse_ok(src);
        let Item::Fn { body, .. } = &prog.items[0] else {
            panic!("expected fn")
        };
        assert_eq!(body.stmts.len(), 1, "{src:?} must be one statement");
        let span = match &body.stmts[0] {
            Stmt::If { span, .. } | Stmt::ForC { span, .. } | Stmt::ForRange { span, .. } => *span,
            other => panic!("expected a block statement, got {other:?}"),
        };
        assert_eq!(span, span_of(src, expected), "the span covers {expected:?}");
    }
}

#[test]
fn while_is_the_condition_only_for_loop() {
    // `while` is sugar, and this is the pin that it is *only* sugar: it builds
    // the same `Stmt::ForC` the equivalent C-style loop builds, so the checker
    // and the backend never learn that `while` exists. There is one loop node,
    // which is why the keyword cannot lower differently from `for ; cond; { }`
    // by accident.
    let src = "fn f(flag: bool) { while flag { } }";
    let prog = parse_ok(src);
    let Item::Fn { body, .. } = &prog.items[0] else {
        panic!("expected fn")
    };
    let Stmt::ForC {
        init,
        cond,
        update,
        body: loop_body,
        span,
    } = &body.stmts[0]
    else {
        panic!("expected the condition-only loop, got {:?}", body.stmts[0])
    };
    assert!(init.is_none(), "a `while` has no initialiser");
    assert!(update.is_none(), "and nothing to run after each turn");
    assert!(
        cond.as_ref()
            .is_some_and(|c| matches!(c, Expr::Ident(path, _) if path == &vec!["flag".to_string()])),
        "the condition is the expression written, got {cond:?}"
    );
    assert!(loop_body.stmts.is_empty());
    assert_eq!(
        *span,
        span_of(src, "while flag { }"),
        "the loop's span covers its own text"
    );
}

#[test]
fn switch_expression_retains_subject() {
    let src = "fn f() { const c = switch x { default { 0 } }; }";
    let program = parse_ok(src);
    match &program.items[0] {
        Item::Fn { body, .. } => match &body.stmts[0] {
            Stmt::Const { init, .. } => match init {
                Expr::SwitchExpr { subject, .. } => match subject.as_ref() {
                    Expr::Ident(path, _) => assert_eq!(path, &vec!["x".to_string()]),
                    other => panic!("expected ident subject, got {other:?}"),
                },
                other => panic!("expected switch expr, got {other:?}"),
            },
            other => panic!("expected const stmt, got {other:?}"),
        },
        other => panic!("expected fn, got {other:?}"),
    }

    let src = "fn f() { const c = switch a.b { default { 0 } }; }";
    let program = parse_ok(src);
    match &program.items[0] {
        Item::Fn { body, .. } => match &body.stmts[0] {
            Stmt::Const { init, .. } => match init {
                Expr::SwitchExpr { subject, .. } => match subject.as_ref() {
                    Expr::Ident(path, _) => assert_eq!(path, &vec!["a".to_string(), "b".to_string()]),
                    other => panic!("expected ident subject, got {other:?}"),
                },
                other => panic!("expected switch expr, got {other:?}"),
            },
            other => panic!("expected const stmt, got {other:?}"),
        },
        other => panic!("expected fn, got {other:?}"),
    }
}

#[test]
fn unit_variant_pattern_is_not_a_struct_pattern_when_a_brace_follows() {
    // `case None { 0 }` is a unit variant plus an arm body. The pattern
    // parser must not read `{ 0 }` as a struct-variant field list just
    // because a brace follows the name: only `{ ident: .. }` or `{ .. }`
    // opens a struct pattern.
    let src = "fn f() { return switch o { case None { 0 } }; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Return(Some(Expr::SwitchExpr { arms, .. }), _) = &body.stmts[0] else {
        panic!("expected switch expression")
    };
    assert_eq!(arms.len(), 1);
    assert!(
        matches!(
            &arms[0].pattern,
            Pattern::UnitVariant { path, .. } if path == &vec!["None".to_string()]
        ),
        "expected a None unit pattern, got {:?}",
        arms[0].pattern
    );
    assert!(
        matches!(&arms[0].body, Expr::Literal(Literal::I64(0), _)),
        "expected the 0 arm body, got {:?}",
        arms[0].body
    );
}

#[test]
fn struct_variant_pattern_still_parses_after_the_lookahead_fix() {
    // The unit-variant fix must not swallow the real struct-variant form:
    // `case RGB { red: r }` still opens a struct pattern.
    let src = "fn f() { return switch s { case RGB { red: r } { r } }; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Return(Some(Expr::SwitchExpr { arms, .. }), _) = &body.stmts[0] else {
        panic!("expected switch expression")
    };
    assert!(
        matches!(&arms[0].pattern, Pattern::StructVariant { path, fields, .. }
            if path == &vec!["RGB".to_string()]
                && fields.len() == 1
                && fields[0].0 == "red"
                && matches!(&fields[0].1, Pattern::Binding { name, .. } if name == "r")),
        "expected RGB struct pattern, got {:?}",
        arms[0].pattern
    );
}

// -- assignment -------------------------------------------------------------

#[test]
fn a_let_binding_keeps_its_name_and_annotation() {
    let src = "fn main() { let x: int = 1; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Let {
        name, ty, init, ..
    } = &body.stmts[0]
    else {
        panic!("expected a let binding, got {:?}", body.stmts[0]);
    };
    assert_eq!(name, "x");
    assert!(ty.is_some(), "the annotation must be kept");
    assert!(
        matches!(init, Expr::Literal(Literal::I64(1), _)),
        "expected the initializer 1, got {init:?}"
    );
}

#[test]
fn assignment_to_a_let_binding_is_a_statement() {
    // Before this, `let x = 1; x = 2;` did not parse at all: the parser read `x`
    // as an expression statement, wanted a `;`, and found `=`. So a binding
    // could be declared mutable and never mutated, which made a separate mutable keyword dead syntax
    // and left every loop body unable to accumulate anything.
    let src = "fn main() { let x = 1; x = 2; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    assert_eq!(body.stmts.len(), 2, "two statements expected: {body:?}");
    let Stmt::Assign { target, value, .. } = &body.stmts[1] else {
        panic!("expected an assignment, got {:?}", body.stmts[1]);
    };
    assert!(
        matches!(target, Expr::Ident(path, _) if path == &vec!["x".to_string()]),
        "expected the assignment to target `x`, got {target:?}"
    );
    assert!(
        matches!(value, Expr::Literal(Literal::I64(2), _)),
        "expected the assigned value 2, got {value:?}"
    );
}

#[test]
fn compound_assignment_is_sugar_for_the_expanded_assignment() {
    // `x += 1` and `x = x + 1` must produce the same statement, because there is
    // no compound-assignment node: the desugar happens in the parser and the
    // checker never learns that `+=` exists. The target is one node reused,
    // not two shapes that could disagree.
    let src = "fn main() { let x = 1; x += 1; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Assign { target, value, .. } = &body.stmts[1] else {
        panic!("expected an assignment, got {:?}", body.stmts[1]);
    };
    assert!(
        matches!(target, Expr::Ident(path, _) if path == &vec!["x".to_string()]),
        "the target is the name written, got {target:?}"
    );
    let Expr::Binary { op, lhs, rhs, .. } = value else {
        panic!("expected the operator applied to the target, got {value:?}")
    };
    assert_eq!(*op, BinOp::Add);
    assert!(
        matches!(lhs.as_ref(), Expr::Ident(path, _) if path == &vec!["x".to_string()]),
        "the left operand is the target, got {lhs:?}"
    );
    assert!(
        matches!(rhs.as_ref(), Expr::Literal(Literal::I64(1), _)),
        "the right operand is the value written, got {rhs:?}"
    );
}

#[test]
fn every_compound_operator_maps_to_its_own_binary() {
    // The lexer's five `…=` tokens had no consumer before this; each must reach
    // its own operator rather than, say, all of them becoming `Add`.
    for (written, expected) in [
        ("+=", BinOp::Add),
        ("-=", BinOp::Sub),
        ("*=", BinOp::Mul),
        ("/=", BinOp::Div),
        ("%=", BinOp::Rem),
    ] {
        let src = format!("fn main() {{ let x = 1; x {written} 2; }}");
        let program = parse_ok(&src);
        let Item::Fn { body, .. } = &program.items[0] else {
            panic!("expected fn")
        };
        let Stmt::Assign {
            value: Expr::Binary { op, .. },
            ..
        } = &body.stmts[1]
        else {
            panic!(
                "`{written}` must parse as an assignment of a binary, got {:?}",
                body.stmts[1]
            )
        };
        assert_eq!(*op, expected, "`{written}` maps to {expected:?}");
    }
}

#[test]
fn equality_is_not_an_assignment() {
    // `=` and `==` are separate tokens, and the statement parser must read them
    // separately: an `Expr::Binary` comparison is a statement in its own right,
    // not the left half of an assignment.
    let src = "fn main() { let x = 1; if x == 1 { } }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::If { cond, .. } = &body.stmts[1] else {
        panic!("expected an if, got {:?}", body.stmts[1]);
    };
    assert!(
        matches!(cond, Expr::Binary { op: BinOp::Eq, .. }),
        "expected an equality comparison, got {cond:?}"
    );
}

#[test]
fn an_assignment_target_is_an_expression_not_a_name() {
    // `1 = 2;` has no meaning in Yashy -- only a `let` binding is assignable --
    // but it must be the *checker* that says so, not the parser. Building the
    // target as an `Expr` is what lets M6 add field and subscript targets by
    // widening the target shape rather than by adding another statement, and a
    // parser that rejected non-names here would have to be reopened to do it.
    let src = "fn f() { 1 = 2; }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Assign { target, .. } = &body.stmts[0] else {
        panic!("expected an assignment, got {:?}", body.stmts[0]);
    };
    assert!(
        matches!(target, Expr::Literal(Literal::I64(1), _)),
        "expected the literal 1 as the target, got {target:?}"
    );
}

// -- string interpolation ---------------------------------------------------

#[test]
fn an_interpolated_string_is_one_node_of_text_and_expressions() {
    // `"n = ${value + 1}"`: text then expression, in source order, as written.
    // The conversion of the expression to text is *not* decided here -- the
    // parser does not know its type, and choosing `str_from_i64` would be a
    // guess the checker would then have to trust.
    let src = "fn main() { let value = 3; print(\"n = ${value + 1}\"); }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Expr(Expr::Call { args, .. }, _) = &body.stmts[1] else {
        panic!("expected the print_str call, got {:?}", body.stmts[1]);
    };
    let Expr::Interp { parts, span } = &args[0] else {
        panic!("expected an interpolation, got {:?}", args[0]);
    };
    assert_eq!(
        span_of(src, "\"n = ${value + 1}\""),
        *span,
        "the node spans the whole literal, quotes included"
    );
    assert_eq!(parts.len(), 2, "text then expression: {parts:?}");
    assert!(
        matches!(&parts[0], InterpPart::Text(lit) if lit.value == "n = "),
        "the leading text, got {:?}",
        parts[0]
    );
    let InterpPart::Expr(inner, InterpFormat::Display) = &parts[1] else {
        panic!("expected the expression part, got {:?}", parts[1]);
    };
    assert_eq!(
        span_of(src, "value + 1"),
        yashy_parser::exprs::expr_span(inner),
        "the inner expression keeps the span of its own text, not the literal's"
    );
}

#[test]
fn a_debug_suffix_marks_one_interpolation_part() {
    // `"${value:?}"`: the `:?` is a formatting request, not part of the
    // expression. The parser keeps the two apart so the checker -- the stage
    // that knows the expression's type -- can choose the debug form.
    let src = "fn main() { print(\"${value:?}\"); }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Expr(Expr::Call { args, .. }, _) = &body.stmts[0] else {
        panic!("expected the print_str call, got {:?}", body.stmts[0]);
    };
    let Expr::Interp { parts, .. } = &args[0] else {
        panic!("expected an interpolation, got {:?}", args[0]);
    };
    assert_eq!(parts.len(), 1, "one part: {parts:?}");
    let InterpPart::Expr(inner, format) = &parts[0] else {
        panic!("expected the expression part, got {:?}", parts[0]);
    };
    assert_eq!(*format, InterpFormat::Debug, "the `:?` suffix asks for debug");
    assert_eq!(
        span_of(src, "value"),
        yashy_parser::exprs::expr_span(inner),
        "the expression is the text before the suffix"
    );
}

#[test]
fn a_display_interpolation_has_no_suffix() {
    // The default is display, and the parts of a mixed literal each carry their
    // own format: `"${a} ${b:?}"` is one display part and one debug part.
    let src = "fn main() { print(\"${a} ${b:?}\"); }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Expr(Expr::Call { args, .. }, _) = &body.stmts[0] else {
        panic!("expected the print_str call, got {:?}", body.stmts[0]);
    };
    let Expr::Interp { parts, .. } = &args[0] else {
        panic!("expected an interpolation, got {:?}", args[0]);
    };
    let formats: Vec<InterpFormat> = parts
        .iter()
        .filter_map(|part| match part {
            InterpPart::Expr(_, format) => Some(*format),
            InterpPart::Text(_) => None,
        })
        .collect();
    assert_eq!(
        formats,
        vec![InterpFormat::Display, InterpFormat::Debug],
        "each part carries its own format"
    );
}

#[test]
fn adjacent_interpolations_have_no_text_between_them() {
    // `"${a}${b}"` is two expressions and no text. An empty text piece would
    // have to be handled -- and could be mishandled -- by every later stage, so
    // the split drops it rather than recording it.
    let src = "fn main() { print(\"${a}${b}\"); }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Expr(Expr::Call { args, .. }, _) = &body.stmts[0] else {
        panic!("expected the print_str call, got {:?}", body.stmts[0]);
    };
    let Expr::Interp { parts, .. } = &args[0] else {
        panic!("expected an interpolation, got {:?}", args[0]);
    };
    assert_eq!(parts.len(), 2, "two expressions, no text: {parts:?}");
    assert!(
        parts.iter().all(|p| matches!(p, InterpPart::Expr(_, InterpFormat::Display))),
        "no text piece, got {parts:?}"
    );
}

#[test]
fn a_literal_without_interpolation_is_a_plain_string_not_an_interp_node() {
    // Code that handles strings must not have to look for an `Interp` case that
    // cannot occur: `"hi"` is a `Literal`, and only `${` makes it an `Interp`.
    let src = "fn main() { print(\"hi\"); }";
    let program = parse_ok(src);
    let Item::Fn { body, .. } = &program.items[0] else {
        panic!("expected fn")
    };
    let Stmt::Expr(Expr::Call { args, .. }, _) = &body.stmts[0] else {
        panic!("expected the print_str call, got {:?}", body.stmts[0]);
    };
    assert!(
        matches!(&args[0], Expr::Literal(Literal::Str(lit), _) if lit.value == "hi"),
        "expected a plain string literal, got {:?}",
        args[0]
    );
}

#[test]
fn an_empty_or_overfull_interpolation_is_reported() {
    // `${}` has nothing to convert; the message has to say so rather than build
    // an expression out of nothing.
    let diags = parse_err("fn main() { print(\"${}\"); }");
    assert!(
        all_messages(&diags).contains("empty"),
        "an empty interpolation must be named as empty: {}",
        all_messages(&diags)
    );

    // `${a b}` is two expressions where one was expected: the second should have
    // been the `}`.
    let diags = parse_err("fn main() { let a = 1; print(\"${a b}\"); }");
    assert!(
        all_messages(&diags).contains("`}`"),
        "the missing `}}` must be named: {}",
        all_messages(&diags)
    );
}

#[test]
fn interpolations_nested_past_the_limit_are_a_diagnostic_not_a_deeper_stack() {
    // Interpolation nests the *parser* as well as the input: each `${ ... }` is
    // read by a fresh sub-parser, so the shared delimiter stack cannot bound it
    // and a literal like `"${"${"${...}"}"}"` would recurse until the machine
    // stack ran out. The limit has to be counted on its own, and hitting it has
    // to be a diagnostic rather than an abort.
    let mut expr = "x".to_string();
    for _ in 0..MAX_NESTING_DEPTH.saturating_add(8) {
        expr = format!("\"${{{expr}}}\"");
    }
    let diags = parse_err(&format!("fn f() {{ const s = {expr}; }}"));
    assert!(
        all_messages(&diags).contains("nested"),
        "deep interpolation must be a nesting diagnostic, got {}",
        all_messages(&diags)
    );
}

#[test]
fn a_few_levels_of_nested_interpolation_still_parse() {
    // The limit above must not be so tight that ordinary nesting trips it:
    // `"a ${"b ${x}"} c"` is awkward, but it is real, and it is three levels.
    let mut expr = "x".to_string();
    for _ in 0..3 {
        expr = format!("\"${{{expr}}}\"");
    }
    let src = format!("fn f() {{ let x = 1; const s = {expr}; }}");
    parse_ok(&src);
}

#[test]
fn an_interpolated_string_cannot_be_a_pattern() {
    // A `case` pattern is matched against a value; it is not built at run time,
    // so `${ ... }` in one is a mistake with a name rather than a silent literal
    // that would never match.
    let diags = parse_err("fn f(s: string) { switch s { case \"${s}\" { 0 } } }");
    assert!(
        all_messages(&diags).contains("pattern"),
        "the message must say a pattern cannot interpolate: {}",
        all_messages(&diags)
    );
}

// -- integer literals that do not fit ----------------------------------------

#[test]
fn an_integer_literal_that_does_not_fit_an_i64_is_a_diagnostic_not_a_zero() {
    // An `I64` token carries no value, so every reader of integer digits has to
    // answer what out-of-range digits mean, and the tempting answer --
    // `text.parse::<i64>().unwrap_or(0)` -- compiles `9223372036854775808` into a
    // silently wrong `0`. That is the worst outcome a compiler has: a wrong
    // answer that looks like a right one. The lexer answers the question once
    // (`literal_as_i64`) and the parser asks it.
    let src = "fn main() { print(9223372036854775808); }";
    let diags = parse_err(src);
    assert!(
        all_messages(&diags).contains("integer literal does not fit an int"),
        "expected the lexer's wording: {}",
        all_messages(&diags)
    );
    assert_eq!(
        diags[0].span,
        span_of(src, "9223372036854775808"),
        "the caret belongs under the digits"
    );
}

#[test]
fn an_integer_pattern_that_does_not_fit_is_a_diagnostic_too() {
    // The same question is asked when a `case` arm's arm is a number, and it has
    // the same wrong answer available: a pattern compiled to `0` matches a value
    // the user never named.
    let diags = parse_err("fn f() { switch 1 { case 99999999999999999999 { print(1); } } }");
    assert!(
        all_messages(&diags).contains("integer literal does not fit an int"),
        "{}",
        all_messages(&diags)
    );
}

#[test]
fn i64_min_is_spelled_by_folding_the_minus_into_the_literal() {
    // `-9223372036854775808` is the one integer a leading minus makes readable
    // that the digits alone are not: they are one past `int::MAX`, so the literal
    // under the minus cannot stand on its own and `int::MIN` would have no
    // spelling at all. The fold is narrow -- the digits must be out of range *and*
    // negate into range -- so `-5` is still a negation of `5`.
    let src = "fn f() { const big = -9223372036854775808; const small = -5; }";
    let prog = parse_ok(src);
    let Item::Fn { body, .. } = &prog.items[0] else { panic!("expected fn") };
    let Stmt::Const { init, .. } = &body.stmts[0] else { panic!("expected const") };
    match init {
        Expr::Literal(Literal::I64(value), span) => {
            assert_eq!(*value, i64::MIN);
            assert_eq!(
                *span,
                span_of(src, "-9223372036854775808"),
                "the span covers the minus, not just the digits"
            );
        }
        other => panic!("expected a folded int::MIN literal, got {other:?}"),
    }
    let Stmt::Const { init, .. } = &body.stmts[1] else { panic!("expected const") };
    assert!(
        matches!(init, Expr::Unary { op: UnOp::Neg, .. }),
        "`-5` stays a negation: {init:?}"
    );
}

#[test]
fn the_three_bracket_types_parse_apart() {
    // One bracket pair, three meanings: empty brackets take the element after
    // them (`[]T`), `T; N` is a fixed array, and `T` alone is a view. Reading
    // the pair in one place is what keeps `[]T` from being read as a view.
    let src = "struct S { grow: []int, fixed: [int; 3], view: [int] }";
    let program = parse_ok(src);
    let fields = match &program.items[0] {
        Item::Struct { fields, .. } => fields,
        other => panic!("expected a struct, got {other:?}"),
    };
    assert!(
        matches!(&fields[0].ty, TypeExpr::Slice(_, _)),
        "`[]int` is a growable array: {:?}",
        fields[0].ty
    );
    match &fields[1].ty {
        TypeExpr::FixedArray(element, length, span) => {
            assert_eq!(*length, 3);
            assert!(matches!(**element, TypeExpr::Name(ref name, _) if name == "int"));
            assert_eq!(*span, span_of(src, "[int; 3]"));
        }
        other => panic!("`[int; 3]` is a fixed array, got {other:?}"),
    }
    assert!(
        matches!(&fields[2].ty, TypeExpr::View(_, _)),
        "`[int]` is a view: {:?}",
        fields[2].ty
    );
}

#[test]
fn a_fixed_array_length_must_be_a_and_fit_in_a_machine_word() {
    assert!(
        all_messages(&parse_err("struct S { a: [int; n] }")).contains("array length"),
        "a length is a literal, not a name"
    );
    assert!(
        all_messages(&parse_err("struct S { a: [int; 99999999999999999999999] }"))
            .contains("machine word"),
        "a length that cannot be stored is reported, not truncated"
    );
}

#[test]
fn a_fixed_array_literal_and_a_range_are_distinct_expressions() {
    let program = parse_ok("fn main() { const a = [1, 2, 3]; const b = a[0..2]; const c = a[1]; }");
    let body = match &program.items[0] {
        Item::Fn { body, .. } => body,
        other => panic!("expected a function, got {other:?}"),
    };
    assert!(
        matches!(&body.stmts[0], Stmt::Const { init, .. } if matches!(init, Expr::FixedArrayLit { .. })),
        "`[1, 2, 3]` is a fixed array literal, not a growable one"
    );
    assert!(
        matches!(&body.stmts[1], Stmt::Const { init, .. } if matches!(init, Expr::IndexRange { .. })),
        "`a[0..2]` is a range, so it makes a view"
    );
    assert!(
        matches!(&body.stmts[2], Stmt::Const { init, .. } if matches!(init, Expr::Index { .. })),
        "`a[1]` is an index"
    );
}
