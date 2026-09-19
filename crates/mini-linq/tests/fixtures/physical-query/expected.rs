{
    let __cq_store_L: &[Value] = L;
    let __cq_store_R: &::std::collections::HashMap<Value, ::std::vec::Vec<Value>> = R;
    let __cq_store_S: &::std::collections::BTreeMap<Value, ::std::vec::Vec<Value>> = S;
    let __cq_store_r0: &[Value] = __cq_store_L;
    let __cq_store_r1: &::std::collections::HashMap<Value, ::std::vec::Vec<Value>> = __cq_store_R;
    let __cq_index_i0: &::std::collections::HashMap<Value, ::std::vec::Vec<Value>> = __cq_store_r1;
    let __cq_store_r3: &::std::collections::BTreeMap<Value, ::std::vec::Vec<Value>> = __cq_store_S;
    let __cq_index_i1: &::std::collections::BTreeMap<Value, ::std::vec::Vec<Value>> = __cq_store_r3;
    mini_linq::access! {
        answer(a, c) => ((a, c,)) :-
            r0(a) => bind a <- ((__cq_store_r0).iter()),
            r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten()),
            r3(b, c) => bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
    }
}
