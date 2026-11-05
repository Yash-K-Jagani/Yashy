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

use crate::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, Literal};

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

/// How an `arena { }` block's promise is broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaFindingKind {
    /// The value crosses the block boundary: it is returned, stored in a
    /// binding declared outside, or handed to a thread.
    Escape,
    /// An individual free is applied to a value the block owns.
    Free,
}

/// One arena violation, at the use that commits it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaFinding {
    /// Whether the value escaped or was freed individually.
    pub kind: ArenaFindingKind,
    /// The binding whose value is arena-owned.
    pub name: String,
    /// The use: the `return`, the assignment, the `thread_spawn`, or the free.
    pub span: Span,
    /// The `arena` block that owns the value.
    pub boundary: Span,
}

/// Every arena violation in `f`, including those in nested lambdas.
///
/// The verifier calls this and nothing else: the placement decision and the
/// proof obligation come from the same walk, so a value cannot be arena-placed
/// by codegen and silently accepted by the verifier.
pub fn arena_findings(f: &HirFn, _facts: &ProofFacts) -> Vec<ArenaFinding> {
    let mut analysis = analyse(f);
    let mut out = std::mem::take(&mut analysis.findings);
    for lambda in analysis.lambdas {
        out.extend(arena_findings(lambda, _facts));
    }
    out
}

/// A placement and the reason escape analysis reached it.
///
/// The reason is the report's whole content: `--opt-report` prints it beside
/// the allocation, so a programmer can see which fact was missing rather than
/// only that the allocation stayed on the heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// Where the allocation lives.
    pub placement: Placement,
    /// Why, in a sentence.
    pub reason: String,
}

/// What one pass over a function learned about its allocation sites.
#[derive(Default)]
struct Analysis<'f> {
    sites: BTreeMap<AllocSite, SiteInfo>,
    bindings: HashMap<String, Binding>,
    frames: Vec<Frame>,
    block_spans: Vec<u32>,
    arena_stack: Vec<Span>,
    findings: Vec<ArenaFinding>,
    lambdas: Vec<&'f HirFn>,
}

/// A binding's allocation site, the block it was declared in, and the arena
/// block (if any) that owns the allocation.
#[derive(Clone, Copy)]
struct Binding {
    site: AllocSite,
    block: Option<u32>,
    arena: Option<u32>,
}

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
    /// The innermost `arena { }` block around the allocation.
    arena_block: Option<u32>,
    /// The span of that block, for the diagnostic.
    arena_boundary: Option<Span>,
    /// The value crosses the arena's boundary. Placement falls back to the
    /// heap so an unverified build cannot leave a dangling pointer behind.
    arena_escape: bool,
}

/// Where an allocation that is too big for the frame, or has no literal
/// element count, can live, and why.
///
/// A block arena can take either kind only when the binding dies with the block
/// that allocates it: an assignment to an outer binding outlives the arena.
fn scope_placement(info: &SiteInfo) -> (Placement, &'static str) {
    match (info.binding_block, info.site_block) {
        (Some(binding), Some(site_block)) if binding == site_block => (
            Placement::Arena(ArenaId(binding)),
            "the pointer does not escape and the binding dies with the block \
             that allocates it, so the block arena owns it",
        ),
        (Some(_), _) => (
            Placement::Heap,
            "the binding outlives the block that allocates it, so a block arena \
             would free it too early",
        ),
        _ => (
            Placement::Heap,
            "the allocation is not bound to a name, so no scope owns its lifetime",
        ),
    }
}

/// Classify every `alloc` site in `f`.
///
/// The result contains an entry for every site, including the ones that stay on
/// the heap: a caller that wants "what changed" can look for the non-heap
/// values, and a caller that wants the default does not have to tell a missing
/// entry from a deliberate one.
pub fn classify(f: &HirFn, _facts: &ProofFacts) -> HashMap<AllocSite, Placement> {
    classify_sites(f, _facts)
        .into_iter()
        .map(|(site, info)| (site, info.placement))
        .collect()
}

/// Classify every `alloc` site and keep the reason, for `--opt-report`.
///
/// One walk answers both questions: the placement is the decision and the
/// reason is why. A report that ran its own analysis could disagree with the
/// compiler, which is worse than no report.
pub fn classify_sites(f: &HirFn, _facts: &ProofFacts) -> HashMap<AllocSite, Site> {
    analyse(f).sites()
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
        for site in analysis.sites().values() {
            match site.placement {
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
    fn sites(&self) -> HashMap<AllocSite, Site> {
        let mut out = HashMap::new();
        for (site, info) in &self.sites {
            let (placement, reason) = if info.arena_escape {
                // The verifier refuses this program with E1003. Keeping the
                // heap placement means a build that skipped the verifier does
                // not hand out a pointer the arena will free.
                (
                    Placement::Heap,
                    "the value crosses its `arena` block's boundary, so the \
                     arena must not own it"
                        .to_string(),
                )
            } else if let (Some(arena), Some(_)) =
                (info.arena_block, info.arena_boundary)
            {
                // An explicit `arena { }` block owns every allocation written
                // inside it, whatever its size. The block releases the whole
                // arena on every exit, so the size heuristic does not apply.
                (
                    Placement::Arena(ArenaId(arena)),
                    "the allocation is written inside an `arena` block, so the \
                     block releases it"
                        .to_string(),
                )
            } else if info.escaped {
                (
                    Placement::Heap,
                    "the pointer escapes the function: it reaches a return, a \
                     call, an aggregate, a move, or a capture"
                        .to_string(),
                )
            } else {
                match info.constant_bytes {
                    Some(bytes) if bytes <= STACK_LIMIT => (
                        Placement::Stack,
                        format!(
                            "a literal count of {bytes} bytes fits in the frame \
                             and the pointer does not escape"
                        ),
                    ),
                    Some(bytes) => {
                        // Too large for the frame. The block arena can still
                        // own it when the binding dies with the block.
                        let (placement, why) = scope_placement(info);
                        (
                            placement,
                            format!(
                                "a literal count of {bytes} bytes is larger than the \
                                 {STACK_LIMIT}-byte frame limit; {why}"
                            ),
                        )
                    }
                    None => {
                        let (placement, why) = scope_placement(info);
                        (placement, why.to_string())
                    }
                }
            };
            out.insert(*site, Site { placement, reason });
        }
        out
    }

    fn current_block(&self) -> Option<u32> {
        self.block_spans.last().copied()
    }

    fn block(&mut self, block: &'f HirBlock) {
        self.frames.push(Frame::default());
        self.block_spans.push(block.span.start);
        if block.arena {
            self.arena_stack.push(block.span);
        }
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if block.arena {
            self.arena_stack.pop();
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
        let arena_boundary = self.arena_stack.last().copied();
        self.sites.entry(site).or_insert(SiteInfo {
            constant_bytes,
            escaped: false,
            binding_block: None,
            site_block,
            arena_block: arena_boundary.map(|span| span.start),
            arena_boundary,
            arena_escape: false,
        });
        Some(site)
    }

    /// The site a value expression was allocated at, following a plain alias.
    ///
    /// `let b = a;` copies the pointer, so `b` names the same allocation as
    /// `a`; treating it as unknown would let `return b;` slip past the arena
    /// check. Only an identifier or a move of one aliases: `a.field` copies the
    /// field's value, not the allocation.
    fn alias_site(&self, expr: &HirExpr) -> Option<AllocSite> {
        match expr {
            HirExpr::Ident(path, ..) if path.len() == 1 => {
                self.bindings.get(&path[0]).map(|binding| binding.site)
            }
            HirExpr::Move(inner, ..) => self.alias_site(inner),
            _ => None,
        }
    }

    /// The plain binding `expr` aliases, when it is one.
    fn alias_source_name(&self, expr: &HirExpr) -> Option<String> {
        match expr {
            HirExpr::Ident(path, ..) if path.len() == 1 => Some(path[0].clone()),
            HirExpr::Move(inner, ..) => self.alias_source_name(inner),
            _ => None,
        }
    }

    fn escape_name(&mut self, name: &str) {
        let Some(binding) = self.bindings.get(name).copied() else {
            return;
        };
        if let Some(info) = self.sites.get_mut(&binding.site) {
            info.escaped = true;
        }
    }

    /// Record that `name`'s arena-owned value leaves its block at `at`.
    fn note_name_escape(&mut self, name: &str, at: Span) {
        let Some(binding) = self.bindings.get(name).copied() else {
            return;
        };
        self.note_site_escape(name, at, binding.site);
    }

    /// Record that the value allocated at `site` leaves its block at `at`.
    fn note_site_escape(&mut self, name: &str, at: Span, site: AllocSite) {
        let Some(boundary) = self.sites.get(&site).and_then(|info| info.arena_boundary) else {
            return;
        };
        if let Some(info) = self.sites.get_mut(&site) {
            info.arena_escape = true;
        }
        let finding = ArenaFinding {
            kind: ArenaFindingKind::Escape,
            name: name.to_string(),
            span: at,
            boundary,
        };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    /// Record a free of an arena-owned value: the block releases it, so the
    /// individual `.free()` would double-free.
    fn note_arena_free(&mut self, name: &str, at: Span) {
        let Some(binding) = self.bindings.get(name).copied() else {
            return;
        };
        let Some(boundary) = self.sites.get(&binding.site).and_then(|info| info.arena_boundary) else {
            return;
        };
        let finding = ArenaFinding {
            kind: ArenaFindingKind::Free,
            name: name.to_string(),
            span: at,
            boundary,
        };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    /// Record every arena-owned value `expr` hands out at `at`.
    ///
    /// A value position, not a read-through one: `a` in `return a`, not `a` in
    /// `a[0]`. Aggregates count element by element, because the aggregate may
    /// outlive the block even when the local holding it does not.
    fn note_arena_escape(&mut self, expr: &HirExpr, at: Span) {
        match expr {
            HirExpr::Ident(path, span, ..) => {
                if let [name] = path.as_slice() {
                    self.note_name_escape(name, *span);
                }
            }
            HirExpr::Move(inner, ..)
            | HirExpr::Cast { expr: inner, .. }
            | HirExpr::Try { expr: inner, .. } => self.note_arena_escape(inner, at),
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.note_arena_escape(value, at);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.note_arena_escape(element, at);
                }
            }
            HirExpr::SwitchExpr { arms, .. } => {
                for arm in arms {
                    self.note_arena_escape(&arm.body, at);
                }
            }
            HirExpr::Lambda { f, .. } => {
                for capture in &f.captures {
                    self.note_name_escape(&capture.name, at);
                }
            }
            _ => {}
        }
    }

    fn stmt(&mut self, stmt: &'f HirStmt) {
        match stmt {
            HirStmt::Expr(expr, ..) => self.value(expr, Mode::Escape),
            HirStmt::Const { name, init, .. } | HirStmt::Var { name, init, .. } => {
                if let Some(site) = self.record_alloc(init) {
                    let block = self.current_block();
                    let arena = self.sites.get(&site).and_then(|info| info.arena_block);
                    self.declare(name, Some(Binding { site, block, arena }));
                    if let Some(info) = self.sites.get_mut(&site) {
                        info.binding_block = block;
                    }
                } else if let Some(site) = self.alias_site(init) {
                    // `let b = a;` copies the pointer, so `b` is another name
                    // for the same allocation and a later escape through `b`
                    // must be found.
                    self.value(init, Mode::Escape);
                    let block = self.current_block();
                    let arena = self.sites.get(&site).and_then(|info| info.arena_block);
                    self.declare(name, Some(Binding { site, block, arena }));
                } else {
                    self.value(init, Mode::Escape);
                    self.declare(name, None);
                }
            }
            HirStmt::Destructure { init, .. } => self.value(init, Mode::Escape),
            HirStmt::Assign { name, value, span, .. } => {
                // The RHS is evaluated before the binding changes, so a shadow
                // or a self-reference still sees the old value.
                let target = self.bindings.get(name).copied();
                let value_site = self
                    .record_alloc(value)
                    .or_else(|| self.alias_site(value));
                let value_arena = value_site
                    .and_then(|site| self.sites.get(&site).and_then(|info| info.arena_block));
                // Storing an arena-owned value in a binding that does not share
                // its arena is an escape, whether the value reaches the target
                // directly or through a copy chain (`let b = a; out = b;`).
                if let Some(site) = value_site {
                    let same_arena = target.is_some_and(|binding| match (binding.arena, value_arena) {
                        (Some(target_arena), Some(value_arena)) => target_arena == value_arena,
                        _ => false,
                    });
                    let reported_name = self
                        .alias_source_name(value)
                        .filter(|source| source != name);
                    if value_arena.is_some() && !same_arena {
                        // Name the value being stored, not its target:
                        // `buf` is the arena value and `out` is only where it
                        // came to rest.
                        let display = reported_name.as_deref().unwrap_or(name);
                        self.note_site_escape(display, *span, site);
                    }
                }
                self.value(value, Mode::Escape);
                // An aggregate literal stores its elements; the aggregate can
                // outlive the arena even when the local binding does not.
                if !matches!(
                    value,
                    HirExpr::Ident(..)
                        | HirExpr::Move(..)
                        | HirExpr::Cast { .. }
                        | HirExpr::Try { .. }
                ) {
                    self.note_arena_escape(value, *span);
                }
                if let Some(site) = value_site {
                    // An assignment to a binding declared outside the current
                    // block does not make the allocation outlive the function,
                    // but it does outlive the block, so the arena rule must not
                    // apply. Keep the declaration block when one is known.
                    let block = target.and_then(|binding| binding.block);
                    self.bindings.insert(
                        name.clone(),
                        Binding {
                            site,
                            block,
                            arena: value_arena,
                        },
                    );
                    if let Some(info) = self.sites.get_mut(&site) {
                        info.binding_block = block;
                    }
                } else {
                    self.bindings.remove(name);
                }
            }
            HirStmt::IndexAssign {
                base,
                index,
                value,
                span,
            } => {
                self.value(base, Mode::Read);
                self.value(index, Mode::Escape);
                self.value(value, Mode::Escape);
                self.note_arena_escape(value, *span);
            }
            HirStmt::FieldAssign { value, span, .. } => {
                self.value(value, Mode::Escape);
                self.note_arena_escape(value, *span);
            }
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                span,
                ..
            } => {
                self.value(pointer, Mode::Read);
                self.value(index, Mode::Escape);
                self.value(value, Mode::Escape);
                self.note_arena_escape(value, *span);
            }
            HirStmt::Return(Some(expr), span) => {
                self.value(expr, Mode::Escape);
                self.note_arena_escape(expr, *span);
            }
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
            HirStmt::IfLet {
                subject,
                then_block,
                else_block,
                ..
            } => {
                // A pattern binding names a value the subject already holds,
                // not a new allocation, so it declares no site. Its own frame
                // keeps the name from marking an outer binding as escaped when
                // it shadows one.
                self.value(subject, Mode::Escape);
                self.push_scope();
                self.block(then_block);
                self.pop_frame();
                if let Some(else_block) = else_block {
                    self.block(else_block);
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
            HirExpr::Call {
                callee,
                args,
                span,
                ..
            } => {
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
                    // pointer to anyone. Inside an `arena` block the block
                    // releases the value, so an individual free is E1004.
                    if let Some(first) = args.first() {
                        if let HirExpr::Ident(path, at, ..) = first {
                            if let [name] = path.as_slice() {
                                self.note_arena_free(name, *at);
                            }
                        }
                        self.value(first, Mode::Read);
                    }
                    for arg in args.iter().skip(1) {
                        self.value(arg, Mode::Escape);
                    }
                } else if callee.last().map(String::as_str) == Some("thread_spawn") {
                    // A spawned function outlives the block that starts it, so
                    // an arena-owned argument (or capture) crosses the
                    // boundary.
                    for arg in args {
                        self.value(arg, Mode::Escape);
                        self.note_arena_escape(arg, *span);
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
