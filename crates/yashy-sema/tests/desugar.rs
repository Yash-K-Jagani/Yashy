//! The sugar proof: each Phase 5 form's HIR equals the HIR of the
//! hand-written program it replaces.
//!
//! The comparison is over the span-stripped `Debug` rendering: two programs
//! that lower to the same thing still differ in byte offsets, because the
//! sugar and its expansion are different text. Stripping every
//! `Span { start, end }` leaves the shape, the names and the types, which is
//! what "the same program" means here. A mismatch prints both shapes, so the
//! differing node is visible without dumping two whole programs by hand.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

fn check(src: &str) -> HirProgram {
    match check_inner(src) {
        Ok(hir) => hir,
        Err(diags) => panic!(
            "{src:?} must type-check, got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        ),
    }
}

fn check_inner(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("desugar.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

/// The `Debug` rendering with every span replaced by `Span`.
fn shape(hir: &HirProgram) -> String {
    // Compact, not pretty: a span only stays on one line there, which is
    // what the stripper below matches.
    let debug = format!("{hir:?}");
    strip_positions(&debug)
}

/// Replaces every `Span { start: N, end: M }` and
/// `FaultSite { file: ..., line: L, column: C }` with its bare name: both
/// carry byte positions, and two programs that lower to the same thing from
/// different text always differ in them.
fn strip_positions(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len());
    let mut rest = debug;
    loop {
        let span_at = rest.find("Span { start: ");
        let fault_at = rest.find("FaultSite { file: ");
        match (span_at, fault_at) {
            (Some(s), Some(f)) if f < s => {
                out.push_str(&rest[..f]);
                let tail = &rest[f + "FaultSite { file: ".len()..];
                match tail.find(" }") {
                    Some(end) => {
                        out.push_str("FaultSite");
                        rest = &tail[end + 2..];
                    }
                    None => {
                        out.push_str(&rest[f..]);
                        return out;
                    }
                }
            }
            (Some(s), _) => {
                out.push_str(&rest[..s]);
                let tail = &rest[s + "Span { start: ".len()..];
                match tail.find(" }") {
                    Some(end) => {
                        out.push_str("Span");
                        rest = &tail[end + 2..];
                    }
                    None => {
                        out.push_str(&rest[s..]);
                        return out;
                    }
                }
            }
            (None, Some(f)) => {
                out.push_str(&rest[..f]);
                let tail = &rest[f + "FaultSite { file: ".len()..];
                match tail.find(" }") {
                    Some(end) => {
                        out.push_str("FaultSite");
                        rest = &tail[end + 2..];
                    }
                    None => {
                        out.push_str(&rest[f..]);
                        return out;
                    }
                }
            }
            (None, None) => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Asserts the sugar and the hand-written form lower to the same HIR.
fn assert_same(name: &str, sugar: &str, expanded: &str) {
    let sugar_shape = shape(&check(sugar));
    let expanded_shape = shape(&check(expanded));
    assert_eq!(
        sugar_shape, expanded_shape,
        "{name}: the sugar and its expansion lowered differently\n\
         --- sugar ---\n{sugar_shape}\n--- hand-written ---\n{expanded_shape}"
    );
}

#[test]
fn a_struct_spread_equals_the_filled_literal() {
    assert_same(
        "struct spread",
        "struct Point { x: int, y: int } fn main() { const Point base = Point { x: 1, y: 2 }; const Point p = Point { x: 9, ..base }; }",
        "struct Point { x: int, y: int } fn main() { const Point base = Point { x: 1, y: 2 }; const Point p = Point { x: 9, y: base.y }; }",
    );
}

#[test]
fn a_field_default_equals_the_filled_literal() {
    assert_same(
        "field default",
        "struct Config { retries: int = 3, name: string } fn main() { const Config c = Config { name: \"x\" }; }",
        "struct Config { retries: int = 3, name: string } fn main() { const Config c = Config { retries: 3, name: \"x\" }; }",
    );
}

#[test]
fn a_trailing_block_equals_a_lambda_argument() {
    assert_same(
        "trailing block",
        "fn twice(int x, fn(int) -> int f) -> int { return f(f(x)); } fn main() { const int y = twice(20) { |x| return x + 1; }; }",
        "fn twice(int x, fn(int) -> int f) -> int { return f(f(x)); } fn main() { const int y = twice(20, |int x| -> int { return x + 1; }); }",
    );
}

#[test]
fn an_indexed_for_lowers_to_a_range_loop_with_a_direct_read() {
    // The hand-written read `xs[i]` is checked indexing and yields
    // `Option[int]`, so it cannot be written out; the lowering reads the
    // element directly, which is exactly why phase 4 can elide the check.
    use yashy_sema::{HirExpr, HirItem, HirStmt};
    let hir = check("fn main() { let xs = []int { 1, 2 }; for i, x in xs { print(x); } }");
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    let Some(HirStmt::ForRange { name, body, .. }) = f.body.stmts.get(1) else {
        panic!("expected a range loop, got {:?}", f.body.stmts);
    };
    assert_eq!(name, "i");
    let Some(HirStmt::Const { name, init, .. }) = body.stmts.first() else {
        panic!("expected the element read, got {:?}", body.stmts);
    };
    assert_eq!(name, "x");
    assert!(
        matches!(init, HirExpr::Index { .. }),
        "the body opens with a direct element read: {init:?}"
    );
}

#[test]
fn an_option_try_lowers_to_a_try_node() {
    use yashy_sema::{HirExpr, HirItem, HirStmt};
    let hir = check(
        "fn lookup(int k) -> Option[int] { return Some(k); } fn f() -> Option[int] { let v = lookup(1)?; return Some(v); }",
    );
    let HirItem::Fn(f) = hir.items.last().expect("the f function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    let Some(HirStmt::Var { init, .. }) = f.body.stmts.first() else {
        panic!("expected the `?` binding, got {:?}", f.body.stmts);
    };
    assert!(
        matches!(init, HirExpr::Try { .. }),
        "`?` on an `Option` is `HirExpr::Try`: {init:?}"
    );
}

#[test]
fn an_if_let_lowers_to_a_single_if_let_node() {
    // `if let` is the one form whose lowered shape is itself a node
    // (`HirStmt::IfLet`): a `switch` arm body is a single expression and
    // cannot carry a block, so there is no hand-written statement form to
    // compare against. This pins that the pattern test is shared and the
    // node carries both blocks.
    use yashy_sema::{HirItem, HirStmt};
    let hir = check(
        "fn main() { const Option[int] o = Some(1); if let Some(x) = o { print(x); } else { print(0); } }",
    );
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    assert!(
        matches!(
            f.body.stmts.get(1),
            Some(HirStmt::IfLet { then_block, else_block: Some(_), .. })
                if then_block.stmts.len() == 1
        ),
        "one `IfLet` with both blocks: {:?}",
        f.body.stmts
    );
}

#[test]
fn a_while_let_lowers_to_a_loop_with_a_breaking_else() {
    use yashy_sema::{HirItem, HirStmt};
    let hir = check(
        "fn main() { const Option[int] o = Some(1); while let Some(x) = o { print(x); } }",
    );
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    let Some(HirStmt::ForC { body, .. }) = f.body.stmts.get(1) else {
        panic!("expected the endless loop, got {:?}", f.body.stmts);
    };
    assert!(
        matches!(
            body.stmts.as_slice(),
            [HirStmt::IfLet { else_block: Some(leave), .. }]
                if matches!(leave.stmts.as_slice(), [HirStmt::Break(_) ] )
        ),
        "one `IfLet` whose else breaks: {:?}",
        body.stmts
    );
}

#[test]
fn an_array_spread_lowers_to_a_spread_marker_call() {
    // `[0, ..xs, 4]` has no source expansion (there is no concatenation
    // operator): the lowering puts a `__spread` marker call inside the
    // slice literal, which codegen copies element by element.
    use yashy_sema::{HirExpr, HirItem, HirStmt};
    let hir = check(
        "fn main() { let rest = []int { 1, 2 }; let xs = []int { 0, ..rest, 4 }; xs.free(); }",
    );
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    let Some(HirStmt::Var { init, .. }) = f.body.stmts.get(1) else {
        panic!("expected the spread binding, got {:?}", f.body.stmts);
    };
    let HirExpr::SliceLit { elements, .. } = init else {
        panic!("expected a slice literal, got {init:?}");
    };
    assert!(
        elements.iter().any(|element| matches!(
            element,
            HirExpr::Call { callee, .. } if callee == &vec!["__spread".to_string()]
        )),
        "the literal carries a `__spread` marker: {elements:?}"
    );
}

#[test]
fn a_labeled_loop_lowers_to_flag_statements() {
    // The flag form, spelled out: mutable `let` flags, `flag = true; break;`
    // at the jump, and a flag check at the end of the labeled loop's body.
    // Full HIR equality is brittle here (the lowering inserts propagation
    // checks after nested loops), so this pins the flag statements
    // themselves; the labeled-loop suite pins the behavior end to end.
    use yashy_sema::{HirItem, HirStmt, Mutability};
    let hir = check(
        "fn main() { outer: for i in 0..3 { for j in 0..3 { break outer; } } }",
    );
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    fn walk<'a>(stmt: &'a HirStmt, out: &mut Vec<&'a HirStmt>) {
        out.push(stmt);
        match stmt {
            HirStmt::If { init, then_block, .. } => {
                if let Some(init) = init {
                    walk(init, out);
                }
                for s in &then_block.stmts {
                    walk(s, out);
                }
            }
            HirStmt::ForRange { body, .. } | HirStmt::ForIn { body, .. } | HirStmt::ForC { body, .. } => {
                for s in &body.stmts {
                    walk(s, out);
                }
            }
            _ => {}
        }
    }
    let mut all = Vec::new();
    for stmt in &f.body.stmts {
        walk(stmt, &mut all);
    }
    assert!(
        all.iter().any(|s| matches!(
            s,
            HirStmt::Var { name, mutability: Mutability::Var, .. }
                if name == "__brk_outer" || name == "__cont_outer"
        )),
        "mutable flags are declared: {all:?}"
    );
    assert!(
        all.iter().any(|s| matches!(
            s,
            HirStmt::Assign { name, .. } if name == "__brk_outer"
        )),
        "the jump sets the break flag: {all:?}"
    );
}

#[test]
fn an_arena_block_lowers_to_an_if_true_with_the_arena_bit() {
    use yashy_sema::{HirExpr, HirItem, HirStmt};
    let hir = check("fn main() { arena { let buf = alloc(16); buf[0] = 1; } }");
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    let Some(HirStmt::If { cond, then_block, .. }) = f.body.stmts.first() else {
        panic!("expected the `if true`, got {:?}", f.body.stmts);
    };
    assert!(
        matches!(cond, HirExpr::Literal(yashy_sema::Literal::Bool(true), ..)),
        "`arena {{ }}` is an `if` on a literal `true`: {cond:?}"
    );
    assert!(then_block.arena, "the block carries the arena bit");
}

#[test]
fn a_chained_comparison_lowers_to_nested_binding_switches() {
    // The hand-written expansion is not spellable in source: a binding arm
    // is not exhaustive for exhaustiveness purposes (only an unguarded
    // `default` is), so the nested switches the lowering builds cannot be
    // written out. This pins the shape instead: three nested switches
    // binding `__c0`, `__c1`, `__c2` once each, over `__c0 <= __c1 && __c1 < __c2`.
    use yashy_sema::{HirExpr, HirItem, HirPattern, HirStmt};
    let hir = check("fn main() { const int x = 5; const b = 0 <= x < 10; }");
    let HirItem::Fn(f) = hir.items.as_slice().first().expect("one function") else {
        panic!("expected a function, got {:?}", hir.items);
    };
    let Some(HirStmt::Const { init, .. }) = f.body.stmts.get(1) else {
        panic!("expected the chain binding, got {:?}", f.body.stmts);
    };
    let mut depth = 0;
    let mut current = init;
    loop {
        match current {
            HirExpr::SwitchExpr { arms, .. } => {
                depth += 1;
                assert_eq!(arms.len(), 1);
                let arm = &arms[0];
                assert!(arm.is_default);
                assert!(
                    matches!(&arm.pattern, HirPattern::Binding { name, .. } if name == &format!("__c{}", depth - 1)),
                    "switch {depth} binds the wrong name: {:?}",
                    arm.pattern
                );
                current = &arm.body;
            }
            other => {
                assert_eq!(depth, 3, "three operands, three switches");
                assert!(
                    matches!(
                        other,
                        HirExpr::Binary { op: yashy_sema::BinOp::And, .. }
                    ),
                    "the innermost body joins the comparisons with `&&`: {other:?}"
                );
                break;
            }
        }
    }
}
