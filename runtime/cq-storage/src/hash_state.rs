//! The keyed hasher fixed by the default recorded hash representations.
use std::hash::BuildHasher;

/// Keyed aHash state for compiler-built storage. Structures records a type
/// naming this state; an explicitly recorded standard collection keeps its
/// own hasher. Counter builds use fixed seeds for reproducible measurements.
#[derive(Clone, Debug)]
pub struct HashState(ahash::RandomState);
impl Default for HashState {
    fn default() -> Self {
        #[cfg(feature = "counters")]
        let state = ahash::RandomState::with_seeds(0, 0, 0, 0);
        #[cfg(not(feature = "counters"))]
        let state = ahash::RandomState::new();
        Self(state)
    }
}
impl BuildHasher for HashState {
    type Hasher = ahash::AHasher;
    fn build_hasher(&self) -> Self::Hasher {
        self.0.build_hasher()
    }
}

/// Default membership and dedup provider, including its concrete hash state.
pub type HashSet<K> = std::collections::HashSet<K, HashState>;

/// The dedup provider's `new()` protocol for sets with a recorded hash state.
/// Standard and custom providers with an inherent `new()` keep that method.
pub trait HashSetConstructor {
    fn new() -> Self;
}
impl<K, S: Default> HashSetConstructor for std::collections::HashSet<K, S> {
    fn new() -> Self {
        Self::with_hasher(S::default())
    }
}
