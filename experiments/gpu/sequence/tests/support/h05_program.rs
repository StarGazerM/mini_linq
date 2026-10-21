use cq_lang_sequence::Program;

pub const ASIA: i32 = 2;
pub const DATE_START: i32 = 8766;
pub const DATE_END: i32 = 9131;

/// The written R-N-C-O-L-S plan. No host-set or key-uniqueness premises.
/// Both the token tests and CUDA macro compile this exact program.
pub fn program() -> Program {
    program_with_capacity(1024)
}

/// Capacity is explicit written advice, never inferred by the translator.
pub fn program_with_capacity(capacity: usize) -> Program {
    syn::parse_quote! {
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
            result = store capacity #capacity canon flatten flatten flatten
                map answer compact matched lookup supplier_index [nation,supplier]
                tile 1024 items 4 store flatten lookup lineitem_index [order]
                tile 1024 items 4 store flatten lookup orders_index [customer]
                tile 256 C;
            output bounded;
        }
        graded gpu {
            RegionASIA at host;
            Nation at host;
            Customer at host;
            Orders at host;
            Lineitem at host;
            Supplier at host;
            C at device;
            result at device;
        }
    }
}
