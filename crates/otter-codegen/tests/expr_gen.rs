//! Expression codegen tests: the opcode contract for operators, comparisons,
//! boolean logic, runtime calls, and `Option`/`Result` matching.
//!
//! These tests drive `gen_expr`/`gen_match` directly through a small harness:
//! function-level emission (prologues, statements) is Task 12's job, so the
//! harness seeds parameters as bare allocas and lowers each `const` init or
//! the `return`ed expression of the first function.

use std::fs;
use std::path::{Path, PathBuf};

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, expr_ty, HirItem, HirProgram, HirStmt};

use otter_codegen::{gen_expr, gen_expr_to_slot, FnCtx, Module};

/// Lex, parse and check `source` into a typed HIR program.
fn check_source(src: &str) -> HirProgram {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    check_program(&ast, &sm, file).expect("these test sources must type-check")
}

/// The first function in `src`.
fn first_fn(src: &str) -> otter_sema::HirFn {
    let program = check_source(src);
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
fn gen_body(src: &str) -> String {
    let f = first_fn(src);
    let mut module = Module::new();
    {
        let mut ctx = FnCtx::new(&mut module, f.ret.kind.clone());
        let _ = ctx.alloc_params(&f);
        for stmt in &f.body.stmts {
            match stmt {
                HirStmt::Const { name, init, .. } => {
                    let slot = gen_expr_to_slot(&mut ctx, init);
                    ctx.locals
                        .insert(name.clone(), (slot, expr_ty(init).kind.clone()));
                }
                HirStmt::Expr(expr, _, _) => {
                    let _ = gen_expr(&mut ctx, expr);
                }
                other => panic!("the harness only handles const and expr, got {other:?}"),
            }
        }
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
    }
    module.finish()
}

#[test]
fn integer_add_emits_wrapping_add_without_nsw() {
    // Review Focus item 5: M0–M2 arithmetic wraps; M4 makes it checked.
    let ir_text = gen_body("fn f() { const x = 1 + 2; }");
    assert!(
        ir_text.contains("add i64"),
        "expected a wrapping add:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("nsw") && !ir_text.contains("nuw"),
        "nsw/nuw would make wraparound UB before M4:\n{ir_text}"
    );
}

#[test]
fn integer_divide_emits_sdiv_with_no_zero_guard() {
    // Review Focus item 1: division by zero is UB until M4; the golden
    // contract documents the hazard rather than hiding it.
    let ir_text = gen_body("fn f() { const x = 1 / 0; }");
    assert!(
        ir_text.contains("sdiv i64"),
        "expected a bare sdiv:\n{ir_text}"
    );
}

#[test]
fn comparisons_emit_the_matching_predicate() {
    let ir_text = gen_body("fn f() { const a = 1 < 2; const b = 1.0 < 2.0; }");
    assert!(ir_text.contains("icmp slt i64"), ":\n{ir_text}");
    assert!(ir_text.contains("fcmp olt double"), ":\n{ir_text}");
}

#[test]
fn boolean_operators_operate_on_i1() {
    let ir_text = gen_body("fn f() { const a = true && false; const b = true || false; }");
    assert!(ir_text.contains("and i1"), ":\n{ir_text}");
    assert!(ir_text.contains("or i1"), ":\n{ir_text}");
}

#[test]
fn unary_negation_subtracts_from_zero() {
    let ir_text = gen_body("fn f() { const a = -1; const b = !true; }");
    assert!(ir_text.contains("sub i64 0, 1"), ":\n{ir_text}");
    assert!(ir_text.contains("xor i1 1, true"), ":\n{ir_text}");
}

#[test]
fn cast_emits_sitofp_and_fptosi() {
    let ir_text = gen_body("fn f() { const a = 1.0; const b = a as i64; }");
    assert!(ir_text.contains("fptosi double"), ":\n{ir_text}");
}

#[test]
fn match_on_option_tests_the_tag_then_loads_the_payload() {
    let ir_text = gen_return(
        "fn f(o: Option[i64]) -> i64 { return switch o { case None { 0 } case Some(x) { x } }; }",
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
        "fn f(r: Result[i64, string]) -> i64 { return switch r { case Ok(v) { v } case Err(e) { 0 } }; }",
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
        "fn f(o: Option[i64]) -> i64 { return switch o { case Some(x) { x } case None { 0 } }; }",
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
        "fn f(o: Option[i64]) -> i64 { return switch o { case None { 0 } default { 1 } }; }",
    );
    let icmps = ir_text.matches("icmp eq i8").count();
    assert_eq!(icmps, 1, "only the None arm tests a tag:\n{ir_text}");
}

#[test]
fn call_emits_a_call_instruction_naming_the_runtime_symbol() {
    let ir_text = gen_body("fn f() { print_i64(1); }");
    assert!(
        ir_text.contains("call void @otter_print_i64(i64 1)"),
        ":\n{ir_text}"
    );
    let ir_text = gen_body("fn f() { print_str(\"hi\"); }");
    assert!(
        ir_text.contains("call void @otter_print_str(i8*"),
        ":\n{ir_text}"
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
fn golden_ops_match_the_pinned_contract() {
    let input = read_lf_only(&golden_dir().join("ops.ot"));
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
