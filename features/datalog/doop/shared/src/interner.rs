use ahash::AHashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Process-global string identity, shared by both engines and all input columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IStr(u32);

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
        assert_eq!(&*res(id), "void");
        assert_eq!(sym(&format!("{}({})", res(id), "int")), sym("void(int)"));
    }
}
