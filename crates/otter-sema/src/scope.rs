//! Scopes and bindings for the type checker.
//!
//! A scope stack: each scope is a flat map from name to binding. Declaring a
//! name that already exists in the *current* scope is an error; shadowing an
//! outer scope is allowed. Item names (`fn`, `struct`, `enum`) live in the
//! root scope, so two items with the same name is an error whatever they are.

use crate::error::TypeError;
use crate::ty::{Mutability, Type};
use otter_common::Span;

use std::collections::HashMap;

/// A bound name and what is known about its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub ty: Type,
    pub mutability: Mutability,
    pub span: Span,
}

/// One level of the scope stack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Frame {
    bindings: HashMap<String, Binding>,
}

/// Nested scopes. `frames[0]` is the root scope; items declared there are
/// visible everywhere.
pub struct Scope {
    frames: Vec<Frame>,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            frames: vec![Frame::default()],
        }
    }

    /// Declare a name in the current scope. Returns an error if the name is
    /// already declared *in this scope*; shadowing an outer scope is fine.
    pub fn declare(&mut self, name: impl Into<String>, binding: Binding) -> Result<(), TypeError> {
        let name = name.into();
        let current = self.frames.last_mut().expect("scope stack is never empty");
        if current.bindings.contains_key(&name) {
            return Err(TypeError::new(
                binding.span,
                format!("duplicate definition of `{name}`"),
            ));
        }
        current.bindings.insert(name, binding);
        Ok(())
    }

    /// Look up a name, searching from the innermost scope outward.
    pub fn resolve(&self, name: &str) -> Option<&Binding> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.bindings.get(name))
    }

    /// Push a new inner scope.
    pub fn push(&mut self) {
        self.frames.push(Frame::default());
    }

    /// Pop the innermost scope. Panics on the root scope -- callers must
    /// balance pushes and pops.
    pub fn pop(&mut self) {
        self.frames.pop().expect("scope stack is never empty");
        assert!(!self.frames.is_empty(), "popped the root scope");
    }

    /// The number of bindings in the current scope -- used by tests that
    /// want to assert a declaration left nothing behind.
    pub fn current_len(&self) -> usize {
        self.frames
            .last()
            .expect("scope stack is never empty")
            .bindings
            .len()
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}
