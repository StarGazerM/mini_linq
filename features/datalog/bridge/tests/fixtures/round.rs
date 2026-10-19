{
    let __cq_datalog_all_0 = &(input);
    let __cq_datalog_all_0 = &{
        struct Canonical<R>(::cq_storage::HashSet<R>);
        impl<R: ::core::marker::Copy> ::cq_storage::rows::Rows for &Canonical<R> {
            type Row = R;
            fn rows(self) -> impl ::core::iter::Iterator<Item = R> {
                self.0.iter().copied()
            }
        }
        Canonical(::cq_storage::rows::Rows::rows(__cq_datalog_all_0).collect())
    };
    let mut __cq_datalog_all_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
    let mut __cq_datalog_seen_1: ::cq_storage::HashSet<(i32,)> = ::core::default::Default::default();
    let mut __cq_datalog_delta_0: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
    let mut __cq_datalog_host_0: ::std::collections::HashMap<(), ::std::vec::Vec<(i32,)>, ::cq_storage::HashState>
        = ::core::default::Default::default();
    for row in ::cq_storage::rows::Rows::rows(__cq_datalog_all_0) {
        __cq_datalog_host_0.entry(()).or_default().push((::core::clone::Clone::clone(row.0),));
    }
    let mut __cq_datalog_delta_host_0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<()>, ::cq_storage::HashState>
        = ::core::default::Default::default();
    {
        let mut __cq_datalog_next_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
        {
            let __cq_datalog_chunk_0 = &__cq_datalog_host_0;
            for row in authored_rows() {
                if __cq_datalog_seen_1.insert(::core::clone::Clone::clone(&row)) {
                    __cq_datalog_next_1.push(row);
                }
            }
        }
        __cq_datalog_all_1.extend(__cq_datalog_next_1.iter().cloned());
        __cq_datalog_delta_0 = __cq_datalog_next_1;
        {
            __cq_datalog_delta_host_0 = ::core::default::Default::default();
            for row in &__cq_datalog_delta_0 {
                __cq_datalog_delta_host_0.entry((::core::clone::Clone::clone(&row.0),))
                    .or_default().push(());
            }
        }
        let mut __cq_datalog_round_0 = 0usize;
        while 0usize + __cq_datalog_delta_0.len() != 0 {
            let mut __cq_datalog_next_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
            {
                let __cq_datalog_chunk_1 = &__cq_datalog_host_0;
                for row in authored_rows() {
                    if __cq_datalog_seen_1.insert(::core::clone::Clone::clone(&row)) {
                        __cq_datalog_next_1.push(row);
                    }
                }
            }
            __cq_datalog_all_1.extend(__cq_datalog_next_1.iter().cloned());
            __cq_datalog_delta_0 = __cq_datalog_next_1;
            {
                __cq_datalog_delta_host_0 = ::core::default::Default::default();
                for row in &__cq_datalog_delta_0 {
                    __cq_datalog_delta_host_0.entry((::core::clone::Clone::clone(&row.0),))
                        .or_default().push(());
                }
            }
            __cq_datalog_round_0 += 1;
        }
    }
    __cq_datalog_all_1.into_iter()
}
