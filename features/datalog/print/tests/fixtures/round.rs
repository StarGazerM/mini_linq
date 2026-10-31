{
    let mut __cq_datalog_delta_0: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
    let mut __cq_datalog_host_0: ::std::collections::HashMap<
        (),
        ::std::vec::Vec<(i32,)>,
        ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
    > = ::core::default::Default::default();
    let mut __cq_datalog_host_1: ::std::collections::HashSet<
        (i32,),
        ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
    > = ::core::default::Default::default();
    let mut __cq_datalog_host_2: ::std::collections::HashSet<
        (i32,),
        ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
    > = ::core::default::Default::default();
    let mut __cq_datalog_delta_host_0: ::std::collections::HashSet<
        (i32,),
        ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
    > = ::core::default::Default::default();
    {
        let __cq_input = &(input);
        for row in ::cq_storage::rows::Rows::rows(__cq_input) {
            {
                let __cq_key = (::core::clone::Clone::clone(row.0),);
                if __cq_datalog_host_1.insert(__cq_key) {
                    __cq_datalog_host_0
                        .entry(())
                        .or_default()
                        .push((::core::clone::Clone::clone(row.0),));
                }
            }
        }
    }
    {
        let mut __cq_datalog_next_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
        #[allow(unused_mut, unused_variables)]
        let mut __cq_datalog_next_canonical_1: ::std::collections::HashSet<
            (i32,),
            ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
        > = ::core::default::Default::default();
        {
            let __cq_datalog_scan_0 = &__cq_datalog_host_0;
            for row in authored_rows() {
                if !__cq_datalog_host_2.contains(&row)
                    && __cq_datalog_next_canonical_1.insert(::core::clone::Clone::clone(&row))
                {
                    __cq_datalog_next_1.push(row);
                }
            }
        }
        {
            for row in &__cq_datalog_next_1 {
                __cq_datalog_host_2.insert((::core::clone::Clone::clone(&row.0),));
            }
        }
        __cq_datalog_delta_0 = __cq_datalog_next_1;
        {
            __cq_datalog_delta_host_0 = ::core::default::Default::default();
            for row in &__cq_datalog_delta_0 {
                __cq_datalog_delta_host_0.insert((::core::clone::Clone::clone(&row.0),));
            }
        }
        let mut __cq_datalog_round_0 = 0usize;
        while 0usize + __cq_datalog_delta_0.len() != 0 {
            let mut __cq_datalog_next_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
            #[allow(unused_mut, unused_variables)]
            let mut __cq_datalog_next_canonical_1: ::std::collections::HashSet<
                (i32,),
                ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
            > = ::core::default::Default::default();
            {
                let __cq_datalog_scan_1 = &__cq_datalog_host_0;
                for row in authored_rows() {
                    if !__cq_datalog_host_2.contains(&row)
                        && __cq_datalog_next_canonical_1.insert(::core::clone::Clone::clone(&row))
                    {
                        __cq_datalog_next_1.push(row);
                    }
                }
            }
            {
                for row in &__cq_datalog_next_1 {
                    __cq_datalog_host_2.insert((::core::clone::Clone::clone(&row.0),));
                }
            }
            __cq_datalog_delta_0 = __cq_datalog_next_1;
            {
                __cq_datalog_delta_host_0 = ::core::default::Default::default();
                for row in &__cq_datalog_delta_0 {
                    __cq_datalog_delta_host_0.insert((::core::clone::Clone::clone(&row.0),));
                }
            }
            __cq_datalog_round_0 += 1;
        }
    }
    __cq_datalog_host_2
        .into_iter()
        .map(|key| (key.0,))
        .collect::<::std::vec::Vec<_>>()
        .into_iter()
}
