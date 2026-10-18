// D85's accepted Written/Coarse merge+HashMap witness exposes a native
// unordered unary host read with LE. Both callers must retain OrderedRead.
macro_rules! d85_unordered_merge {
    ($($runs:path,)*) => {
        fn main() {
            use std::collections::HashMap;
            let RetailD85WebSales = HashMap::<(i32,), Vec<(i32, i32, i32)>>::new();
            let RetailD85WebReturns = HashMap::<(i32,), Vec<(i32, i32, i32, i32, i32)>>::new();
            let RetailD85WebPage = HashMap::<(i32,), Vec<()>>::new();
            let RetailD85CustomerDemo = HashMap::<(i32,), Vec<()>>::new();
            let RetailD85CustomerAddress = HashMap::<(i32,), Vec<()>>::new();
            let RetailD85DateDim = HashMap::<(i32,), Vec<()>>::new();
            let RetailD85Reason = HashMap::<(i32,), Vec<()>>::new();
            let _ = mini_linq::compile! {
                pipeline {
                    plan = cq_planners::order(le);
                    physical = [cq_rewrites::canon_prefixes, cq_rewrites::lazy_semijoins, cq_rewrites::merge_joins];
                    structures = [cq_workload_passes::ordered_dedup, $($runs,)* cq_workload_passes::sorted_payload];
                }
                pub struct RetailD85Program;
                relation RetailD85WebSales(c0: i32, c1: i32, c2: i32, c3: i32);
                relation RetailD85WebReturns(c0: i32, c1: i32, c2: i32, c3: i32, c4: i32, c5: i32);
                relation RetailD85WebPage(c0: i32);
                relation RetailD85CustomerDemo(c0: i32);
                relation RetailD85CustomerAddress(c0: i32);
                relation RetailD85DateDim(c0: i32);
                relation RetailD85Reason(c0: i32);
                retail_d85_result(order_number, item_key, reason_key) :-
                    RetailD85WebSales(order_number, item_key, web_page, sold_date),
                    RetailD85WebReturns(order_number, item_key, refunded_demo, returning_demo, address_key, reason_key),
                    RetailD85WebPage(web_page),
                    RetailD85CustomerDemo(refunded_demo),
                    RetailD85CustomerDemo(returning_demo),
                    RetailD85CustomerAddress(address_key),
                    RetailD85DateDim(sold_date),
                    RetailD85Reason(reason_key);
            }.count();
        }
    };
}
