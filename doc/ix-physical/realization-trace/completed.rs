{
    let __cq_store_L: &[Value] = L;
    let __cq_store_R: &::std::collections::HashMap<Value, ::std::vec::Vec<Value>> = R;
    let __cq_store_S: &::std::collections::BTreeMap<Value, ::std::vec::Vec<Value>> = S;
    let __cq_store_l: &[Value] = __cq_store_L;
    let __cq_store_r: &::std::collections::HashMap<Value, ::std::vec::Vec<Value>> =
        __cq_store_R;
    let __cq_store_s: &::std::collections::BTreeMap<Value, ::std::vec::Vec<Value>> =
        __cq_store_S;
    let __cq_index_i0: &::std::collections::HashMap<Value, ::std::vec::Vec<Value>> =
        __cq_store_r;
    let __cq_index_i1: &::std::collections::BTreeMap<Value, ::std::vec::Vec<Value>> =
        __cq_store_s;
    mini_linq::access! {
        answer(a, b, c) => ((a, b, c,)) : - l(a) => bind a <-
        ((__cq_store_l).iter()), r(a, b) => bind b <-
        ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c) => bind c <-
        ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
