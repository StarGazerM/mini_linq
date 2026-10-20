use crate::Rows;

/// Stored i32 ID of "ASIA" in region.r_name, written as a literal: a
/// captured host value (`meta.asia`) is exact but the planner lowers it to a
/// late bind + join instead of an early selection. main checks the value.
pub const ASIA: i32 = 2;
use cq_gpu_runtime::{DeviceColumns, For};

/// Stored IDs: (orderkey, suppkey, nationkey) domain IDs.
pub type Answer = (u32, u32, u32);

pub fn plan_a<'a>(data: &'a Rows) -> DeviceColumns<'a, Answer, For> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::build_host_reads];
            // r2 is ASIA-filtered Region, r3 reduced Nation; LE needs only their key memberships.
            structures = [::cq_rewrites::sorted_runs, ::cq_gpu::prefix_runs, ::cq_rewrites::retain_lookups,
                ::cq_rewrites::retype(r2 => ::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
                ::cq_rewrites::retype(r3 => ::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
                ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)];
            print = ::cq_gpu::print(capacity(Lineitem));
        }
        struct H05A;
        relation Customer(c_custkey:u32,c_nationkey:u32) = (&data.customer);
        relation Orders(o_orderkey:u32,o_custkey:u32,o_orderdate:i32) = (&data.orders);
        relation Lineitem(l_orderkey:u32,l_suppkey:u32) = (&data.lineitem);
        relation Supplier(s_suppkey:u32,s_nationkey:u32) = (&data.supplier);
        relation Nation(n_nationkey:u32,n_regionkey:u32) = (&data.nation);
        relation Region(r_regionkey:u32,r_name:i32) = (&data.region);
        answer(order_key,supplier_key,nation_key) :-
            Lineitem(order_key,supplier_key),
            Orders(order_key,customer_key,order_date),
            #[total] if *order_date >= 8766 && *order_date < 9131,
            Customer(customer_key,nation_key), Nation(nation_key,region_key),
            Region(region_key,2), Supplier(supplier_key,nation_key);
    }
}

pub fn plan_b<'a>(data: &'a Rows) -> DeviceColumns<'a, Answer, For> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::build_host_reads];
            // Supplier's directory is explicit; nation is searched exactly within
            // that prefix. A separate nation run preserves the earlier semijoin.
            // r2 is ASIA-filtered Region, r3 reduced Nation; record both membership runs explicitly.
            structures = [::cq_rewrites::sorted_runs, ::cq_gpu::prefix_runs(Supplier => supplier_key), ::cq_rewrites::retain_lookups,
                ::cq_rewrites::retype(r2 => ::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
                ::cq_rewrites::retype(r3 => ::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
                ::cq_rewrites::expand_lookups(256, depth 3),
                ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)];
            print = ::cq_gpu::print(capacity(Lineitem), tile(256,4), frontier(8), directory(rank));
        }
        struct H05B;
        relation Customer(c_custkey:u32,c_nationkey:u32) = (&data.customer);
        relation Orders(o_orderkey:u32,o_custkey:u32,o_orderdate:i32) = (&data.orders);
        relation Lineitem(l_orderkey:u32,l_suppkey:u32) = (&data.lineitem);
        relation Supplier(s_suppkey:u32,s_nationkey:u32) = (&data.supplier);
        relation Nation(n_nationkey:u32,n_regionkey:u32) = (&data.nation);
        relation Region(r_regionkey:u32,r_name:i32) = (&data.region);
        answer(order_key,supplier_key,nation_key) :-
            Customer(customer_key,nation_key), Nation(nation_key,region_key),
            Region(region_key,2), Orders(order_key,customer_key,order_date),
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
        relation Input(order_key:u32,supplier_key:u32,nation_key:u32) = (input);
        answer(order_key,supplier_key,nation_key) :-
            Input(order_key,supplier_key,nation_key);
    }
}
