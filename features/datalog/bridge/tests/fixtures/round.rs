{
    let __cq_datalog_all_0 = &(input);
    let mut __cq_datalog_all_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
    let mut __cq_datalog_seen_1: ::std::collections::HashSet<(i32,)> = ::std::collections::HashSet::new();
    let mut __cq_datalog_delta_0: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
    let mut __cq_datalog_host_0: ::std::collections::HashMap<(), ::std::vec::Vec<(i32,)>>
        = ::std::collections::HashMap::new();
    for row in ::cq_storage::rows::Rows::rows(__cq_datalog_all_0) {
        __cq_datalog_host_0.entry(()).or_default().push((::core::clone::Clone::clone(row.0),));
    }
    {
        let mut __cq_datalog_next_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
        for row in authored_rows() {
            if __cq_datalog_seen_1.insert(::core::clone::Clone::clone(&row)) {
                __cq_datalog_next_1.push(row);
            }
        }
        __cq_datalog_all_1.extend(__cq_datalog_next_1.iter().cloned());
        __cq_datalog_delta_0 = __cq_datalog_next_1;
        let mut __cq_datalog_round_0 = 0usize;
        while 0usize + __cq_datalog_delta_0.len() != 0 {
            let mut __cq_datalog_next_1: ::std::vec::Vec<(i32,)> = ::std::vec::Vec::new();
            __cq_datalog_all_1.extend(__cq_datalog_next_1.iter().cloned());
            __cq_datalog_delta_0 = __cq_datalog_next_1;
            __cq_datalog_round_0 += 1;
        }
    }
    __cq_datalog_all_1.into_iter()
}
