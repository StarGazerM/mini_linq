//! Frozen executable H05 schedules, independent of the planner under test.

#[path = "h05_program.rs"]
mod reference;

use cq_lang_sequence::{OutputProtocol, Program, Term};

pub fn r() -> Program {
    let mut program = reference::program();
    let Term::Store { capacity, .. } = &mut program.sequence.definitions.last_mut().unwrap().term
    else {
        unreachable!("the authored reference publishes a store")
    };
    *capacity = None;
    program.sequence.output = Some(OutputProtocol::Count);
    program
}

pub fn b() -> Program {
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
            r1 = let project region_selected keep {region};
            nation_base = pipe rename Nation@1 {n_nationkey -> nation,n_regionkey -> region};
            r2 = let semi eager [region] nation_base,r1;
            customer = let rename Customer@2 {c_custkey -> customer,c_nationkey -> nation};
            orders_base = pipe rename Orders@3 {o_orderkey -> order,o_custkey -> customer,o_orderdate -> date};
            orders_selected = pipe select orders_base if (date >= 8766,date < 9131);
            orders = let project orders_selected keep {order,customer};
            lineitem = let rename Lineitem@4 {l_orderkey -> order,l_suppkey -> supplier};
            supplier = let rename Supplier@5 {s_suppkey -> supplier,s_nationkey -> nation};
            asia = pipe semi eager [nation] customer,r2;
            co = pipe probe first [customer] asia,orders;
            col = pipe probe first [order] co,lineitem;
            matched = pipe semi eager [nation,supplier] col,supplier;
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
            answer canon = sort using ::cq_gpu_sequence_runtime::canon::PackedCanon;
            customer rows = ::cq_gpu_sequence_runtime::RangeProvider;
            orders = ::cq_gpu_sequence_runtime::RangeProvider [customer];
            lineitem = ::cq_gpu_sequence_runtime::RangeProvider [order];
            supplier = ::cq_gpu_sequence_runtime::RangeProvider [supplier][nation];
        }
        sequence {
            R1 = store map r1 compact region_selected rows region_base;
            N = store rows nation_base;
            R2 = store compact r2 lookup r1 [region] flatten tile 256 N;
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
            output count;
        }
        graded gpu {
            R1 at host;
            N at host;
            R2 at device;
            L at host;
            O at host;
            C at host;
            S at host;
            result at device;
        }
    }
}

pub fn a() -> Program {
    let mut program = b();
    let physical = &mut program.structures.physical.program.definitions;
    physical.retain(|definition| {
        !["asia", "co", "col", "matched", "answer"].contains(&definition.name.to_string().as_str())
    });
    physical.push(syn::parse_quote!(lo = pipe probe first [order] lineitem,orders;));
    physical.push(syn::parse_quote!(loc = pipe probe first [customer] lo,customer;));
    physical.push(syn::parse_quote!(asia = pipe semi eager [nation] loc,r2;));
    physical.push(syn::parse_quote!(matched = pipe semi eager [nation,supplier] asia,supplier;));
    physical
        .push(syn::parse_quote!(answer = pipe canon project matched keep {order,supplier,nation};));
    let entries = &mut program.structures.structures.entries;
    entries.retain(|entry| {
        !["customer", "orders", "lineitem", "supplier"].contains(&entry.name.to_string().as_str())
            || matches!(entry.structure, cq_lang_structures::Structure::Host { .. })
    });
    entries.push(syn::parse_quote!(lineitem rows = ::cq_gpu_sequence_runtime::RangeProvider;));
    entries.push(syn::parse_quote!(orders = ::cq_gpu_sequence_runtime::RangeProvider [order];));
    entries
        .push(syn::parse_quote!(customer = ::cq_gpu_sequence_runtime::RangeProvider [customer];));
    entries.push(
        syn::parse_quote!(supplier = ::cq_gpu_sequence_runtime::RangeProvider [nation,supplier];),
    );
    program.sequence = syn::parse_quote! {
        sequence {
            R1 = store map r1 compact region_selected rows region_base;
            N = store rows nation_base;
            R2 = store compact r2 lookup r1 [region] flatten tile 256 N;
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
            output count;
        }
    };
    program
}
