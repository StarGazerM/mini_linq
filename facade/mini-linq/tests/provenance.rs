//! Opaque Rust observes the first-bound stored value, including its provenance.
use proc_macro2::TokenStream;
use quote::quote;

fn expression(source: &str, options: cq_order::Options, policy: &str) -> TokenStream {
    let source = cq_lang_cq::normal::normalize(&syn::parse_str(source).unwrap()).unwrap();
    let vo = cq_order::compile(&source, &options).unwrap();
    cq_order::check(&source, &vo, &options).unwrap();
    let ra = cq_elaborate::compile(&vo).unwrap();
    cq_elaborate::check(&vo, &ra).unwrap();
    let ix = cq_index_policy::compile(&ra).unwrap();
    cq_index_policy::check(&ra, &ix).unwrap();
    let policy = syn::parse_str(policy).unwrap();
    let physical = cq_physical_lowering::compile(&ix, &policy).unwrap();
    cq_physical_lowering::check(&ix, &physical, &policy).unwrap();
    let structures = cq_structure_selection::compile(&physical, &policy).unwrap();
    cq_structure_selection::check(&physical, &structures, &policy).unwrap();
    let expression = cq_physical_realization::compile(
        &structures,
        &cq_physical_realization::Endpoints {
            distinct_state: syn::parse_quote!(distinct),
            index_provider: syn::parse_quote!(indexes),
            intersect: false,
        },
    )
    .unwrap();
    quote!(#expression)
}

#[test]
fn equal_columns_and_literal_reads_preserve_the_first_bound_value() {
    let mut cases = Vec::new();
    for heuristic in [
        cq_order::Heuristic::Written,
        cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
        cq_order::Heuristic::Prefix(cq_order::Ranking::Auto),
        cq_order::Heuristic::Elimination,
    ] {
        for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
            for policy in [
                "policy {join=seek; project=keep;}",
                "policy {join=seek; project=canon;}",
                "policy {join=merge; project=keep;}",
                "policy {join=merge; project=canon;}",
            ] {
                let label = format!("{heuristic:?}/{split:?}/{policy}");
                // A slice has no native OrderedRead. Merge fixtures use full
                // row keys so both distinct source witnesses remain present.
                let (r_host, s_host) = if policy.contains("join=merge") {
                    (
                        quote!(let R: ::std::collections::BTreeMap<(Key,i32),Vec<()>> = rows_r.iter().cloned().map(|row| (row,vec![()])).collect();),
                        quote!(let S: ::std::collections::BTreeMap<(Key,),Vec<()>> = rows_s.iter().cloned().map(|row| (row,vec![()])).collect();),
                    )
                } else {
                    (
                        quote!(let R = rows_r.as_slice();),
                        quote!(let S = rows_s.as_slice();),
                    )
                };
                let query = expression(
                    "struct Q; relation R(k:Key,r:i32); relation S(k:Key); \
                 answer(y) :- R(x,z), S(x), let y:bool = x.flag;",
                    cq_order::Options { heuristic, split },
                    policy,
                );
                cases.push(quote!({
                    let rows_r = [(Key { id: 1, flag: false }, 0)];
                    let rows_s = [(Key { id: 1, flag: true },)];
                    #r_host #s_host
                    let mut written = Vec::new();
                    for (x, _) in &rows_r {
                        for (alias,) in &rows_s {
                            if x == alias { written.push((x.flag,)); }
                        }
                    }
                    let actual: Vec<_> = (#query).collect();
                    assert_eq!(actual, written, "first-bound representative: {}", #label);
                }));
                let correlated = expression(
                    "struct Q; relation R(k:Key,r:i32); answer(flag,z) :- R(x,z), let flag:bool=x.flag;",
                    cq_order::Options { heuristic, split },
                    policy,
                );
                cases.push(quote!({
                    let rows_r = [(Key {id:1,flag:false},0),(Key {id:1,flag:true},1)];
                    #r_host
                    let mut written: Vec<_> = rows_r.iter().map(|(x,z)| (x.flag,z)).collect();
                    let mut actual: Vec<_> = (#correlated).collect();
                    written.sort(); actual.sort();
                    assert_eq!(actual,written,"full-row correlation: {}",#label);
                }));
                let partial = expression(
                    "struct Q; relation R(k:Key,r:i32); answer(y) :- R(x,z), let y:i32=if x.flag {10 / *z} else {0};",
                    cq_order::Options { heuristic, split },
                    policy,
                );
                cases.push(quote!({
                let rows_r = [(Key {id:1,flag:true},1),(Key {id:1,flag:false},0)];
                #r_host
                let mut written: Vec<_> = rows_r.iter().map(|(x,z)| (if x.flag {10 / *z} else {0},)).collect();
                let mut actual: Vec<_> = (#partial).collect();
                written.sort(); actual.sort();
                assert_eq!(actual,written,"correlation cannot introduce a panic: {}",#label);
            }));
            }
        }
    }
    let point = expression(
        "struct Q; relation R(k:i32,p:i32); \
         answer(y) :- R(x,z), x=5, let y:bool = ::std::ptr::eq(x,captured);",
        cq_order::Options::default(),
        "policy {}",
    );
    cases.push(quote!({
        let rows = [(5_i32,7_i32)];
        let R = rows.as_slice();
        let captured = &rows[0].0;
        let written: Vec<_> = R.iter().filter(|(key,_)| *key == 5)
            .map(|(x,_)| (::std::ptr::eq(x,captured),)).collect();
        let actual: Vec<_> = (#point).collect();
        assert_eq!(actual, written, "slice const-read retains the stored key");
    }));
    cases.push(quote!({
        let R = ::std::collections::HashMap::from([((5_i32,),vec![(7_i32,)])]);
        let captured = &R.get_key_value(&(5,)).unwrap().0.0;
        let written: Vec<_> = R.iter().filter(|((key,),_)| *key == 5)
            .flat_map(|((x,), payload)| payload.iter().map(move |_| (::std::ptr::eq(x,captured),))).collect();
        let actual: Vec<_> = (#point).collect();
        assert_eq!(actual, written, "native const-read retains the stored key");
    }));
    let program = quote! {
        #![allow(non_snake_case, unused_variables, unused_macros)]
        macro_rules! distinct {
            (operator $id:ident; new) => { ::std::collections::HashSet::new() };
            (operator $id:ident; insert($set:expr,$row:expr)) => { $set.insert(Clone::clone($row)) };
        }
        macro_rules! indexes {
            (probe) => { ::cq_storage::HashIndex<'_,_,_> };
            (semi) => { ::std::collections::HashSet<_> };
            (ordered) => { ::std::collections::BTreeMap<_,Vec<_>> };
            (trie $inner:ty) => { ::cq_storage::Trie<'_,_,$inner> };
        }
        #[derive(Clone,Debug)]
        struct Key { id:i32, flag:bool }
        impl PartialEq for Key { fn eq(&self,other:&Self)->bool { self.id == other.id } }
        impl Eq for Key {}
        impl ::std::hash::Hash for Key {
            fn hash<H: ::std::hash::Hasher>(&self,state:&mut H) { self.id.hash(state); }
        }
        impl Ord for Key {
            fn cmp(&self,other:&Self)->::std::cmp::Ordering { self.id.cmp(&other.id) }
        }
        impl PartialOrd for Key {
            fn partial_cmp(&self,other:&Self)->Option<::std::cmp::Ordering> { Some(self.cmp(other)) }
        }
        fn main() { #(#cases)* }
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("cq-provenance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), program.to_string()).unwrap();
    let runtime = std::process::Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name",
            "cq_storage",
            "--crate-type",
            "rlib",
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
    std::fs::remove_dir_all(dir).unwrap();
}
