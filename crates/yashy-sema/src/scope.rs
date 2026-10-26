//! Scopes and bindings for the type checker.
//!
//! A scope stack: each scope is a flat map from name to binding. Declaring a
//! name that already exists in the *current* scope is an error; shadowing an
//! outer scope is allowed. Item names (`fn`, `struct`, `enum`) live in the
//! root scope, so two items with the same name is an error whatever they are.

use crate::error::TypeError;
use crate::ownership::Ownership;
use crate::ty::{Mutability, Type};
use yashy_common::Span;

use std::collections::{BTreeSet, HashMap, HashSet};

/// A bound name and what is known about its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub ty: Type,
    pub mutability: Mutability,
    pub span: Span,
    /// Whether this binding still owns its value. Always `Live` at declaration;
    /// flipped to `Moved` by [`Scope::move_out`]. Only meaningful for linear
    /// types, but tracked for every binding so the state is uniform.
    pub ownership: Ownership,
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

    /// The depth of the scope a name is bound in, searching outward.
    ///
    /// The root scope is depth 0, the scope a function body is checked in is
    /// depth 1, and so on. The depth is what tells a *global* (a top-level
    /// function, in the root scope) from a *local* of an enclosing function:
    /// a lambda may use a global, but a local of the function it is written in
    /// is a capture, and Task 8 has no way to take one.
    pub fn resolve_depth(&self, name: &str) -> Option<usize> {
        self.frames
            .iter()
            .enumerate()
            .rev()
            .find_map(|(depth, frame)| frame.bindings.get(name).map(|_| depth))
    }

    /// How many scopes are open.
    ///
    /// The depth of the scope a lambda's parameters live in is this number
    /// after the lambda's own scope has been pushed.
    pub fn depth(&self) -> usize {
        self.frames.len()
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

    /// Every name visible from the current scope, innermost first.
    ///
    /// A shadowed name appears once, under the binding a read would find. The
    /// list is what a "did you mean" suggestion searches: the caller has
    /// already failed to resolve a name, and the nearest spellings are the
    /// ones the reader probably meant.
    pub fn names(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for frame in self.frames.iter().rev() {
            for name in frame.bindings.keys() {
                if seen.insert(name.as_str()) {
                    names.push(name.as_str());
                }
            }
        }
        names
    }

    /// Find the binding for `name`, innermost scope first, mutably. Shadowing
    /// means there may be several; this is the one a read would resolve to.
    fn resolve_mut(&mut self, name: &str) -> Option<&mut Binding> {
        for frame in self.frames.iter_mut().rev() {
            if frame.bindings.contains_key(name) {
                return frame.bindings.get_mut(name);
            }
        }
        None
    }

    /// Mark `name` as moved out. Errors if it has already been moved, because a
    /// second move is a double free waiting to happen.
    pub fn move_out(&mut self, name: &str, span: Span) -> Result<(), TypeError> {
        match self.resolve_mut(name) {
            Some(binding) if binding.ownership == Ownership::Moved => Err(TypeError::new(
                span,
                format!("value `{name}` has already been moved"),
            )),
            Some(binding) => {
                binding.ownership = Ownership::Moved;
                Ok(())
            }
            None => {
                let mut error = TypeError::new(
                    span,
                    format!("cannot find value `{name}` in this scope"),
                );
                if let Some(suggestion) = yashy_common::closest(name, self.names()) {
                    error = error.with_label(format!("did you mean `{suggestion}`?"));
                }
                Err(error)
            }
        }
    }

    /// Error on a read of a binding that has been moved out. A name that is not
    /// bound is *not* an error here -- the caller already knows how to report
    /// that, and reporting it twice would double the diagnostic.
    pub fn check_live(&self, name: &str, span: Span) -> Result<(), TypeError> {
        match self.resolve(name) {
            Some(binding) if binding.ownership == Ownership::Moved => Err(TypeError::new(
                span,
                format!("value `{name}` has already been moved"),
            )),
            _ => Ok(()),
        }
    }

    /// Re-initialise a binding: an assignment installs a fresh value, so the
    /// binding owns something again whatever it held before.
    pub fn rebind(&mut self, name: &str) {
        if let Some(binding) = self.resolve_mut(name) {
            binding.ownership = Ownership::Live;
        }
    }

    /// Every currently-moved binding, keyed by scope depth and name. Used to
    /// compare the ownership state of two branches of an `if`.
    pub fn moved_bindings(&self) -> BTreeSet<(usize, String)> {
        let mut moved = BTreeSet::new();
        for (depth, frame) in self.frames.iter().enumerate() {
            for (name, binding) in &frame.bindings {
                if binding.ownership == Ownership::Moved {
                    moved.insert((depth, name.clone()));
                }
            }
        }
        moved
    }

    /// Set every binding's ownership to match `moved`: `Moved` for the pairs it
    /// contains, `Live` for everything else.
    pub fn apply_moved(&mut self, moved: &BTreeSet<(usize, String)>) {
        for (depth, frame) in self.frames.iter_mut().enumerate() {
            for (name, binding) in frame.bindings.iter_mut() {
                binding.ownership = if moved.contains(&(depth, name.clone())) {
                    Ownership::Moved
                } else {
                    Ownership::Live
                };
            }
        }
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}
