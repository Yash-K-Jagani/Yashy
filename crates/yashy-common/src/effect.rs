//! Effects: what a function does to the world.

/// What a function does to the world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Effect {
    pub io: bool,
    pub unsafe_: bool,
    pub async_: bool,
}

impl Effect {
    pub const PURE: Effect = Effect {
        io: false,
        unsafe_: false,
        async_: false,
    };

    pub fn join(self, other: Effect) -> Effect {
        Effect {
            io: self.io || other.io,
            unsafe_: self.unsafe_ || other.unsafe_,
            async_: self.async_ || other.async_,
        }
    }

    pub fn is_pure(self) -> bool {
        !self.io && !self.unsafe_ && !self.async_
    }
}

/// What the programmer declared about a function's effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeclaredEffect {
    /// No declaration: the effect is inferred.
    #[default]
    Inferred,
    /// `pure`: the function must be pure.
    Pure,
    /// `io`: the function may perform IO.
    Io,
    /// `unsafe`: the function may use unsafe operations.
    Unsafe,
    /// `async`: the function is async.
    Async,
}

impl DeclaredEffect {
    pub fn to_effect(self) -> Option<Effect> {
        match self {
            DeclaredEffect::Inferred => None,
            DeclaredEffect::Pure => Some(Effect::PURE),
            DeclaredEffect::Io => Some(Effect { io: true, unsafe_: false, async_: false }),
            DeclaredEffect::Unsafe => Some(Effect { io: false, unsafe_: true, async_: false }),
            DeclaredEffect::Async => Some(Effect { io: false, unsafe_: false, async_: true }),
        }
    }
}