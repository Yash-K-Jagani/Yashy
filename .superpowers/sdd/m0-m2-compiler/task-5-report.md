# Task 5 Report

## Created
- `crates/otter-parser/src/stmts.rs` — statement parsing + real `parse_block`.
- `crates/otter-parser/src/exprs.rs` — full expression parser (defined here per brief; Task 6 inherits).
- Wired both modules in `lib.rs`.

## Seam for Task 6
`parse_expr` is fully implemented in `exprs.rs` (precedence climbing). Task 6 tests inherit it; no second parser needed.

## Test output
`cargo test --workspace`: all pass. `cargo clippy --workspace --all-targets`: clean.

## Mutation checks
Replaced empty-body assertion in `the_block_seam_consumes...` with real `Stmt::Return` assertion; verified red before implementation.

## Commit
d52783a7589120fb741c035bbfc6368b17a92c67

## Concerns
- Switch expression subject is consumed but discarded because AST `SwitchExpr` has no subject field (frozen).
- Deeply nested delimiter test adjusted to run in a larger stack thread to avoid overflow; not a parser change.
## Fix round 1
- Changed Expr::SwitchExpr to SwitchExpr { subject: Box<Expr>, arms, span } in st.rs.
- exprs.rs: retained subject instead of discarding; expr_span already used ...
- Updated test construction sites in 	ests/parse.rs (full_expr_tail and debug field list) and added subject to expected debug fields.
- Added switch_expression_retains_subject test in 	ests/items.rs (bare ident x and complex .b).
- Fixed struct-literal ambiguity so a bare subject followed by { (switch arms) is not misread as a struct literal: only treat {...} as struct literal when it looks like {} or { ident: ... }. Added peek_third helper.
- Mutation check: temporarily made parser store a dummy i64 subject; covering test failed (red), restored.
- Gates: cargo test --workspace green; cargo clippy --workspace --all-targets clean.
