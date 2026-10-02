#[path = "../../../test-support/relational/rewrites.rs"]
mod authored;
// Leibniz identity and functional extensionality permit canonical projection.
// Compile and execute both optimized queries and the written-order reference.
use proc_macro2::TokenStream;
use quote::quote;

fn expression(source: &str, options: cq_order::Options, decisions: &str) -> TokenStream {
    elaborate_expression(source, options, decisions, false)
}

fn written_expression(source: &str, decisions: &str) -> TokenStream {
    elaborate_expression(source, cq_order::Options::default(), decisions, true)
}

fn elaborate_expression(
    source: &str,
    options: cq_order::Options,
    decisions: &str,
    written: bool,
) -> TokenStream {
    let source = cq_lang_cq::normal::normalize(&syn::parse_str(source).unwrap()).unwrap();
    let vo = cq_order::compile(&source, &options).unwrap();
    cq_order::check(&source, &vo, &options).unwrap();
    let ra = if written {
        cq_elaborate::compile_written(&vo).unwrap()
    } else {
        cq_elaborate::compile(&vo).unwrap()
    };
    cq_elaborate::check(&vo, &ra).unwrap();
    realize(&ra, decisions)
}

fn realize(ra: &cq_lang_ra::Program, decisions: &str) -> TokenStream {
    let ix = cq_index_policy::compile(ra).unwrap();
    cq_index_policy::check(ra, &ix).unwrap();
    let mut physical = cq_physical_lowering::compile(&ix).unwrap();
    if decisions.contains("merge") {
        physical = authored::merged(&physical).unwrap();
    }
    if decisions.contains("canon") {
        physical = authored::canonical_projects(&physical).unwrap();
    }
    cq_physical_lowering::check(&ix, &physical).unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    cq_structure_selection::check(&physical, &structures).unwrap();
    let expression = cq_physical_realization::compile(&structures).unwrap();
    quote!(#expression)
}

#[test]
fn admissible_values_and_literal_reads_refine_the_written_order() {
    let mut cases = Vec::new();
    for heuristic in [
        cq_order::Heuristic::Written,
        cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
        cq_order::Heuristic::Prefix(cq_order::Ranking::Auto),
        cq_order::Heuristic::Elimination,
    ] {
        for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
            for decisions in ["probe keep", "probe canon", "merge keep", "merge canon"] {
                let label = format!("{heuristic:?}/{split:?}/{decisions}");
                // A slice has no native OrderedRead. Merge fixtures group on
                // the declared leading key and retain all payload rows.
                let (r_host, s_host) = if decisions.contains("merge") {
                    (
                        quote!(let R: ::std::collections::BTreeMap<(Key,),Vec<(i32,)>> = rows_r.iter().cloned().fold(::std::collections::BTreeMap::new(), |mut map,(key,row)| { map.entry((key,)).or_default().push((row,)); map });),
                        quote!(let S: ::std::collections::BTreeMap<(Key,),Vec<()>> = rows_s.iter().cloned().map(|row| (row,vec![()])).collect();),
                    )
                } else {
                    (
                        quote!(let R = rows_r.as_slice();),
                        quote!(let S = rows_s.as_slice();),
                    )
                };
                let source = "struct Q; relation R(k:Key,r:i32); relation S(k:Key); \
                              answer(y) :- R(x,z), S(x), let y:bool = x.flag;";
                let written = written_expression(source, decisions);
                let query = expression(source, cq_order::Options { heuristic, split }, decisions);
                cases.push(quote!({
                    let rows_r = [(Key { id: 1, flag: false }, 0)];
                    let rows_s = [(Key { id: 1, flag: false },), (Key { id: 1, flag: true },)];
                    #r_host #s_host
                    let expected: Vec<_> = (#written).collect();
                    let actual: Vec<_> = (#query).collect();
                    assert_eq!(actual, expected, "Leibniz join: {}", #label);
                }));
                let source = "struct Q; relation R(k:Key,r:i32); answer(flag,z) :- R(x,z), let flag:bool=x.flag;";
                let written = written_expression(source, decisions);
                let correlated =
                    expression(source, cq_order::Options { heuristic, split }, decisions);
                cases.push(quote!({
                    let rows_r = [(Key {id:1,flag:false},0),(Key {id:1,flag:true},1)];
                    #r_host
                    let mut written: Vec<_> = (#written).collect();
                    let mut actual: Vec<_> = (#correlated).collect();
                    written.sort(); actual.sort();
                    assert_eq!(actual,written,"full-row correlation: {}",#label);
                }));
                let source = "struct Q; relation R(k:Key,r:i32); answer(y) :- R(x,z), let y:i32=if x.flag {10 / *z} else {0};";
                let written = written_expression(source, decisions);
                let partial = expression(source, cq_order::Options { heuristic, split }, decisions);
                cases.push(quote!({
                    let rows_r = [(Key {id:1,flag:true},1),(Key {id:1,flag:false},0)];
                    #r_host
                    let mut written: Vec<_> = (#written).collect();
                    let mut actual: Vec<_> = (#partial).collect();
                    written.sort(); actual.sort();
                    assert_eq!(actual,written,"correlation cannot introduce a panic: {}",#label);
                }));
            }
        }
    }
    let source = "struct Q; relation R(k:i32,p:i32); \
                  answer(y) :- R(x,z), if * x == 5, let y:i32 = *x + 1;";
    let written = written_expression(source, "decisions {}");
    let point = expression(source, cq_order::Options::default(), "decisions {}");
    cases.push(quote!({
        let rows = [(5_i32,7_i32), (5,8), (6,9)];
        let R = rows.as_slice();
        let expected: Vec<_> = (#written).collect();
        let actual: Vec<_> = (#point).collect();
        assert_eq!(actual, expected, "slice constant read retains the key value");
    }));
    cases.push(quote!({
        let R = ::std::collections::HashMap::from([((5_i32,),vec![(7_i32,), (8,)]), ((6,),vec![(9,)])]);
        let expected: Vec<_> = (#written).collect();
        let actual: Vec<_> = (#point).collect();
        assert_eq!(actual, expected, "native constant read retains the key value");
    }));
    run_cases(&cases);
}

#[test]
fn canonical_projection_before_an_opaque_guard_enumerates_fan_out_linearly() {
    let source = "struct Q; relation R(k:i32,z:i32); relation S(k:i32,w:i32); \
                  answer(y) :- R(x,z), S(x,w), if observe(x,visits), let y:i32=10 / *x;";
    let normalized = cq_lang_cq::normal::normalize(&syn::parse_str(source).unwrap()).unwrap();
    let vo = cq_order::compile(&normalized, &cq_order::Options::default()).unwrap();
    let ra = syn::parse_quote!(relational {
        relation R(k:i32,z:i32); relation S(k:i32,w:i32);
        r = rename R {k -> x, z -> z}; rp = project r keep {x};
        s = rename S {k -> x, w -> w}; sp = project s keep {x};
        j = natural_join rp with sp;
        g = guard j if (observe(x,visits)); b = bind g with y:i32 = (10 / *x);
        p = project b keep {y}; output p as answer(y);
    });
    cq_elaborate::check(&vo, &ra).unwrap();
    let optimized = realize(&ra, "decisions {project=canon;}");
    let written = written_expression(source, "decisions {}");
    run_cases(&[quote!({
        fn observe(x: &i32, visits: &::std::cell::Cell<usize>) -> bool {
            assert_ne!(*x, 0, "both full written atoms must constrain the guard");
            visits.set(visits.get()+1);
            *x > 1
        }
        let mut previous_rows = None;
        for scale in [24_i32,48,96] {
            let mut rows_r: Vec<_> = (0..scale).map(|z| (2,z)).collect();
            rows_r.push((0,0)); // S has no 0: the written reference never panics.
            let rows_s: Vec<_> = (0..scale).map(|w| (2,w)).collect();
            let R = rows_r.as_slice(); let S = rows_s.as_slice();
            let written_visits = ::std::cell::Cell::new(0);
            let visits = &written_visits;
            let expected: ::std::collections::BTreeSet<_> = (#written).collect();
            assert_eq!(written_visits.get(), (scale*scale) as usize);
            let optimized_visits = ::std::cell::Cell::new(0);
            let visits = &optimized_visits;
            ::cq_storage::counters::reset();
            let actual: ::std::collections::BTreeSet<_> = (#optimized).collect();
            let counts = ::cq_storage::counters::snapshot();
            assert_eq!(actual, expected);
            assert_eq!(actual, ::std::collections::BTreeSet::from([(5,)]));
            assert_eq!(optimized_visits.get(), 1);
            assert!(counts.rows_enumerated > 0);
            if let Some(previous) = previous_rows {
                assert!(counts.rows_enumerated*2 < previous*5, "fan-out must enumerate linearly: {previous} -> {counts:?}");
            }
            previous_rows = Some(counts.rows_enumerated);
            println!("fan scale={scale} guard_visits={} counts={counts:?}", optimized_visits.get());
        }
        let R: &[(i32,i32)] = &[(0,0)];
        let S: &[(i32,i32)] = &[];
        let visits = &::std::cell::Cell::new(0);
        assert_eq!((#written).count(), 0);
        assert_eq!((#optimized).count(), 0);
        assert_eq!(visits.get(), 0);
    })]);
}

fn run_cases(cases: &[TokenStream]) {
    let program = quote! {
        #![allow(non_snake_case, unused_variables, unused_macros)]
        #[derive(Clone,Debug,PartialEq,Eq,Hash,PartialOrd,Ord)]
        struct Key { id:i32, flag:bool }
        fn main() { #(#cases)* }
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir()
        .join(format!("cq-provenance-{}", std::process::id()))
        .join(std::thread::current().name().unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), program.to_string()).unwrap();
    let runtime = std::process::Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name",
            "cq_storage",
            "--crate-type",
            "rlib",
            "--cfg",
            "feature=\"counters\"",
        ])
        .arg(root.join("runtime/cq-storage/src/lib.rs"))
        .arg("--out-dir")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        runtime.status.success(),
        "{}",
        String::from_utf8_lossy(&runtime.stderr)
    );
    let build = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg(dir.join("main.rs"))
        .arg("--extern")
        .arg(format!(
            "cq_storage={}",
            dir.join("libcq_storage.rlib").display()
        ))
        .arg("-o")
        .arg(dir.join("run"))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(dir.join("run"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    print!("{}", String::from_utf8_lossy(&run.stdout));
    std::fs::remove_dir_all(dir).unwrap();
}
