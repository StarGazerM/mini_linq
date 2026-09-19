{
    let mut __cq_output = ::std::collections::HashSet::new();
    for a in ((__cq_store_r0).iter()) {
        for b in ((__cq_index_i0).get(a).into_iter().flatten()) {
            let a = ::core::clone::Clone::clone(&a);
            for c in ((__cq_index_i1).get(b).into_iter().flatten()) {
                let a = ::core::clone::Clone::clone(&a);
                __cq_output.insert((a, c));
            }
        }
    }
    __cq_output
}
