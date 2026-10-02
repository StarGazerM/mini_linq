struct MissingRead<K, V>(std::marker::PhantomData<(K, V)>);
impl<K, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for MissingRead<K, V> {
    fn build(_: I) -> Self {
        Self(std::marker::PhantomData)
    }
}
fn main() {
    let l = [(1,)];
    let r = [(1, 2)];
    let _ = mini_linq::passes::select! {
 pipeline { cq_rewrites::retype(payload => crate::MissingRead<_,_>), mini_linq::passes::print_pull };
        physical {
            relation l(a: i32); relation r(a: i32, b: i32);
            left = pipe rename l {a -> a}; right = let rename r {a -> a, b -> b};
            j = pipe canon probe first[a] left, right; return j as answer(a, b);
        }
    }.count();
}
