use crate::Rows;
use cq_gpu_runtime::DeviceColumns;

/// Stored i32 ID of "ASIA" in region.r_name; main checks the stored value.
pub const ASIA: i32 = 2;
pub type Answer = (u32, u32, u32);
pub type QueryResult<'a> = Result<DeviceColumns<'a, Answer>, Box<dyn std::error::Error>>;

// TODO(Sequence resident derived providers): A and B retain the LE planner's
// ASIA-filtered Region r1 and reduced Nation r2 membership runs. r2 is a
// semijoin result, not a resident-origin range provider. Do not substitute R:
// Rows::provider rejects it with the exact diagnostic below.
// A traversal: Lineitem(order,supplier), Orders(order,customer,date),
// 8766 <= date < 9131, Customer(customer,nation), Nation(nation,region),
// Region(region,2), Supplier(supplier,nation).
// B traversal: Customer(customer,nation), Nation(nation,region), Region(region,2),
// Orders(order,customer,date), 8766 <= date < 9131, Lineitem(order,supplier),
// Supplier(supplier,nation).
// Advice mapping: sorted_runs/prefix_runs/retain_lookups -> recorded RangeProvider
// stores; r1/r2 membership remains explicit; dedup DeviceSet<For> -> PackedCanon.
// B's Supplier prefix [supplier] then exact nation lookup remains two levels;
// expand_lookups(256, depth 3) -> store_level at that flatten + tile(256,items 4).
// capacity(Lineitem) -> recorded store capacity + output(bounded), not an inferred
// bound. frontier(8)/directory(rank) have no Sequence rewrite equivalent; do not
// silently manufacture those choices. No old printer is used as a fallback.
pub fn plan_a<'a>(_data: &'a Rows) -> QueryResult<'a> {
    Err("TODO H05 A: T-Gpu: r2 has no recorded resident range layout".into())
}

pub fn plan_b<'a>(_data: &'a Rows) -> QueryResult<'a> {
    Err("TODO H05 B: T-Gpu: r2 has no recorded resident range layout".into())
}

// Compile these independently to exercise the unsupported plans, rather than
// testing the runtime TODO text or substituting the supported R traversal.
#[cfg(any(
    feature = "h05-a-sequence-rejection",
    feature = "h05-b-sequence-rejection"
))]
macro_rules! rejected_le {
    ($data:ident; physical { $($physical:tt)* }
        structures { $($structures:tt)* } sequence { $($sequence:tt)* }) => {
        cq_sequence_rewrites::output! {
            arguments { count };
            pipeline {
                ::cq_sequence_rewrites::grade(gpu,
                    R1 at host, R2 at host, L at host, O at host,
                    C at host, S at host, result at device),
                ::cq_gpu_sequence::machine_grade,
                ::cq_gpu_sequence::translate
            };
            physical {
                relation Region(r_regionkey:u32,r_name:i32) = (&$data.region);
                relation Nation(n_nationkey:u32,n_regionkey:u32) = (&$data.nation);
                relation Customer(c_custkey:u32,c_nationkey:u32) = (&$data.customer);
                relation Orders(o_orderkey:u32,o_custkey:u32,o_orderdate:i32) = (&$data.orders);
                relation Lineitem(l_orderkey:u32,l_suppkey:u32) = (&$data.lineitem);
                relation Supplier(s_suppkey:u32,s_nationkey:u32) = (&$data.supplier);
                region_base = pipe rename Region@0 {r_regionkey -> region,r_name -> region_name};
                region_selected = pipe select region_base if (region_name = 2);
                r1 = let project region_selected keep {region};
                nation_base = pipe rename Nation@1 {n_nationkey -> nation,n_regionkey -> region};
                r2 = let semi eager [region] nation_base,r1;
                customer = let rename Customer@2 {c_custkey -> customer,c_nationkey -> nation};
                orders_base = pipe rename Orders@3 {o_orderkey -> order,o_custkey -> customer,o_orderdate -> date};
                orders_selected = pipe select orders_base if (date >= 8766,date < 9131);
                orders = let project orders_selected keep {order,customer};
                lineitem = let rename Lineitem@4 {l_orderkey -> order,l_suppkey -> supplier};
                supplier = let rename Supplier@5 {s_suppkey -> supplier,s_nationkey -> nation};
                $($physical)*
                answer = pipe canon project matched keep {order,supplier,nation};
                return answer as answer(order,supplier,nation);
            }
            structures {
                region_base host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,i32),::cq_gpu_runtime::For> [];
                nation_base host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
                customer host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
                orders_base host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i32),::cq_gpu_runtime::For> [];
                lineitem host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
                supplier host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
                r1 = ::cq_gpu_sequence_runtime::RangeProvider [region];
                r2 = ::cq_gpu_sequence_runtime::RangeProvider [nation];
                answer canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                $($structures)*
            }
            sequence {
                R1 = store map r1 compact region_selected rows region_base;
                R2 = store compact r2 lookup r1 [region] rows nation_base;
                $($sequence)*
                output count;
            }
        }
    };
}

#[cfg(feature = "h05-a-sequence-rejection")]
fn reject_plan_a(data: &Rows) {
    let _ = rejected_le! {
        data;
        physical {
            lo = pipe probe first [order] lineitem,orders;
            loc = pipe probe first [customer] lo,customer;
            asia = pipe semi eager [nation] loc,r2;
            matched = pipe semi eager [nation,supplier] asia,supplier;
        }
        structures {
            lineitem rows = ::cq_gpu_sequence_runtime::RangeProvider;
            orders = ::cq_gpu_sequence_runtime::RangeProvider [order];
            customer = ::cq_gpu_sequence_runtime::RangeProvider [customer];
            supplier = ::cq_gpu_sequence_runtime::RangeProvider [nation,supplier];
        }
        sequence {
            L = store rows lineitem;
            O = store map orders compact orders_selected rows orders_base;
            C = store rows customer;
            S = store rows supplier;
            result = store canon map answer
                compact matched lookup supplier [nation,supplier]
                compact asia lookup r2 [nation]
                flatten lookup customer [customer]
                flatten lookup orders [order]
                flatten tile 256 L;
        }
    };
}

#[cfg(feature = "h05-b-sequence-rejection")]
fn reject_plan_b(data: &Rows) {
    let _ = rejected_le! {
        data;
        physical {
            asia = pipe semi eager [nation] customer,r2;
            co = pipe probe first [customer] asia,orders;
            col = pipe probe first [order] co,lineitem;
            matched = pipe semi eager [nation,supplier] col,supplier;
        }
        structures {
            customer rows = ::cq_gpu_sequence_runtime::RangeProvider;
            orders = ::cq_gpu_sequence_runtime::RangeProvider [customer];
            lineitem = ::cq_gpu_sequence_runtime::RangeProvider [order];
            supplier = ::cq_gpu_sequence_runtime::RangeProvider [supplier][nation];
        }
        sequence {
            C = store rows customer;
            O = store map orders compact orders_selected rows orders_base;
            L = store rows lineitem;
            S = store rows supplier;
            result = store canon map answer
                compact matched lookup supplier [nation,supplier]
                flatten lookup lineitem [order]
                flatten lookup orders [customer]
                compact asia lookup r2 [nation]
                flatten tile 256 items 4 C;
        }
    };
}

/// The reference R-N-C-O-L-S traversal, retaining the ASIA customer prefix.
/// sorted_indexes/prefix_runs become explicit RangeProvider key levels;
/// retain_streams(r4) becomes the C store. Expansion is the three written tile
/// boundaries, with 4 items at the Orders/Lineitem boundaries. PackedCanon
/// replaces DeviceSet<For>; count publication replaces the unbounded old output.
/// No dynamic choose is recorded: this plan had no size-guarded alternative.
/// frontier(8)/directory(rank) are unsupported old implementation advice, not
/// fabricated scheduling facts in this Sequence plan.
pub fn plan_r<'a>(data: &'a Rows) -> QueryResult<'a> {
    Ok(cq_sequence_rewrites::output! {
        arguments { count };
        pipeline {
            ::cq_sequence_rewrites::grade(gpu,
                RegionASIA at host, Nation at host, Customer at host,
                Orders at host, Lineitem at host, Supplier at host,
                C at device, result at device),
            ::cq_gpu_sequence::machine_grade,
            ::cq_gpu_sequence::translate
        };
        physical {
            relation Region(r_regionkey:u32,r_name:i32) = (&data.region);
            relation Nation(n_nationkey:u32,n_regionkey:u32) = (&data.nation);
            relation Customer(c_custkey:u32,c_nationkey:u32) = (&data.customer);
            relation Orders(o_orderkey:u32,o_custkey:u32,o_orderdate:i32) = (&data.orders);
            relation Lineitem(l_orderkey:u32,l_suppkey:u32) = (&data.lineitem);
            relation Supplier(s_suppkey:u32,s_nationkey:u32) = (&data.supplier);
            region_base = pipe rename Region@0 {r_regionkey -> region,r_name -> region_name};
            region_selected = pipe select region_base if (region_name = 2);
            region_asia = let project region_selected keep {region};
            nation_index = let rename Nation@1 {n_nationkey -> nation,n_regionkey -> region};
            customer_index = let rename Customer@2 {c_custkey -> customer,c_nationkey -> nation};
            orders_base = pipe rename Orders@3 {o_orderkey -> order,o_custkey -> customer,o_orderdate -> order_date};
            orders_selected = pipe select orders_base if (order_date >= 8766,order_date < 9131);
            orders_index = let project orders_selected keep {order,customer};
            lineitem_index = let rename Lineitem@4 {l_orderkey -> order,l_suppkey -> supplier};
            supplier_index = let rename Supplier@5 {s_suppkey -> supplier,s_nationkey -> nation};
            rn = pipe probe first [region] region_asia,nation_index;
            rnc = let probe first [nation] rn,customer_index;
            rnco = pipe probe first [customer] rnc,orders_index;
            rncol = pipe probe first [order] rnco,lineitem_index;
            matched = pipe semi eager [nation,supplier] rncol,supplier_index;
            answer = pipe canon project matched keep {order,supplier,nation};
            return answer as answer(order,supplier,nation);
        }
        structures {
            region_base host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,i32),::cq_gpu_runtime::For> [];
            nation_index host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            customer_index host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            orders_base host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i32),::cq_gpu_runtime::For> [];
            lineitem_index host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            supplier_index host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            region_asia rows = ::cq_gpu_sequence_runtime::RangeProvider;
            nation_index = ::cq_gpu_sequence_runtime::RangeProvider [region];
            customer_index = ::cq_gpu_sequence_runtime::RangeProvider [nation];
            orders_index = ::cq_gpu_sequence_runtime::RangeProvider [customer];
            lineitem_index = ::cq_gpu_sequence_runtime::RangeProvider [order];
            supplier_index = ::cq_gpu_sequence_runtime::RangeProvider [nation][supplier];
            rnc rows = ::cq_gpu_sequence_runtime::RangeProvider;
            answer canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
        }
        sequence {
            RegionASIA = store map region_asia compact region_selected rows region_base;
            Nation = store rows nation_index;
            Customer = store rows customer_index;
            Orders = store map orders_index compact orders_selected rows orders_base;
            Lineitem = store rows lineitem_index;
            Supplier = store rows supplier_index;
            C = store flatten lookup customer_index [nation]
                flatten lookup nation_index [region] RegionASIA;
            result = store canon flatten flatten flatten
                map answer compact matched lookup supplier_index [nation,supplier]
                tile 1024 items 4 store flatten lookup lineitem_index [order]
                tile 1024 items 4 store flatten lookup orders_index [customer]
                tile 256 C;
            output count;
        }
    })
}

/// Consume the resident packed answer without downloading tuples. The identity
/// input has a known length: old capacity(Input) maps to count publication,
/// an explicit output interpretation rather than a different traversal.
/// Canon remains recorded; no host-set certificate is invented.
pub fn chain<'a>(input: &'a DeviceColumns<'_, Answer>) -> DeviceColumns<'a, Answer> {
    cq_sequence_rewrites::output! {
        arguments { count };
        pipeline {
            ::cq_sequence_rewrites::grade(gpu, source at host, result at device),
            ::cq_gpu_sequence::machine_grade,
            ::cq_gpu_sequence::translate
        };
        physical {
            relation Input(order:u32,supplier:u32,nation:u32) = (input);
            source = pipe rename Input@0 {order -> order,supplier -> supplier,nation -> nation};
            answer = pipe canon project source keep {order,supplier,nation};
            return answer as answer(order,supplier,nation);
        }
        structures {
            source host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,u32)> [];
            answer canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
        }
        sequence {
            source = store rows source;
            result = store canon map answer flatten tile 256 source;
            output count;
        }
    }
}
