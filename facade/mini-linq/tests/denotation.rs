//! Compile and run every planner/split/reducer for opaque total Rust steps.
use quote::quote;

#[test]
fn every_planner_preserves_total_rust_and_relational_selection() {
    let machine = cq_sequence_pull::machine();
    let queries: [(&str, proc_macro2::TokenStream); 3] = [
        (
            "struct Q; relation R(x:i32); relation S(w:i32); answer(x,y) :- R(x), S(w), where x != 0 && w > 1, let y:i32 = 10 / (*x).max(1);",
            quote!(let R: &[(i32,)] = &[(0,), (2,)]; let S: &[(i32,)] = &[];),
        ),
        (
            "struct Q; relation R(x:i32); relation S(k:i32,y:i32); answer(x,y) :- R(x), where x != 0, S(10 / (*x).max(1),y);",
            quote!(let R: &[(i32,)] = &[(0,)]; let S: &[(i32,i32)] = &[(10,4)];),
        ),
        (
            "struct Q; relation R(x:i32); relation S(w:i32); answer(y,w) :- R(x), where x != 0, let y:i32 = 10 / (*x).max(1), S(w);",
            quote!(let R: &[(i32,)] = &[(0,)]; let S: &[(i32,)] = &[(1,), (2,)];),
        ),
    ];
    let mut tests = Vec::new();
    for (query, bindings) in queries {
        let source = cq_lang_cq::normal::normalize(&syn::parse_str(query).unwrap()).unwrap();
        for heuristic in [
            cq_order::Heuristic::Written,
            cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
            cq_order::Heuristic::Elimination,
        ] {
            for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
                let options = cq_order::Options { heuristic, split };
                let ordered = cq_order::compile(&source, &options).unwrap();
                cq_order::check(&source, &ordered, &options).unwrap();
                {
                    for semijoins in [cq_reduce::Reductions::None, cq_reduce::Reductions::Tree] {
                        let reduction = cq_reduce::Options { semijoins };
                        let vo = cq_reduce::compile(&ordered, &reduction).unwrap();
                        cq_reduce::check(&ordered, &vo, &reduction).unwrap();
                        let ra = cq_elaborate::compile(&vo).unwrap();
                        cq_elaborate::check(&vo, &ra).unwrap();
                        let ix = cq_index_policy::compile(&ra).unwrap();
                        cq_index_policy::check(&ra, &ix).unwrap();
                        let physical = cq_physical_lowering::compile(&ix).unwrap();
                        cq_physical_lowering::check(&ix, &physical).unwrap();
                        let structures = cq_structure_selection::compile(&physical).unwrap();
                        cq_structure_selection::check(&physical, &structures).unwrap();
                        let sequence = cq_sequence_lowering::compile(&structures).unwrap();
                        cq_sequence_lowering::check(&structures, &sequence).unwrap();
                        let analysis =
                            cq_lang_sequence::check_grading(&sequence, &machine).unwrap();
                        let expression = cq_sequence_pull::translate(&sequence, &analysis).unwrap();
                        tests.push(quote!({ #bindings let rows: Vec<_> = (#expression).into_iter().collect(); assert!(rows.is_empty()); }));
                    }
                }
            }
        }
    }
    // A lazy keyed input containing total Rust preserves the empty join result,
    // regardless of where its expression is evaluated after its inputs bind.
    let ra = syn::parse_quote!(relational {
        relation R(x:i32); relation S(w:i32); relation T(y:i32);
        r = rename R@0 {x -> x}; s = rename S@1 {w -> w};
        j = natural_join r with s;
        g = guard j if (*x != 0 && *w > 1);
        b = bind g with y:i32 = (10 / (*x).max(1));
        t = rename T@2 {y -> y}; indexed = natural_join t with b;
        p = project indexed keep {x,y}; output p as answer(x,y);
    });
    let ix = cq_index_policy::compile(&ra).unwrap();
    let physical = cq_physical_lowering::compile(&ix).unwrap();
    cq_physical_lowering::check(&ix, &physical).unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    assert!(
        structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "b")
    );
    let sequence = cq_sequence_lowering::compile(&structures).unwrap();
    cq_sequence_lowering::check(&structures, &sequence).unwrap();
    let analysis = cq_lang_sequence::check_grading(&sequence, &machine).unwrap();
    let expression = cq_sequence_pull::translate(&sequence, &analysis).unwrap();
    tests.push(quote!({
        let R: &[(i32,)] = &[(0,)]; let S: &[(i32,)] = &[]; let T: &[(i32,)] = &[(1,)];
        let rows: Vec<_> = (#expression).into_iter().collect(); assert!(rows.is_empty());
    }));
    let program = quote! {
        #![allow(non_snake_case, unused_variables)]
        fn main() { #(#tests)* }
    };
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    let runtime = cq_testkit::rustc::Runtime::new(&source, false);
    let run = runtime.run(&program.to_string());
    print!("{}", String::from_utf8_lossy(&run.stdout));
}
