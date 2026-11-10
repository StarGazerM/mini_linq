//! Source-document statistics preserve the legacy ordering policy without a
//! second count/key grammar. TPC-H rows come from the checked-in SF1 document.
use cq_planners::{cq, vo};
use cq_planning::{PlanOptions, prefix_valuation};
use cq_order::{Heuristic, Options, Ranking, Split};
use cq_workload_authored::tpch_evidence;
use std::sync::Arc;

fn reads(plan: &vo::Program) -> Vec<String> {
    plan.valuation.steps.iter().filter_map(|step| match step {
        vo::Step::Atom(atom, _) => Some(quote::ToTokens::to_token_stream(atom).to_string()),
        _ => None,
    }).collect()
}

#[test]
fn file_statistics_stream_the_key_functional_root() {
    let mut source: cq::Program = syn::parse_quote! {
        struct Q;
        relation Orders(o_orderkey:i64,o_orderdate:i32);
        relation Lineitem(l_orderkey:i64,l_commitdate:i32,l_receiptdate:i32);
        answer(o) :- Orders(o,d), where d >= 10 && d < 20,
            Lineitem(o,c,r), where c < r;
    };
    source.declared_evidence = tpch_evidence::load_sf1(&source.declarations).unwrap();
    let options = PlanOptions::default();
    let (normal, plan) = prefix_valuation(&source, &options).unwrap();
    assert_eq!(reads(&plan), ["Lineitem @ 3 (o , c , r)", "Orders @ 0 (o , d)"]);
    let ordering = Options {
        heuristic: Heuristic::Prefix(Ranking::Connected), split: Split::Coarse,
    };
    let written = cq_order::compile(&normal, &ordering).unwrap();
    assert!(cq_order::evidence::check_evidence(&normal, &written, &ordering).is_err());

    // Missing key statistics keep the specified default; they are not guessed
    // from the familiar Orders/column names.
    let mut no_key = source.clone();
    Arc::make_mut(&mut no_key.declared_evidence.as_mut().unwrap().document)
        .relations.get_mut("orders").unwrap().keys.clear();
    let (_, plan) = prefix_valuation(&no_key, &options).unwrap();
    assert_eq!(reads(&plan), ["Orders @ 0 (o , d)", "Lineitem @ 3 (o , c , r)"]);

    let mut no_count = source.clone();
    Arc::make_mut(&mut no_count.declared_evidence.as_mut().unwrap().document)
        .relations.get_mut("orders").unwrap().rows = None;
    let (_, plan) = prefix_valuation(&no_count, &options).unwrap();
    assert_eq!(reads(&plan), ["Orders @ 0 (o , d)", "Lineitem @ 3 (o , c , r)"]);

    // A data attachment does not override an explicit checked schedule.
    let (_, plan) = prefix_valuation(&source, &syn::parse_quote!(order(Orders, Lineitem))).unwrap();
    assert_eq!(reads(&plan), ["Orders @ 0 (o , d)", "Lineitem @ 3 (o , c , r)"]);
}

#[test]
fn file_statistics_look_up_filtered_keys_before_expansion() {
    let mut source: cq::Program = syn::parse_str(include_str!("fixtures/evidence_lookup.cq")).unwrap();
    let json = include_str!("fixtures/evidence_lookup.json");
    source.declared_evidence = Some(syn::parse_quote! {
        declared_evidence #json {
            A = "A" full [a = "a", x = "x"];
            B = "B" full [b = "b", y = "y"];
            F = "F" full [a = "a", b = "b", v = "v"];
        }
    });
    let (_, plan) = prefix_valuation(&source, &syn::parse_quote!(no_reduce)).unwrap();
    assert_eq!(reads(&plan), [
        "F @ 4 (a , b , v)", "B @ 2 (b , _)",
        "A @ 0 (a , x)", "B @ 2 (b , y)",
    ]);
}

#[test]
fn file_statistics_keep_written_root_with_a_filtered_neighbour() {
    let mut source: cq::Program = syn::parse_quote! {
        struct Q;
        relation P(p:i64,s:i64,v:i64);
        relation L(o:i64,p:i64,s:i64);
        relation O(o:i64,d:i32);
        answer(o,p) :- P(p,s,v), where v > 0, L(o,p,s), O(o,d), where d >= 10 && d < 20;
    };
    source.declared_evidence = tpch_evidence::bind_sf1(
        &source.declarations,
        vec![
            syn::parse_quote!(P = "partsupp" full [p="ps_partkey",s="ps_suppkey",v="ps_supplycost"];),
            syn::parse_quote!(L = "lineitem" full [o="l_orderkey",p="l_partkey",s="l_suppkey"];),
            syn::parse_quote!(O = "orders" full [o="o_orderkey",d="o_orderdate"];),
        ],
    ).unwrap();
    let (_, plan) = prefix_valuation(&source, &PlanOptions::default()).unwrap();
    assert_eq!(reads(&plan)[0], "P @ 0 (p , s , v)");
}
