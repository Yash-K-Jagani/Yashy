//! Typestate: the state a resource is in.
//!
//! A resource is not a value the program owns in the linear sense; it is a
//! handle whose *state* determines which operations are legal. A mutex is
//! `Unlocked` until `mutex_lock` makes it `Locked`; a channel is `Open` until
//! `chan_close` makes it `Closed`; a pointer from `alloc` is `Allocated` until
//! `free` makes it `Freed`. Using a resource in a terminal state, or releasing
//! it twice, is a compile error.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceState {
    Allocated,
    Freed,
    Unlocked,
    Locked,
    Open,
    Closed,
}

impl ResourceState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, ResourceState::Freed | ResourceState::Closed)
    }
}

impl std::fmt::Display for ResourceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceState::Allocated => write!(f, "allocated"),
            ResourceState::Freed => write!(f, "freed"),
            ResourceState::Unlocked => write!(f, "unlocked"),
            ResourceState::Locked => write!(f, "locked"),
            ResourceState::Open => write!(f, "open"),
            ResourceState::Closed => write!(f, "closed"),
        }
    }
}
