//! Parameter facts: what each pointer parameter promises to the optimizer.
//!
//! Code generation emits LLVM parameter attributes only from this pass's
//! answers. An attribute is a promise the optimizer may use to delete work, so
//! a fact this pass cannot prove is simply absent: the program is not weaker
//! for it, only less optimizable.
//!
//! # What is proved
//!
//! Only a bare `*mut T` parameter is a pointer in the emitted signature.
//! `Option[T]`, a slice, a view and a function value are aggregates, and `align`
//! or `nonnull` on an aggregate would be a claim about a different thing. So a
//! parameter whose checked type is not `TypeKind::Ptr` gets nothing.
//!
//! For a `*mut T` parameter:
//!
//! * `nonnull` always. The type has no null value: a `*mut T` is produced by
//!   `alloc`, `alloc_bytes` or the arena, never by a null literal.
//! * `align N` is the pointee's ABI alignment when the layout is known. The
//!   allocator returns memory aligned to the element it was asked for, so this
//!   is the alignment the pointer itself carries. A declared pointee has no
//!   layout here (`NoDecls`) and therefore no `align`, which is conservative.
//! * `noalias` when the parameter never escapes the body. A pointer parameter
//!   arrives as the unique live owner of its allocation (linear values cannot
//!   be copied), and if the body never returns it, stores it, or hands it to a
//!   call, no second reference can name the same memory during the call.
//! * `readonly` when the body neither writes through the parameter nor lets it
//!   escape. Escape always defeats `readonly`: a callee may write through the
//!   pointer, and this pass does not look across functions. `.free()` is a
//!   write: it ends the allocation's life through the pointer.
//!
//! `dereferenceable(N)` is deliberately not emitted. It needs a length and a
//! promise that `N` bytes are readable, and this language has no pointer
//! parameter that carries a length: a slice is a `{ i64, i64, T* }` aggregate,
//! not a pointer parameter. When that changes, the fact belongs here rather
//! than in the backend.
//!
//! # Escape is conservative
//!
//! A bare parameter name in any position that is not a pointer read or a
//! pointer write is an escape: a return, a call argument, a struct/tuple/slice
//! field, a move, a closure capture, a reassignment. Treating a call argument
//! as an escape is stronger than LLVM needs -- the callee is handed the same
//! pointer, not a copy -- but it is the one rule that stays true without an
//! interprocedural analysis, and a missing attribute costs performance rather
//! than correctness.
//!
//! The walk is syntactic. A local that shadows a parameter is read as the
//! parameter, so a shadow can only hide an optimization, never invent one.

use yashy_common::{ParamAttrs, ParamFacts, ProofFacts};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm};
use yashy_hir::layout::{layout, NoDecls};
use yashy_hir::ty::TypeKind;

/// Record every function's parameter facts, lambdas included.
pub fn check(program: &HirProgram, facts: &mut ProofFacts) {
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            analyse(function, facts);
        }
    }
}

fn analyse<'f>(function: &'f HirFn, facts: &mut ProofFacts) {
    let mut scan = Scan::new(&function.params);
    scan.block(&function.body);

    let attrs: Vec<ParamAttrs> = function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| match &param.ty.kind {
            TypeKind::Ptr(pointee) => ParamAttrs {
                noalias: !scan.escapes[index],
                nonnull: true,
                readonly: !scan.writes[index] && !scan.escapes[index],
                align: layout(&pointee.kind, &NoDecls)
                    .and_then(|layout| u32::try_from(layout.align).ok())
                    .filter(|align| *align > 1),
            },
            // Not a pointer in the signature: no attribute is a claim about it.
            _ => ParamAttrs::default(),
        })
        .collect();

    if attrs.iter().any(|attr| !attr.is_empty()) {
        facts.insert_params(
            function.file,
            &function.name,
            function.body.span.start,
            ParamFacts::new(attrs),
        );
    }

    // A lambda is a function with its own parameters. The scan collected them
    // without descending into their bodies; analyse each one as its own body so
    // a name inside a lambda is never mistaken for an outer parameter.
    for lambda in std::mem::take(&mut scan.lambdas) {
        analyse(lambda, facts);
    }
}

/// How a bare parameter name was used.
#[derive(Clone, Copy)]
enum Use {
    /// The pointer is written through, or freed: it is used, but not escaped.
    Write,
    /// The pointer value leaves the expression's control: it may be observed or
    /// retained by someone else.
    Escape,
}

/// One function body's use of its parameters.
struct Scan<'f> {
    params: &'f [yashy_hir::hir::HirParam],
    writes: Vec<bool>,
    escapes: Vec<bool>,
    /// Lambdas found while walking, to be analysed as functions of their own.
    lambdas: Vec<&'f HirFn>,
}

impl<'f> Scan<'f> {
    fn new(params: &'f [yashy_hir::hir::HirParam]) -> Self {
        Self {
            params,
            writes: vec![false; params.len()],
            escapes: vec![false; params.len()],
            lambdas: Vec::new(),
        }
    }

    fn mark(&mut self, name: &str, use_: Use) {
        for (index, param) in self.params.iter().enumerate() {
            if param.name == name {
                match use_ {
                    Use::Write => self.writes[index] = true,
                    Use::Escape => self.escapes[index] = true,
                }
            }
        }
    }

    fn mark_path(&mut self, path: &[String], use_: Use) {
        if let [name] = path {
            self.mark(name, use_);
        }
    }

    /// The parameter name `expr` is, when it is a bare single-segment name.
    fn bare_param<'e>(&self, expr: &'e HirExpr) -> Option<&'e str> {
        let HirExpr::Ident(path, ..) = expr else {
            return None;
        };
        let [name] = path.as_slice() else {
            return None;
        };
        self.params
            .iter()
            .any(|param| param.name == *name)
            .then_some(name.as_str())
    }

    fn block(&mut self, block: &'f HirBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &'f HirStmt) {
        match stmt {
            HirStmt::Expr(expr, ..) => self.expr(expr),
            HirStmt::Const { init, .. }
            | HirStmt::Var { init, .. }
            | HirStmt::Destructure { init, .. } => self.expr(init),
            // Reassigning the binding abandons the old value; the pass cannot
            // see what happens to it, so it is not a read.
            HirStmt::Assign { name, value, .. } => {
                self.mark(name, Use::Escape);
                self.expr(value);
            }
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                match self.bare_param(base) {
                    Some(name) => self.mark(name, Use::Write),
                    None => self.expr(base),
                }
                self.expr(index);
                self.expr(value);
            }
            // A pointer has no fields, so the root here is a struct binding. A
            // struct that happens to share a parameter's name is a shadow; the
            // field write cannot reach the pointer the parameter named.
            HirStmt::FieldAssign { value, .. } => self.expr(value),
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                match self.bare_param(pointer) {
                    Some(name) => self.mark(name, Use::Write),
                    None => self.expr(pointer),
                }
                self.expr(index);
                self.expr(value);
            }
            HirStmt::Return(Some(expr), _) => self.expr(expr),
            HirStmt::Return(None, _) => {}
            HirStmt::If {
                init,
                cond,
                then_block,
                else_block,
                ..
            } => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                self.expr(cond);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
            }
            HirStmt::ForRange {
                start, end, body, ..
            } => {
                self.expr(start);
                self.expr(end);
                self.block(body);
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.expr(iterable);
                self.block(body);
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                if let Some(update) = update {
                    self.expr(update);
                }
                self.block(body);
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    self.arm(arm);
                }
            }
            HirStmt::IfLet {
                subject,
                then_block,
                else_block,
                ..
            } => {
                // A pattern binding is a new value, not the parameter it may
                // shadow, and it cannot be written through.
                self.expr(subject);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
            }
            HirStmt::Defer { body, .. } => self.block(body),
            HirStmt::LoopInvariant(contract) => self.expr(&contract.expr),
            HirStmt::Break(_) | HirStmt::Continue(_) => {}
        }
    }

    fn arm(&mut self, arm: &'f HirSwitchArm) {
        if let Some(guard) = &arm.guard {
            self.expr(guard);
        }
        self.expr(&arm.body);
    }

    fn expr(&mut self, expr: &'f HirExpr) {
        match expr {
            HirExpr::Literal(..) | HirExpr::AddrOf { .. } | HirExpr::Capture { .. } => {}
            HirExpr::Ident(path, ..) => self.mark_path(path, Use::Escape),
            HirExpr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            HirExpr::Unary { operand, .. } => self.expr(operand),
            HirExpr::Move(inner, ..) => self.expr(inner),
            HirExpr::Call { callee, args, .. } => {
                // `.free()` reaches the HIR as `dealloc`. It uses the pointer
                // for the last time; it does not hand it to anyone.
                let free = matches!(callee.as_slice(), [name] if name == "dealloc");
                for arg in args {
                    match self.bare_param(arg) {
                        Some(name) => self.mark(name, if free { Use::Write } else { Use::Escape }),
                        None => self.expr(arg),
                    }
                }
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.expr(value);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.expr(element);
                }
            }
            // A read through the pointer, not a use of its value.
            HirExpr::FieldAccess { base, .. } => {
                if self.bare_param(base).is_none() {
                    self.expr(base);
                }
            }
            HirExpr::Index { base, index, .. } => {
                if self.bare_param(base).is_none() {
                    self.expr(base);
                }
                self.expr(index);
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.expr(base);
                self.expr(start);
                self.expr(end);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.expr(expr),
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    self.arm(arm);
                }
            }
            // A capture holds a copy of the value for as long as the closure
            // lives, so a captured parameter has escaped. The checker refuses a
            // linear capture today, but the fact is still the right one if that
            // rule ever widens.
            HirExpr::Lambda { f, .. } => {
                for capture in &f.captures {
                    self.mark(&capture.name, Use::Escape);
                }
                self.lambdas.push(f.as_ref());
            }
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.expr(receiver);
                self.expr(func);
                if let Some(init) = init {
                    self.expr(init);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use yashy_common::{ParamFacts, SourceMap};
    use yashy_lexer::Lexer;
    use yashy_parser::Parser;
    use yashy_sema::check_program;

    use crate::{verify, VerifyOptions};

    /// The parameter facts the verifier recorded for `name`, if any.
    fn params_of(source: &str, name: &str) -> Option<ParamFacts> {
        let mut sources = SourceMap::new();
        let file = sources.add_file("params.ya", source.to_string());
        let tokens = Lexer::new(source)
            .tokenize()
            .expect("the fixture's lexer must succeed");
        let mut parser = Parser::new(tokens, &sources, file);
        let program = parser.parse_program().expect("the fixture must parse");
        let hir = check_program(&program, &sources, file).expect("the fixture must check");
        let report = verify(&hir, &VerifyOptions::default());
        assert_eq!(
            report.error_count(),
            0,
            "the fixture must verify: {:?}",
            report.diagnostics
        );
        let found = report
            .facts
            .param_entries()
            .find(|(_, function, _, _)| *function == name)
            .map(|(_, _, _, facts)| facts.clone());
        found
    }

    #[test]
    fn reading_through_a_pointer_is_all_three_facts_and_its_alignment() {
        let source = "\
fn read(*mut int p) -> int {
    return p[0];
}
fn main() { }
";
        let facts = params_of(source, "read").expect("read has facts");
        let attrs = facts.get(0).expect("the pointer parameter has facts");
        assert!(attrs.noalias);
        assert!(attrs.nonnull);
        assert!(attrs.readonly);
        assert_eq!(attrs.align, Some(8));
    }

    #[test]
    fn writing_through_a_pointer_keeps_noalias_and_drops_readonly() {
        let source = "\
fn write(*mut int p) {
    p[0] = 1;
}
fn main() { }
";
        let facts = params_of(source, "write").expect("write has facts");
        let attrs = facts.get(0).expect("the pointer parameter has facts");
        assert!(attrs.noalias);
        assert!(attrs.nonnull);
        assert!(!attrs.readonly);
        assert_eq!(attrs.align, Some(8));
    }

    #[test]
    fn handing_a_pointer_to_a_call_is_an_escape() {
        let source = "\
fn hand(*mut int p) {
    consume(p);
}
fn consume(*mut int p) { }
fn main() { }
";
        let facts = params_of(source, "hand").expect("hand has facts");
        let attrs = facts.get(0).expect("the pointer parameter has facts");
        assert!(!attrs.noalias);
        assert!(attrs.nonnull);
        assert!(!attrs.readonly);
    }

    #[test]
    fn an_option_pointer_is_not_a_pointer_parameter() {
        let source = "\
fn maybe(Option[*mut int] p) -> int {
    return 0;
}
fn main() { }
";
        assert!(params_of(source, "maybe").is_none());
    }
}
