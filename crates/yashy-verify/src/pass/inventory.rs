//! The inventory of everything the program trusts.
//!
//! A verifier proves what it can. Everything it cannot prove and the program
//! does anyway -- a call into C, a call that crosses a raw pointer -- is an
//! assumption, and this pass lists them. The list is evidence, not a verdict:
//! `yashy verify --inventory` answers "what does this program trust?", so that
//! "verified" never quietly means "trusted, and nobody wrote down what".
//!
//! Nothing here changes the exit code. A program with a thousand assumptions is
//! still a program the verifier accepted, exactly as it was before this pass
//! existed; what changed is that the assumptions can now be read.

use yashy_common::{FileId, Span};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm};

/// What kind of thing the program is trusting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssumptionKind {
    /// A declaration inside an `extern` block: a function whose body the
    /// program cannot see and whose ABI it accepts on the block's word.
    ExternDecl,
    /// A call the source marked `unsafe`: a raw pointer crossed the boundary
    /// unchanged, so the checker could not vouch for what the callee does with
    /// it.
    UnsafeCall,
}

impl AssumptionKind {
    /// The name this kind carries in the report, human and JSON alike.
    ///
    /// One spelling for both surfaces: a reader who greps the human table and a
    /// tool that reads the JSON must not have to learn two vocabularies for the
    /// same fact.
    pub fn as_str(self) -> &'static str {
        match self {
            AssumptionKind::ExternDecl => "extern_decl",
            AssumptionKind::UnsafeCall => "unsafe_call",
        }
    }
}

/// One thing the program trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assumption {
    pub kind: AssumptionKind,
    /// The name the source used: the C function for a declaration, the callee
    /// for a call.
    pub name: String,
    pub file: FileId,
    pub span: Span,
}

/// Every assumption in `hir`, in source order.
///
/// Items are flattened across modules, so the walk follows `hir.items` in the
/// order the checker produced them -- the order the files were loaded in -- and
/// takes each function's clauses and body in the order they were written. The
/// result reads top to bottom like the source does.
pub fn inventory(hir: &HirProgram) -> Vec<Assumption> {
    let mut found = Vec::new();
    for item in &hir.items {
        match item {
            HirItem::Extern(block) => {
                for decl in &block.decls {
                    found.push(Assumption {
                        kind: AssumptionKind::ExternDecl,
                        // The symbol, not the Yashy name: the assumption is
                        // that this linker symbol exists and has this
                        // signature, and `link_name` is exactly the case where
                        // the two names differ. The span still points at the
                        // declaration the reader wrote.
                        name: decl.symbol().to_string(),
                        file: block.file,
                        span: decl.span,
                    });
                }
            }
            HirItem::Fn(function) => walk_function(function, &mut found),
            HirItem::Struct(_) | HirItem::Enum(_) | HirItem::Import(_) => {}
        }
    }
    found
}

/// Walk one function, starting a walker that knows which file it is in.
///
/// The file is a parameter rather than a field the walker has to be handed at
/// construction: a function carries its own, and a lambda carries the one the
/// checker resolved it in, which is not always the file the enclosing function
/// came from.
fn walk_function(function: &HirFn, found: &mut Vec<Assumption>) {
    let mut walker = Walker {
        file: function.file,
        found,
    };
    walker.function_body(function);
}

struct Walker<'a> {
    file: FileId,
    found: &'a mut Vec<Assumption>,
}

impl Walker<'_> {
    /// A function's signature clauses, then its body.
    ///
    /// The clauses come first because that is where they are written, and a
    /// call inside one is as much an assumption as a call in the body.
    fn function_body(&mut self, function: &HirFn) {
        for clause in function.requires.iter().chain(function.ensures.iter()) {
            self.expr(&clause.expr);
        }
        self.block(&function.body);
    }

    fn block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Expr(expr, _, _) => self.expr(expr),
            HirStmt::Const { init, .. }
            | HirStmt::Var { init, .. }
            | HirStmt::Destructure { init, .. } => self.expr(init),
            HirStmt::Assign { value, .. } => self.expr(value),
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.expr(base);
                self.expr(index);
                self.expr(value);
            }
            HirStmt::FieldAssign { value, .. } => self.expr(value),
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.expr(pointer);
                self.expr(index);
                self.expr(value);
            }
            HirStmt::Return(value, _) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            HirStmt::If {
                init,
                cond,
                then_block,
                else_block,
                ..
            } => {
                // The `if`'s own binding runs before its condition, and the
                // condition before either arm.
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
            HirStmt::ForIn {
                iterable, body, ..
            } => {
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
                self.arms(arms);
            }
            HirStmt::Break(_) | HirStmt::Continue(_) => {}
            HirStmt::Defer { body, .. } => self.block(body),
            HirStmt::LoopInvariant(clause) => self.expr(&clause.expr),
        }
    }

    /// The arms of a `switch`, in the order they were written.
    ///
    /// A guard runs before its arm's body, so it comes first here too.
    fn arms(&mut self, arms: &[HirSwitchArm]) {
        for arm in arms {
            if let Some(guard) = &arm.guard {
                self.expr(guard);
            }
            self.expr(&arm.body);
        }
    }

    fn expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Literal(..)
            | HirExpr::Ident(..)
            | HirExpr::AddrOf { .. }
            | HirExpr::Capture { .. } => {}
            HirExpr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            HirExpr::Unary { operand, .. } => self.expr(operand),
            HirExpr::Move(inner, _, _) => self.expr(inner),
            HirExpr::Call {
                callee,
                args,
                unsafe_call,
                span,
                ..
            } => {
                // The call itself is the assumption; its arguments are walked
                // after it so the list reads outermost first, as the source
                // does.
                if *unsafe_call {
                    self.found.push(Assumption {
                        kind: AssumptionKind::UnsafeCall,
                        name: callee.join("."),
                        file: self.file,
                        span: *span,
                    });
                }
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
            HirExpr::FieldAccess { base, .. } => self.expr(base),
            HirExpr::Index { base, index, .. } => {
                self.expr(base);
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
                self.arms(arms);
            }
            HirExpr::Lambda { f, .. } => {
                // A lambda is written where it is written, which may be another
                // module than the function holding it. Its own `file` is the
                // answer, so the walk borrows it for the body and gives the
                // enclosing file back afterwards.
                let enclosing = self.file;
                self.file = f.file;
                self.function_body(f);
                self.file = enclosing;
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
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
