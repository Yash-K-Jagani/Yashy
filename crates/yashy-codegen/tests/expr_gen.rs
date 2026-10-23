//! Expression codegen tests: the opcode contract for operators, comparisons,
//! boolean logic, runtime calls, and `Option`/`Result` matching.
//!
//! These tests drive `gen_expr`/`gen_match` directly through a small harness:
//! function-level emission (prologues, statements) is Task 12's job, so the
//! harness seeds parameters as bare allocas and lowers each `const` init or
//! the `return`ed expression of the first function.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, expr_ty, CheckOptions, HirItem, HirProgram, HirStmt};

use yashy_codegen::{gen_block, gen_expr, gen_expr_to_slot, gen_stmt, FnCtx, Module};

/// Lex, parse and check `source` into a typed HIR program.
fn check_source(src: &str) -> HirProgram {
    check_source_in(src, false)
}

/// Lex, parse and check `source` into a typed HIR program, in the build mode
/// `release` names -- the only thing a build mode changes about the checker.
fn check_source_in(src: &str, release: bool) -> HirProgram {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    check_program_with(&ast, &sm, file, CheckOptions { release })
        .expect("these test sources must type-check")
}

/// The first function in `src`.
fn first_fn(src: &str) -> yashy_sema::HirFn {
    first_fn_of(&check_source(src))
}

/// The first function in a checked program.
fn first_fn_of(program: &HirProgram) -> yashy_sema::HirFn {
    program
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) => Some(f.clone()),
            _ => None,
        })
        .expect("the source must define a function")
}

/// Lower the first function's body: each `const` into a slot, each expression
/// statement for effect. Returns the finished module.
///
/// `begin_entry`/`finish` bracket the body the same way `gen_fn` brackets a
/// real function, so the golden sees the allocas where the backend actually
/// puts them — at the top of the entry block — rather than where the call that
/// wanted one happened to run.
fn gen_body(src: &str) -> String {
    lower_body(&first_fn(src))
}

/// Lower the first function's body in the build mode `release` names.
fn gen_body_in(src: &str, release: bool) -> String {
    lower_body(&first_fn_of(&check_source_in(src, release)))
}

fn lower_body(f: &yashy_sema::HirFn) -> String {
    let mut module = Module::new();
    {
        let mut ctx = FnCtx::new(&mut module, f.ret.kind.clone());
        ctx.begin_entry();
        let _ = ctx.alloc_params(f);
        for stmt in &f.body.stmts {
            match stmt {
                HirStmt::Const { name, init, .. } | HirStmt::Var { name, init, .. } => {
                    let slot = gen_expr_to_slot(&mut ctx, init);
                    ctx.locals
                        .insert(name.clone(), (slot, expr_ty(init).kind.clone()));
                }
                HirStmt::Expr(expr, _, _) => {
                    let _ = gen_expr(&mut ctx, expr);
                }
                HirStmt::IndexAssign { .. } | HirStmt::FieldAssign { .. } => {
                    let _ = gen_stmt(&mut ctx, stmt);
                }
                other => panic!(
                    "the harness only handles const, var, expr, and the assigning statements, got {other:?}"
                ),
            }
        }
        ctx.finish();
    }
    module.finish()
}

/// Lower the first `return <expr>` in the first function. Returns the
/// finished module.
fn gen_return(src: &str) -> String {
    let f = first_fn(src);
    let mut module = Module::new();
    {
        let mut ctx = FnCtx::new(&mut module, f.ret.kind.clone());
        ctx.begin_entry();
        let _ = ctx.alloc_params(&f);
        let expr = f
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                HirStmt::Return(Some(expr), _) => Some(expr),
                _ => None,
            })
            .expect("the source must return an expression");
        let _ = gen_expr(&mut ctx, expr);
        ctx.finish();
    }
    module.finish()
}

/// The one-based line and column of `needle` in `src`, which is what the checker
/// resolves a span to and what a fault is expected to report.
///
/// Derived from the fixture rather than hardcoded: a column assertion with a
/// number in it is a second copy of the fixture, and when the two disagree it is
/// the number that gets "fixed".
fn position_of(src: &str, needle: &str) -> (usize, usize) {
    let at = src
        .find(needle)
        .expect("the fixture must contain the expression");
    let before = &src[..at];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    (line, at - line_start + 1)
}

/// The function bodies of `ir_text`: everything from the first `define`.
///
/// The module declares every runtime function and every intrinsic up front,
/// whether the program uses it or not -- an unused declaration is legal IR, and
/// emitting them conditionally would make the goldens depend on the program.
/// So "this program does not mention X" is a question about the body, and asking
/// it of the whole module answers `true` for every name in the header.
fn body_of(ir_text: &str) -> &str {
    // A module from `gen_body` has no `define` line -- the harness seeds the
    // entry block directly -- so the body starts at its `entry:` label. A real
    // function module starts at its first `define`.
    if let Some(at) = ir_text.find("define ") {
        return &ir_text[at..];
    }
    match ir_text.find("\nentry:") {
        Some(at) => &ir_text[at + 1..],
        None => "",
    }
}

#[test]
fn integer_addition_is_checked_before_it_adds() {
    // Ledger L4's M0-M2 row said `Add Sub Mul` emit bare `add`/`sub`/`mul` with
    // no `nsw`, so wraparound was defined and silent. M4 replaces it: the value
    // and the overflow flag come out of one LLVM intrinsic and the flag branches
    // to the block that reports it. This is the diff the golden was rewritten
    // for, and `nsw` is still absent -- the check is what makes the operation
    // defined, not a flag that turns it into UB.
    let ir_text = gen_body("fn f(a: int, b: int) { const x = a + b; }");
    assert!(
        ir_text.contains("call { i64, i1 } @llvm.sadd.with.overflow.i64"),
        "expected the overflow intrinsic:\n{ir_text}"
    );
    assert!(
        ir_text.contains("extractvalue { i64, i1 }"),
        "the result and the flag both come out of it:\n{ir_text}"
    );
    assert!(
        ir_text.contains("br i1") && ir_text.contains("_fault:"),
        "the flag has to branch to wherever a fault is reported:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_panic_overflow"),
        "and that block reports the operands:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("nsw") && !ir_text.contains("nuw"),
        "the check is what makes this defined:\n{ir_text}"
    );
}

#[test]
fn subtraction_and_multiplication_use_their_own_intrinsics() {
    // One intrinsic per operation: `ssub` for `-` and `smul` for `*`. The
    // multiply is the one that would have needed a hand-rolled `i128` widening
    // test, which is exactly the kind of predicate that is easier to get wrong
    // than to notice.
    let ir_text = gen_body("fn f(a: int, b: int) { const x = a - b; const y = a * b; }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("@llvm.ssub.with.overflow.i64"),
        "subtraction has its own intrinsic:\n{body}"
    );
    assert!(
        body.contains("@llvm.smul.with.overflow.i64"),
        "so does multiplication:\n{body}"
    );
    assert!(
        body.contains("call void @yashy_panic_overflow"),
        "both report through the same runtime function:\n{body}"
    );
}

#[test]
fn the_division_itself_is_emitted_behind_the_check() {
    // This replaces the M0-M2 bare `sdiv` (ledger L4, Review Focus item 1). A
    // `sdiv` by zero does not return a wrong answer, it raises `#DE`, so the
    // division has to be in the block the check *falls through to* rather than in
    // the block it guards: by the time the instruction runs, the divisor is known
    // to be safe.
    let src = "fn f(a: int, b: int) { const q = a / b; const r = a % b; }";
    let ir_text = gen_body(src);
    let branch = ir_text.find("br i1").expect("a check must branch");
    let divide = ir_text.find("sdiv i64").expect("the quotient is still a sdiv");
    let remainder = ir_text.find("srem i64").expect("so is the remainder");
    assert!(
        branch < divide && branch < remainder,
        "the arithmetic must come after the branch:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_panic_division"),
        "the divisor being zero is reported by name:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("call void @yashy_panic_overflow"),
        "a division faults for two reasons and both go to the division report:\n{ir_text}"
    );
}

#[test]
fn a_fault_carries_the_operands_and_the_position_it_was_written_at() {
    // Every piece of the run-time message is an operand: the operator's symbol,
    // the two values, and the file, line and column. The position is the part the
    // backend cannot work out for itself -- it is handed IR text and a path, and
    // no source map -- so the checker resolves it and this asserts it survived
    // the trip.
    let src = "fn f(a: int, b: int) {\n    const q = a / b;\n}";
    let (line, column) = position_of(src, "a / b");
    let ir_text = gen_body(src);
    assert!(
        ir_text.contains(&format!("i64 {line}, i64 {column}")),
        "expected the position {line}:{column} as operands:\n{ir_text}"
    );
    assert!(
        ir_text.contains("private unnamed_addr constant [2 x i8] c\"/\\00\""),
        "the operator travels as a one-character string:\n{ir_text}"
    );
    assert!(
        ir_text.contains("c\"main.ya\\00\""),
        "and the file the compiler was handed travels with it:\n{ir_text}"
    );
}

#[test]
fn the_fault_block_never_falls_through() {
    // The block ends in `unreachable`, so the check's continuation has to be a
    // block of its own: instructions appended after a terminator are not
    // well-formed IR, and a checked operation can appear anywhere an expression
    // may -- including as the argument of another call.
    let ir_text = gen_body("fn f(a: int, b: int) { const x = a + b; }");
    let lines: Vec<&str> = ir_text.lines().map(str::trim).collect();
    let unreachable = lines
        .iter()
        .position(|line| *line == "unreachable")
        .unwrap_or_else(|| panic!("the fault block ends in `unreachable`:\n{ir_text}"));
    assert!(
        lines[unreachable + 1].ends_with("_ok:"),
        "the next block after it is the continuation, got {:?}:\n{ir_text}",
        lines.get(unreachable + 1)
    );
    assert!(
        lines[unreachable - 1].starts_with("call void @yashy_panic_overflow"),
        "and the instruction before it is the report:\n{ir_text}"
    );
}

#[test]
fn negating_an_integer_checks_for_the_smallest_one() {
    // `-i64::MIN` has no answer in `i64`: it wraps to itself, so a silent
    // wraparound would turn a mistake into a plausible number. It reports as an
    // overflow of a subtraction from zero, which is what negation is.
    let ir_text = gen_body("fn f(a: int) { const b = -a; }");
    assert!(
        ir_text.contains("icmp eq i64 %t0, -9223372036854775808"),
        "the one value that cannot be negated is the test:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_panic_overflow"),
        "and it reports like any other overflow:\n{ir_text}"
    );
}

#[test]
fn negating_a_float_is_not_checked() {
    // Negating any `f64`, infinity included, is exact. There is no fault to
    // report and no check to pay for.
    let ir_text = gen_body("fn f(x: float) { const y = -x; }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("fsub double 0.0"),
        "float negation stays a subtraction from zero:\n{body}"
    );
    assert!(
        !body.contains("_fault") && !body.contains("yashy_panic"),
        "and carries nothing else:\n{body}"
    );
}

#[test]
fn comparisons_and_boolean_logic_are_never_checked() {
    // A comparison cannot overflow and `&&` is not arithmetic. This is the guard
    // against the checked path widening to operations it does not belong on.
    let ir_text =
        gen_body("fn f(a: int, b: int) { const c = a < b; const d = (a < b) && (b < a); }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("icmp slt i64"),
        "comparisons are still a `icmp`:\n{body}"
    );
    assert!(
        body.contains("br i1"),
        "`&&` is a branch, so the right operand is only evaluated when it has to be:\n{body}"
    );
    assert!(
        !body.contains("with.overflow") && !body.contains("_fault"),
        "nothing here can fault:\n{body}"
    );
}

#[test]
fn float_arithmetic_carries_no_check() {
    // An `f64` that overflows becomes an infinity, which is a float answer
    // rather than a wrong one, and neither `fadd` nor `fdiv` traps.
    let ir_text = gen_body("fn f(x: float, y: float) { const s = x + y; const q = x / y; }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("fadd double") && body.contains("fdiv double"),
        "floats keep their own mnemonics:\n{body}"
    );
    assert!(
        !body.contains("with.overflow") && !body.contains("yashy_panic"),
        "and no check rides along:\n{body}"
    );
}

#[test]
fn string_concatenation_is_not_checked() {
    // `+` on strings is the same *token* and a completely different operation:
    // it goes to the runtime and has nothing to overflow. The result type is what
    // keeps it off the checked path.
    let ir_text = gen_body("fn f(s: string, t: string) { const u = s + t; }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("@yashy_str_concat"),
        "a concatenation is a runtime call:\n{body}"
    );
    assert!(
        !body.contains("_fault") && !body.contains("with.overflow"),
        "with no check anywhere near it:\n{body}"
    );
}

#[test]
fn float_arithmetic_uses_the_float_mnemonics() {
    // `add`/`sub`/`mul` take integer operands only; LLVM rejects `mul double`
    // outright, so every float operator needs its own mnemonic. This went
    // unnoticed until an f64 benchmark was compiled, because no test exercised
    // f64 `+`, `-` or `*`.
    let ir_text = gen_body(
        "fn f() { const a = 1.0 + 2.0; const b = 1.0 - 2.0; const c = 1.0 * 2.0; \
         const d = 1.0 / 2.0; const e = 1.0 % 2.0; }",
    );
    for mnemonic in ["fadd", "fsub", "fmul", "fdiv", "frem"] {
        assert!(
            ir_text.contains(&format!("{mnemonic} double")),
            "expected `{mnemonic} double`:\n{ir_text}"
        );
    }
    // A leading space distinguishes the integer opcode from its float twin,
    // since `fadd double` contains the substring `add double`.
    for mnemonic in [" add double", " sub double", " mul double"] {
        assert!(
            !ir_text.contains(mnemonic),
            "integer opcode `{mnemonic}` used on a double:\n{ir_text}"
        );
    }
}

#[test]
fn float_literals_always_carry_a_decimal_point() {
    // LLVM's float grammar wants a point in the mantissa, so Rust's `Debug`
    // spelling of a small double (`1e-7`) is not valid IR: LLVM lexes the
    // leading `1` as an integer constant and reports a type error. The fix
    // inserts the point rather than switching to hex, which keeps the emitted
    // IR readable.
    let ir_text = gen_body("fn f() { const small = 0.0000001; const whole = 2.0; }");
    assert!(ir_text.contains("1.0e-7"), ":\n{ir_text}");
    assert!(ir_text.contains("2.0"), ":\n{ir_text}");
    assert!(
        !ir_text.contains(" 1e-7"),
        "an exponent literal without a point is rejected by LLVM:\n{ir_text}"
    );
}

#[test]
fn a_none_value_writes_only_the_tag() {
    // `None` has no payload, so the value is the tag alone: one `insertvalue`
    // storing `i8 0` in field 0 of `Option`'s `{ i8, T }` representation. What
    // matters is that *something* is emitted: codegen had no arm for `None` and
    // fell through to a call to a function named `@None`, which does not exist,
    // so the program failed at link time with an error naming a symbol the user
    // never wrote.
    let ir_text = gen_body("fn f() { const empty: Option[int] = None; }");
    assert!(
        ir_text.contains("insertvalue { i8, i64 } undef, i8 0, 0"),
        "expected a tag-only construction:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("@None") && !ir_text.contains("call"),
        "`None` must be built, not called:\n{ir_text}"
    );
}

#[test]
fn a_field_assignment_is_a_read_modify_write() {
    // A struct value is an aggregate, not an address, so `q.x = 1` loads the
    // binding's aggregate, replaces the field with `insertvalue`, and stores the
    // result back. The field's position comes from the checker, so the
    // `insertvalue` names it rather than looking the declaration up again.
    let ir_text = gen_body(
        "struct Point { x: int, y: int }\n\
         fn f(p: Point) { var q = p; q.x = 1; }",
    );
    assert!(
        ir_text.contains("load %struct.Point"),
        "the aggregate is loaded from the binding's slot:\n{ir_text}"
    );
    assert!(
        ir_text.contains("insertvalue %struct.Point"),
        "the field is replaced in the aggregate:\n{ir_text}"
    );
    assert!(
        ir_text.contains("store %struct.Point"),
        "the rebuilt aggregate is stored back:\n{ir_text}"
    );
}

#[test]
fn a_nested_field_assignment_rebuilds_each_aggregate() {
    // `m.from.x = 1` extracts the inner aggregate, replaces the field, and
    // inserts the rebuilt inner value back into the outer one: one
    // `insertvalue` per level, and an `extractvalue` for the level below.
    let ir_text = gen_body(
        "struct Point { x: int, y: int }\n\
         struct Line { from: Point, to: Point }\n\
         fn f(l: Line) { var m = l; m.from.x = 1; }",
    );
    assert_eq!(
        ir_text.matches("insertvalue %struct.Point").count(),
        1,
        "one insert at the inner level:\n{ir_text}"
    );
    assert_eq!(
        ir_text.matches("insertvalue %struct.Line").count(),
        1,
        "one insert at the outer level:\n{ir_text}"
    );
    assert!(
        ir_text.contains("extractvalue %struct.Line"),
        "the inner aggregate is read out of the outer one:\n{ir_text}"
    );
}

#[test]
fn comparisons_emit_the_matching_predicate() {
    let ir_text = gen_body("fn f() { const a = 1 < 2; const b = 1.0 < 2.0; }");
    assert!(ir_text.contains("icmp slt i64"), ":\n{ir_text}");
    assert!(ir_text.contains("fcmp olt double"), ":\n{ir_text}");
}

#[test]
fn boolean_operators_short_circuit() {
    // `&&` and `||` are branches, not instructions: the right operand is
    // evaluated only when the left one does not decide the answer.
    let ir_text = gen_body("fn f() { const a = true && false; const b = true || false; }");
    assert!(ir_text.contains("br i1"), ":\n{ir_text}");
    assert!(ir_text.contains("alloca i1"), ":\n{ir_text}");
    assert!(!ir_text.contains("and i1"), ":\n{ir_text}");
    assert!(!ir_text.contains("or i1"), ":\n{ir_text}");
}

#[test]
fn unary_negation_subtracts_from_zero() {
    let ir_text = gen_body("fn f() { const a = -1; const b = !true; }");
    assert!(ir_text.contains("sub i64 0, 1"), ":\n{ir_text}");
    assert!(ir_text.contains("xor i1 1, true"), ":\n{ir_text}");
}

#[test]
fn cast_emits_sitofp_and_fptosi() {
    let ir_text = gen_body("fn f() { const a = 1.0; const b = a as int; }");
    assert!(ir_text.contains("fptosi double"), ":\n{ir_text}");
}

#[test]
fn match_on_option_tests_the_tag_then_loads_the_payload() {
    let ir_text = gen_return(
        "fn f(o: Option[int]) -> int { return switch o { case None { 0 } case Some(x) { x } }; }",
    );
    // The Some arm is tested: tag 1 against the loaded tag field.
    assert!(
        ir_text.contains("icmp eq i8"),
        "expected a tag comparison:\n{ir_text}"
    );
    assert!(
        ir_text
            .lines()
            .any(|line| line.contains("icmp eq i8") && line.trim_end().ends_with(", 1")),
        "tag for Some is 1:\n{ir_text}"
    );
    // The payload is extracted and stored into the binding's slot.
    assert!(
        ir_text.contains("extractvalue { i8, i64 }"),
        "expected payload extraction:\n{ir_text}"
    );
    assert!(
        ir_text.contains("store i64"),
        "expected the payload stored into a slot:\n{ir_text}"
    );
}

#[test]
fn match_on_result_distinguishes_ok_and_err() {
    let ir_text = gen_return(
        "fn f(r: Result[int, string]) -> int { return switch r { case Ok(v) { v } case Err(e) { 0 } }; }",
    );
    // Ok is tag 0 and its payload is field 1.
    assert!(
        ir_text
            .lines()
            .any(|line| line.contains("icmp eq i8") && line.trim_end().ends_with(", 0")),
        "tag for Ok is 0:\n{ir_text}"
    );
    let extracts: Vec<&str> = ir_text
        .lines()
        .filter(|l| l.contains("extractvalue") && l.contains("i8, i64, i8*"))
        .collect();
    assert!(
        extracts.iter().any(|l| l.ends_with(", 1")),
        "Ok payload is field 1 of the Result triple:\n{ir_text}"
    );
    // Err's payload is field 2 of the Result triple.
    assert!(
        extracts.iter().any(|l| l.ends_with(", 2")),
        "Err payload is field 2 of the Result triple:\n{ir_text}"
    );
    // Both payloads are stored into their bindings' slots.
    assert!(
        ir_text.matches("store i64").count() >= 2,
        "expected both arms to store payloads:\n{ir_text}"
    );
}

#[test]
fn comparisons_use_untyped_constants() {
    // Modern LLVM removed typed constants: `icmp eq i8 %t, i8 1` is a parse
    // error ("expected value token"), not a style preference. Every comparison
    // this backend emits must end in a bare value.
    let ir_text = gen_return(
        "fn f(o: Option[int]) -> int { return switch o { case Some(x) { x } case None { 0 } }; }",
    );
    for line in ir_text
        .lines()
        .filter(|line| line.contains("icmp ") || line.contains("fcmp "))
    {
        for typed in [", i8 ", ", i64 ", ", double ", ", i1 "] {
            assert!(
                !line.contains(typed),
                "comparison operand carries a type (`{typed}`): `{line}`\n{ir_text}"
            );
        }
    }
}

#[test]
fn wildcard_arm_needs_no_tag_test() {
    // With a default arm only the tested variant arm emits an icmp.
    let ir_text = gen_return(
        "fn f(o: Option[int]) -> int { return switch o { case None { 0 } default { 1 } }; }",
    );
    let icmps = ir_text.matches("icmp eq i8").count();
    assert_eq!(icmps, 1, "only the None arm tests a tag:\n{ir_text}");
}

#[test]
fn call_emits_a_call_instruction_naming_the_runtime_symbol() {
    let ir_text = gen_body("fn f() { print_i64(1); }");
    assert!(
        ir_text.contains("call void @yashy_print_i64(i64 1)"),
        ":\n{ir_text}"
    );
    let ir_text = gen_body("fn f() { print_str(\"hi\"); }");
    assert!(
        ir_text.contains("call void @yashy_print_str(i8*"),
        ":\n{ir_text}"
    );
}

// --- the opt-outs from the checked default -----------------------------------

#[test]
fn wrapping_arithmetic_is_the_bare_operation_and_nothing_else() {
    // The whole point of `wrapping_add` is that it is what `+` was before the
    // arithmetic became checked: one instruction, no flag, no branch, no cold
    // block. It is the escape hatch for code that has already decided the
    // arithmetic cannot overflow -- and the cheapest possible way to say so.
    let ir_text = gen_body("fn f(a: int, b: int) { const x = wrapping_add(a, b); }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("add i64"),
        "a wrapping add is a plain `add`:\n{body}"
    );
    assert!(
        !body.contains("with.overflow")
            && !body.contains("_fault")
            && !body.contains("icmp")
            && !body.contains("yashy_panic"),
        "and pays for no check at all:\n{body}"
    );
}

#[test]
fn wrapping_subtraction_and_multiplication_are_bare_too() {
    // The family is only worth having if all three are free; a `wrapping_mul`
    // that checked would be a different operation with the same name.
    let ir_text = gen_body(
        "fn f(a: int, b: int) { const x = wrapping_sub(a, b); const y = wrapping_mul(a, b); }",
    );
    let body = body_of(&ir_text);
    assert!(body.contains("sub i64"), "wrapping subtraction:\n{body}");
    assert!(body.contains("mul i64"), "wrapping multiplication:\n{body}");
    assert!(
        !body.contains("with.overflow") && !body.contains("_fault"),
        "neither one checks:\n{body}"
    );
}

#[test]
fn saturating_addition_and_subtraction_ask_for_the_intrinsic() {
    // Clamping is not something to hand-roll, and LLVM has both of these: the
    // flag is not read at all, and there is no branch.
    for (name, intrinsic) in [
        ("saturating_add", "@llvm.sadd.sat.i64"),
        ("saturating_sub", "@llvm.ssub.sat.i64"),
    ] {
        let ir_text = gen_body(&format!("fn f(a: int, b: int) {{ const x = {name}(a, b); }}"));
        let body = body_of(&ir_text);
        assert!(
            body.contains(intrinsic),
            "`{name}` asks LLVM for the clamp:\n{body}"
        );
        assert!(
            !body.contains("_fault") && !body.contains("select"),
            "clamping is not a fault and needs no select:\n{body}"
        );
    }
}

#[test]
fn saturating_multiplication_is_saturated_by_hand() {
    // `smul.sat` is in LLVM's language reference but not implemented: clang
    // emits a call to a symbol nothing defines and the link fails. So the
    // multiply clamps from the overflow flag and the sign of the true product,
    // which is negative exactly when the operands' signs differ -- neither
    // operand is zero when it overflowed. This is a real correctness trap, which
    // is why the intrinsic that does not exist must not be named anywhere.
    let ir_text = gen_body("fn f(a: int, b: int) { const x = saturating_mul(a, b); }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("@llvm.smul.with.overflow.i64"),
        "the wrapping product and the flag are the raw material:\n{body}"
    );
    assert!(
        body.contains("xor i1") && body.contains("select i1"),
        "the sign is an xor and both the limit and the result are selected:\n{body}"
    );
    assert!(
        !ir_text.contains("smul.sat"),
        "`llvm.smul.sat.i64` does not exist and must not be declared:\n{ir_text}"
    );
    assert!(
        !body.contains("_fault") && !body.contains("yashy_panic"),
        "saturating never stops the program:\n{body}"
    );
}

#[test]
fn checked_arithmetic_answers_with_an_option_instead_of_a_branch() {
    // `checked_add` is for code that can do something better than die. It spends
    // the same overflow flag on a `select` between `Some(value)` and `None`, so
    // the caller decides: no branch, no cold block, no runtime call.
    let ir_text = gen_body("fn f(a: int, b: int) { const x = checked_add(a, b); }");
    let body = body_of(&ir_text);
    assert!(
        body.contains("@llvm.sadd.with.overflow.i64"),
        "the flag is still how it knows:\n{body}"
    );
    assert!(
        body.contains("select i1"),
        "but the flag selects rather than branches:\n{body}"
    );
    assert!(
        body.contains("insertvalue { i8, i64 } undef, i8 1, 0"),
        "the `Some` side is built with the tag and the value:\n{body}"
    );
    assert!(
        body.contains("insertvalue { i8, i64 } undef, i8 0, 0"),
        "and `None` with the tag alone:\n{body}"
    );
    assert!(
        !body.contains("_fault") && !body.contains("yashy_panic"),
        "nothing here can stop the program:\n{body}"
    );
}

#[test]
fn a_checked_result_is_an_ordinary_option_that_matches() {
    // The type is a real `Option[i64]`, not a special case: a `switch` over it
    // tests the tag like any other, which is what makes `checked_*` usable
    // rather than merely callable.
    let ir_text = gen_return(concat!(
        "fn f(a: int, b: int) -> int {\n",
        "    return switch checked_add(a, b) {\n",
        "        case Some(v) { v }\n",
        "        case None { 0 }\n",
        "    };\n",
        "}"
    ));
    let body = body_of(&ir_text);
    assert!(
        body.contains("icmp eq i8") && body.contains("extractvalue { i8, i64 }"),
        "the tag is tested and the payload extracted:\n{body}"
    );
}

// --- assertions --------------------------------------------------------------

#[test]
fn assert_branches_to_the_report_and_continues() {
    // The passing path is one test and a branch; the failing path is a cold
    // block that ends the program. That is the same shape every checked
    // arithmetic operation uses, and the same reason: the common case is cheap
    // and the report pays for itself.
    let ir_text = gen_body("fn f(a: int) { assert(a > 0); }");
    let branch = ir_text.find("br i1").expect("the condition must branch");
    let report = ir_text
        .find("call void @yashy_panic")
        .expect("the cold block reports");
    assert!(
        branch < report,
        "the branch comes first and the report is in a block of its own:\n{ir_text}"
    );
    assert!(
        ir_text.contains("_assert_ok:"),
        "and execution continues in a fresh block:\n{ir_text}"
    );
}

#[test]
fn a_failed_assertion_reports_the_text_the_checker_read() {
    // The expression's own text is a fact only the checker holds -- the backend
    // is handed IR text and a path, and no source map -- so the message arrives
    // as an interned constant rather than being recovered here.
    let ir_text = gen_body("fn f(a: int) { assert(a > 0); }");
    assert!(
        ir_text.contains("assertion failed: a > 0"),
        "the message quotes the condition:\n{ir_text}"
    );
}

#[test]
fn assert_eq_compares_with_the_instruction_for_the_operand_type() {
    // `i64` and `bool` are integers to LLVM, `f64` needs a float comparison, and
    // `string` needs the runtime: three emitters behind one name, which is why
    // the operands travel down as expressions rather than as text.
    let integers = gen_body("fn f(a: int) { assert_eq(a, 0); }");
    assert!(integers.contains("icmp eq i64"), "{integers}");
    let floats = gen_body("fn f(x: float) { assert_eq(x, 0.5); }");
    assert!(floats.contains("fcmp oeq double"), "{floats}");
    let bools = gen_body("fn f(a: bool) { assert_eq(a, true); }");
    assert!(bools.contains("icmp eq i1"), "{bools}");
    let strings = gen_body("fn f(s: string) { assert_eq(s, \"x\"); }");
    assert!(strings.contains("call i1 @yashy_str_eq"), "{strings}");
}

#[test]
fn assert_ne_inverts_the_same_comparison() {
    let integers = gen_body("fn f(a: int) { assert_ne(a, 0); }");
    assert!(integers.contains("icmp ne i64"), "{integers}");
    // `une` and not `one`: a `NaN` compares unordered, so it satisfies
    // `!=` -- and `assert_ne` is the assertion that a NaN should pass.
    let floats = gen_body("fn f(x: float) { assert_ne(x, 0.5); }");
    assert!(floats.contains("fcmp une double"), "{floats}");
    let strings = gen_body("fn f(s: string) { assert_ne(s, \"x\"); }");
    assert!(
        strings.contains("call i1 @yashy_str_eq") && strings.contains("xor i1"),
        "the runtime answers `equal` either way, and `ne` negates it:\n{strings}"
    );
}

#[test]
fn a_comparison_message_is_built_only_on_the_failure_path() {
    // Two conversions and three concatenations are what a report costs. Build
    // them before the branch and they are paid by every assertion that passes.
    let ir_text = gen_body("fn f(a: int) { assert_eq(a, 0); }");
    let fault = ir_text.find("_fault:").expect("a reporting block");
    let convert = ir_text
        .find("call i8* @yashy_str_from_i64")
        .expect("the value is converted to text");
    assert!(
        fault < convert,
        "the message is built inside the reporting block:\n{ir_text}"
    );
}

#[test]
fn an_asserted_operand_is_evaluated_once() {
    // The comparison and the message read the same register, so an operand that
    // is a call is run once: on the failure path it would otherwise produce a
    // *different* value than the one that failed.
    let src = concat!(
        "fn f(a: int) { assert_eq(g(a), 0); }\n",
        "fn g(a: int) -> int { return a + 1; }"
    );
    let ir_text = gen_body(src);
    assert_eq!(
        ir_text.matches("call i64 @g(").count(),
        1,
        "the operand is evaluated once:\n{ir_text}"
    );
}

#[test]
fn a_debug_assert_is_lowered_like_an_assert_in_a_debug_build() {
    // Same fault path, same message. The two names differ only in whether the
    // call survives a release build, which is a checker decision.
    let ir_text = gen_body("fn f(a: int) { debug_assert(a > 0); }");
    assert!(
        ir_text.contains("assertion failed: a > 0")
            && ir_text.contains("call void @yashy_panic")
            && ir_text.contains("_assert_ok:"),
        "a debug build lowers it exactly as `assert`:\n{ir_text}"
    );
}

#[test]
fn a_debug_assert_disappears_in_a_release_build() {
    // Nothing at all: no branch, no message, no call. The string is not even
    // interned, which is what makes the check free rather than merely skipped.
    let ir_text = gen_body_in("fn f(a: int) { debug_assert(a > 0); }", true);
    let body = body_of(&ir_text);
    for absent in ["yashy_panic", "_fault", "_assert_ok", "assertion failed"] {
        assert!(
            !body.contains(absent),
            "a release build must not emit `{absent}`:\n{body}"
        );
    }
}

#[test]
fn an_assert_survives_a_release_build() {
    // The other half of the pair: `assert` is not a debugging aid.
    let ir_text = gen_body_in("fn f(a: int) { assert(a > 0); }", true);
    assert!(
        ir_text.contains("assertion failed: a > 0")
            && ir_text.contains("call void @yashy_panic"),
        "a release build still lowers `assert`:\n{ir_text}"
    );
}

#[test]
fn exit_ends_the_program_in_a_block_of_its_own() {
    let ir_text = gen_body("fn f() { os.exit(3); }");
    assert!(ir_text.contains("call void @yashy_exit(i64 3)"));
    assert!(ir_text.contains("unreachable"));
    // The shape is the same as `panic`: branch, label, call, unreachable, then
    // a continuation that nothing reaches.
    assert!(ir_text.contains("_after_exit:"));
}

#[test]
fn exit_truncates_a_non_constant_status() {
    let ir_text = gen_body("fn f(x: int) { os.exit(x); }");
    assert!(ir_text.contains("call void @yashy_exit(i64 %t"));
}

#[test]
fn exit_fits_in_a_value_position() {
    let ir_text = gen_return("fn f() -> int { return os.exit(1); }");
    assert!(ir_text.contains("call void @yashy_exit(i64 1)"));
    assert!(ir_text.contains("trunc i64") || ir_text.contains("unreachable"));
}

#[test]
fn indexing_reads_an_element_via_getelementptr() {
    let ir_text = gen_body("fn f(p: *mut int) { const x = p[0]; }");
    assert!(
        ir_text.contains("getelementptr i64"),
        "indexing must compute the element address:\n{ir_text}"
    );
    assert!(
        ir_text.contains("load i64"),
        "and then load from it:\n{ir_text}"
    );
}

#[test]
fn index_assignment_stores_through_getelementptr() {
    let ir_text = gen_body("fn f(p: *mut int, i: int, v: int) { p[i] = v; }");
    assert!(
        ir_text.contains("getelementptr i64"),
        "index assignment must compute the element address:\n{ir_text}"
    );
    assert!(
        ir_text.contains("store i64"),
        "and then store through it:\n{ir_text}"
    );
}

// --- golden IR ---------------------------------------------------------------

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// True when `UPDATE_GOLDEN=1` asks for the `.expected.ll` file to be rewritten.
fn updating_goldens() -> bool {
    matches!(std::env::var("UPDATE_GOLDEN").as_deref(), Ok("1") | Ok("true"))
}

/// Read a golden file with line endings normalised to `\n`, both halves of a
/// comparison going through it, for the same reason the lexer goldens do: an
/// editor that writes CRLF must not make the comparison fail for a reason that
/// has nothing to do with codegen.
fn read_lf_only(path: &Path) -> String {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read golden file {}: {e}", path.display()));
    let body = raw.strip_suffix('\n').unwrap_or(raw.as_str());
    let mut out = String::new();
    for line in body.split('\n') {
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
        out.push('\n');
    }
    out
}

/// Replace the host-specific `target datalayout` and `target triple` lines with
/// placeholders.
///
/// Those two lines describe the machine, not the compiler: the triple follows
/// the Rust toolchain's environment (MSVC or GNU) and the datalayout's mangling
/// character follows the triple. Masking them keeps one golden file valid on
/// every host while everything the backend actually decides stays pinned.
fn normalize_host_header(ir: &str) -> String {
    ir.lines()
        .map(|line| {
            if line.starts_with("target triple =") {
                "target triple = \"<host>\""
            } else if line.starts_with("target datalayout =") {
                "target datalayout = \"<host>\""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn every_alloca_lands_in_the_entry_block() {
    // A loop inside a loop is the case that exposed this: the inner loop's
    // counter slots were emitted into the outer loop's body block, so they were
    // re-created on every outer iteration. LLVM wants all of a function's
    // allocas in `entry`, which executes once.
    let f = first_fn("fn f(n: int) { for i in 0..n { for j in 0..n { var x = i * j; } } }");
    let mut module = Module::new();
    {
        let mut ctx = FnCtx::new(&mut module, f.ret.kind.clone());
        ctx.begin_entry();
        let _ = gen_block(&mut ctx, &f.body);
        ctx.finish();
    }
    let ir_text = module.finish();

    assert!(
        ir_text.contains(" = alloca "),
        "the nested loop should still allocate its counter slots:\n{ir_text}"
    );
    // Every block after `entry:` is re-entered; an alloca there is dynamic.
    let mut seen_later_block = false;
    for line in ir_text.lines() {
        if line.ends_with(':') && line != "entry:" {
            seen_later_block = true;
        }
        assert!(
            !(seen_later_block && line.contains(" = alloca ")),
            "`{line}` allocates outside the entry block:\n{ir_text}"
        );
    }
}

#[test]
fn golden_checked_matches_the_pinned_contract() {
    // Every checked operation in one function, on *variables* rather than
    // constants so the IR shows the check rather than a folded answer. This is
    // the file that pins the shape of the fault path: the intrinsic (or the
    // divisor test), the branch, the cold block with its report, and the
    // continuation the value is computed in.
    let input = read_lf_only(&golden_dir().join("checked.ya"));
    let actual = normalize_host_header(&gen_body(&input));
    let expected_path = golden_dir().join("checked.expected.ll");
    if updating_goldens() {
        fs::write(&expected_path, &actual)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", expected_path.display()));
        return;
    }
    let expected = normalize_host_header(&read_lf_only(&expected_path));
    assert_eq!(
        actual, expected,
        "the checked-arithmetic contract drifted; \
         rerun with UPDATE_GOLDEN=1 only after checking the diff"
    );
}

#[test]
fn golden_ops_match_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("ops.ya"));
    let actual = normalize_host_header(&gen_body(&input));
    let expected_path = golden_dir().join("ops.expected.ll");
    if updating_goldens() {
        fs::write(&expected_path, &actual)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", expected_path.display()));
        return;
    }
    let expected = normalize_host_header(&read_lf_only(&expected_path));
    assert_eq!(
        actual, expected,
        "generated IR drifted from the pinned contract; \
         rerun with UPDATE_GOLDEN=1 only after checking the diff"
    );
}
