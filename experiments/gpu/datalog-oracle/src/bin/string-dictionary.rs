#![recursion_limit = "2048"]
use cq_gpu_runtime::{Codec, Dictionary, Runtime};
use std::collections::BTreeSet;

fn main() {
    let runtime = Runtime::new().unwrap();
    let mut dictionary = Dictionary::default();
    static ROWS: [(i64,); 2] = [(1,), (2,)];
    let input = runtime
        .upload_rows(&ROWS, &mut dictionary, &[Codec::Plain])
        .unwrap();
    let result = cq_gpu_datalog::datalog! {
        struct DistinctLiteralHeads;
        relation E(x:i64) = (&input);
        relation T(x:i64,tag:&'static str);
        output T;
        T(*x,"a") :- E(x);
        T(*x,"b") :- E(x);
    };
    let rows = result.to_host().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(1, "a"), (1, "b"), (2, "a"), (2, "b")])
    );
    println!("distinct-literal-heads exact=true");

    // Inputs retain compatible but differently sized dictionary snapshots.
    let mut dictionary = Dictionary::default();
    let owned = runtime
        .upload_owned_rows(
            &[(10i64, String::from("a"))],
            &mut dictionary,
            &[Codec::Plain, Codec::Plain],
        )
        .unwrap();
    let input = runtime
        .upload_rows(&ROWS, &mut dictionary, &[Codec::Plain])
        .unwrap();
    static PAIRS: [(i64, &str); 2] = [(1, "external"), (2, "b")];
    let pairs = runtime
        .upload_rows(&PAIRS, &mut dictionary, &[Codec::Plain, Codec::Plain])
        .unwrap();
    let (joined, missing, guarded, constants, owned_output) = cq_gpu_datalog::datalog! {
        struct SharedLiteralDomain;
        relation E(x:i64) = (&input);
        relation M(x:i64,tag:&'static str) = (&pairs);
        relation Owned(x:i64,tag:String) = (&owned);
        relation A(x:i64,tag:&'static str);
        relation B(x:i64,tag:&'static str);
        relation Joined(x:i64,left:&'static str,right:&'static str);
        relation Missing(x:i64);
        relation Guarded(x:i64);
        relation Constants(x:i64,tag:&'static str);
        output Joined, Missing, Guarded, Constants, Owned;
        A(*x,"a") :- E(x);
        B(*x,"b") :- E(x);
        Joined(*x,*left,*right) :- A(x,left), B(x,right);
        Missing(*x) :- E(x), !M(x,"b");
        Guarded(*x) :- M(x,tag), #[total] if *tag == "external";
        Constants(7i64,"c") :- E(x);
    };
    assert_eq!(
        joined
            .to_host()
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([(1, "a", "b"), (2, "a", "b")])
    );
    assert_eq!(missing.to_host().unwrap(), [(1,)]);
    assert_eq!(guarded.to_host().unwrap(), [(1,)]);
    assert_eq!(constants.to_host().unwrap(), [(7, "c")]);
    assert_eq!(owned_output.to_host().unwrap(), [(10, String::from("a"))]);
    println!(
        "shared-dictionary-join-negation-compatible-prefixes-owned-input-constant-heads exact=true"
    );
}
