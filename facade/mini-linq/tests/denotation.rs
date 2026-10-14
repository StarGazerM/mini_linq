//! Compile and run every planner/split/reducer for opaque partial Rust steps.
use quote::quote;

#[test]
fn every_planner_keeps_partial_steps_in_their_written_domain() {
    let queries: [(&str, proc_macro2::TokenStream); 3] = [
        (
            "struct Q; relation R(x:i32); relation S(w:i32); answer(x,y) :- R(x), S(w), if *x != 0 && *w > 1, let y:i32 = 10 / *x;",
            quote!(let R: &[(i32,)] = &[(0,), (2,)]; let S: &[(i32,)] = &[];),
        ),
        (
            "struct Q; relation R(x:i32); relation S(k:i32,y:i32); answer(x,y) :- R(x), if *x != 0, S(10 / *x,y);",
            quote!(let R: &[(i32,)] = &[(0,)]; let S: &[(i32,i32)] = &[(10,4)];),
        ),
        (
            "struct Q; relation R(x:i32); relation S(w:i32); answer(y,w) :- R(x), if *x != 0, let y:i32 = 10 / *x, S(w);",
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
                        let expression = cq_physical_realization::compile(&structures).unwrap();
                        tests.push(quote!({ #bindings let rows: Vec<_> = (#expression).collect(); assert!(rows.is_empty()); }));
                    }
                }
            }
        }
    }
    // The right keyed input contains the chain itself. Its lazy index is
    // initialized by nonempty T, but the earlier empty S prevents the bind.
    let ra = syn::parse_quote!(relational {
        relation R(x:i32); relation S(w:i32); relation T(y:i32);
        r = rename R@0 {x -> x}; s = rename S@1 {w -> w};
        j = natural_join r with s;
        g = guard j if (*x != 0 && *w > 1);
        b = bind g with y:i32 = (10 / *x);
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
    let expression = cq_physical_realization::compile(&structures).unwrap();
    tests.push(quote!({
        let R: &[(i32,)] = &[(0,)]; let S: &[(i32,)] = &[]; let T: &[(i32,)] = &[(1,)];
        let rows: Vec<_> = (#expression).collect(); assert!(rows.is_empty());
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
