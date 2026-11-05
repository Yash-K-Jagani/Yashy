//! Escape analysis, re-exported from `yashy-hir`.
//!
//! The verifier owns the arena findings (`arena_findings`) and codegen owns
//! the placement decision, so both read one walk. Keeping a copy here would
//! let the two disagree, which is exactly the miscompile the shared module
//! exists to prevent.

pub use yashy_hir::escape::*;

#[cfg(test)]
mod tests {
    use yashy_common::{ProofFacts, SourceMap};
    use yashy_lexer::Lexer;
    use yashy_parser::Parser;
    use yashy_sema::{check_program, HirExpr, HirFn, HirItem};

    use crate::escape::{classify, AllocSite, Placement};

    fn function(source: &str, name: &str) -> HirFn {
        let mut sources = SourceMap::new();
        let file = sources.add_file("escape.ya", source.to_string());
        let tokens = Lexer::new(source).tokenize().expect("the fixture must lex");
        let mut parser = Parser::new(tokens, &sources, file);
        let program = parser.parse_program().expect("the fixture must parse");
        let hir = check_program(&program, &sources, file).expect("the fixture must check");
        hir.items
            .iter()
            .find_map(|item| match item {
                HirItem::Fn(f) if f.name == name => Some(f.clone()),
                _ => None,
            })
            .expect("the fixture must define the function")
    }

    fn site(f: &HirFn) -> AllocSite {
        fn find(expr: &HirExpr) -> Option<AllocSite> {
            match expr {
                HirExpr::Call {
                    callee, args, span, ..
                } => {
                    if callee.last().map(String::as_str) == Some("alloc") {
                        return Some(AllocSite::of(*span));
                    }
                    args.iter().find_map(find)
                }
                HirExpr::Move(inner, ..) | HirExpr::Cast { expr: inner, .. } => find(inner),
                HirExpr::Binary { lhs, rhs, .. } => find(lhs).or_else(|| find(rhs)),
                HirExpr::Unary { operand, .. } => find(operand),
                HirExpr::StructLit { fields, .. } => fields.iter().find_map(|(_, v, _)| find(v)),
                HirExpr::SliceLit { elements, .. }
                | HirExpr::FixedArrayLit { elements, .. }
                | HirExpr::TupleLit { elements, .. } => elements.iter().find_map(find),
                HirExpr::FieldAccess { base, .. } | HirExpr::Index { base, .. } => find(base),
                HirExpr::View { base, .. } => find(base),
                HirExpr::SwitchExpr { subject, arms, .. } => {
                    find(subject).or_else(|| arms.iter().find_map(|arm| find(&arm.body)))
                }
                HirExpr::HigherOrder { receiver, init, .. } => {
                    find(receiver).or_else(|| init.as_deref().and_then(find))
                }
                HirExpr::CallCapture { args, .. } => args.iter().find_map(find),
                _ => None,
            }
        }

        fn find_stmt(stmt: &yashy_sema::HirStmt) -> Option<AllocSite> {
            match stmt {
                yashy_sema::HirStmt::Const { init, .. }
                | yashy_sema::HirStmt::Var { init, .. }
                | yashy_sema::HirStmt::Destructure { init, .. } => find(init),
                yashy_sema::HirStmt::Expr(expr, ..) => find(expr),
                yashy_sema::HirStmt::Assign { value, .. } => find(value),
                yashy_sema::HirStmt::Return(Some(expr), _) => find(expr),
                _ => None,
            }
        }

        f.body
            .stmts
            .iter()
            .find_map(find_stmt)
            .expect("the fixture must allocate")
    }

    #[test]
    fn a_constant_allocation_that_never_escapes_goes_on_the_stack() {
        let f = function(
            "fn f() -> int { let p = alloc(8); p[0] = 1; let v = p[0]; p.free(); return v; }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert_eq!(classify(&f, &facts).get(&site(&f)), Some(&Placement::Stack));
    }

    #[test]
    fn a_returned_pointer_stays_on_the_heap() {
        let f = function("fn f() -> *mut int { let p = alloc(8); return p; }", "f");
        let facts = ProofFacts::empty();
        assert_eq!(classify(&f, &facts).get(&site(&f)), Some(&Placement::Heap));
    }

    #[test]
    fn a_dynamic_allocation_bound_in_a_block_uses_its_arena() {
        let f = function(
            "fn f(int n) { let p = alloc(n); p[0] = 1; p.free(); }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert!(matches!(
            classify(&f, &facts).get(&site(&f)),
            Some(Placement::Arena(_))
        ));
    }

    #[test]
    fn a_dynamic_assignment_to_an_outer_binding_stays_on_the_heap() {
        let f = function(
            "fn f(int n) { let p = alloc(1); if n > 0 { p = alloc(n); } p.free(); }",
            "f",
        );
        let facts = ProofFacts::empty();
        // The first allocation is a declaration of `p` in the body, so it is
        // constant and stack-placed. The second is an assignment inside a
        // nested block, but the binding outlives that block, so the dynamic one
        // is not arena-placed.
        let placements = classify(&f, &facts);
        assert!(placements
            .values()
            .any(|placement| *placement == Placement::Heap));
    }

    #[test]
    fn a_large_constant_allocation_uses_the_block_arena() {
        let f = function("fn f() { let p = alloc(1024); p[0] = 1; p.free(); }", "f");
        let facts = ProofFacts::empty();
        assert!(matches!(
            classify(&f, &facts).get(&site(&f)),
            Some(Placement::Arena(_))
        ));
    }

    #[test]
    fn an_allocation_handed_to_a_function_stays_on_the_heap() {
        let f = function(
            "fn consume(*mut int p) { p[0] = 1; }\n\
             fn f() { let p = alloc(8); consume(p); }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert_eq!(classify(&f, &facts).get(&site(&f)), Some(&Placement::Heap));
    }

    #[test]
    fn a_binding_read_through_is_not_an_escape() {
        let f = function(
            "fn f() -> int { let p = alloc(8); p[0] = 1; let v = p[0]; p.free(); return v; }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert!(matches!(
            classify(&f, &facts).get(&site(&f)),
            Some(Placement::Stack)
        ));
    }

    #[test]
    fn an_allocation_inside_an_arena_block_uses_the_block_arena() {
        let f = function(
            "fn f() -> int { arena { let p = alloc(8); p[0] = 1; return p[0]; } }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert!(
            classify(&f, &facts).values().any(|placement| matches!(placement, Placement::Arena(_))),
            "every allocation inside the block must use its arena"
        );
        let findings = crate::escape::arena_findings(&f, &facts);
        assert!(
            findings.is_empty(),
            "a value read inside the block is not an escape"
        );
    }

    #[test]
    fn an_arena_value_returned_is_reported_and_stays_on_the_heap() {
        use crate::escape::{arena_findings, ArenaFindingKind};

        let f = function("fn f() -> *mut int { arena { let p = alloc(8); return p; } }", "f");
        let facts = ProofFacts::empty();
        assert_eq!(classify(&f, &facts).get(&site(&f)), Some(&Placement::Heap));
        let findings = arena_findings(&f, &facts);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, ArenaFindingKind::Escape);
        assert_eq!(findings[0].name, "p");
    }

    #[test]
    fn an_arena_value_assigned_to_an_outer_binding_is_reported() {
        use crate::escape::{arena_findings, ArenaFindingKind};

        let f = function(
            "fn f() { let out = alloc(8); arena { let p = alloc(8); out = move p; } out.free(); }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert!(arena_findings(&f, &facts)
            .iter()
            .any(|finding| finding.kind == ArenaFindingKind::Escape));
    }

    #[test]
    fn an_arena_value_freed_individually_is_reported() {
        use crate::escape::{arena_findings, ArenaFindingKind};

        let f = function("fn f() { arena { let p = alloc(8); p.free(); } }", "f");
        let facts = ProofFacts::empty();
        let findings = arena_findings(&f, &facts);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, ArenaFindingKind::Free);
    }

    #[test]
    fn a_value_returned_from_inside_the_arena_block_is_not_an_escape() {
        use crate::escape::arena_findings;

        let f = function(
            "fn f() -> int { arena { let p = alloc(8); p[0] = 1; return p[0]; } }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert!(arena_findings(&f, &facts).is_empty());
    }

    #[test]
    fn a_break_out_of_an_arena_still_releases_it() {
        let f = function(
            "fn f() -> int { let total = 0; for i in 0..3 { arena { let p = alloc(8); p[0] = i; if i == 1 { break; } total = total + p[0]; } } total }",
            "f",
        );
        let facts = ProofFacts::empty();
        assert!(
            classify(&f, &facts).values().any(|placement| matches!(placement, Placement::Arena(_))),
            "the arena must allocate so its block exit can release it"
        );
        let findings = crate::escape::arena_findings(&f, &facts);
        assert!(
            findings.is_empty(),
            "a `break` inside the block is not an escape"
        );
    }
}
