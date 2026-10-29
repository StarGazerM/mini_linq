//! DESIGN contract8-doop-interner
//! Owns process-global Mutex first-sight codes; IStr Ord is code order, not string order (contract 8 §10).
//! Sources: doc/STORAGE.md §10; Data Blocks §3.3 (local dictionaries require cross-domain translation).
//! DO NOT parallelize assignment as a codec-local tweak: stable/dense codes govern joins, routing and merges.
//! DO NOT claim lexical order or deterministic first-sight codes under concurrent interning.
//! Publication-point assignment/remapping is a measured direction for Plan P4, not today's implementation.
//! Query constant lookup must not be confused with inserting symbols at invocation preparation.

use ahash::AHashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Process-global string identity, shared by both engines and all input columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IStr(u32);

impl IStr {
    /// Returns this symbol's process-global dictionary code.
    pub fn code(self) -> u32 {
        self.0
    }

    /// Restores a code from this process's dictionary.
    ///
    /// Panics if the code has never been interned.
    // Code restoration retains IStr's identity representation; it does not resolve dictionary strings.
    pub fn from_code(code: u32) -> Self {
        let exists = (code as usize)
            < interner()
                .lock()
                .expect("string interner poisoned")
                .strings
                .len();
        assert!(exists, "symbol code {code} has never been interned");
        Self(code)
    }
}

#[derive(Default)]
struct Interner {
    ids: AHashMap<Arc<str>, IStr>,
    strings: Vec<Arc<str>>,
}

fn interner() -> &'static Mutex<Interner> {
    static INTERNER: LazyLock<Mutex<Interner>> = LazyLock::new(Mutex::default);
    &INTERNER
}

pub fn sym(value: &str) -> IStr {
    let mut interner = interner().lock().expect("string interner poisoned");
    if let Some(&id) = interner.ids.get(value) {
        return id;
    }
    let id = IStr(u32::try_from(interner.strings.len()).expect("u32 symbol space exhausted"));
    // One allocation per distinct string, shared by both lookup directions.
    let value: Arc<str> = Arc::from(value);
    interner.strings.push(Arc::clone(&value));
    interner.ids.insert(value, id);
    id
}

// DECODE ?: res resolves dictionary strings for opaque descriptor computations and diagnostics; generic callers have no single boundary.
pub fn res(value: IStr) -> Arc<str> {
    Arc::clone(&interner().lock().expect("string interner poisoned").strings[value.0 as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_identity_and_cat() {
        let id = sym("void");
        assert_eq!(id, sym("void"));
        assert_ne!(id, sym("int"));
        // DECODE P: global_identity_and_cat exposes the resolved symbol only to its diagnostic string assertion.
        assert_eq!(&*res(id), "void");
        // DECODE R: this diagnostic format! reads resolved text to compute a new descriptor, not to reorganize unchanged codes.
        assert_eq!(sym(&format!("{}({})", res(id), "int")), sym("void(int)"));
        assert_eq!(IStr::from_code(id.code()), id);
    }

    #[test]
    fn unknown_code_does_not_poison_interner() {
        assert!(std::panic::catch_unwind(|| IStr::from_code(u32::MAX)).is_err());
        // DECODE P: unknown_code_does_not_poison_interner publishes resolved text to its recovery assertion.
        assert_eq!(&*res(sym("still usable")), "still usable");
    }
}
