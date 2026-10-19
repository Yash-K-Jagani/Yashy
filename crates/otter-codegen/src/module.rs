//! The LLVM module being assembled: accumulated IR lines, interned string
//! globals, and the temporary counter.
//!
//! This is a plain text builder, deliberately not an AST: M0–M2 emits LLVM
//! IR as text and hands it to `clang` as a subprocess (the master plan's
//! "No LLVM build dependency" constraint), so the only structure the
//! backend needs is "the lines so far, in order".

/// A module under construction.
pub struct Module {
    /// The body lines accumulated by `push`, in emission order.
    lines: Vec<String>,
    /// Interned string globals: `(struct name, struct body, data body)`.
    /// The bodies exclude the `@.str.N = ` prefix so two equal data bodies
    /// compare equal, which is what deduplicates identical literals. The
    /// bytes live in a separate global named `@.str.N.data`; the struct
    /// global holds the refcount, length, capacity, and a pointer to them.
    strings: Vec<(String, String, String)>,
    /// Module-level named type definitions: `(name, field reps)`. These are
    /// hoisted out of any function body and emitted at the top of the module,
    /// because a `%struct.X = type { … }` line inside a `define` block is not
    /// valid LLVM IR.
    named_types: Vec<(String, Vec<String>)>,
    /// The `source_filename` the driver wants reported in the header.
    source_filename: String,
    /// Whether floating-point multiply-add contraction is allowed; see
    /// [`Module::set_fp_contract`].
    fp_contract: bool,
    /// The index the next `%tN` temporary will use.
    next_value: u32,
}

impl Module {
    /// An empty module.
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            strings: Vec::new(),
            named_types: Vec::new(),
            source_filename: "main.ot".to_string(),
            fp_contract: false,
            next_value: 0,
        }
    }

    /// Allow floating-point contraction in this module.
    ///
    /// When this is on, a floating-point multiply and add/subtract are emitted
    /// with LLVM's `contract` fast-math flag, which lets the backend fuse them
    /// into one multiply-add. It is off by default because a fused multiply-add
    /// rounds once where `a * b + c` rounds twice — that is where the speed
    /// comes from, and why it changes what a program prints. The decision and
    /// its reasoning live in [`crate::CodegenOptions`]; this is only the carrier
    /// the per-instruction emitter reads.
    pub fn set_fp_contract(&mut self, allowed: bool) {
        self.fp_contract = allowed;
    }

    /// Whether contraction is allowed; read by the binary-operation emitter.
    pub fn fp_contract(&self) -> bool {
        self.fp_contract
    }

    /// Set the `source_filename` reported in the module header.
    pub fn set_source_filename(&mut self, name: &str) {
        self.source_filename = name.to_string();
    }

    /// Define a named aggregate type once, at module level.
    ///
    /// A second call for the same name is ignored, so every struct literal of
    /// one struct shares one `%struct.Name` definition regardless of which
    /// function emitted it.
    pub fn define_named_type(&mut self, name: &str, fields: Vec<String>) {
        if !self.named_types.iter().any(|(n, _)| n == name) {
            self.named_types.push((name.to_string(), fields));
        }
    }

    /// Append one line of IR to the module body.
    pub fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    /// How many body lines have been pushed so far.
    ///
    /// A caller uses this to remember a position it will later insert at; see
    /// [`Module::insert_lines`].
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Insert `lines` before `index`, shifting everything after it down.
    ///
    /// This is how the allocas gathered while lowering a function body reach
    /// the entry block: the body is lowered in one pass with each `alloca`
    /// diverted into a buffer, and the buffer is spliced in at the remembered
    /// position once the function is finished.
    pub fn insert_lines(&mut self, index: usize, lines: Vec<String>) {
        self.lines.splice(index..index, lines);
    }

    /// Intern `bytes` as a static string global, returning its name.
    ///
    /// Identical byte slices share one global: the second `intern_string`
    /// of `b"hi"` returns the same `@.str.0` and emits nothing new. Two
    /// globals are emitted per literal: a NUL-terminated `[N x i8]` byte
    /// array named `@.str.N.data` (`b"a"` becomes `[2 x i8] c"a\00"`), and
    /// the `%otter_str` struct that points at it with refcount -1, so the
    /// literal is never freed. Every byte outside printable ASCII, plus `"`
    /// and `\`, is escaped as `\XX` so the definition is pure ASCII.
    pub fn intern_string(&mut self, bytes: &[u8]) -> String {
        let data_body = Self::data_body(bytes);
        if let Some((name, _, _)) = self.strings.iter().find(|(_, _, def)| *def == data_body) {
            return name.clone();
        }
        let name = format!("@.str.{}", self.strings.len());
        let data_name = format!("{name}.data");
        let body = Self::struct_body(bytes, &data_name);
        self.strings.push((name.clone(), body, data_body));
        name
    }

    /// The byte-array body of a string global: everything after `@.str.N.data = `.
    fn data_body(bytes: &[u8]) -> String {
        let mut escaped = String::with_capacity(bytes.len() * 4 + 2);
        for &byte in bytes {
            match byte {
                // Printable ASCII except the two characters that would end
                // or escape the LLVM `c"..."` literal.
                0x20..=0x7e if byte != b'"' && byte != b'\\' => escaped.push(byte as char),
                _ => escaped.push_str(&format!("\\{byte:02X}")),
            }
        }
        // The NUL terminator counts toward the array length.
        let length = bytes.len() + 1;
        format!("private unnamed_addr constant [{length} x i8] c\"{escaped}\\00\"")
    }

    /// The `%otter_str` struct body of a string global: everything after
    /// `@.str.N = `. A literal is *static* — refcount -1, so a release is a
    /// no-op and the bytes are never freed — with its length and capacity
    /// written down and a pointer to the `@.str.N.data` byte array. The
    /// array's address *is* the address of its first byte, so the data
    /// pointer is the global itself, with no `getelementptr` to spell.
    fn struct_body(bytes: &[u8], data_name: &str) -> String {
        let len = bytes.len() as i64;
        format!("private unnamed_addr constant %otter_str {{ i64 -1, i64 {len}, i64 {len}, i8* {data_name} }}")
    }

    /// The next SSA temporary, `%t0`, `%t1`, …
    pub fn next_value(&mut self) -> String {
        let name = format!("%t{}", self.next_value);
        self.next_value += 1;
        name
    }

    /// Intern `bytes` and return the operand that names its **static**
    /// `otter_str` global: the global itself, since a `string` value is a
    /// pointer to the struct and `@.str.N` is already that pointer.
    ///
    /// This replaces the M3 interim, where a string literal was a bare
    /// `[N x i8]` global and the value was a `getelementptr` into it. Every
    /// `string` value is now a pointer to an `otter_str`, and a literal is
    /// never freed, which is what refcount -1 says.
    pub fn intern_bytes_ptr(&mut self, bytes: &[u8]) -> String {
        self.intern_string(bytes)
    }

    /// Finish the module: the fixed header, the string globals, the body.
    pub fn finish(&self) -> String {
        let triple = host_target_triple();
        let mut out = String::new();
        out.push_str("; Generated by the Otter compiler. Do not edit.\n");
        out.push_str(&format!(
            "source_filename = \"{}\"\n",
            self.source_filename
        ));
        out.push_str(&format!(
            "target datalayout = \"{}\"\n",
            host_data_layout()
        ));
        out.push_str(&format!("target triple = \"{triple}\"\n"));
        out.push('\n');
        // The refcounted string value: refcount, byte length, capacity, data.
        // `-1` in the refcount means static (a literal, never freed); a field
        // order and width that match `struct otter_str` in the runtime, which
        // is what lets one layout describe a value on both sides of a call.
        out.push_str("%otter_str = type { i64, i64, i64, i8* }\n");
        for (name, fields) in &self.named_types {
            out.push_str(&format!("{name} = type {{ {} }}\n", fields.join(", ")));
        }
        out.push('\n');
        out.push_str("declare void @otter_print_str(i8*)\n");
        out.push_str("declare void @otter_print_i64(i64)\n");
        out.push_str("declare void @otter_print_f64(double)\n");
        out.push_str("declare i8* @otter_alloc(i64, i64)\n");
        out.push_str("declare void @otter_free(i8*)\n");
        out.push_str("declare i8* @otter_arena_new()\n");
        out.push_str("declare i8* @otter_arena_alloc(i8*, i64, i64)\n");
        out.push_str("declare void @otter_arena_free_all(i8*)\n");
        // The string half of ledger L1. All of it is declared up front so a
        // later task that starts emitting one of these does not change every
        // golden file a second time; an unused `declare` is legal IR.
        out.push_str("declare i64 @otter_str_len(i8*)\n");
        out.push_str("declare i64 @otter_str_byte_at(i8*, i64)\n");
        out.push_str("declare i8* @otter_str_concat(i8*, i8*)\n");
        out.push_str("declare i8* @otter_str_from_i64(i64)\n");
        out.push_str("declare i8* @otter_str_from_f64(double)\n");
        out.push_str("declare i8* @otter_str_from_bool(i64)\n");
        out.push_str("declare i1 @otter_str_eq(i8*, i8*)\n");
        // Reference counting (M3 Task 5). `retain` is declared with the rest so
        // the pairing's second half does not rewrite every golden file; the
        // release half is emitted wherever an owned temporary is consumed.
        out.push_str("declare i8* @otter_str_retain(i8*)\n");
        out.push_str("declare void @otter_str_release(i8*)\n");
        // Input (ledger L1's input additions). Declared up front for the same
        // reason as the string set.
        out.push_str("declare i8* @otter_read_line()\n");
        out.push_str("declare i64 @otter_parse_i64(i8*)\n");
        out.push_str("declare double @otter_parse_f64(i8*)\n");
        // The diagnostic path (M4). Declared up front like the rest, so a
        // program that starts panicking does not change every golden file
        // again; an unused `declare` is legal IR.
        out.push_str("declare void @otter_panic(i8*, i8*, i64, i64)\n");
        // The checked-arithmetic path (M4 Task 11). The three intrinsics are
        // LLVM's own non-trapping overflow tests: each computes the result *and*
        // says whether it overflowed in one instruction, which is what keeps a
        // checked `a + b` to one `add`, two `extractvalue`s and a branch. The
        // hand-rolled version of this -- comparing sign bits, or widening to
        // `i128` for the multiply -- is more instructions and one wrong
        // predicate away from being a wrong answer.
        //
        // Whether a program needs any of them is not known until its body is
        // lowered, and they are declared here with the rest for the same reason
        // as every other declare: an unused declaration is legal IR, and
        // emitting one conditionally would make every golden file depend on the
        // program rather than on the compiler.
        out.push_str("declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)\n");
        out.push_str("declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)\n");
        out.push_str("declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)\n");
        // The saturating family (`saturating_*`), which clamps instead of
        // reporting anything. `smul.sat` is deliberately absent: it is not
        // implemented in LLVM, so `faults.rs` saturates the multiply by hand from
        // the overflow flag.
        out.push_str("declare i64 @llvm.sadd.sat.i64(i64, i64)\n");
        out.push_str("declare i64 @llvm.ssub.sat.i64(i64, i64)\n");
        // The two ways integer arithmetic stops a program, with the operands, so
        // the message can say which numbers and which operator. The operator is
        // a symbol (`+`, `-`, `*`, `/`, `%`); the runtime owns the wording around
        // it, the way `otter_panic` owns `panic:`.
        out.push_str("declare void @otter_panic_overflow(i8*, i64, i64, i8*, i64, i64)\n");
        out.push_str("declare void @otter_panic_division(i8*, i64, i64, i8*, i64, i64)\n");
        // `os.exit(code)` (spec 6.3). Declared up front like everything else, so
        // a program that exits early does not rewrite every golden file; an
        // unused `declare` is legal IR.
        out.push_str("declare void @otter_exit(i64)\n");
        for (name, body, data) in &self.strings {
            out.push_str(&format!("{name}.data = {data}\n"));
            out.push_str(&format!("{name} = {body}\n"));
        }
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

/// The `target triple` this compiler emits IR for and drives LLVM with.
///
/// It follows the **Rust toolchain's** environment, not the OS: a
/// `x86_64-pc-windows-gnu` build must not claim `x86_64-pc-windows-msvc`,
/// because the two need different CRTs and `clang` would link the wrong one —
/// or, on a machine with no Visual Studio at all, fail to link at all.
///
/// The driver reads this same function so the triple baked into the IR and the
/// `--target` it passes to `clang` can never disagree.
pub fn host_target_triple() -> &'static str {
    if cfg!(target_env = "msvc") {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "windows") {
        "x86_64-pc-windows-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

/// The `target datalayout` for [`host_target_triple`].
///
/// The only difference that matters here is the mangling character: COFF wants
/// `m:w`, ELF wants `m:e`.
fn host_data_layout() -> &'static str {
    if cfg!(target_env = "msvc") {
        "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128"
    } else {
        "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128"
    }
}
