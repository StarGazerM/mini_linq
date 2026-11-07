//! Planner acceptance keeps the authored H05 schedules as independent costs
//! and semantic references; generated layouts need not duplicate their syntax.

#[allow(dead_code)]
#[path = "support/h05_plans.rs"]
mod h05_plans;

use cq_lang_sequence::{check_grading, check_rewrite};
include!("support/stack.rs");

fn planned(authored: &Stack) -> Stack {
    let machine = cq_gpu_sequence::machine();
    let source = cq_sequence_lowering::compile(&authored.physical, &authored.structures).unwrap();
    let source = cq_gpu_sequence_core::runs::record_representation(&authored.physical, &authored.structures, &source).unwrap();
    let result =
        cq_sequence_plan::decompose(&authored.physical, &authored.structures, &source, &machine, &cq_gpu_sequence::catalog()).unwrap();
    let repeated =
        cq_sequence_plan::decompose(&authored.physical, &authored.structures, &source, &machine, &cq_gpu_sequence::catalog()).unwrap();
    assert_eq!(
        result, repeated,
        "PL-Det: repeated planning must be identical"
    );
    let replayed = result
        .trace
        .iter()
        .fold((authored.physical.clone(), authored.structures.clone(), source.clone()), |previous, advice| {
            let next = advice.apply(&previous.0, &previous.1, &previous.2).unwrap();
            check_rewrite(&previous.0, &previous.1, &previous.2, &next.0, &next.1, &next.2).unwrap();
            next
        });
    assert_eq!(
        replayed, (result.physical.clone(), result.structures.clone(), result.program.clone()),
        "PL-Closure: replay must reproduce every generated decision"
    );
    assert_eq!(
        result.structures, authored.structures,
        "decomposition must preserve the authored traversal and provider choices"
    );
    let analysis = check_grading(&result.physical, &result.structures, &result.program, &machine).unwrap();
    cq_gpu_sequence::translate(&result.physical, &result.structures, &result.program, &analysis).unwrap();
    Stack { physical: result.physical, structures: result.structures, program: result.program }
}

#[test]
fn planned_r_grades_translates_replays_and_costs_no_more_than_authored() {
    assert_cost_acceptance("R", h05_plans::r());
}

#[test]
fn planned_b_grades_translates_replays_and_costs_no_more_than_authored() {
    assert_cost_acceptance("B", h05_plans::b());
}

#[test]
fn planned_a_retains_its_real_authored_traversal_choices() {
    // A is a separately authored Physical traversal, not an alternative a
    // decomposition planner may substitute for either B or R.
    let authored = h05_plans::a();
    planned(&authored);
}

/// The preserved step-R order: one ASIA region row fans out through Nation,
/// Customer, Orders and Lineitem, then checks Supplier membership.
fn step_r() -> (cq_lang_sequence::structures::physical::Program, cq_lang_sequence::structures::Program) {
    syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
        let physical = input.parse()?;
        let structures = cq_lang_sequence::structures::Program::parse(input, &physical)?;
        Ok((physical, structures))
    }, quote::quote! {
        physical {
            relation Region(r_regionkey:u32,r_name:i32) = (&data.region);
            relation Nation(n_nationkey:u32,n_regionkey:u32) = (&data.nation);
            relation Customer(c_custkey:u32,c_nationkey:u32) = (&data.customer);
            relation Orders(o_orderkey:u32,o_custkey:u32,o_orderdate:i32) = (&data.orders);
            relation Lineitem(l_orderkey:u32,l_suppkey:u32) = (&data.lineitem);
            relation Supplier(s_suppkey:u32,s_nationkey:u32) = (&data.supplier);
            region_base = pipe rename Region@0 {r_regionkey -> region,r_name -> region_name};
            region_selected = pipe select region_base if (region_name = 2);
            nation = let rename Nation@1 {n_nationkey -> nation,n_regionkey -> region};
            rn = pipe probe first [region] region_selected,nation;
            customer = let rename Customer@2 {c_custkey -> customer,c_nationkey -> nation};
            rnc = pipe probe first [nation] rn,customer;
            orders_base = pipe rename Orders@3 {o_orderkey -> order,o_custkey -> customer,o_orderdate -> date};
            orders_selected = pipe select orders_base if (date >= 8766,date < 9131);
            orders = let project orders_selected keep {order,customer};
            rnco = pipe probe first [customer] rnc,orders;
            lineitem = let rename Lineitem@4 {l_orderkey -> order,l_suppkey -> supplier};
            rncol = pipe probe first [order] rnco,lineitem;
            supplier = let rename Supplier@5 {s_suppkey -> supplier,s_nationkey -> nation};
            matched = pipe semi eager [nation,supplier] rncol,supplier;
            answer = let canon project matched keep {order,supplier,nation};
            return answer as answer(order,supplier,nation);
        }
        structures {
            providers {
                device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range };
                radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred };
            }
            region_base host scan = @device_columns input { keys 0; columns [u32 => u32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,i32),::cq_gpu_runtime::For> [];
            nation host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            customer host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            orders_base host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i32),::cq_gpu_runtime::For> [];
            lineitem host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            supplier host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32),::cq_gpu_runtime::For> [];
            answer rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
            answer canon = rows;
            nation = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [region];
            customer = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [nation];
            orders = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [customer];
            lineitem = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [order];
            supplier = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [supplier][nation];
        }
    }).unwrap()
}

#[test]
fn evidence_retains_a_small_rooted_prefix_and_segments_its_fan_out() {
    let machine = cq_gpu_sequence::machine();
    let (physical, structures) = step_r();
    let source = cq_sequence_lowering::compile(&physical, &structures).unwrap();
    let source = cq_gpu_sequence_core::runs::record_representation(&physical, &structures, &source).unwrap();
    let evidence: cq_sequence_plan::Advice = syn::parse_quote!(evidence from authored("experiments/gpu/sequence/tests/planner.rs fixture") {
        count Region = 5; trusted_key Region [r_regionkey];
        count Nation = 25; trusted_key Nation [n_nationkey];
        count Customer = 150000; trusted_key Customer [c_custkey];
        count Orders = 1500000; trusted_key Orders [o_orderkey];
        count Lineitem = 6001215;
        count Supplier = 10000; trusted_key Supplier [s_suppkey];
    });
    let (physical, structures, source) = evidence.apply(&physical, &structures, &source).unwrap();
    let result =
        cq_sequence_plan::decompose(&physical, &structures, &source, &machine, &cq_gpu_sequence::catalog()).unwrap();
    let replayed = result
        .trace
        .iter()
        .fold((physical.clone(), structures.clone(), source.clone()), |previous, advice| {
            let next = advice.apply(&previous.0, &previous.1, &previous.2).unwrap();
            check_rewrite(&previous.0, &previous.1, &previous.2, &next.0, &next.1, &next.2).unwrap();
            next
        });
    assert_eq!(replayed, (result.physical.clone(), result.structures.clone(), result.program.clone()), "PL-Closure");
    let trace: Vec<String> = result
        .trace
        .iter()
        .map(|advice| quote::ToTokens::to_token_stream(advice).to_string())
        .collect();
    // Region (5) and the Nation prefix (25) fill less than a tile; the
    // Customer prefix is bounded by its 150000 customers. Orders and Lineitem
    // fan out (their lookups cover no provider key); Supplier is membership.
    assert_eq!(
        &trace[..4],
        [
            "retain (rnc , 150000 , 256 , 65536)",
            "partition (__sequence_output , 256)",
            "segment (rnco , 1024 , items 4)",
            "segment (rncol , 1024 , items 4)",
        ]
    );
    let analysis = check_grading(&result.physical, &result.structures, &result.program, &machine).unwrap();
    cq_gpu_sequence::translate(&result.physical, &result.structures, &result.program, &analysis).unwrap();
    // A recorded bound that the evidence does not establish is rejected.
    let forged: cq_sequence_plan::Advice = syn::parse_quote!(retain(rnc, 1, 256, 65536));
    assert!(forged.apply(&physical, &structures, &source).is_err());
    // Without evidence the order keeps its single small root.
    let (physical, structures) = step_r();
    let plain = cq_sequence_lowering::compile(&physical, &structures).unwrap();
    let plain = cq_gpu_sequence_core::runs::record_representation(&physical, &structures, &plain).unwrap();
    let unadvised =
        cq_sequence_plan::decompose(&physical, &structures, &plain, &machine, &cq_gpu_sequence::catalog()).unwrap();
    assert!(
        !unadvised
            .trace
            .iter()
            .any(|advice| matches!(advice, cq_sequence_plan::Advice::Retain { .. }))
    );
}

fn assert_cost_acceptance(name: &str, mut authored: Stack) {
    let machine = cq_gpu_sequence::machine();
    authored.program = cq_gpu_sequence_core::runs::record_representation(&authored.physical, &authored.structures, &authored.program).unwrap();
    check_grading(&authored.physical, &authored.structures, &authored.program, &machine).unwrap();
    let generated = planned(&authored);
    let authored_cost = cq_sequence_plan::cost(&authored.physical, &authored.structures, &authored.program, &machine).unwrap();
    let planned_cost = cq_sequence_plan::cost(&generated.physical, &generated.structures, &generated.program, &machine).unwrap();
    eprintln!("H05 {name}: authored={authored_cost}; planned={planned_cost}");
    eprintln!(
        "H05 {name}: planned/authored ordering={:?}",
        planned_cost.partial_cmp(&authored_cost)
    );
    assert!(
        planned_cost.leq(&authored_cost),
        "H05 {name}: planned cost must be provably no greater than the authored reference"
    );
}
