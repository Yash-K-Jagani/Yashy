//! Tests for the AST node types.
//!
//! The AST is a frozen contract: the parser builds it, the type checker reads
//! it, code generation lowers it, and golden tests dump whole trees of it with
//! `{:?}`. So these tests pin three things at once -- that every variant exists
//! and can be built, that `Debug` names each variant and each of its fields,
//! and that every node keeps the span it was handed.

use std::fmt::Debug;

use otter_common::Span;
use otter_parser::ast::{
    BinOp, Block, Expr, ExternDecl, FieldDef, Item, Literal, Param, Pattern, Program, Stmt,
    StringLit, SwitchArm, TypeExpr, UnOp, VariantDef,
};

// -- helpers ---------------------------------------------------------------

/// A span that is not the synthetic one, so a node built with it cannot be
/// mistaken for one the compiler invented.
fn here() -> Span {
    Span::new(11, 13)
}

/// A `StringLit` built from its two public fields rather than decoded, so this
/// file cannot fail for a reason that belongs to the lexer's escape handling.
fn text(value: &str) -> StringLit {
    StringLit {
        value: value.to_string(),
        bytes: value.as_bytes().to_vec(),
    }
}

fn int(value: i64, span: Span) -> Expr {
    Expr::Literal(Literal::I64(value), span)
}

/// A subtree field. `Expr` is recursive, so `lhs`, `rhs`, `operand`, `base`,
/// `expr` and `args`' elements are boxed; this keeps that one word out of every
/// literal in this file.
fn bx(expr: Expr) -> Box<Expr> {
    Box::new(expr)
}

fn empty_block(span: Span) -> Block {
    Block {
        stmts: Vec::new(),
        span,
    }
}

/// Hands out a distinct span per node and remembers every one it handed out.
///
/// Distinctness is the point: with one span per node, the `Debug` output of the
/// finished tree must contain each span exactly once, which is what catches a
/// node that drops its span or carries the same one twice.
#[derive(Default)]
struct Spans {
    next: u32,
    used: Vec<Span>,
}

impl Spans {
    fn take(&mut self) -> Span {
        let start = self.next;
        self.next = self.next.saturating_add(1);
        let span = Span::new(start, start.saturating_add(2));
        self.used.push(span);
        span
    }
}

// -- one node of every kind, each with its own span ------------------------

fn full_program(spans: &mut Spans) -> Program {
    let items = vec![
        // Item::Import
        Item::Import {
            path: vec!["std".to_string(), "io".to_string()],
            span: spans.take(),
        },
        // Item::Struct, with a FieldDef.
        Item::Struct {
            name: "P".to_string(),
            fields: vec![FieldDef {
                name: "x".to_string(),
                ty: TypeExpr::Name("f64".to_string(), spans.take()),
                span: spans.take(),
            }],
            span: spans.take(),
        },
        // Item::Enum, with a bare VariantDef and one carrying a FieldDef.
        Item::Enum {
            name: "C".to_string(),
            variants: vec![
                VariantDef {
                    name: "Red".to_string(),
                    fields: Vec::new(),
                    span: spans.take(),
                },
                VariantDef {
                    name: "Green".to_string(),
                    fields: vec![FieldDef {
                        name: "y".to_string(),
                        ty: TypeExpr::Unit(spans.take()),
                        span: spans.take(),
                    }],
                    span: spans.take(),
                },
            ],
            span: spans.take(),
        },
        // Item::Fn, with a Param, a Result type, an Option return type and a
        // body that holds every statement.
        Item::Fn {
            name: "d".to_string(),
            params: vec![Param {
                name: "a".to_string(),
                ty: TypeExpr::Result(
                    Box::new(TypeExpr::Name("i64".to_string(), spans.take())),
                    Box::new(TypeExpr::Tuple(
                        vec![TypeExpr::Unit(spans.take())],
                        spans.take(),
                    )),
                    spans.take(),
                ),
                span: spans.take(),
            }],
            ret: Some(TypeExpr::Option(
                Box::new(TypeExpr::Name("f64".to_string(), spans.take())),
                spans.take(),
            )),
            body: full_block(spans),
            span: spans.take(),
        },
        // Item::Extern, with two declarations: one with a parameter and a
        // return type, one bare.
        Item::Extern {
            link: Some("C".to_string()),
            decls: vec![
                ExternDecl {
                    name: "puts".to_string(),
                    params: vec![Param {
                        name: "s".to_string(),
                        ty: TypeExpr::Name("string".to_string(), spans.take()),
                        span: spans.take(),
                    }],
                    ret: Some(TypeExpr::Name("i64".to_string(), spans.take())),
                    span: spans.take(),
                },
                ExternDecl {
                    name: "now".to_string(),
                    params: Vec::new(),
                    ret: None,
                    span: spans.take(),
                },
            ],
            span: spans.take(),
        },
    ];

    Program { items }
}

/// A block holding one statement of every kind, and inside them one node of
/// every kind that a statement can reach: every `Stmt`, every `Pattern`, every
/// `Expr` and all fifteen operators.
fn full_block(spans: &mut Spans) -> Block {
    let stmts = vec![
        // Stmt::Expr
        Stmt::Expr(int(1, spans.take()), spans.take()),
        // Stmt::Const
        Stmt::Const {
            name: "c".to_string(),
            ty: None,
            init: Expr::Literal(Literal::F64(0.5), spans.take()),
            span: spans.take(),
        },
        // Stmt::Var
        Stmt::Var {
            name: "v".to_string(),
            ty: Some(TypeExpr::Name("f64".to_string(), spans.take())),
            init: Expr::Literal(Literal::Bool(true), spans.take()),
            span: spans.take(),
        },
        // Stmt::Return, with a value
        Stmt::Return(Some(int(0, spans.take())), spans.take()),
        // Stmt::If, with a condition binding and an else branch
        Stmt::If {
            init: Some(Box::new(Stmt::Const {
                name: "d".to_string(),
                ty: None,
                init: Expr::Call {
                    callee: vec!["f".to_string()],
                    args: Vec::new(),
                    span: spans.take(),
                },
                span: spans.take(),
            })),
            cond: Expr::Binary {
                op: BinOp::And,
                lhs: bx(Expr::Ident(vec!["p".to_string()], spans.take())),
                rhs: bx(Expr::Ident(vec!["q".to_string()], spans.take())),
                span: spans.take(),
            },
            then_block: empty_block(spans.take()),
            else_block: Some(empty_block(spans.take())),
            span: spans.take(),
        },
        // Stmt::ForRange
        Stmt::ForRange {
            name: "i".to_string(),
            start: int(0, spans.take()),
            end: int(10, spans.take()),
            body: empty_block(spans.take()),
            span: spans.take(),
        },
        // Stmt::ForC, all three of whose parts are optional and here all set
        Stmt::ForC {
            init: Some(Box::new(Stmt::Const {
                name: "j".to_string(),
                ty: None,
                init: int(0, spans.take()),
                span: spans.take(),
            })),
            cond: Some(Expr::Binary {
                op: BinOp::Lt,
                lhs: bx(Expr::Ident(vec!["j".to_string()], spans.take())),
                rhs: bx(int(10, spans.take())),
                span: spans.take(),
            }),
            update: Some(Expr::Binary {
                op: BinOp::Add,
                lhs: bx(Expr::Ident(vec!["j".to_string()], spans.take())),
                rhs: bx(int(1, spans.take())),
                span: spans.take(),
            }),
            body: empty_block(spans.take()),
            span: spans.take(),
        },
        // Stmt::Switch, with one arm of every pattern kind
        Stmt::Switch {
            subject: Expr::Ident(vec!["s".to_string()], spans.take()),
            arms: full_arms(spans),
            span: spans.take(),
        },
        // Stmt::Break
        Stmt::Break(spans.take()),
        // Stmt::Continue
        Stmt::Continue(spans.take()),
        // Stmt::Defer
        Stmt::Defer {
            body: empty_block(spans.take()),
            span: spans.take(),
        },
        // Stmt::Expr over the expression forms that no other statement reaches:
        // every remaining operator, a call, a struct literal, a field access, a
        // cast, a try and a switch expression.
        Stmt::Expr(full_expr_tail(spans), spans.take()),
    ];

    Block {
        stmts,
        span: spans.take(),
    }
}

fn full_arms(spans: &mut Spans) -> Vec<SwitchArm> {
    fn literal_pattern(spans: &mut Spans) -> Pattern {
        Pattern::Literal(Literal::I64(7), spans.take())
    }
    vec![
        SwitchArm {
            pattern: Pattern::UnitVariant {
                path: vec!["Red".to_string()],
                span: spans.take(),
            },
            body: int(1, spans.take()),
            is_default: false,
            span: spans.take(),
        },
        SwitchArm {
            pattern: Pattern::TupleVariant {
                path: vec!["RGB".to_string()],
                args: vec![literal_pattern(spans)],
                span: spans.take(),
            },
            body: int(2, spans.take()),
            is_default: false,
            span: spans.take(),
        },
        SwitchArm {
            pattern: Pattern::StructVariant {
                path: vec!["RGB".to_string()],
                fields: vec![("red".to_string(), literal_pattern(spans), spans.take())],
                span: spans.take(),
            },
            body: int(3, spans.take()),
            is_default: false,
            span: spans.take(),
        },
        SwitchArm {
            pattern: Pattern::Binding {
                name: "other".to_string(),
                span: spans.take(),
            },
            body: int(4, spans.take()),
            is_default: false,
            span: spans.take(),
        },
        SwitchArm {
            pattern: literal_pattern(spans),
            body: int(5, spans.take()),
            is_default: false,
            span: spans.take(),
        },
        SwitchArm {
            pattern: Pattern::Wildcard(spans.take()),
            body: int(6, spans.take()),
            is_default: true,
            span: spans.take(),
        },
    ]
}

/// Every operator, and the expression forms that only this statement reaches.
fn full_expr_tail(spans: &mut Spans) -> Expr {
    let mut ops = Vec::new();
    for op in [
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Rem,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
        BinOp::And,
        BinOp::Or,
    ] {
        ops.push(Expr::Binary {
            op,
            lhs: bx(int(1, spans.take())),
            rhs: bx(int(2, spans.take())),
            span: spans.take(),
        });
    }
    ops.push(Expr::Unary {
        op: UnOp::Neg,
        operand: bx(int(1, spans.take())),
        span: spans.take(),
    });
    ops.push(Expr::Unary {
        op: UnOp::Not,
        operand: bx(Expr::Literal(Literal::Bool(true), spans.take())),
        span: spans.take(),
    });
    ops.push(Expr::Call {
        callee: vec!["print".to_string()],
        args: vec![int(1, spans.take())],
        span: spans.take(),
    });
    ops.push(Expr::StructLit {
        path: vec!["P".to_string()],
        fields: vec![(
            "x".to_string(),
            Expr::Literal(Literal::F64(1.5), spans.take()),
            spans.take(),
        )],
        span: spans.take(),
    });
    ops.push(Expr::FieldAccess {
        base: bx(Expr::Ident(vec!["p".to_string()], spans.take())),
        field: "x".to_string(),
        span: spans.take(),
    });
    ops.push(Expr::Cast {
        expr: bx(int(1, spans.take())),
        ty: TypeExpr::Name("f64".to_string(), spans.take()),
        span: spans.take(),
    });
    ops.push(Expr::Try {
        expr: bx(Expr::Call {
            callee: vec!["f".to_string()],
            args: Vec::new(),
            span: spans.take(),
        }),
        span: spans.take(),
    });
    ops.push(Expr::SwitchExpr {
        arms: vec![SwitchArm {
            pattern: Pattern::Wildcard(spans.take()),
            body: int(0, spans.take()),
            is_default: true,
            span: spans.take(),
        }],
        span: spans.take(),
    });
    ops.push(Expr::Literal(Literal::Str(text("hi")), spans.take()));

    // Folded into a right-leaning chain, so the tree is not a list: every
    // element but the first becomes the `lhs` of the next `Add`.
    let mut iter = ops.into_iter();
    let mut chain = match iter.next() {
        Some(first) => first,
        None => return int(0, spans.take()),
    };
    for expr in iter {
        chain = Expr::Binary {
            op: BinOp::Add,
            lhs: bx(chain),
            rhs: bx(expr),
            span: spans.take(),
        };
    }
    chain
}

// -- Debug is load-bearing -------------------------------------------------

/// One `Debug` rendering, the names it must contain, and the names it must not.
struct Case {
    label: &'static str,
    debug: String,
    expected: &'static [&'static str],
    forbidden: &'static [&'static str],
}

fn case<T: Debug>(
    label: &'static str,
    value: T,
    expected: &'static [&'static str],
    forbidden: &'static [&'static str],
) -> Case {
    Case {
        label,
        debug: format!("{value:?}"),
        expected,
        forbidden,
    }
}

fn run(cases: Vec<Case>) {
    for case in cases {
        for name in case.expected {
            assert!(
                case.debug.contains(name),
                "{}: {:?} must contain {name:?}",
                case.label,
                case.debug
            );
        }
        for name in case.forbidden {
            assert!(
                !case.debug.contains(name),
                "{}: {:?} must not contain {name:?}",
                case.label,
                case.debug
            );
        }
    }
}

#[test]
fn ast_nodes_round_trip_through_debug() {
    let mut spans = Spans::default();
    let program = full_program(&mut spans);
    let debug = format!("{program:?}");

    // One of every Item variant.
    for name in ["Import", "Struct", "Enum", "Fn", "Extern", "ExternDecl"] {
        assert!(
            debug.contains(name),
            "the tree must contain a {name}: {debug}"
        );
    }
    // One of every Stmt variant.
    for name in [
        "Expr", "Const", "Var", "Return", "If", "ForRange", "ForC", "Switch", "Break", "Continue",
        "Defer",
    ] {
        assert!(
            debug.contains(name),
            "the tree must contain a {name}: {debug}"
        );
    }
    // The structure the debug output is read for.
    for name in ["Program", "items", "stmts", "span", "start", "end"] {
        assert!(
            debug.contains(name),
            "the tree must contain {name:?}: {debug}"
        );
    }

    // Every span handed out appears, and appears once: a node that dropped its
    // span or repeated a neighbour's would change these counts.
    for span in &spans.used {
        let needle = format!("{span:?}");
        let found = debug.matches(&needle).count();
        assert_eq!(found, 1, "{needle:?} must appear exactly once in {debug}");
    }

    // Nothing was invented: five items, one of each.
    assert_eq!(program.items.len(), 5);
}

#[test]
fn debug_names_every_item_type_variant_and_its_fields() {
    run(vec![
        case(
            "Item::Import",
            Item::Import {
                path: vec!["a".to_string()],
                span: here(),
            },
            &["Import", "path", "span", "start", "end"],
            &["Struct", "Enum", "Fn", "Extern"],
        ),
        case(
            "Item::Struct",
            Item::Struct {
                name: "S".to_string(),
                fields: Vec::new(),
                span: here(),
            },
            &["Struct", "name", "fields", "span"],
            &["Import", "Enum", "Fn", "Extern"],
        ),
        case(
            "Item::Enum",
            Item::Enum {
                name: "E".to_string(),
                variants: Vec::new(),
                span: here(),
            },
            &["Enum", "name", "variants", "span"],
            &["Import", "Struct", "Fn", "Extern"],
        ),
        case(
            "Item::Fn",
            Item::Fn {
                name: "f".to_string(),
                params: Vec::new(),
                ret: None,
                body: empty_block(here()),
                span: here(),
            },
            &["Fn", "name", "params", "ret", "body", "span"],
            &["Import", "Struct", "Enum", "Extern"],
        ),
        case(
            "Item::Extern",
            Item::Extern {
                link: None,
                decls: Vec::new(),
                span: here(),
            },
            &["Extern", "link", "decls", "span"],
            &["Import", "Struct", "Enum", "Fn"],
        ),
        case(
            "ExternDecl",
            ExternDecl {
                name: "puts".to_string(),
                params: Vec::new(),
                ret: None,
                span: here(),
            },
            &["ExternDecl", "name", "params", "ret", "span"],
            &["Extern {", "FieldDef", "VariantDef"],
        ),
        case(
            "FieldDef",
            FieldDef {
                name: "x".to_string(),
                ty: TypeExpr::Unit(here()),
                span: here(),
            },
            &["FieldDef", "name", "ty", "span"],
            &["VariantDef", "Param"],
        ),
        case(
            "VariantDef",
            VariantDef {
                name: "V".to_string(),
                fields: Vec::new(),
                span: here(),
            },
            &["VariantDef", "name", "fields", "span"],
            &["FieldDef", "Param"],
        ),
        case(
            "Param",
            Param {
                name: "p".to_string(),
                ty: TypeExpr::Unit(here()),
                span: here(),
            },
            &["Param", "name", "ty", "span"],
            &["FieldDef", "VariantDef"],
        ),
        case(
            "Block",
            empty_block(here()),
            &["Block", "stmts", "span"],
            &["Program"],
        ),
    ]);
}

#[test]
fn debug_names_every_type_expr_variant() {
    run(vec![
        case(
            "TypeExpr::Name",
            TypeExpr::Name("i64".to_string(), here()),
            &["Name"],
            &["Option", "Result", "Tuple", "Unit"],
        ),
        case(
            "TypeExpr::Option",
            TypeExpr::Option(Box::new(TypeExpr::Unit(here())), here()),
            &["Option"],
            &["Result", "Tuple"],
        ),
        case(
            "TypeExpr::Result",
            TypeExpr::Result(
                Box::new(TypeExpr::Unit(here())),
                Box::new(TypeExpr::Unit(here())),
                here(),
            ),
            &["Result"],
            &["Option", "Tuple"],
        ),
        case(
            "TypeExpr::Tuple",
            // Never empty: `()` is `Unit`. A tuple is built with at least one
            // element, which is the shape the parser produces.
            TypeExpr::Tuple(vec![TypeExpr::Unit(here())], here()),
            &["Tuple"],
            &["Option", "Result"],
        ),
        case(
            "TypeExpr::Unit",
            TypeExpr::Unit(here()),
            &["Unit"],
            &["Name", "Option", "Result", "Tuple"],
        ),
    ]);
}

/// R4: `()` is [`TypeExpr::Unit`] and never an empty [`TypeExpr::Tuple`].
///
/// Two spellings of one type would make `PartialEq` call two identical types
/// different and would make the AST golden dumps drift, so the distinction is
/// pinned here rather than left to whoever writes the parser.
#[test]
fn unit_is_not_an_empty_tuple_and_a_tuple_is_never_empty() {
    let span = here();

    assert_ne!(
        TypeExpr::Unit(span),
        TypeExpr::Tuple(Vec::new(), span),
        "`()` is Unit, never an empty Tuple"
    );
    assert_ne!(
        TypeExpr::Tuple(Vec::new(), span),
        TypeExpr::Unit(span),
        "and the comparison is symmetric"
    );

    // Their `Debug` output differs too, so a golden dump shows which one a
    // parser actually built rather than hiding the difference behind `()`.
    assert_ne!(
        format!("{:?}", TypeExpr::Unit(span)),
        format!("{:?}", TypeExpr::Tuple(Vec::new(), span))
    );

    // A one-element tuple is a different type again, so the two cannot be
    // confused by a parser that gets the parens wrong.
    let one = TypeExpr::Tuple(vec![TypeExpr::Unit(span)], span);
    assert_ne!(one, TypeExpr::Unit(span));
    assert_ne!(one, TypeExpr::Tuple(Vec::new(), span));

    // Element order is part of the type.
    let a = TypeExpr::Tuple(vec![TypeExpr::Name("i64".to_string(), span)], span);
    let b = TypeExpr::Tuple(vec![TypeExpr::Name("f64".to_string(), span)], span);
    assert_ne!(a, b);
    assert_eq!(a.clone(), a);
}

/// R2: an `extern` block keeps its link name and every declaration under it.
///
/// `Task 4` rejects `extern` with a diagnostic -- the blocks arrive in M5 -- so
/// nothing constructs this variant yet. It is declared now so that M5 does not
/// have to change a contract four tasks code against, and it is built here so
/// that adding it did not add something that cannot even be built.
#[test]
fn extern_block_keeps_its_link_name_and_declarations() {
    let mut spans = Spans::default();
    // No span is taken for the link name: `link` is a bare `String` in the
    // frozen shape, so a diagnostic about an unknown ABI points at the block.
    let ty_span = spans.take();
    let param_span = spans.take();
    let ret_span = spans.take();
    let decl_span = spans.take();
    let bare_span = spans.take();
    let block_span = spans.take();

    let item = Item::Extern {
        link: Some("C".to_string()),
        decls: vec![
            ExternDecl {
                name: "puts".to_string(),
                params: vec![Param {
                    name: "s".to_string(),
                    ty: TypeExpr::Name("string".to_string(), ty_span),
                    span: param_span,
                }],
                ret: Some(TypeExpr::Name("i64".to_string(), ret_span)),
                span: decl_span,
            },
            ExternDecl {
                name: "now".to_string(),
                params: Vec::new(),
                ret: None,
                span: bare_span,
            },
        ],
        span: block_span,
    };

    let debug = format!("{item:?}");
    // Every span in the block is the span it was given, so a diagnostic about
    // one declaration does not point at another.
    for span in [
        ty_span, param_span, ret_span, decl_span, bare_span, block_span,
    ] {
        let needle = format!("{span:?}");
        let found = debug.matches(&needle).count();
        assert_eq!(found, 1, "{needle:?} must appear exactly once in {debug}");
    }

    // The link name is kept as written, and an absent one is not an empty
    // string: `extern "C"` and `extern fn` are different blocks.
    assert!(
        debug.contains("link: Some(\"C\")"),
        "the link name must be kept: {debug}"
    );
    let unlinked = Item::Extern {
        link: None,
        decls: Vec::new(),
        span: block_span,
    };
    assert_ne!(
        item, unlinked,
        "a block without a link name is a different block"
    );

    // The declarations are ordered and distinct; two blocks that differ in one
    // declaration are different blocks.
    let mut other = item.clone();
    if let Item::Extern { decls, .. } = &mut other {
        decls.push(ExternDecl {
            name: "extra".to_string(),
            params: Vec::new(),
            ret: None,
            span: Span::SYNTHETIC,
        });
    }
    assert_ne!(item, other, "an extra declaration is a different block");
}

/// R3: `if d := f(); d > 5.0` keeps the binding as its own `init` node.
///
/// The binding is not smuggled into the comparison's left operand, because a
/// type checker and a code generator both need to find it, and a shape that
/// hides it has to be rediscovered by each of them.
#[test]
fn an_if_binding_is_its_own_init_node() {
    let mut spans = Spans::default();
    let call_span = spans.take();
    let binding_span = spans.take();
    let lhs_span = spans.take();
    let rhs_span = spans.take();
    let cond_span = spans.take();
    let then_span = spans.take();
    let if_span = spans.take();

    let stmt = Stmt::If {
        init: Some(Box::new(Stmt::Const {
            name: "d".to_string(),
            ty: None,
            init: Expr::Call {
                callee: vec!["f".to_string()],
                args: Vec::new(),
                span: call_span,
            },
            span: binding_span,
        })),
        cond: Expr::Binary {
            op: BinOp::Gt,
            lhs: bx(Expr::Ident(vec!["d".to_string()], lhs_span)),
            rhs: bx(Expr::Literal(Literal::F64(5.0), rhs_span)),
            span: cond_span,
        },
        then_block: empty_block(then_span),
        else_block: None,
        span: if_span,
    };

    let debug = format!("{stmt:?}");

    // The binding is a node of its own, with the name the source wrote.
    assert!(
        debug.contains("init: Some(Const { name: \"d\""),
        "the binding must be a distinct init node: {debug}"
    );
    assert!(
        debug.contains("callee: [\"f\"]"),
        "the binding initialises from the call the source wrote: {debug}"
    );
    // The condition is an ordinary comparison that reads the bound name.
    assert!(
        debug.contains("cond: Binary { op: Gt, lhs: Ident([\"d\"],"),
        "the condition must be an ordinary comparison: {debug}"
    );
    assert!(
        debug.contains("rhs: Literal(F64(5.0)"),
        "against the literal the source wrote: {debug}"
    );
    // The binding is not repeated inside the condition: the name appears in
    // the init node and in the condition's operand, and nowhere else.
    assert_eq!(
        debug.matches("name: \"d\"").count(),
        1,
        "the name is declared once, in init: {debug}"
    );

    // Every span survives to the tree, once each.
    for span in [
        call_span,
        binding_span,
        lhs_span,
        rhs_span,
        cond_span,
        then_span,
        if_span,
    ] {
        let needle = format!("{span:?}");
        let found = debug.matches(&needle).count();
        assert_eq!(found, 1, "{needle:?} must appear exactly once in {debug}");
    }

    // An `if` that binds is not an `if` that does not, and an `if` whose
    // binding initialises from something else is not this one.
    let unbound = Stmt::If {
        init: None,
        cond: Expr::Binary {
            op: BinOp::Gt,
            lhs: bx(Expr::Ident(vec!["d".to_string()], lhs_span)),
            rhs: bx(Expr::Literal(Literal::F64(5.0), rhs_span)),
            span: cond_span,
        },
        then_block: empty_block(then_span),
        else_block: None,
        span: if_span,
    };
    assert_ne!(stmt, unbound, "a binding is part of the statement's value");
    assert_eq!(unbound.clone(), unbound);

    let other_binding = Stmt::If {
        init: Some(Box::new(Stmt::Const {
            name: "d".to_string(),
            ty: None,
            init: Expr::Call {
                callee: vec!["g".to_string()],
                args: Vec::new(),
                span: call_span,
            },
            span: binding_span,
        })),
        cond: Expr::Binary {
            op: BinOp::Gt,
            lhs: bx(Expr::Ident(vec!["d".to_string()], lhs_span)),
            rhs: bx(Expr::Literal(Literal::F64(5.0), rhs_span)),
            span: cond_span,
        },
        then_block: empty_block(then_span),
        else_block: None,
        span: if_span,
    };
    assert_ne!(stmt, other_binding, "what the binding calls is part of it");
}

#[test]
fn debug_names_every_stmt_variant_and_its_fields() {
    run(vec![
        case(
            "Stmt::Expr",
            Stmt::Expr(int(1, here()), here()),
            &["Expr", "Literal", "I64", "start", "end"],
            &[
                "Const",
                "Var",
                "Return",
                "If",
                "ForRange",
                "ForC",
                "Switch",
                "Break",
                "Continue",
                "Defer",
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Stmt::Const",
            Stmt::Const {
                name: "c".to_string(),
                ty: None,
                init: int(1, here()),
                span: here(),
            },
            &["Const", "name", "ty", "init", "span"],
            &[
                "Var", "Return", "If", "ForRange", "ForC", "Switch", "Break", "Continue", "Defer",
                "Binary", "Call",
            ],
        ),
        case(
            "Stmt::Var",
            Stmt::Var {
                name: "v".to_string(),
                ty: None,
                init: int(1, here()),
                span: here(),
            },
            &["Var", "name", "ty", "init", "span"],
            &[
                "Const", "Return", "If", "ForRange", "ForC", "Switch", "Break", "Continue",
                "Defer", "Binary", "Call",
            ],
        ),
        case(
            "Stmt::Return",
            Stmt::Return(Some(int(1, here())), here()),
            &["Return", "start", "end"],
            &[
                "Const", "Var", "If", "ForRange", "ForC", "Switch", "Break", "Continue", "Defer",
                "Binary", "Call",
            ],
        ),
        case(
            "Stmt::Return(None)",
            Stmt::Return(None, here()),
            &["Return", "None"],
            &[
                "Const", "Var", "If", "ForRange", "ForC", "Switch", "Break", "Continue", "Defer",
            ],
        ),
        case(
            "Stmt::If",
            Stmt::If {
                init: None,
                cond: Expr::Literal(Literal::Bool(true), here()),
                then_block: empty_block(here()),
                else_block: None,
                span: here(),
            },
            &["If", "init", "cond", "then_block", "else_block", "span"],
            &[
                "Const", "Var", "Return", "ForRange", "ForC", "Switch", "Break", "Continue",
                "Defer", "Binary", "Call",
            ],
        ),
        case(
            "Stmt::ForRange",
            Stmt::ForRange {
                name: "i".to_string(),
                start: int(0, here()),
                end: int(1, here()),
                body: empty_block(here()),
                span: here(),
            },
            &["ForRange", "name", "start", "end", "body", "span"],
            &[
                "Const", "Var", "Return", "If", "ForC", "Switch", "Break", "Continue", "Defer",
                "Binary", "Call",
            ],
        ),
        case(
            "Stmt::ForC",
            Stmt::ForC {
                init: None,
                cond: None,
                update: None,
                body: empty_block(here()),
                span: here(),
            },
            &["ForC", "init", "cond", "update", "body", "span"],
            &[
                "Const", "Var", "Return", "If", "ForRange", "Switch", "Break", "Continue", "Defer",
                "Binary", "Call",
            ],
        ),
        case(
            "Stmt::Switch",
            Stmt::Switch {
                subject: int(0, here()),
                arms: Vec::new(),
                span: here(),
            },
            &["Switch", "subject", "arms", "span"],
            &[
                "Const",
                "Var",
                "Return",
                "If",
                "ForRange",
                "ForC",
                "Break",
                "Continue",
                "Defer",
                "SwitchExpr",
            ],
        ),
        case(
            "Stmt::Break",
            Stmt::Break(here()),
            &["Break", "start", "end"],
            &["Continue", "Return", "Defer"],
        ),
        case(
            "Stmt::Continue",
            Stmt::Continue(here()),
            &["Continue", "start", "end"],
            &["Break", "Return", "Defer"],
        ),
        case(
            "Stmt::Defer",
            Stmt::Defer {
                body: empty_block(here()),
                span: here(),
            },
            &["Defer", "body", "span"],
            &[
                "Const", "Var", "Return", "If", "ForRange", "ForC", "Switch", "Break", "Continue",
            ],
        ),
        case(
            "SwitchArm",
            SwitchArm {
                pattern: Pattern::Wildcard(here()),
                body: int(0, here()),
                is_default: true,
                span: here(),
            },
            &["SwitchArm", "pattern", "body", "is_default", "span"],
            &["Break", "Continue", "Return"],
        ),
    ]);
}

#[test]
fn debug_names_every_pattern_variant_and_its_fields() {
    run(vec![
        case(
            "Pattern::Wildcard",
            Pattern::Wildcard(here()),
            &["Wildcard", "start", "end"],
            &["UnitVariant", "TupleVariant", "StructVariant", "Binding"],
        ),
        case(
            "Pattern::UnitVariant",
            Pattern::UnitVariant {
                path: vec!["Red".to_string()],
                span: here(),
            },
            &["UnitVariant", "path", "span"],
            &["Wildcard", "TupleVariant", "StructVariant", "Binding"],
        ),
        case(
            "Pattern::TupleVariant",
            Pattern::TupleVariant {
                path: vec!["RGB".to_string()],
                args: Vec::new(),
                span: here(),
            },
            &["TupleVariant", "path", "args", "span"],
            &["Wildcard", "UnitVariant", "StructVariant", "Binding"],
        ),
        case(
            "Pattern::StructVariant",
            Pattern::StructVariant {
                path: vec!["RGB".to_string()],
                fields: Vec::new(),
                span: here(),
            },
            &["StructVariant", "path", "fields", "span"],
            &["Wildcard", "UnitVariant", "TupleVariant", "Binding"],
        ),
        case(
            "Pattern::Binding",
            Pattern::Binding {
                name: "x".to_string(),
                span: here(),
            },
            &["Binding", "name", "span"],
            &["Wildcard", "UnitVariant", "TupleVariant", "StructVariant"],
        ),
        case(
            "Pattern::Literal",
            Pattern::Literal(Literal::Bool(true), here()),
            &["Literal", "Bool", "start", "end"],
            &[
                "Wildcard",
                "UnitVariant",
                "TupleVariant",
                "StructVariant",
                "Binding",
            ],
        ),
    ]);
}

#[test]
fn debug_names_every_literal_variant() {
    run(vec![
        case(
            "Literal::I64",
            Literal::I64(7),
            &["I64"],
            &["F64", "Bool", "Str"],
        ),
        case(
            "Literal::F64",
            Literal::F64(1.5),
            &["F64"],
            &["I64", "Bool", "Str"],
        ),
        case(
            "Literal::Bool",
            Literal::Bool(true),
            &["Bool"],
            &["I64", "F64", "Str"],
        ),
        case(
            "Literal::Str",
            Literal::Str(text("hi")),
            &["Str", "value", "bytes"],
            &["I64", "F64", "Bool"],
        ),
    ]);
}

#[test]
fn debug_names_every_expr_variant_and_its_fields() {
    run(vec![
        case(
            "Expr::Literal",
            Expr::Literal(Literal::I64(1), here()),
            &["Literal", "I64", "start", "end"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::Ident",
            Expr::Ident(vec!["a".to_string(), "b".to_string()], here()),
            &["Ident", "start", "end"],
            &[
                "Literal",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::Binary",
            Expr::Binary {
                op: BinOp::Add,
                lhs: bx(int(1, here())),
                rhs: bx(int(2, here())),
                span: here(),
            },
            &["Binary", "op", "Add", "lhs", "rhs", "span"],
            &[
                "Ident",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::Unary",
            Expr::Unary {
                op: UnOp::Neg,
                operand: bx(int(1, here())),
                span: here(),
            },
            &["Unary", "op", "Neg", "operand", "span"],
            &[
                "Ident",
                "Binary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::Call",
            Expr::Call {
                callee: vec!["f".to_string()],
                args: Vec::new(),
                span: here(),
            },
            &["Call", "callee", "args", "span"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::StructLit",
            Expr::StructLit {
                path: vec!["P".to_string()],
                fields: Vec::new(),
                span: here(),
            },
            &["StructLit", "path", "fields", "span"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "FieldAccess",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::FieldAccess",
            Expr::FieldAccess {
                base: bx(int(1, here())),
                field: "x".to_string(),
                span: here(),
            },
            &["FieldAccess", "base", "field", "span"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "Cast",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::Cast",
            Expr::Cast {
                expr: bx(int(1, here())),
                ty: TypeExpr::Name("f64".to_string(), here()),
                span: here(),
            },
            &["Cast", "expr", "ty", "span"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Try",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::Try",
            Expr::Try {
                expr: bx(int(1, here())),
                span: here(),
            },
            &["Try", "expr", "span"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "SwitchExpr",
            ],
        ),
        case(
            "Expr::SwitchExpr",
            Expr::SwitchExpr {
                arms: Vec::new(),
                span: here(),
            },
            &["SwitchExpr", "arms", "span"],
            &[
                "Ident",
                "Binary",
                "Unary",
                "Call",
                "StructLit",
                "FieldAccess",
                "Cast",
                "Try",
            ],
        ),
    ]);
}

#[test]
fn struct_variant_field_names_are_reachable_and_checked() {
    // The `Vec<(String, Pattern, Span)>` shape is the one the brief freezes, so
    // its three parts are read back by position: the rendered tuple is asserted
    // whole, which pins the order as well as the contents.
    let mut spans = Spans::default();
    let binding_span = spans.take();
    let field_span = spans.take();
    let path = vec!["RGB".to_string()];
    let pattern = Pattern::StructVariant {
        path: path.clone(),
        fields: vec![(
            "red".to_string(),
            Pattern::Binding {
                name: "r".to_string(),
                span: binding_span,
            },
            field_span,
        )],
        span: spans.take(),
    };

    assert_eq!(format!("{pattern:?}"), format!(
        "StructVariant {{ path: [\"RGB\"], fields: [(\"red\", Binding {{ name: \"r\", span: {binding_span:?} }}, {field_span:?})], span: {:?} }}",
        spans.used.last().copied().unwrap_or(Span::SYNTHETIC)
    ));
}

// -- operators -------------------------------------------------------------

#[test]
fn bin_op_variants_and_symbols_are_exactly_these_thirteen() {
    let expected: [(&str, BinOp, &str); 13] = [
        ("Add", BinOp::Add, "+"),
        ("Sub", BinOp::Sub, "-"),
        ("Mul", BinOp::Mul, "*"),
        ("Div", BinOp::Div, "/"),
        ("Rem", BinOp::Rem, "%"),
        ("Eq", BinOp::Eq, "=="),
        ("Ne", BinOp::Ne, "!="),
        ("Lt", BinOp::Lt, "<"),
        ("Le", BinOp::Le, "<="),
        ("Gt", BinOp::Gt, ">"),
        ("Ge", BinOp::Ge, ">="),
        ("And", BinOp::And, "&&"),
        ("Or", BinOp::Or, "||"),
    ];

    for (name, op, symbol) in expected {
        // `Debug` of a unit variant is the variant name, which is what the
        // golden tests compare against.
        assert_eq!(format!("{op:?}"), name, "{name} must debug as its own name");
        assert_eq!(op.symbol(), symbol, "{name} must be spelled {symbol:?}");
    }

    // Distinct variants, distinct spellings: two operators sharing a symbol
    // would make every diagnostic that prints one ambiguous.
    for (i, (_, left, left_symbol)) in expected.iter().enumerate() {
        for (_, right, right_symbol) in expected.iter().skip(i.saturating_add(1)) {
            assert_ne!(left, right, "{left:?} and {right:?} must differ");
            assert_ne!(
                left_symbol, right_symbol,
                "{left:?} and {right:?} share the symbol {left_symbol:?}"
            );
        }
    }
}

#[test]
fn un_op_variants_and_symbols_are_exactly_these_two() {
    let expected: [(&str, UnOp, &str); 2] = [("Neg", UnOp::Neg, "-"), ("Not", UnOp::Not, "!")];

    for (name, op, symbol) in expected {
        assert_eq!(format!("{op:?}"), name, "{name} must debug as its own name");
        assert_eq!(op.symbol(), symbol, "{name} must be spelled {symbol:?}");
    }

    let (neg, not) = (UnOp::Neg, UnOp::Not);
    assert_ne!(neg, not);
    assert_ne!(neg.symbol(), not.symbol());
}

// -- equality and cloning --------------------------------------------------

#[test]
fn nodes_that_differ_in_any_one_field_are_not_equal() {
    let span = here();
    // Functional record update cannot reach into an enum variant, so the four
    // `Binary` nodes below are spelled out: that is what makes each one differ
    // from `base` in exactly the field named.
    let base = Expr::Binary {
        op: BinOp::Add,
        lhs: bx(int(1, Span::new(1, 2))),
        rhs: bx(int(2, Span::new(3, 4))),
        span,
    };
    assert_eq!(base, base.clone());

    let other_op = Expr::Binary {
        op: BinOp::Sub,
        lhs: bx(int(1, Span::new(1, 2))),
        rhs: bx(int(2, Span::new(3, 4))),
        span,
    };
    assert_ne!(base, other_op, "the operator is part of the value");

    let other_lhs = Expr::Binary {
        op: BinOp::Add,
        lhs: bx(int(9, Span::new(1, 2))),
        rhs: bx(int(2, Span::new(3, 4))),
        span,
    };
    assert_ne!(base, other_lhs, "the left operand is part of the value");

    let other_span = Expr::Binary {
        op: BinOp::Add,
        lhs: bx(int(1, Span::new(1, 2))),
        rhs: bx(int(2, Span::new(3, 4))),
        span: Span::new(0, 99),
    };
    assert_ne!(base, other_span, "the span is part of the value");

    // A nested span counts too: the diagnostic it feeds must be able to change.
    let other_nested = Expr::Binary {
        op: BinOp::Add,
        lhs: bx(int(1, Span::new(1, 3))),
        rhs: bx(int(2, Span::new(3, 4))),
        span,
    };
    assert_ne!(base, other_nested, "a nested span is part of the value");

    let missing_operand = Expr::Binary {
        op: BinOp::Add,
        lhs: bx(int(1, Span::new(1, 2))),
        rhs: bx(int(3, Span::new(3, 4))),
        span,
    };
    assert_ne!(
        base, missing_operand,
        "the right operand is part of the value"
    );

    assert_ne!(
        Literal::I64(1),
        Literal::F64(1.0),
        "a literal's type is part of its value"
    );
    assert_ne!(
        Expr::Ident(vec![], span),
        Expr::Ident(vec!["a".to_string()], span),
        "an identifier path is part of the value"
    );
    assert_ne!(
        Pattern::Literal(Literal::I64(1), span),
        Pattern::Literal(Literal::I64(1), Span::new(0, 1)),
        "a pattern's span is part of its value"
    );

    let arm = |is_default: bool| SwitchArm {
        pattern: Pattern::Wildcard(span),
        body: int(0, span),
        is_default,
        span,
    };
    assert_ne!(arm(false), arm(true), "is_default is part of the value");

    let stmt = |ty: Option<TypeExpr>| Stmt::Const {
        name: "c".to_string(),
        ty,
        init: int(1, span),
        span,
    };
    assert_ne!(
        stmt(None),
        stmt(Some(TypeExpr::Unit(span))),
        "an annotation is part of the value"
    );
}

#[test]
fn clone_is_deep_so_editing_a_clone_leaves_the_original_alone() {
    let mut spans = Spans::default();
    let mut program = full_program(&mut spans);
    let copy = program.clone();
    assert_eq!(program, copy);

    // Editing the clone must not reach the original or the clone it came from.
    // Both directions are checked: a shared buffer fails whichever way the
    // push happens to go first.
    let mut edited = copy.clone();
    edited.items.push(Item::Import {
        path: vec!["extra".to_string()],
        span: Span::SYNTHETIC,
    });

    assert_eq!(
        program.items.len(),
        5,
        "editing a clone must not add to the original"
    );
    assert_eq!(
        copy.items.len(),
        5,
        "editing a clone must not add to its source"
    );
    assert_ne!(program, edited);
    assert_ne!(copy, edited);
    assert_eq!(program, copy, "the two untouched trees are still equal");

    // And the other way round: editing the original must not reach the clone.
    program.items.push(Item::Import {
        path: vec!["also-extra".to_string()],
        span: Span::SYNTHETIC,
    });
    assert_eq!(
        copy.items.len(),
        5,
        "editing the original must not add to a clone taken earlier"
    );
    assert_eq!(edited.items.len(), 6);
}
