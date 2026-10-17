//! The LLVM IR backend: typed HIR in, LLVM IR text out.
//!
//! The backend is a text emitter, not an AST builder: the master plan
//! forbids linking LLVM, so every stage here writes IR source lines and the
//! driver hands them to `clang`/`llc` as a subprocess.
//!
//! This module holds the pieces every later emission task shares:
//! [`module::Module`] (the accumulating module: body lines, interned string
//! globals, SSA temporaries), [`ir::llvm_type_string`] (the checked-type to
//! LLVM-type mapping), and `main` discovery (a program with no `fn main`
//! must fail as a diagnostic here, not as a linker error 400 lines later).

pub mod ir;
pub mod module;

pub use ir::llvm_type_string;
pub use module::Module;

use otter_common::{Diagnostic, FileId, Severity, Span};
use otter_sema::{HirFn, HirItem, TypeKind};

/// The `fn main` item, when the program has one.
///
/// Sema (Task 8) already rejected duplicate `main`s, so the first match is
/// the only match.
pub fn find_main(items: &[HirItem]) -> Option<&HirFn> {
    items.iter().find_map(|item| match item {
        HirItem::Fn(f) if f.name == "main" => Some(f),
        _ => None,
    })
}

/// Check that the program has a `fn main` whose return type is an entry
/// point (`()` or `i64`).
///
/// The diagnostic's span is the synthetic `Span { start: 0, end: 0 }`: a
/// missing `main` points at nothing in particular, and a file-level caret
/// at offset zero reads as "the start of the file", which is exactly what a
/// missing entry point means. The `FileId` is index zero because only the
/// driver knows which file is being compiled; presence, not attribution, is
/// this function's job.
pub fn check_main_exists(items: &[HirItem]) -> Result<(), Diagnostic> {
    let file = FileId::from_index(0);
    match find_main(items) {
        None => Err(Diagnostic {
            severity: Severity::Error,
            message: "no `fn main` found: every Otter program must define one".to_string(),
            file,
            span: Span::new(0, 0),
            label: None,
        }),
        Some(main) => {
            let ok = matches!(
                main.ret.kind,
                TypeKind::Unit | TypeKind::I64 | TypeKind::Never
            );
            if ok {
                Ok(())
            } else {
                Err(Diagnostic {
                    severity: Severity::Error,
                    message: format!("`main` must return nothing or `i64`, found `{}`", main.ret),
                    file,
                    span: main.span,
                    label: None,
                })
            }
        }
    }
}
