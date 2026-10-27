//! Compile-time obligations for provider-authored support declarations.
//!
//! Retention preserves a lookup's immutable occurrence cursor. Expansion also
//! requires its exact remaining length; neither proof traverses the provider.
use crate::{IndexRead, Keys};

#[inline(always)]
pub fn require_retain<R: IndexRead>(_: R)
where
    R::Cursor: Copy,
{
}

#[inline(always)]
pub fn require_expand<R: IndexRead>(_: R)
where
    R::Cursor: Copy + ExactSizeIterator,
{
}

#[inline(always)]
pub fn require_keys<P: Keys + ?Sized>(_: &P) {}

/// Look up once and preserve the exact cursor type, including its borrow lifetime.
#[inline(always)]
pub fn retain<R: IndexRead>(read: R, key: &R::Key) -> R::Cursor
where
    R::Cursor: Copy,
{
    read.lookup(key)
}
