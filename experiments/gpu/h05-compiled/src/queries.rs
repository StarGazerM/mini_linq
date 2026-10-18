use crate::Rows;

pub type Answer = (i64, i64, i64);

pub fn plan_a(data: &Rows) -> Vec<Answer> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::build_host_reads];
            structures = [::cq_rewrites::sorted_runs, ::cq_gpu::prefix_runs, ::cq_rewrites::retain_lookups,
                ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print;
        }
        struct H05A;
        relation Customer(c_custkey:i64,c_nationkey:i64) = (&data.customer);
        relation Orders(o_orderkey:i64,o_custkey:i64,o_orderdate:i64) = (&data.orders);
        relation Lineitem(l_orderkey:i64,l_suppkey:i64) = (&data.lineitem);
        relation Supplier(s_suppkey:i64,s_nationkey:i64) = (&data.supplier);
        relation Nation(n_nationkey:i64,n_regionkey:i64) = (&data.nation);
        relation Region(r_regionkey:i64,r_name:String) = (&data.region);
        answer(order_key,supplier_key,nation_key) :-
            Lineitem(order_key,supplier_key),
            Orders(order_key,customer_key,order_date),
            #[total] if *order_date >= 8766 && *order_date < 9131,
            Customer(customer_key,nation_key), Nation(nation_key,region_key),
            Region(region_key,"ASIA"), Supplier(supplier_key,nation_key);
    }.collect()
}

pub fn plan_b(data: &Rows) -> Vec<Answer> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::build_host_reads];
            structures = [::cq_rewrites::sorted_runs, ::cq_gpu::prefix_runs, ::cq_rewrites::retain_lookups,
                ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print;
        }
        struct H05B;
        relation Customer(c_custkey:i64,c_nationkey:i64) = (&data.customer);
        relation Orders(o_orderkey:i64,o_custkey:i64,o_orderdate:i64) = (&data.orders);
        relation Lineitem(l_orderkey:i64,l_suppkey:i64) = (&data.lineitem);
        relation Supplier(s_suppkey:i64,s_nationkey:i64) = (&data.supplier);
        relation Nation(n_nationkey:i64,n_regionkey:i64) = (&data.nation);
        relation Region(r_regionkey:i64,r_name:String) = (&data.region);
        answer(order_key,supplier_key,nation_key) :-
            Customer(customer_key,nation_key), Nation(nation_key,region_key),
            Region(region_key,"ASIA"), Orders(order_key,customer_key,order_date),
            #[total] if *order_date >= 8766 && *order_date < 9131,
            Lineitem(order_key,supplier_key), Supplier(supplier_key,nation_key);
    }.collect()
}
