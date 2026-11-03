//! Escape analysis: where one allocation site may live.
//!
//! The language already has linear ownership, so an allocation reaches exactly
//! one `dealloc`, one `move`, or one returned owner. Correctness does not depend
//! on this pass — a missed promotion only leaves a heap allocation in place.
//! What the pass does is answer one question per `alloc` site: does the pointer
//! outlive its function, and does it outlive the block that allocated it?
//!
//! Three placements, one rule each:
//!
//! * [`Placement::Stack`] when the pointer never escapes the function and the
//!   size is a constant small enough for a frame. The allocation becomes one
//!   `alloca` in the entry block.
//! * [`Placement::Arena`] when the pointer never escapes the function, the
//!   binding was declared in the same block as the allocation, and the size is
//!   not known to be small. The block gets one arena, freed when the block
//!   exits, so a loop body recycles its memory once per iteration instead of
//!   calling the allocator twice.
//! * [`Placement::Heap`] otherwise — and that is the conservative default. An
//!   escape this pass cannot follow is an escape.
//!
//! # What counts as an escape
//!
//! A bare binding name in a value position is an escape: a return, a call
//! argument, a struct/tuple/slice literal, a move, a closure capture, a
//! reassignment. A read through the pointer (`p[i]`, `p[0].field`) is not, and
//! neither is a write through it (`p[i] = v`) or a `dealloc(p)`/`p.free()`:
//! those use the pointer, they do not hand it to anyone else.
//!
//! Treating a call argument as an escape is stronger than LLVM needs — the
//! callee is handed the same pointer, not a copy — but it is the rule that stays
//! true without an interprocedural analysis. A callee can store the pointer in a
//! global, return it, or keep it alive after the call, and this pass does not
//! look across functions.
//!
//! # The verifier's facts
//!
//! Phase 3 does not record a "this value is shared" fact yet, so there is
//! nothing to cross-check here. The `facts` argument is part of the interface
//! for the day it does: a disagreement between the ownership pass and this one
//! would be a verifier bug, and the pass would refuse to promote rather than
//! turn it into a miscompile.

use std::collections::{BTreeMap, HashMap, HashSet};

use yashy_common::{ProofFacts, Span};
use yashy_sema::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, Literal};

/// The largest constant allocation a frame will hold.
///
/// A source-level `alloc` can ask for any count. Promoting a one-megabyte buffer
/// to the stack overflows it, and a stack overflow is a crash rather than a
/// diagnostic. Four KiB is the cap; anything larger stays on the heap or in a
/// block arena.
pub const STACK_LIMIT: u64 = 4096;

/// One `alloc` call, identified by the span the checker gave the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AllocSite {
    start: u32,
    end: u32,
}

impl AllocSite {
    /// The site named by a call's span.
    pub fn of(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }

    /// The site's first byte, for diagnostics and tests.
    pub fn start(self) -> u32 {
        self.start
    }

    /// The site's end byte.
    pub fn end(self) -> u32 {
        self.end
    }
}

/// One block's arena, named by the block's span start.
///
/// The id is the block's position in the source, which is unique within a
/// function and stable across runs. Phase 5's `arena { }` syntax reuses this
/// type with an explicit name; nothing in this pass invents an id that the
/// code generator cannot map back to the block it is lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArenaId(pub u32);

impl ArenaId {
    /// The arena belonging to `block`.
    pub fn of_block(block: &HirBlock) -> Self {
        Self(block.span.start)
    }
}

/// Where one allocation site may live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The ordinary runtime allocator. The conservative default.
    Heap,
    /// One `alloca` in the function's entry block.
    Stack,
    /// The innermost enclosing block's arena.
    Arena(ArenaId),
}

/// What one pass over a function learned about its allocation sites.
#[derive(Default)]
struct Analysis<'f> {
    sites: BTreeMap<AllocSite, SiteInfo>,
    bindings: HashMap<String, (AllocSite, Option<u32>)>,
    frames: Vec<Frame>,
    block_spans: Vec<u32>,
    lambdas: Vec<&'f HirFn>,
}

/// A binding's allocation site and the block it was declared in.
type Binding = (AllocSite, Option<u32>);

/// One lexical scope: the names it declared, and what they shadowed.
#[derive(Default)]
struct Frame {
    declared: HashSet<String>,
    saved: Vec<(String, Option<Binding>)>,
}

/// What the walk learned about one site.
#[derive(Clone, Copy)]
struct SiteInfo {
    /// `Some(bytes)` when the count is a literal and the product fits.
    constant_bytes: Option<u64>,
    /// The pointer reached a return, a call, an aggregate, a move or a capture.
    escaped: bool,
    /// The block the allocating binding was declared in.
    binding_block: Option<u32>,
    /// The block the allocation expression itself is written in.
    site_block: Option<u32>,
}

/// Classify every `alloc` site in `f`.
///
/// The result contains an entry for every site, including the ones that stay on
/// the heap: a caller that wants "what changed" can look for the non-heap
/// values, and a caller that wants the default does not have to tell a missing
/// entry from a deliberate one.
pub fn classify(f: &HirFn, _facts: &ProofFacts) -> HashMap<AllocSite, Placement> {
    analyse(f).placements()
}

/// Count the sites this pass would promote, for `--timings`.
///
/// Lambdas are functions too, so their allocations are classified and counted
/// as well. A lambda's body is emitted by its own `gen_body` call, which
/// classifies it independently; the count walks the tree because no single
/// function item owns a nested lambda.
pub fn promotion_counts(hir: &HirProgram, facts: &ProofFacts) -> (usize, usize) {
    fn count(f: &HirFn, facts: &ProofFacts, stack: &mut usize, arena: &mut usize) {
        let analysis = analyse(f);
        for placement in analysis.placements().values() {
            match placement {
                Placement::Stack => *stack += 1,
                Placement::Arena(_) => *arena += 1,
                Placement::Heap => {}
            }
        }
        for lambda in analysis.lambdas {
            count(lambda, facts, stack, arena);
        }
    }

    let mut stack = 0;
    let mut arena = 0;
    for item in &hir.items {
        if let HirItem::Fn(function) = item {
            count(function, facts, &mut stack, &mut arena);
        }
    }
    (stack, arena)
}

fn analyse<'f>(f: &'f HirFn) -> Analysis<'f> {
    let mut analysis = Analysis::default();
    analysis.block(&f.body);
    analysis
}

impl<'f> Analysis<'f> {
    fn placements(&self) -> HashMap<AllocSite, Placement> {
        let mut out = HashMap::new();
        for (site, info) in &self.sites {
            let placement = if info.escaped {
                Placement::Heap
            } else {
                match info.constant_bytes {
                    Some(bytes) if bytes <= STACK_LIMIT => Placement::Stack,
                    _ => match (info.binding_block, info.site_block) {
                        // A dynamic (or large) allocation can use the block's
                        // arena only when the binding dies with the block. An
                        // assignment to an outer binding outlives it.
                        (Some(binding), Some(site_block)) if binding == site_block => {
                            Placement::Arena(ArenaId(binding))
                        }
                        _ => Placement::Heap,
                    },
                }
            };
            out.insert(*site, placement);
        }
        out
    }

    fn current_block(&self) -> Option<u32> {
        self.block_spans.last().copied()
    }

    fn block(&mut self, block: &'f HirBlock) {
        self.frames.push(Frame::default());
        self.block_spans.push(block.span.start);
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        self.block_spans.pop();
        self.pop_frame();
    }

    fn push_scope(&mut self) {
        self.frames.push(Frame::default());
    }

    fn pop_frame(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        for (name, previous) in frame.saved.into_iter().rev() {
            match previous {
                Some(binding) => {
                    self.bindings.insert(name, binding);
                }
                None => {
                    self.bindings.remove(&name);
                }
            }
        }
    }

    fn declare(&mut self, name: &str, binding: Option<Binding>) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        if frame.declared.insert(name.to_string()) {
            let previous = self.bindings.get(name).copied();
            frame.saved.push((name.to_string(), previous));
        }
        match binding {
            Some(binding) => {
                self.bindings.insert(name.to_string(), binding);
            }
            None => {
                self.bindings.remove(name);
            }
        }
    }

    /// Record an `alloc` call and answer its site.
    ///
    /// The first walk wins: the same expression cannot have two counts, and a
    /// later encounter is the same site seen from a different context.
    fn record_alloc(&mut self, expr: &HirExpr) -> Option<AllocSite> {
        let HirExpr::Call {
            callee, args, span, ..
        } = expr
        else {
            return None;
        };
        if callee.last().map(String::as_str) != Some("alloc") {
            return None;
        }
        let site = AllocSite::of(*span);
        let constant_bytes = match args.first() {
            Some(HirExpr::Literal(Literal::I64(count), ..)) if *count > 0 => u64::try_from(*count)
                .ok()
                .and_then(|count| count.checked_mul(8)),
            _ => None,
        };
        let site_block = self.current_block();
        self.sites.entry(site).or_insert(SiteInfo {
            constant_bytes,
            escaped: false,
            binding_block: None,
            site_block,
        });
        Some(site)
    }

    fn escape_name(&mut self, name: &str) {
        let Some((site, _)) = self.bindings.get(name).copied() else {
            return;
        };
        if let Some(info) = self.sites.get_mut(&site) {
            info.escaped = true;
        }
    }

    fn stmt(&mut self, stmt: &'f HirStmt) {
        match stmt {
            HirStmt::Expr(expr, ..) => self.value(expr, Mode::Escape),
            HirStmt::Const { name, init, .. } | HirStmt::Var { name, init, .. } => {
                if let Some(site) = self.record_alloc(init) {
                    let block = self.current_block();
                    self.declare(name, Some((site, block)));
                    if let Some(info) = self.sites.get_mut(&site) {
                        info.binding_block = block;
                    }
                } else {
                    self.value(init, Mode::Escape);
                    self.declare(name, None);
                }
            }
            HirStmt::Destructure { init, .. } => self.value(init, Mode::Escape),
            HirStmt::Assign { name, value, .. } => {
                // The RHS is evaluated before the binding changes, so a shadow
                // or a self-reference still sees the old value.
                if let Some(site) = self.record_alloc(value) {
                    // An assignment to a binding declared outside the current
                    // block does not make the allocation outlive the function,
                    // but it does outlive the block, so the arena rule must not
                    // apply. Keep the declaration block when one is known.
                    let block = self.bindings.get(name).and_then(|(_, block)| *block);
                    self.bindings.insert(name.clone(), (site, block));
                    if let Some(info) = self.sites.get_mut(&site) {
                        info.binding_block = block;
                    }
                } else {
                    self.value(value, Mode::Escape);
                    self.bindings.remove(name);
                }
            }
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.value(base, Mode::Read);
                self.value(index, Mode::Escape);
                self.value(value, Mode::Escape);
            }
            HirStmt::FieldAssign { value, .. } => self.value(value, Mode::Escape),
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.value(pointer, Mode::Read);
                self.value(index, Mode::Escape);
                self.value(value, Mode::Escape);
            }
            HirStmt::Return(Some(expr), _) => self.value(expr, Mode::Escape),
            HirStmt::Return(None, _) => {}
            HirStmt::If {
                init,
                cond,
                then_block,
                else_block,
                ..
            } => {
                self.push_scope();
                if let Some(init) = init {
                    self.stmt(init);
                }
                self.value(cond, Mode::Escape);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
                self.pop_frame();
            }
            HirStmt::ForRange {
                start, end, body, ..
            } => {
                self.value(start, Mode::Escape);
                self.value(end, Mode::Escape);
                self.block(body);
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.value(iterable, Mode::Escape);
                self.block(body);
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                self.push_scope();
                if let Some(init) = init {
                    self.stmt(init);
                }
                if let Some(cond) = cond {
                    self.value(cond, Mode::Escape);
                }
                if let Some(update) = update {
                    self.value(update, Mode::Escape);
                }
                self.block(body);
                self.pop_frame();
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.value(subject, Mode::Escape);
                for arm in arms {
                    self.push_scope();
                    if let Some(guard) = &arm.guard {
                        self.value(guard, Mode::Escape);
                    }
                    self.value(&arm.body, Mode::Escape);
                    self.pop_frame();
                }
            }
            HirStmt::Defer { body, .. } => self.block(body),
            HirStmt::LoopInvariant(contract) => self.value(&contract.expr, Mode::Escape),
            HirStmt::Break(_) | HirStmt::Continue(_) => {}
        }
    }

    /// Walk `expr`, marking every promoted binding it hands away.
    ///
    /// `Read` is the pointer-read position: `p` in `p[i]` or `p[0].field` names
    /// storage, not a value being given away.
    fn value(&mut self, expr: &'f HirExpr, mode: Mode) {
        if mode == Mode::Read {
            match expr {
                HirExpr::Ident(..) => return,
                HirExpr::Index { base, index, .. } => {
                    self.value(base, Mode::Read);
                    self.value(index, Mode::Escape);
                    return;
                }
                HirExpr::FieldAccess { base, .. } => {
                    self.value(base, Mode::Read);
                    return;
                }
                _ => {}
            }
        }

        match expr {
            HirExpr::Literal(..) | HirExpr::AddrOf { .. } | HirExpr::Capture { .. } => {}
            HirExpr::Ident(path, ..) => {
                if let [name] = path.as_slice() {
                    self.escape_name(name);
                }
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.value(lhs, Mode::Escape);
                self.value(rhs, Mode::Escape);
            }
            HirExpr::Unary { operand, .. } => self.value(operand, Mode::Escape),
            HirExpr::Move(inner, ..) => {
                self.value(inner, Mode::Escape);
                if let HirExpr::Ident(path, ..) = &**inner {
                    if let [name] = path.as_slice() {
                        self.bindings.remove(name);
                    }
                }
            }
            HirExpr::Call { callee, args, .. } => {
                if callee.last().map(String::as_str) == Some("alloc") {
                    // Reaching here means the allocation is not the direct
                    // initializer of a binding: it is an argument, a return
                    // value, or a discarded temporary. It has no owner this
                    // pass can follow, so it stays on the heap.
                    if let Some(site) = self.record_alloc(expr) {
                        if let Some(info) = self.sites.get_mut(&site) {
                            info.escaped = true;
                        }
                    }
                } else if callee.last().map(String::as_str) == Some("dealloc") {
                    // `.free()` ends the allocation; it does not hand the
                    // pointer to anyone.
                    if let Some(first) = args.first() {
                        self.value(first, Mode::Read);
                    }
                    for arg in args.iter().skip(1) {
                        self.value(arg, Mode::Escape);
                    }
                } else {
                    for arg in args {
                        self.value(arg, Mode::Escape);
                    }
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.value(value, Mode::Escape);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.value(element, Mode::Escape);
                }
            }
            HirExpr::FieldAccess { base, .. } => self.value(base, Mode::Read),
            HirExpr::Index { base, index, .. } => {
                self.value(base, Mode::Read);
                self.value(index, Mode::Escape);
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.value(base, Mode::Read);
                self.value(start, Mode::Escape);
                self.value(end, Mode::Escape);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => {
                self.value(expr, Mode::Escape)
            }
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.value(subject, Mode::Escape);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.value(guard, Mode::Escape);
                    }
                    self.value(&arm.body, Mode::Escape);
                }
            }
            // The body is a function of its own and is classified separately by
            // its own `gen_body`. A capture copies the value into the closure's
            // environment, so an outer promoted pointer captured here escapes.
            HirExpr::Lambda { f, .. } => {
                for capture in &f.captures {
                    self.escape_name(&capture.name);
                }
                self.lambdas.push(f.as_ref());
            }
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.value(receiver, Mode::Escape);
                self.value(func, Mode::Escape);
                if let Some(init) = init {
                    self.value(init, Mode::Escape);
                }
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.value(arg, Mode::Escape);
                }
            }
        }
    }
}

/// Whether a name is being read through or handed away.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Escape,
}

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
            "fn f(n: int) { let p = alloc(n); p[0] = 1; p.free(); }",
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
            "fn f(n: int) { let p = alloc(1); if n > 0 { p = alloc(n); } p.free(); }",
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
            "fn consume(p: *mut int) { p[0] = 1; }\n\
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
}
