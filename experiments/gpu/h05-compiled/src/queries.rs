use crate::Rows;
use cq_gpu_runtime::{DeviceColumns, For};

pub type Answer = (i64, i64, i64);

pub fn plan_a(data: &Rows) -> DeviceColumns<'_, Answer, For> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::build_host_reads];
            structures = [::cq_rewrites::sorted_runs, ::cq_gpu::prefix_runs, ::cq_rewrites::retain_lookups,
                ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)];
            print = ::cq_gpu::print(capacity(Lineitem));
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
    }
}

pub fn plan_b(data: &Rows) -> DeviceColumns<'_, Answer, For> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::build_host_reads];
            // Supplier's directory is explicit; nation_key stays an exact residual,
            // with a separate recorded nation run preserving the earlier semijoin.
            structures = [::cq_rewrites::sorted_runs, ::cq_gpu::prefix_runs(Supplier => supplier_key), ::cq_rewrites::retain_lookups,
                ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)];
            print = ::cq_gpu::print(capacity(Lineitem));
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
    }
}

// A second independently compiled query consumes the first query's packed
// resident result. Only the caller explicitly downloads the final answer.
pub fn chain<'a>(input: &'a DeviceColumns<'_, Answer, For>) -> DeviceColumns<'a, Answer> {
    mini_linq::compile! {
        pipeline {
            structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print(capacity(Input));
        }
        struct H05ResidentChain;
        relation Input(order_key:i64,supplier_key:i64,nation_key:i64) = (input);
        answer(order_key,supplier_key,nation_key) :-
            Input(order_key,supplier_key,nation_key);
    }
}
