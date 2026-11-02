//! IF-Capture handles: how a host-once `let` capture is shared with the
//! `move` closures of a lazy answer.
//!
//! The printer emits the same tokens for every capture,
//! `{ use ShareCopy as _; use ShareArc as _; Value::new(c).share() }`, and Rust
//! method resolution chooses the handle: a `Copy` value is its own handle
//! ([`Copied`]; closures copy it and nothing is allocated), any other value is
//! moved once into an [`Arc`] that closures clone. Both dereference to the
//! captured value, so a closure rebinds its binder as `&*handle` either way.
//! No pass inspects the capture's type.
use std::sync::Arc;

/// A captured value awaiting its handle.
pub struct Value<T>(Option<T>);

impl<T> Value<T> {
    pub fn new(value: T) -> Self {
        Self(Some(value))
    }
}

/// The handle of a `Copy` capture: the value itself.
#[derive(Clone, Copy, Debug)]
pub struct Copied<T>(T);

impl<T> std::ops::Deref for Copied<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Chosen first (by-value receiver) when the captured type is `Copy`.
pub trait ShareCopy {
    type Handle;
    fn share(self) -> Self::Handle;
}

impl<T: Copy> ShareCopy for Value<T> {
    type Handle = Copied<T>;
    fn share(self) -> Copied<T> {
        Copied(self.0.expect("a capture value is shared once"))
    }
}

/// Chosen through autoref when the captured type is not `Copy`.
pub trait ShareArc {
    type Handle;
    fn share(self) -> Self::Handle;
}

impl<T> ShareArc for &mut Value<T> {
    type Handle = Arc<T>;
    fn share(self) -> Arc<T> {
        Arc::new(self.0.take().expect("a capture value is shared once"))
    }
}

#[cfg(test)]
mod tests {
    use super::{ShareArc as _, ShareCopy as _, Value};

    #[test]
    fn copy_captures_are_their_own_handle_and_others_share_one_arc() {
        let code = Value::new(7_u32).share();
        let copied = code;
        assert_eq!((*code, *copied), (7, 7));
        let _: super::Copied<u32> = code;

        let names = Value::new(vec![String::from("FRANCE")]).share();
        let shared = std::sync::Arc::clone(&names);
        assert_eq!(std::sync::Arc::strong_count(&names), 2);
        assert_eq!(shared[0], "FRANCE");
    }
}
