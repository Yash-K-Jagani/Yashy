//! Codegen scaffold tests: the LLVM module header, string interning, the type
//! mapping, and `main` discovery.
//!
//! The `check_source` helper is the same lex -> parse -> check pipeline the
//! driver will run, written once here so these tests do not depend on the
//! driver existing.

use yashy_common::{Severity, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram, TypeKind};

use yashy_codegen::{check_main_exists, emit_program, find_main, llvm_type_string, CodegenOptions, Module};

/// Lex, parse and check `source` into a typed HIR program.
fn check_source(src: &str) -> HirProgram {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    check_program(&ast, &sm, file).expect("these test sources must type-check")
}

#[test]
fn finish_emits_the_opaque_pointer_header() {
    let ir = Module::new().finish();
    assert!(
        ir.contains("target datalayout"),
        "missing datalayout:\n{ir}"
    );
    assert!(ir.contains("%yashy_str = type"), "missing str type:\n{ir}");
    assert!(
        ir.contains("declare void @yashy_print_str(i8*)"),
        "the print symbols must return void per ledger L1:\n{ir}"
    );
    // The triple follows the LLVM target the *driver* will pass to `clang`, so
    // assert against that one function rather than a second copy of the rule.
    let triple = yashy_codegen::host_target_triple();
    assert!(
        ir.contains(&format!("target triple = \"{triple}\"")),
        "wrong target triple for this host:\n{ir}"
    );
}

#[test]
fn intern_string_dedupes_identical_bytes() {
    let mut module = Module::new();
    let first = module.intern_string(b"hi");
    let second = module.intern_string(b"hi");
    let third = module.intern_string(b"ho");
    assert_eq!(first, "@.str.0");
    assert_eq!(second, "@.str.0", "identical bytes must share one global");
    assert_eq!(
        third, "@.str.1",
        "different bytes must get their own global"
    );
    let ir = module.finish();
    // Two literals, four globals: a struct and a byte array each. The struct
    // is what a `string` value points at; the bytes live beside it.
    let structs = ir
        .lines()
        .filter(|line| line.starts_with("@.str.") && !line.starts_with("@.str.0.data") && !line.starts_with("@.str.1.data"))
        .count();
    assert_eq!(structs, 2, "two struct globals for three interns:\n{ir}");
    let datas = ir
        .lines()
        .filter(|line| line.contains(".data = private unnamed_addr constant"))
        .count();
    assert_eq!(datas, 2, "two byte-array globals for three interns:\n{ir}");
}

#[test]
fn string_global_is_nul_terminated_and_escaped() {
    let mut module = Module::new();
    module.intern_string(b"a\"b");
    let ir = module.finish();
    assert!(
        ir.contains("c\"a\\22b\\00\""),
        "quote must escape as \\22 and the global must end with \\00:\n{ir}"
    );
    assert!(
        ir.contains("[4 x i8]"),
        "a 3-byte string plus its NUL is 4 bytes:\n{ir}"
    );
    assert!(
        ir.contains("%yashy_str { i64 -1, i64 3, i64 3, i8* @.str.0.data }"),
        "a literal is a static string: refcount -1, length 3, capacity 3:\n{ir}"
    );
}

#[test]
fn missing_main_is_a_diagnostic_not_a_link_failure() {
    let program = check_source("fn helper() -> int { return 1; }");
    assert!(find_main(&program.items).is_none());
    let err = check_main_exists(&program.items)
        .expect_err("a program with no main must fail here, not at link time");
    assert_eq!(err.severity, Severity::Error);
    assert!(
        err.message.contains("main"),
        "the diagnostic must name main: {}",
        err.message
    );
    assert_eq!(err.span, Span::new(0, 0));
}

#[test]
fn main_may_be_absent_or_return_i64() {
    // "absent" here means the `-> T` annotation is absent: `main` returns
    // nothing.
    let unit = check_source("fn main() {}");
    let found = find_main(&unit.items).expect("main must be found");
    assert_eq!(found.name, "main");
    assert!(check_main_exists(&unit.items).is_ok());

    let i64_ret = check_source("fn main() -> int { return 0; }");
    let _found = find_main(&i64_ret.items).expect("main must be found");
    assert!(check_main_exists(&i64_ret.items).is_ok());

    let bad_ret = check_source("fn main() -> bool { return true; }");
    let err = check_main_exists(&bad_ret.items)
        .expect_err("main returning bool is not a valid entry point");
    assert!(
        err.message.contains("i64") || err.message.contains("return"),
        "the diagnostic must explain the return type: {}",
        err.message
    );
}

#[test]
fn main_may_not_take_parameters() {
    // The C runtime calls the entry point with its own argument convention, and
    // the symbol this backend emits takes none. Accepting a parameter would
    // produce a `main` the loader cannot call.
    let with_param = check_source("fn main(argc: int) { print(argc); }");
    let err = check_main_exists(&with_param.items)
        .expect_err("a `main` with a parameter is not callable");
    assert!(
        err.message.contains("parameter") && err.message.contains("argc"),
        "the diagnostic must name the offending parameter: {}",
        err.message
    );
    assert_eq!(
        err.span,
        with_param
            .items
            .iter()
            .find_map(|item| match item {
                yashy_sema::HirItem::Fn(f) => f.params.first().map(|p| p.span),
                _ => None,
            })
            .expect("the parameter must have a span"),
        "the diagnostic must point at the parameter, not the function"
    );
}

/// Lower a whole program's source to one IR module.
fn ir_of(src: &str) -> String {
    emit_program(&check_source(src), "main.ya", CodegenOptions::default())
}

#[test]
fn a_unit_main_is_emitted_as_i32_and_exits_zero() {
    // `void main` leaves the process exit status as whatever the last call left
    // in the return register, so `fn main() { print("hello"); }` exits with
    // the length its last call happened to return. The entry point therefore
    // returns `i32` even when the Yashy signature is `()`.
    let ir_text = ir_of("fn main() { print(\"hi\"); }");
    assert!(
        ir_text.contains("define i32 @main(i32 %argc, i8** %argv)"),
        "the entry point is an `i32` function:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("define void @main"),
        "and never a `void` one:\n{ir_text}"
    );
    assert!(
        ir_text.contains("\nret i32 0"),
        "the fall-through terminator yields zero:\n{ir_text}"
    );
}

#[test]
fn an_explicit_unit_return_in_main_still_yields_zero() {
    let ir_text = ir_of("fn main() { print(\"hi\"); return; }");
    assert!(
        ir_text.contains("ret i32 0"),
        "`return;` in `main` is the entry point's zero:\n{ir_text}"
    );
    assert!(!ir_text.contains("ret void"), "no `ret void` in `main`:\n{ir_text}");
}

#[test]
fn an_i64_main_truncates_to_the_entry_point_width() {
    let ir_text = ir_of("fn main() -> int { return 7; }");
    assert!(
        ir_text.contains("define i32 @main(i32 %argc, i8** %argv)"),
        "the entry point is still `i32`:\n{ir_text}"
    );
    assert!(
        ir_text.contains("trunc i64 ") && ir_text.contains("to i32"),
        "the `i64` result narrows to the exit code's width:\n{ir_text}"
    );
}

#[test]
fn an_ordinary_function_keeps_its_own_return_type() {
    // The `i32` rule is the entry point's, not every function's.
    let ir_text = ir_of("fn seven() -> int { return 7; }\nfn main() { print(seven()); }");
    assert!(
        ir_text.contains("define i64 @seven()"),
        "a helper keeps `i64`:\n{ir_text}"
    );
    assert!(
        ir_text.contains("ret i64 7"),
        "and so does its `ret`:\n{ir_text}"
    );
}

#[test]
fn input_calls_use_the_runtime_symbols() {
    // The Yashy names are `read_line`/`parse_i64`, and the runtime's carry the
    // `yashy_` prefix. Nothing else changes: the argument and result types are
    // already the IR types, so the call is the whole lowering.
    let ir_text = ir_of(
        "fn main() {\n\
         \x20   const line = read_line();\n\
         \x20   print(line.parse_i64());\n\
         \x20   print(line.parse_f64());\n\
         }",
    );
    assert!(
        ir_text.contains("call i8* @yashy_read_line()"),
        "a no-argument call to the runtime reader:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 @yashy_parse_i64(i8* "),
        "the line is passed through unchanged:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call double @yashy_parse_f64(i8* "),
        "and the floating-point parser is the same shape:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("call i8* @read_line"),
        "the bare Yashy name is not a symbol the runtime defines:\n{ir_text}"
    );
}

#[test]
fn the_entry_point_initializes_the_argument_vector() {
    // The process vector is handed to the runtime once, at the top of the entry
    // point, before any Yashy statement runs. The Yashy surface stays
    // `fn main()`; `%argc`/`%argv` are the hidden C parameters.
    let ir_text = ir_of("fn main() { print(\"hi\"); }");
    assert!(
        ir_text.contains("call void @yashy_args_init(i32 %argc, i8** %argv)"),
        "the entry point stores the vector:\n{ir_text}"
    );
    let init = ir_text
        .find("call void @yashy_args_init")
        .expect("the init call must exist");
    let body = ir_text
        .find("call void @yashy_print_str")
        .expect("the body must print");
    assert!(
        init < body,
        "the vector is stored before the body runs:\n{ir_text}"
    );
}

#[test]
fn args_fills_a_slice_through_the_runtime() {
    // `args()` is not built element by element in IR: one runtime call fills a
    // zeroed stack header with owned strings, the same shape `split` uses.
    let ir_text = ir_of("fn main() { const xs = args(); print(xs.len()); xs.free(); }");
    assert!(
        ir_text.contains("store { i8*, i64, i64 } zeroinitializer, ptr %slot"),
        "the out slot starts empty:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_args_slice(ptr %slot"),
        "the runtime fills the slice:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_slice_free_strings(ptr "),
        "`.free()` releases the strings too:\n{ir_text}"
    );
}

#[test]
fn the_process_and_clock_builtins_use_the_runtime_symbols() {
    let ir_text = ir_of(
        "fn main() {\n\
         \x20   const here = getcwd();\n\
         \x20   const stamp = now();\n\
         \x20   sleep(5);\n\
         \x20   print(here + str_from_i64(stamp));\n\
         }",
    );
    assert!(
        ir_text.contains("call i8* @yashy_getcwd()"),
        "the working directory is a no-argument call:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 @yashy_now_ms()"),
        "the clock returns an `i64`:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call void @yashy_sleep_ms(i64 5)"),
        "the sleep passes the millisecond count through:\n{ir_text}"
    );
}

#[test]
fn a_recursive_main_supplies_the_hidden_entry_parameters() {
    // The definition carries `(i32, i8**)`, so a Yashy call with no arguments
    // has to supply a null pair. The runtime keeps the real vector because the
    // first `yashy_args_init` wins.
    let ir_text = ir_of("fn main() { main(); }");
    assert!(
        ir_text.contains("call i32 @main(i32 0, i8** null)"),
        "the recursive call matches the definition:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("call void @main("),
        "and is never typed as the Yashy unit return:\n{ir_text}"
    );
}

#[test]
fn a_recursive_int_main_widens_the_entry_point_result() {
    // `main` is emitted as `i32`, so a recursive `-> int` call sign-extends the
    // value back to the type the call site expects.
    let ir_text = ir_of("fn main() -> int { return main(); }");
    assert!(
        ir_text.contains("call i32 @main(i32 0, i8** null)"),
        "the call still uses the entry signature:\n{ir_text}"
    );
    assert!(
        ir_text.contains("sext i32 ") && ir_text.contains("to i64"),
        "the result is widened to the Yashy `int`:\n{ir_text}"
    );
}

#[test]
fn a_struct_field_is_read_with_the_declaration_order_index() {
    // A struct value is an aggregate, so reading a field is `extractvalue` and
    // needs no address. The index is the declaration order position: this
    // literal writes `y` first, and `p.x` must still extract field 0.
    let ir_text = ir_of(
        "struct P { x: int, y: float }\n\
         fn main() {\n\
         \x20   const p = P { y: 2.0, x: 1 };\n\
         \x20   print(p.x);\n\
         \x20   print(p.y);\n\
         }",
    );
    assert!(
        ir_text.contains("%struct.P = type { i64, double }"),
        "the aggregate follows the declaration:\n{ir_text}"
    );
    let extracts: Vec<&str> = ir_text
        .lines()
        .filter(|line| line.contains("extractvalue"))
        .collect();
    assert_eq!(extracts.len(), 2, "one extract per field read:\n{ir_text}");
    assert!(
        extracts[0].trim_end().ends_with(", 0"),
        "`p.x` is field 0: {}",
        extracts[0]
    );
    assert!(
        extracts[1].trim_end().ends_with(", 1"),
        "`p.y` is field 1: {}",
        extracts[1]
    );
}

#[test]
fn the_number_to_string_conversions_call_the_runtime() {
    let ir_text = ir_of(
        "fn main() {\n\
         \x20   const a = str_from_i64(1);\n\
         \x20   const b = str_from_f64(1.5);\n\
         \x20   const c = str_from_bool(true);\n\
         \x20   print(a + b + c);\n\
         }",
    );
    assert!(
        ir_text.contains("call i8* @yashy_str_from_i64(i64 "),
        "the integer conversion is a direct call:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @yashy_str_from_f64(double "),
        "and so is the float one:\n{ir_text}"
    );
    // The bool takes a different path: the runtime declares an `i64` because C
    // has no one-bit type, so the `i1` is widened first rather than handed over
    // as a value the callee would read differently.
    assert!(
        ir_text.contains("zext i1 "),
        "the bool is widened:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @yashy_str_from_bool(i64 "),
        "and the call passes the widened value:\n{ir_text}"
    );
    assert!(
        !ir_text.contains("@yashy_str_from_bool(i1 "),
        "never as an `i1`, which the declaration does not take:\n{ir_text}"
    );
}

#[test]
fn a_debug_form_becomes_a_printer_function() {
    // `${value:?}` is lowered by the checker into a call to a printer it
    // synthesized, so the backend sees an ordinary function. The printer is
    // emitted with the rest of the program, and the call names it.
    let ir_text = ir_of(
        "struct P { x: int }\n\
         fn main() {\n\
         \x20   const p = P { x: 3 };\n\
         \x20   print(\"${p:?}\");\n\
         \x20 }",
    );
    assert!(
        ir_text.contains("define i8* @__debug."),
        "the printer is a function in the module:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i8* @__debug."),
        "and the interpolation calls it:\n{ir_text}"
    );
}

#[test]
fn the_debug_form_of_a_string_calls_the_runtime() {
    // A string's debug form is a byte operation -- quoting and escaping --
    // so it is the runtime's entry point rather than a synthesized printer.
    let ir_text = ir_of("fn main() { const s = \"a\"; print(\"${s:?}\"); }");
    assert!(
        ir_text.contains("call i8* @yashy_str_debug(i8* "),
        "the quoted form is a runtime call:\n{ir_text}"
    );
}

// --- panic -------------------------------------------------------------------
#[test]
fn a_panic_call_is_followed_by_unreachable_in_a_block_of_its_own() {
    // `panic` never returns, so the call cannot be followed by more instructions
    // in the same block. It goes into a block of its own instead: the current
    // block branches to it, it ends in `unreachable`, and a fresh block follows
    // for whatever the enclosing expression still had to emit. Emitting
    // `unreachable` in place would put two terminators in one block -- which LLVM
    // rejects -- and the enclosing expression can be anywhere, so the lowering
    // cannot know whether anything follows.
    let ir_text = ir_of("fn f() { panic(\"stop\"); } fn main() { f(); }");
    let lines: Vec<&str> = ir_text.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains("call void @yashy_panic("))
        .expect("the runtime call must be emitted");
    assert!(
        lines[at - 1].ends_with(':'),
        "the call opens a block of its own: {}",
        lines[at - 1]
    );
    assert!(
        lines[at - 2].starts_with("br label %"),
        "and the block before it branches there: {}",
        lines[at - 2]
    );
    assert_eq!(lines[at + 1], "unreachable", "nothing may follow the call");
    assert!(
        lines[at + 2].ends_with(':'),
        "a fresh block must follow the panic, so the rest of the expression has \
         somewhere valid to go: {}",
        lines[at + 2]
    );
}

#[test]
fn the_panic_call_carries_the_message_and_the_position_it_was_written_at() {
    // The message is whatever the program wrote. The file, line and column are
    // the checker's: it resolved them from the call's span, because the backend
    // is handed IR text and a path rather than a source map. Both arrive as
    // ordinary arguments, so the runtime needs no global state to report them.
    let ir_text = ir_of("fn f() {\n    panic(\"boom\");\n}\nfn main() { f(); }");
    let call = ir_text
        .lines()
        .find(|line| line.contains("call void @yashy_panic("))
        .expect("the runtime call must be emitted");
    assert!(
        call.ends_with("i64 2, i64 5)"),
        "the position `panic` was written at -- line 2, column 5 -- is passed as \
         operand constants: {call}"
    );
    assert!(
        ir_text.contains("c\"boom\\00\""),
        "the message is interned as a string global:\n{ir_text}"
    );
    assert!(
        ir_text.contains("c\"main.ya\\00\""),
        "and so is the file name the compiler was given:\n{ir_text}"
    );
    assert!(
        ir_text.contains("declare void @yashy_panic(i8*, i8*, i64, i64)"),
        "the declaration must match the runtime's signature:\n{ir_text}"
    );
}

#[test]
fn every_panic_in_a_program_routes_to_the_one_runtime_function() {
    // There is no recovery and no second path: a panic inside a callee stops the
    // process exactly as a panic in `main` does, because both are the same call.
    let ir_text = ir_of(
        "fn inner() { panic(\"inner\"); }\n\
         fn outer() { inner(); }\n\
         fn main() { outer(); panic(\"main\"); }",
    );
    let calls = ir_text
        .lines()
        .filter(|line| line.contains("call void @yashy_panic("))
        .count();
    assert_eq!(
        calls, 2,
        "one call per `panic`, all to the same function:\n{ir_text}"
    );
}

#[test]
fn a_program_that_never_panics_still_declares_the_symbol_and_never_calls_it() {
    // The declaration is emitted up front for every module, so a program that
    // starts panicking does not rearrange every line of the IR it already had --
    // and an unused `declare` is legal IR.
    let ir_text = ir_of("fn main() { print(\"hi\"); }");
    assert!(ir_text.contains("declare void @yashy_panic("));
    assert!(!ir_text.contains("call void @yashy_panic("));
}

#[test]
fn a_returned_panic_terminates_the_block_with_unreachable() {
    // `return panic("...")` has no value to return, so the block that would have
    // returned is unreachable. Falling through to the ordinary `ret` would emit
    // `ret i64 ` with a missing operand, which is not valid IR.
    let ir_text = ir_of("fn f() -> int { return panic(\"nope\"); }\nfn main() { f(); }");
    for line in ir_text.lines() {
        assert!(
            line.trim_end() != "ret i64"
                && line.trim_end() != "ret double"
                && line.trim_end() != "ret i1"
                && !line.trim_end().ends_with("ret i8*"),
            "a `ret` with no operand is not valid IR: `{line}`"
        );
    }
    let lines: Vec<&str> = ir_text.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains("call void @yashy_panic("))
        .expect("the runtime call must be emitted");
    assert_eq!(lines[at + 1], "unreachable");
    assert!(
        lines[at + 2..].iter().any(|line| *line == "unreachable"),
        "the return's own block is unreachable too:\n{ir_text}"
    );
}

#[test]
fn a_switch_whose_every_arm_diverges_is_lowered_with_no_result_slot() {
    // The switch expression's type is `!`, which has no representation, so there
    // is nothing to store an arm's value into and no load after the merge. The
    // arms still have to be well formed, which is what the fresh block after each
    // panic is for.
    let ir_text = ir_of(
        "fn f(o: Option[int]) -> int {\n\
         \x20   return switch o {\n\
         \x20       case None { panic(\"none\") }\n\
         \x20       case Some(v) { panic(\"some\") }\n\
         \x20   };\n\
         }",
    );
    assert!(
        !ir_text.contains("alloca void"),
        "a `!`-valued switch has no result slot to allocate:\n{ir_text}"
    );
    assert_eq!(
        ir_text
            .lines()
            .filter(|line| line.contains("call void @yashy_panic("))
            .count(),
        2,
        "both arms lower to the one runtime call:\n{ir_text}"
    );
}

#[test]
fn llvm_type_mapping_matches_the_spec() {
    assert_eq!(llvm_type_string(&TypeKind::I64), "i64");
    assert_eq!(llvm_type_string(&TypeKind::F64), "double");
    assert_eq!(llvm_type_string(&TypeKind::Bool), "i1");
    assert_eq!(llvm_type_string(&TypeKind::Str), "{ i32, i32, i8* }");
    assert_eq!(llvm_type_string(&TypeKind::Unit), "void");
}
