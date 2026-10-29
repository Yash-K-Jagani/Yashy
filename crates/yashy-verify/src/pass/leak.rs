//! The leak pass: every linear value is released before its scope ends.
//!
//! A linear value has one owner and lives until that owner releases it. When
//! the owner's scope ends and the value was never freed, moved, returned, or
//! handed to something else, the allocation is unreachable and the memory is
//! gone. That is `E1000`.
//!
//! # The join is an intersection
//!
//! The pass walks a function body as a small control-flow graph and keeps, for
//! every binding in scope, one bit: *released on every path so far*. At an `if`
//! or a `switch` the bit after the merge is the **intersection** of the
//! branches -- released only when every path released it -- because a value
//! that one path forgot is leaked on that path.
//!
//! `yashy-sema` keeps the opposite set, a **union**: a binding moved on any
//! path is moved after the merge, because a later read could be reading the
//! spent value. Both joins are needed and neither implies the other. A value
//! freed on both branches is released (intersection) and moved (union); a value
//! freed on one branch is moved (union, so a later use is `E0500`) but not
//! released (intersection, so the other path leaked it).
//!
//! # Generosity is deliberate
//!
//! Handing a value to anything counts as releasing it: a call argument, a
//! struct field, a slice element, an assignment that overwrites it. A missed
//! leak is caught by the runtime report (`YASHY_LEAK_REPORT=1`); a false
//! "leak" on correct code costs the user trust in every other diagnostic, so
//! the pass errs towards silence. Parameters are not tracked at all: a function
//! that receives a pointer may free it, store it, or hand it back, and proving
//! which is interprocedural analysis this slice does not attempt.
//!
//! A `defer` body is walked where the `defer` is written, which is exactly
//! where `yashy-sema` checks it: registering a defer that frees `buf` marks
//! `buf` spent from that statement on, so the release belongs to that point's
//! state rather than to a separate rule about scope exit.
//!
//! # The arena is a borrow
//!
//! `arena_alloc(arena, n)` is the one intrinsic that neither takes nor gives
//! ownership: it borrows the handle and returns a block the arena owns. The
//! pass therefore does not track the block and does not count the handle as
//! handed off; the handle is released by `arena_free_all`. The fact lives in
//! [`yashy_hir::hir::is_arena_alloc`] so the checker and the verifier cannot
//! disagree about it.
//!
//! # A viewed array leaks by design
//!
//! A `[]T` that has been viewed cannot be released: `xs.free()` is refused
//! while the view is alive, and a view lives until the function returns. That
//! is documented behaviour (`docs/tutorial/09-arrays.md`), so the pass reports
//! it as a **warning** rather than an error: the program is not wrong, the
//! language has no way to release the value. A warning keeps the fact visible
//! and keeps `build` working; an error would refuse a documented idiom, and
//! silence would be a verifier that claims to catch leaks and does not.

use yashy_common::{codes, Diagnostic, FileId, ProofTrace, Severity, Span};
use yashy_hir::hir::{
    expr_ty, is_arena_alloc, HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm,
};
use yashy_hir::ty::Type;

use crate::ownership::{linearity_of, Linearity};

/// Run the leak pass over every function in `program`.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    let mut pass = LeakPass {
        out,
        file: FileId::from_index(0),
        frames: Vec::new(),
    };
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            pass.check_fn(function);
        }
    }
}

/// One tracked binding.
#[derive(Clone)]
struct Binding {
    name: String,
    /// Where the binding was declared, which is where a leak is reported.
    span: Span,
    /// True when every path reaching this point has released the value.
    released: bool,
    /// True when a view of the value exists.
    ///
    /// A `[]T` that has been viewed cannot be released: the checker refuses
    /// `xs.free()` while the view is alive, and a view lives until the function
    /// returns. That leak is a property of the language rather than a mistake in
    /// the program -- `docs/tutorial/09-arrays.md` documents it -- so it is
    /// reported as a warning that says so, not as an error that refuses the
    /// program.
    borrowed: bool,
}

/// The tracked bindings, innermost scope last.
///
/// Two bindings with one name in different scopes are two bindings, and the
/// innermost one is the one a read resolves to.
type State = Vec<Vec<Binding>>;

/// How a statement leaves control.
///
/// Only [`Flow::Falls`] reaches the statement after this one. The other three
/// matter because a branch that leaves early does not get a say in the merge
/// that follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// Execution continues after the statement.
    Falls,
    /// A `return` left the function.
    Returns,
    /// A `break` left the loop.
    Breaks,
    /// A `continue` started the next iteration.
    Continues,
}

struct LeakPass<'a> {
    out: &'a mut Vec<Diagnostic>,
    /// The file the function being walked was written in.
    file: FileId,
    frames: State,
}

impl LeakPass<'_> {
    /// Walk one function.
    ///
    /// A lambda is a function of its own, so its body is checked with its own
    /// scopes and the enclosing function's state is put back afterwards.
    fn check_fn(&mut self, function: &HirFn) {
        let outer_frames = std::mem::take(&mut self.frames);
        let outer_file = self.file;
        self.file = function.file;
        let _ = self.block(&function.body);
        self.frames = outer_frames;
        self.file = outer_file;
    }

    /// Walk a block, reporting the bindings it declared that are still live
    /// when it ends.
    fn block(&mut self, block: &HirBlock) -> Flow {
        self.frames.push(Vec::new());
        let mut flow = Flow::Falls;
        for stmt in &block.stmts {
            flow = self.stmt(stmt);
            if flow != Flow::Falls {
                break;
            }
        }
        // A block that was left by `return`, `break` or `continue` did not
        // reach its end, and the `return` path has already reported what it
        // abandoned. Reporting again here would double every diagnostic.
        if flow == Flow::Falls {
            self.report_scope_leaks();
        }
        self.frames.pop();
        flow
    }

    fn stmt(&mut self, stmt: &HirStmt) -> Flow {
        match stmt {
            HirStmt::Expr(expr, _, _) => {
                self.walk(expr);
                Flow::Falls
            }
            HirStmt::Const {
                name,
                ty,
                init,
                span,
                ..
            }
            | HirStmt::Var {
                name,
                ty,
                init,
                span,
                ..
            } => {
                self.walk(init);
                let ty = ty.clone().unwrap_or_else(|| expr_ty(init));
                // A block from `arena_alloc` is owned by the arena, not by the
                // binding: the arena is the value that has to be released, and
                // tracking the block would demand a release the language has no
                // way to write.
                if !is_arena_view(init) {
                    self.declare(name, &ty, *span);
                }
                Flow::Falls
            }
            HirStmt::Destructure {
                init,
                bindings,
                span,
                ..
            } => {
                self.walk(init);
                for binding in bindings.iter().flatten() {
                    self.declare(&binding.0, &binding.1, *span);
                }
                Flow::Falls
            }
            // The store gives the binding a new live value. The old one was
            // either released by the store (`release_previous`) or had already
            // been released, and the new one has not been released at all.
            HirStmt::Assign { name, value, .. } => {
                self.walk(value);
                self.rebind(name);
                Flow::Falls
            }
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.walk(base);
                self.walk(index);
                self.hand_off(value);
                self.walk(value);
                Flow::Falls
            }
            HirStmt::FieldAssign { value, .. } => {
                self.hand_off(value);
                self.walk(value);
                Flow::Falls
            }
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.walk(pointer);
                self.walk(index);
                self.hand_off(value);
                self.walk(value);
                Flow::Falls
            }
            HirStmt::Return(value, _) => {
                if let Some(value) = value {
                    // Returning a value hands it to the caller, who owns it.
                    self.hand_off(value);
                    self.walk(value);
                }
                // Every binding still live at a return is abandoned there.
                self.report_all_live();
                Flow::Returns
            }
            HirStmt::If {
                init,
                cond,
                then_block,
                else_block,
                ..
            } => {
                if let Some(init) = init {
                    let flow = self.stmt(init);
                    if flow != Flow::Falls {
                        return flow;
                    }
                }
                self.walk(cond);
                self.branch(then_block, else_block.as_ref())
            }
            HirStmt::ForRange {
                start, end, body, ..
            } => {
                self.walk(start);
                self.walk(end);
                self.loop_body(body);
                Flow::Falls
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.walk(iterable);
                self.loop_body(body);
                Flow::Falls
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    let flow = self.stmt(init);
                    if flow != Flow::Falls {
                        return flow;
                    }
                }
                if let Some(cond) = cond {
                    self.walk(cond);
                }
                let entry = self.frames.clone();
                let _ = self.block(body);
                if let Some(update) = update {
                    self.walk(update);
                }
                self.frames = entry;
                Flow::Falls
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.walk(subject);
                self.switch_arms(arms)
            }
            HirStmt::Break(_) => Flow::Breaks,
            HirStmt::Continue(_) => Flow::Continues,
            // The checker reads a defer body where the `defer` is written and
            // marks what it frees as spent from there, so the pass does the
            // same instead of inventing a second, scope-exit rule.
            HirStmt::Defer { body, .. } => {
                let _ = self.block(body);
                Flow::Falls
            }
            // A contract is not executed, but the checker did read it, so a
            // value it hands to a call is spent from here on. Walking the
            // clause keeps the two halves agreeing about what is still owned.
            HirStmt::LoopInvariant(contract) => {
                self.walk(&contract.expr);
                Flow::Falls
            }
        }
    }

    /// Merge the two paths of an `if`.
    fn branch(&mut self, then_block: &HirBlock, else_block: Option<&HirBlock>) -> Flow {
        let entry = self.frames.clone();
        let mut falling: Vec<State> = Vec::new();
        if self.block(then_block) == Flow::Falls {
            falling.push(self.frames.clone());
        }
        self.frames = entry.clone();
        match else_block {
            Some(block) => {
                if self.block(block) == Flow::Falls {
                    falling.push(self.frames.clone());
                }
            }
            // No `else` is a path of its own, and it released nothing.
            None => falling.push(entry.clone()),
        }
        match intersect(&falling) {
            Some(state) => {
                self.frames = state;
                Flow::Falls
            }
            None => {
                // Every branch left before the merge, so the `if` does not fall
                // through either. Which way it left does not matter to the
                // enclosing statement: all that matters is that it did.
                self.frames = entry;
                Flow::Returns
            }
        }
    }

    /// Walk a loop body for the errors inside it, then forget what it released.
    ///
    /// A loop may run zero times, so a release inside it is not a release after
    /// it: the state after the loop is the state before it.
    fn loop_body(&mut self, body: &HirBlock) {
        let entry = self.frames.clone();
        let _ = self.block(body);
        self.frames = entry;
    }

    /// Merge the arms of a `switch`.
    ///
    /// The checker proved the arms are exhaustive, so every path goes through
    /// exactly one of them and the merge is the intersection of all of them.
    fn switch_arms(&mut self, arms: &[HirSwitchArm]) -> Flow {
        if arms.is_empty() {
            return Flow::Falls;
        }
        let entry = self.frames.clone();
        let mut falling: Vec<State> = Vec::with_capacity(arms.len());
        for arm in arms {
            self.frames = entry.clone();
            if let Some(guard) = &arm.guard {
                self.walk(guard);
            }
            self.walk(&arm.body);
            falling.push(self.frames.clone());
        }
        match intersect(&falling) {
            Some(state) => self.frames = state,
            None => self.frames = entry,
        }
        Flow::Falls
    }

    /// Walk an expression for the hand-offs and lambdas inside it.
    ///
    /// A plain read is not a hand-off: `p[0]`, `h.p` and `&x` borrow the value,
    /// so the binding that owns it stays live.
    fn walk(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Literal(..)
            | HirExpr::Ident(..)
            | HirExpr::AddrOf { .. }
            | HirExpr::Capture { .. } => {}
            HirExpr::Move(inner, _, _) => {
                self.hand_off(inner);
                self.walk(inner);
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            HirExpr::Unary { operand, .. } => self.walk(operand),
            HirExpr::Call { callee, args, .. } => {
                // An arena hands out a view into memory it owns. Passing the
                // handle does not give it up -- the next `arena_alloc` uses the
                // same handle -- so the first argument of `arena_alloc` is a
                // borrow, not a hand-off. `arena_free_all` is an ordinary call
                // and does release the handle.
                let arena = names_arena_alloc(callee);
                for (index, arg) in args.iter().enumerate() {
                    if !(arena && index == 0) {
                        self.hand_off(arg);
                    }
                    self.walk(arg);
                }
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.hand_off(arg);
                    self.walk(arg);
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.hand_off(value);
                    self.walk(value);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.hand_off(element);
                    self.walk(element);
                }
            }
            HirExpr::FieldAccess { base, .. } => self.walk(base),
            HirExpr::Index { base, index, .. } => {
                self.walk(base);
                self.walk(index);
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.borrow(base);
                self.walk(base);
                self.walk(start);
                self.walk(end);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.walk(expr),
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.walk(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.walk(guard);
                    }
                    self.walk(&arm.body);
                }
            }
            HirExpr::Lambda { f, .. } => self.check_fn(f),
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.walk(receiver);
                self.walk(func);
                if let Some(init) = init {
                    self.walk(init);
                }
            }
        }
    }

    /// Count `expr` as released when it is a bare name of a tracked binding.
    ///
    /// Only a bare name hands the value over. `p[0]` and `h.p` read through the
    /// value and `&x` borrows it; none of them gives the owner up, and the
    /// checker refuses `free` on them for the same reason.
    fn hand_off(&mut self, expr: &HirExpr) {
        let HirExpr::Ident(path, _, ty) = expr else {
            return;
        };
        if path.len() != 1 || linearity_of(&ty.kind) != Linearity::Linear {
            return;
        }
        let Some(name) = path.first() else {
            return;
        };
        if let Some(binding) = self.find(name) {
            binding.released = true;
        }
    }

    /// The innermost binding with this name.
    fn find(&mut self, name: &str) -> Option<&mut Binding> {
        self.frames
            .iter_mut()
            .rev()
            .flat_map(|frame| frame.iter_mut().rev())
            .find(|binding| binding.name == name)
    }

    /// Record that `expr` has been viewed, when it names a tracked binding.
    ///
    /// A view borrows its source for the rest of the function, which is why the
    /// checker refuses a release after one. Only a bare name is a binding:
    /// `xs[0..2][0..1]` views a view, and the first view already borrowed the
    /// array.
    fn borrow(&mut self, expr: &HirExpr) {
        let HirExpr::Ident(path, _, ty) = expr else {
            return;
        };
        if path.len() != 1 || linearity_of(&ty.kind) != Linearity::Linear {
            return;
        }
        let Some(name) = path.first() else {
            return;
        };
        if let Some(binding) = self.find(name) {
            binding.borrowed = true;
        }
    }

    /// Track a new binding when its type is linear.
    fn declare(&mut self, name: &str, ty: &Type, span: Span) {
        if linearity_of(&ty.kind) != Linearity::Linear {
            return;
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.push(Binding {
                name: name.to_string(),
                span,
                released: false,
                borrowed: false,
            });
        }
    }

    /// Give a binding a new, live value.
    fn rebind(&mut self, name: &str) {
        if let Some(binding) = self.find(name) {
            binding.released = false;
        }
    }

    /// Report every binding of the scope that is ending that is still live.
    fn report_scope_leaks(&mut self) {
        let live: Vec<(String, Span, bool)> = self
            .frames
            .last()
            .map(|frame| {
                frame
                    .iter()
                    .filter(|binding| !binding.released)
                    .map(|binding| (binding.name.clone(), binding.span, binding.borrowed))
                    .collect()
            })
            .unwrap_or_default();
        for (name, span, borrowed) in live {
            self.report(&name, span, borrowed);
        }
    }

    /// Report every binding still live at a `return`.
    fn report_all_live(&mut self) {
        let live: Vec<(String, Span, bool)> = self
            .frames
            .iter()
            .flatten()
            .filter(|binding| !binding.released)
            .map(|binding| (binding.name.clone(), binding.span, binding.borrowed))
            .collect();
        for (name, span, borrowed) in live {
            self.report(&name, span, borrowed);
        }
    }

    /// Report one live value at the point it is abandoned.
    ///
    /// The severity depends on whether the program had a way to release it. A
    /// value the language cannot release is a warning; anything else is an
    /// error.
    fn report(&mut self, name: &str, span: Span, borrowed: bool) {
        if borrowed {
            self.borrowed_leak(name, span);
        } else {
            self.leak(name, span);
        }
    }

    fn leak(&mut self, name: &str, span: Span) {
        self.out.push(
            Diagnostic::new(
                Severity::Error,
                self.file,
                span,
                format!("linear value `{name}` is never released"),
            )
            .with_label("this value is still live when its scope ends")
            .with_note(format!(
                "release it before the scope ends, as in `defer {{ {name}.free(); }}`"
            ))
            .with_code(codes::LEAK)
            .with_proof(ProofTrace {
                obligation: "leak",
                missing_fact: format!(
                    "`{name}` is not released on every path out of its scope"
                ),
                remedy: format!(
                    "release it with `{name}.free()`, `defer {{ {name}.free(); }}`, or by returning it"
                ),
            }),
        );
    }

    /// Report a value the language gives no way to release.
    ///
    /// This is the pass's one warning, and it exists because both alternatives
    /// are worse: an error would refuse a documented idiom
    /// (`docs/tutorial/09-arrays.md` shows a view over a `[]T` and says the
    /// array is not freed), and silence would be a verifier that claims to catch
    /// leaks and does not.
    fn borrowed_leak(&mut self, name: &str, span: Span) {
        self.out.push(
            Diagnostic::new(
                Severity::Warning,
                self.file,
                span,
                format!("linear value `{name}` is borrowed by a view and cannot be released"),
            )
            .with_label("the view keeps this array alive until the function returns")
            .with_note(format!(
                "this leaks by design: `{name}.free()` is refused while the view is alive"
            ))
            .with_code(codes::LEAK)
            .with_proof(ProofTrace {
                obligation: "leak",
                missing_fact: format!(
                    "`{name}` has a live view, so the language gives no way to release it"
                ),
                remedy: "use a fixed array `[T; N]`, or copy the elements into a `[]T` you own"
                    .to_string(),
            }),
        );
    }
}

/// Whether `callee` names `arena_alloc` rather than a module path or a user
/// function.
fn names_arena_alloc(callee: &[String]) -> bool {
    callee.len() == 1 && is_arena_alloc(&callee[0])
}

/// Whether `expr` produces a block the arena owns rather than a new owner.
fn is_arena_view(expr: &HirExpr) -> bool {
    matches!(expr, HirExpr::Call { callee, .. } if names_arena_alloc(callee))
}

/// The state that is released on every one of `states`.
///
/// Every state was taken at the same point of the same function, so they have
/// the same shape: the same scopes holding the same bindings. `zip` ignores a
/// mismatch rather than panicking, and a binding a later state does not know
/// about keeps its bit, which can only hide a leak, never invent one.
fn intersect(states: &[State]) -> Option<State> {
    let mut states = states.iter();
    let mut merged = states.next()?.clone();
    for state in states {
        for (frame, other) in merged.iter_mut().zip(state.iter()) {
            for (binding, other) in frame.iter_mut().zip(other.iter()) {
                binding.released = binding.released && other.released;
                // A view on any path means the value may not be releasable
                // after the merge, so the warning wins over the error.
                binding.borrowed = binding.borrowed || other.borrowed;
            }
        }
    }
    Some(merged)
}

#[cfg(test)]
mod tests {
    use yashy_common::SourceMap;
    use yashy_lexer::Lexer;
    use yashy_parser::Parser;
    use yashy_sema::check_program;

    use crate::{verify, VerifyOptions, VerifyReport};

    /// Parse and check `source`, then verify it.
    ///
    /// A program that does not type-check has no HIR, so its check diagnostics
    /// are the whole answer: the verifier reasons about typed code and has
    /// nothing to say about code that never became typed.
    fn verify_source(source: &str) -> VerifyReport {
        let mut sources = SourceMap::new();
        let file = sources.add_file("verify.ya", source.to_string());
        let tokens = Lexer::new(source)
            .tokenize()
            .expect("the fixture's lexer must succeed");
        let mut parser = Parser::new(tokens, &sources, file);
        let program = parser.parse_program().expect("the fixture must parse");
        match check_program(&program, &sources, file) {
            Ok(hir) => verify(&hir, &VerifyOptions::default()),
            Err(diagnostics) => VerifyReport {
                diagnostics,
                obligations: Vec::new(),
            },
        }
    }

    fn codes(report: &VerifyReport) -> Vec<&'static str> {
        report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.unwrap_or("<uncoded>"))
            .collect()
    }

    #[test]
    fn an_unfreed_pointer_is_a_leak() {
        let report = verify_source("fn main() { let buf = alloc(16); }");
        assert_eq!(codes(&report), vec!["E1000"]);
        assert!(report.diagnostics[0].message.contains("`buf`"));
    }

    #[test]
    fn freeing_the_pointer_is_not_a_leak() {
        assert!(
            verify_source("fn main() { let buf = alloc(16); buf.free(); }")
                .diagnostics
                .is_empty()
        );
    }

    #[test]
    fn returning_the_pointer_is_not_a_leak() {
        let source = "fn make() -> *mut int { let buf = alloc(16); return buf; } \
                      fn main() { let p = make(); p.free(); }";
        assert!(verify_source(source).diagnostics.is_empty());
    }

    #[test]
    fn storing_the_pointer_in_a_struct_field_is_not_a_leak() {
        let source = "struct Holder { p: *mut int } \
                      fn main() { let buf = alloc(16); let h = Holder { p: buf }; }";
        assert!(verify_source(source).diagnostics.is_empty());
    }

    #[test]
    fn freeing_on_one_branch_and_reading_after_the_merge_is_rejected() {
        let source = "fn main() { let buf = alloc(16); let c = 1 > 0; \
                      if c { buf.free(); } else { } buf.free(); }";
        assert_eq!(codes(&verify_source(source)), vec!["E0500"]);
    }

    #[test]
    fn freeing_on_both_branches_is_not_a_leak() {
        let source = "fn main() { let buf = alloc(16); let c = 1 > 0; \
                      if c { buf.free(); } else { buf.free(); } }";
        assert!(verify_source(source).diagnostics.is_empty());
    }

    #[test]
    fn freeing_on_only_one_branch_is_a_leak_on_the_other_path() {
        let source = "fn main() { let buf = alloc(16); let c = 1 > 0; \
                      if c { buf.free(); } else { } }";
        assert_eq!(codes(&verify_source(source)), vec!["E1000"]);
    }

    #[test]
    fn an_arena_block_is_not_a_leak() {
        let source = "fn main() { let arena = arena_new(); \
                      let first = arena_alloc(arena, 4); \
                      first[0] = 1; \
                      arena_free_all(arena); }";
        assert!(verify_source(source).diagnostics.is_empty());
    }

    #[test]
    fn a_leaked_arena_is_still_a_leak() {
        let source = "fn main() { let arena = arena_new(); \
                      let first = arena_alloc(arena, 4); \
                      first[0] = 1; }";
        let report = verify_source(source);
        assert_eq!(codes(&report), vec!["E1000"]);
        assert!(
            report.diagnostics[0].message.contains("`arena`"),
            "the arena is the value to release, not the block: {}",
            report.diagnostics[0].message
        );
    }

    #[test]
    fn a_viewed_array_is_a_warning_not_an_error() {
        let source = "fn main() { let xs = []int { 1, 2, 3 }; \
                      const view = xs[0..2]; \
                      print(view.len()); \
                      println(\"\"); }";
        let report = verify_source(source);
        assert_eq!(codes(&report), vec!["E1000"]);
        assert!(
            !report.diagnostics[0].is_error(),
            "a value the language cannot release is a warning: {:?}",
            report.diagnostics[0]
        );
        assert_eq!(report.error_count(), 0);
        assert!(
            report.diagnostics[0].message.contains("view"),
            "the warning must say why: {}",
            report.diagnostics[0].message
        );
    }

    #[test]
    fn a_viewed_array_that_is_never_released_is_not_an_error() {
        // The tutorial's own example: the array is viewed, so `xs.free()` would
        // be refused. The program must still build.
        let source = "fn main() { let xs = []int { 10, 20, 30 }; \
                      const mid = xs[1..3]; \
                      let sum = 0; \
                      for value in mid { sum = sum + value; } \
                      print(sum); \
                      println(\"\"); }";
        assert_eq!(verify_source(source).error_count(), 0);
    }
}
