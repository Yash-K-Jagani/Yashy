//! The LLVM module being assembled: accumulated IR lines, interned string
//! globals, and the temporary counter.
//!
//! This is a plain text builder, deliberately not an AST: M0–M2 emits LLVM
//! IR as text and hands it to `clang` as a subprocess (the master plan's
//! "No LLVM build dependency" constraint), so the only structure the
//! backend needs is "the lines so far, in order".

use std::collections::HashMap;

use otter_sema::{HirExternDecl, TypeKind};

use crate::ir::repr_type;

/// One variant of a user-declared enum, as the backend needs it.
///
/// The field list is flattened in source order and carries the LLVM
/// representation of each payload field, because the aggregate layout is
/// `{ i8, <variant 0 fields>, <variant 1 fields>, ... }`. A unit variant has no
/// fields and contributes no struct members, but it still consumes a tag, so
/// the variant's *index* is its tag and its first *field* is found by counting
/// the fields of the variants before it.
#[derive(Clone, Debug)]
pub struct VariantRepr {
    /// The variant's source name, which is what a pattern spells.
    pub name: String,
    /// The payload fields in source order: `(field name, LLVM repr)`. The name
    /// is `None` for a positional field and `Some` for a named one, which is
    /// what lets a `case Rect { w, h }` pattern find its field.
    pub fields: Vec<(Option<String>, String)>,
}

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
    /// User-declared enums by name, in declaration order. This is the table
    /// the tag and payload lookups need: the pattern side is handed only a
    /// variant's spelling, so the declaration order has to come from here.
    enums: HashMap<String, Vec<VariantRepr>>,
    /// The `source_filename` the driver wants reported in the header.
    source_filename: String,
    /// Whether floating-point multiply-add contraction is allowed; see
    /// [`Module::set_fp_contract`].
    fp_contract: bool,
    /// The index the next `%tN` temporary will use.
    next_value: u32,
    /// Functions the lowering synthesized, in creation order. They cannot be
    /// appended to `lines` while the function that wanted them is still being
    /// emitted, so they are queued here and flushed after every checked
    /// function has closed.
    synthesized: Vec<Vec<String>>,
    /// The index the next `@otter_thread_entry_N` will use. One entry is
    /// synthesized per `thread_spawn` site, so the argument struct's shape is
    /// already known when the entry is written.
    next_thread_entry: u32,
    /// Top-level function signatures, used when a function name is used as a
    /// value. The key is the source name; the value is the parameter and return
    /// types the thunk must reproduce.
    fn_signatures: HashMap<String, (Vec<TypeKind>, TypeKind)>,
    /// Thunks already synthesized for top-level functions, keyed by source
    /// name. A function used as a value more than once shares one thunk.
    fn_value_thunks: HashMap<String, String>,
    /// The index the next `@__lambda_N` will use.
    ///
    /// A lambda has no name in the source, so the backend gives it one: the
    /// index is unique in the module, which is all a symbol has to be.
    next_lambda: u32,
    /// Where the module's lines go while a capture is open.
    ///
    /// A lambda's body is written while another function's body is still open,
    /// and a `define` cannot be written inside a function. The capture
    /// diverts the lines into a buffer that is closed into one complete
    /// `define` block and queued as a synthesized function.
    ///
    /// The buffers are a stack, not one buffer: a lambda may contain another
    /// lambda, and the inner one's body must not swallow the outer one's.
    captures: Vec<Vec<String>>,
    /// Whether string refcounts in this module are atomic.
    ///
    /// Set once, from the checked program's "any spawn?" bit; see
    /// `HirProgram::uses_threads`. Kept on the module because every function's
    /// [`FnCtx`](crate::FnCtx) is built from it and the answer must not differ
    /// between two functions in one module.
    atomic_strings: bool,
    /// C declarations by name, so a call can tell an extern callee from a user
    /// function and apply the boundary rules (spec §8.1).
    externs: HashMap<String, HirExternDecl>,
}

impl Module {
    /// An empty module.
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            strings: Vec::new(),
            named_types: Vec::new(),
            enums: HashMap::new(),
            source_filename: "main.ot".to_string(),
            fp_contract: false,
            next_value: 0,
            synthesized: Vec::new(),
            next_thread_entry: 0,
            fn_signatures: HashMap::new(),
            fn_value_thunks: HashMap::new(),
            next_lambda: 0,
            captures: Vec::new(),
            atomic_strings: false,
            externs: HashMap::new(),
        }
    }

    /// Record a C declaration for this module's call sites.
    ///
    /// Emission of the `declare` itself is `gen_extern_decl`'s job; this is the
    /// lookup table the call lowering needs, and it is separate because a
    /// backend test may lower a call without going through `emit_program`.
    pub fn add_extern_decl(&mut self, decl: HirExternDecl) {
        self.externs.insert(decl.name.clone(), decl);
    }

    /// The C declaration named `name`, when the program declared one.
    pub fn extern_decl(&self, name: &str) -> Option<&HirExternDecl> {
        self.externs.get(name)
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

    /// Define a user enum's aggregate type and record its variants.
    ///
    /// The layout is `{ i8, <variant 0 payload>, <variant 1 payload>, … }`,
    /// flattened in declaration order. A unit variant contributes no members
    /// but still owns a tag, so the tag is the variant's index while a
    /// payload's first member is found by counting the fields before it.
    /// Defining the name twice is a no-op, matching [`Self::define_named_type`].
    pub fn define_enum(&mut self, name: &str, variants: Vec<VariantRepr>) {
        if self.enums.contains_key(name) {
            return;
        }
        let mut fields = vec!["i8".to_string()];
        for variant in &variants {
            for (_, repr) in &variant.fields {
                fields.push(repr.clone());
            }
        }
        self.define_named_type(&format!("%enum.{name}"), fields);
        self.enums.insert(name.to_string(), variants);
    }

    /// The variants of the enum `ty` names, or `None` when `ty` is not one.
    pub fn enum_variants(&self, ty: &TypeKind) -> Option<&[VariantRepr]> {
        match ty {
            TypeKind::Enum(name) => self.enums.get(name).map(Vec::as_slice),
            _ => None,
        }
    }

    /// Append one line of IR to the module body.
    pub fn push(&mut self, line: impl Into<String>) {
        match self.captures.last_mut() {
            Some(buffer) => buffer.push(line.into()),
            None => self.lines.push(line.into()),
        }
    }

    /// Start diverting the module's lines into a fresh buffer.
    ///
    /// A lambda's body is lowered while the function that contains it is
    /// still open, and a `define` block cannot be written inside a function.
    /// The capture collects the lines so they can be closed into one block and
    /// queued as a synthesized function.
    pub fn begin_capture(&mut self) {
        self.captures.push(Vec::new());
    }

    /// Stop diverting and return the lines collected so far.
    ///
    /// An empty stack means no capture was open, which is a caller error
    /// rather than a program error; the empty list is the harmless answer.
    pub fn end_capture(&mut self) -> Vec<String> {
        self.captures.pop().unwrap_or_default()
    }

    /// The next lambda index, for `@__lambda_N`.
    pub fn next_lambda(&mut self) -> u32 {
        let index = self.next_lambda;
        self.next_lambda += 1;
        index
    }

    /// How many body lines have been pushed so far.
    ///
    /// A caller uses this to remember a position it will later insert at; see
    /// [`Module::insert_lines`].
    pub fn line_count(&self) -> usize {
        match self.captures.last() {
            Some(buffer) => buffer.len(),
            None => self.lines.len(),
        }
    }

    /// Insert `lines` before `index`, shifting everything after it down.
    ///
    /// This is how the allocas gathered while lowering a function body reach
    /// the entry block: the body is lowered in one pass with each `alloca`
    /// diverted into a buffer, and the buffer is spliced in at the remembered
    /// position once the function is finished.
    pub fn insert_lines(&mut self, index: usize, lines: Vec<String>) {
        match self.captures.last_mut() {
            Some(buffer) => {
                buffer.splice(index..index, lines);
            }
            None => {
                self.lines.splice(index..index, lines);
            }
        }
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

    /// The next thread-entry index, for `@otter_thread_entry_N`.
    pub fn next_thread_entry(&mut self) -> u32 {
        let index = self.next_thread_entry;
        self.next_thread_entry += 1;
        index
    }

    /// Record one top-level function's signature.
    ///
    /// The table is separate from the emitted `define` because a function can
    /// be used as a value before its body is emitted, and a thunk needs the
    /// parameter and return representations without re-reading the HIR.
    pub fn add_fn_signature(&mut self, name: &str, params: Vec<TypeKind>, ret: TypeKind) {
        self.fn_signatures.insert(name.to_string(), (params, ret));
    }

    /// The `@__fnvalue_<name>` thunk that turns a top-level function into a
    /// `{ i8* code, i8* env }` value, synthesizing it on first use.
    ///
    /// The thunk's first parameter is the environment pointer the indirect
    /// call always passes; a top-level function has no environment, so the
    /// thunk ignores it and calls the real function directly. The name is
    /// stable per source function, so two uses share one definition.
    ///
    /// `None` means the name is not a recorded top-level function (a builtin,
    /// a local, or a typo); the caller lowers that to a zero value rather than
    /// inventing a symbol.
    pub fn fn_value_thunk(&mut self, name: &str) -> Option<String> {
        if let Some(thunk) = self.fn_value_thunks.get(name) {
            return Some(thunk.clone());
        }
        let (params, ret) = self.fn_signatures.get(name)?.clone();
        let thunk = format!("__fnvalue_{name}");
        let ret_repr = if matches!(ret, TypeKind::Unit | TypeKind::Never) {
            "void".to_string()
        } else {
            repr_type(&ret)
        };

        let mut params_ir = Vec::with_capacity(params.len());
        for (index, param) in params.iter().enumerate() {
            params_ir.push(format!("{} %arg{index}", repr_type(param)));
        }
        let signature = if params_ir.is_empty() {
            "i8* %env".to_string()
        } else {
            format!("i8* %env, {}", params_ir.join(", "))
        };

        let mut lines = vec![
            format!("define {ret_repr} @{thunk}({signature}) {{"),
            "entry:".to_string(),
        ];
        if ret_repr == "void" {
            lines.push(format!("  call void @{name}({})", params_ir.join(", ")));
            lines.push("  ret void".to_string());
        } else {
            lines.push(format!(
                "  %result = call {ret_repr} @{name}({})",
                params_ir.join(", ")
            ));
            lines.push(format!("  ret {ret_repr} %result"));
        }
        lines.push("}".to_string());
        self.add_synthesized_function(lines);
        self.fn_value_thunks.insert(name.to_string(), thunk.clone());
        Some(thunk)
    }

    /// Choose the reference-count pair every string operation in this module
    /// uses.
    ///
    /// Called once, before any function is emitted: a program that spawns a
    /// thread can share a string with it from anywhere, so the whole module
    /// pays for atomics; a program with no spawn keeps M3's plain calls.
    pub fn set_atomic_strings(&mut self, atomic: bool) {
        self.atomic_strings = atomic;
    }

    /// Whether this module uses the atomic reference-count pair.
    pub fn atomic_strings(&self) -> bool {
        self.atomic_strings
    }

    /// Queue a complete `define` block for emission after the checked
    /// functions.
    pub fn add_synthesized_function(&mut self, lines: Vec<String>) {
        self.synthesized.push(lines);
    }

    /// Take every queued function, leaving the queue empty.
    pub fn take_synthesized_functions(&mut self) -> Vec<Vec<String>> {
        std::mem::take(&mut self.synthesized)
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
        // The slice half of ledger L1's M6 block. Declared up front for the
        // same reason as the string set: an unused `declare` is legal IR.
        out.push_str("declare void @otter_slice_push(ptr, ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_get(ptr, i64, ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_pop(ptr, ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_set(ptr, i64, ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_insert(ptr, i64, ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_remove(ptr, i64, ptr, i64)\n");
        out.push_str("declare void @otter_slice_clear(ptr)\n");
        out.push_str("declare void @otter_slice_clear_strings(ptr)\n");
        out.push_str("declare void @otter_slice_reverse(ptr, i64)\n");
        out.push_str("declare void @otter_slice_sort_i64(ptr)\n");
        out.push_str("declare i64 @otter_slice_index_of_i64(ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_index_of_f64(ptr, double)\n");
        out.push_str("declare i64 @otter_slice_index_of_bool(ptr, i64)\n");
        out.push_str("declare i64 @otter_slice_index_of_string(ptr, i8*)\n");
        out.push_str("declare void @otter_slice_extend(ptr, ptr, i64)\n");
        out.push_str("declare void @otter_slice_extend_strings(ptr, ptr)\n");
        out.push_str("declare void @otter_slice_slice(ptr, ptr, i64, i64, i64)\n");
        out.push_str("declare void @otter_slice_slice_strings(ptr, ptr, i64, i64)\n");
        out.push_str("declare void @otter_slice_free(ptr)\n");
        out.push_str("declare void @otter_slice_free_strings(ptr)\n");
        // The map half of ledger L1's M6 block. An unused `declare` is legal
        // IR, so these are always present for the same reason as the slices.
        out.push_str("declare i8* @otter_map_new(i64, i64, i64)\n");
        out.push_str("declare void @otter_map_free(i8*)\n");
        out.push_str("declare void @otter_map_insert(i8*, ptr, ptr)\n");
        out.push_str("declare i64 @otter_map_get(i8*, ptr, ptr)\n");
        out.push_str("declare i64 @otter_map_remove(i8*, ptr, ptr)\n");
        out.push_str("declare i64 @otter_map_len(i8*)\n");
        // Threads, mutexes and channels (M7). The entry is a function pointer,
        // so the spawn call hands the runtime its synthesized entry directly;
        // the argument block is a byte copy whose size the caller computes from
        // the struct's own layout.
        out.push_str("declare i32 @otter_thread_spawn(i8*, i8*, i64)\n");
        out.push_str("declare i32 @otter_thread_join(i32)\n");
        out.push_str("declare i8* @otter_mutex_new()\n");
        out.push_str("declare void @otter_mutex_lock(i8*)\n");
        out.push_str("declare void @otter_mutex_unlock(i8*)\n");
        out.push_str("declare void @otter_mutex_free(i8*)\n");
        out.push_str("declare i8* @otter_chan_new(i64)\n");
        out.push_str("declare i8 @otter_chan_send(i8*, i8*, i64)\n");
        out.push_str("declare i8 @otter_chan_recv(i8*, i8*, i64)\n");
        out.push_str("declare void @otter_chan_close(i8*)\n");
        out.push_str("declare void @otter_chan_free(i8*)\n");
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
        out.push_str("declare i64 @otter_str_get(i8*, i64, ptr)\n");
        out.push_str("declare i64 @otter_str_index_of(i8*, i8*)\n");
        out.push_str("declare i1 @otter_str_contains(i8*, i8*)\n");
        out.push_str("declare i1 @otter_str_starts_with(i8*, i8*)\n");
        out.push_str("declare i1 @otter_str_ends_with(i8*, i8*)\n");
        out.push_str("declare i8* @otter_str_trim(i8*)\n");
        out.push_str("declare i8* @otter_str_trim_start(i8*)\n");
        out.push_str("declare i8* @otter_str_trim_end(i8*)\n");
        out.push_str("declare i8* @otter_str_to_upper(i8*)\n");
        out.push_str("declare i8* @otter_str_to_lower(i8*)\n");
        out.push_str("declare i8* @otter_str_replace(i8*, i8*, i8*)\n");
        out.push_str("declare i8* @otter_str_repeat(i8*, i64)\n");
        out.push_str("declare void @otter_str_split(ptr, i8*, i8*)\n");
        out.push_str("declare void @otter_str_lines(ptr, i8*)\n");
        // Reference counting (M3 Task 5). `retain` is declared with the rest so
        // the pairing's second half does not rewrite every golden file; the
        // release half is emitted wherever an owned temporary is consumed.
        out.push_str("declare i8* @otter_str_retain(i8*)\n");
        out.push_str("declare void @otter_str_release(i8*)\n");
        // The atomic pair (M7 Task 4). Separate symbols rather than one
        // function with a flag: a program with no thread keeps the plain path,
        // which the spec promises stays free of locks.
        out.push_str("declare i8* @otter_str_retain_atomic(i8*)\n");
        out.push_str("declare void @otter_str_release_atomic(i8*)\n");
        // The closure environment (M13 Task 9): a reference-counted block
        // holding the values a closure captured.
        out.push_str("declare i8* @otter_env_new(i64, i64, i64)\n");
        out.push_str("declare i8* @otter_env_retain(i8*)\n");
        out.push_str("declare void @otter_env_release(i8*)\n");
        // The C boundary (spec §8.1, M5 Task 3). `cstr` allocates a copy for a
        // safe call; `ptr` lends the string's own bytes to an `unsafe` one.
        out.push_str("declare i8* @otter_str_cstr(i8*)\n");
        out.push_str("declare i8* @otter_str_ptr(i8*)\n");
        // The substring builtin (M6 Task 10). Declared with the string set: the
        // stdlib's `trim` and `split` are written on top of it.
        out.push_str("declare i8* @otter_str_slice(i8*, i64, i64)\n");
        // Input (ledger L1's input additions). Declared up front for the same
        // reason as the string set.
        out.push_str("declare i8* @otter_read_line()\n");
        out.push_str("declare i64 @otter_parse_i64(i8*)\n");
        out.push_str("declare double @otter_parse_f64(i8*)\n");
        // Files and the environment (M6 Task 10). Declared up front for the same
        // reason; `read_file`/`getenv` return an owned string, `write_file`
        // reports 1/0 and is handed the text's pointer and length separately.
        out.push_str("declare i8* @otter_read_file(i8*)\n");
        out.push_str("declare i64 @otter_write_file(i8*, i8*, i64)\n");
        out.push_str("declare i8* @otter_getenv(i8*)\n");
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
