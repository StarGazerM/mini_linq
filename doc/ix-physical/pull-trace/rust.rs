::core::iter::Iterator::flatten(::core::iter::once_with(move || {
    let iter0 = ::core::iter::Iterator::map(
        ::core::iter::IntoIterator::into_iter(observe(0, (__cq_store_r0).iter())),
        move |a| (a,),
    );
    let iter1 = ::core::iter::Iterator::flat_map(iter0, move |(a,)| {
        ::core::iter::Iterator::map(
            ::core::iter::IntoIterator::into_iter(observe(
                1,
                (__cq_index_i0).get(a).into_iter().flatten(),
            )),
            move |b| {
                let a = ::core::clone::Clone::clone(&a);
                (a, b)
            },
        )
    });
    let iter2 = ::core::iter::Iterator::flat_map(iter1, move |(a, b)| {
        ::core::iter::Iterator::map(
            ::core::iter::IntoIterator::into_iter(observe(
                2,
                (__cq_index_i1).get(b).into_iter().flatten(),
            )),
            move |c| {
                let a = ::core::clone::Clone::clone(&a);
                let b = ::core::clone::Clone::clone(&b);
                (a, b, c)
            },
        )
    });
    let iter3 =
        ::core::iter::Iterator::map(iter2, move |#[allow(unused_variables)] (a, b, c)| (a, c));
    let iter4 = {
        let mut seen = ::std::collections::HashSet::new();
        ::core::iter::Iterator::filter(iter3, move |row| {
            seen.insert(::core::clone::Clone::clone(row))
        })
    };
    iter4
}))
