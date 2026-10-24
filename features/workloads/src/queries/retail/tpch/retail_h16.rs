//! TPC-H Q16 part/supplier core, without grouped supplier counts or ordering.
//! dbgen keys are non-null, so SQL NOT IN equals the CQ anti-join below.
//! SQL is the independent DuckDB oracle; LIKE preserves substring order.
crate::tpch_runtime::retail_kernel! { $;
    name "retail_h16";
    sql r#"SELECT DISTINCT p.p_partkey, ps.ps_suppkey
FROM part p JOIN partsupp ps ON ps.ps_partkey = p.p_partkey
WHERE p.p_brand <> 'Brand#45' AND p.p_type NOT LIKE 'MEDIUM POLISHED%'
AND p.p_size IN (49, 14, 23, 45, 19, 3, 36, 9)
AND ps.ps_suppkey NOT IN (SELECT s.s_suppkey FROM supplier s
WHERE s.s_comment LIKE '%Customer%Complaints%')"#;
    candidate hypergraph;
    advise plan(no_reduce, order(Part, Partsupp));
    setup {
        let BadSupplier: Vec<(i64,)> = Supplier.iter().filter(|row|
            row.1.split_once("Customer").is_some_and(|(_, rest)| rest.contains("Complaints")))
            .map(|row| (row.0,)).collect();
    }
    relations {
        relation Part => part(p_partkey: i64, p_brand: String, p_type: String, p_size: i32);
        relation Partsupp => partsupp(ps_partkey: i64, ps_suppkey: i64);
        relation Supplier => supplier(s_suppkey: i64, s_comment: String);
    }
    derived { relation BadSupplier(suppkey: i64); }
    answer(part_key: i64, suppkey: i64) :-
        Part(part_key, brand, kind, size),
        if brand != "Brand#45" && !kind.starts_with("MEDIUM POLISHED")
            && [49, 14, 23, 45, 19, 3, 36, 9].contains(size),
        Partsupp(part_key, suppkey), !BadSupplier(suppkey);
}
