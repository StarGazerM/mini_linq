struct MissingRead<K, V>(std::marker::PhantomData<(K, V)>);
impl<K, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for MissingRead<K, V> {
    fn build(_: I) -> Self {
        Self(std::marker::PhantomData)
    }
}
mod mini_linq {
    pub(crate) use ::mini_linq::{select_structures, structures_to_rust};
    pub(crate) use cq_state_providers::hash_distinct as distinct_state;
    macro_rules! indexes {
        (probe) => { crate::MissingRead<_, _> };
    }
    pub(crate) use indexes as index_provider;
}
fn main() {
    let l = [(1,)];
    let r = [(1, 2)];
    let _ = mini_linq::select_structures! {
        physical {
            relation l(a: i32); relation r(a: i32, b: i32);
            left = pipe rename l {a -> a}; right = let rename r {a -> a, b -> b};
            j = pipe canon probe[a] left, right; return j as answer(a, b);
        } policy {}
    }.count();
}
