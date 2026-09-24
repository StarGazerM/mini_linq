//! Authored Stream programs check the language contract independently of any
//! lowering: capabilities, scope, affine pipes, IX discharge, and support.
use cq_ir::stream::{self, Operation, Realization};
use quote::{ToTokens, quote};

fn streamed(definition: &mut stream::Definition) -> &mut stream::Streamed {
    match &mut definition.realization {
        Realization::Stream(streamed) => streamed,
        Realization::Alias { .. } => panic!("expected a stream definition"),
    }
}

fn chain(stream: proc_macro2::TokenStream) -> stream::Module {
    syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            p = project j1 keep {a, c};
            output p as answer(a, c);
        }
        indexes { i0 = @r[0]; i1 = @s[0]; }
        uses { j0.right = i0; j1.right = i1; }
        stream { #stream }
    }
}

fn hash_chain() -> stream::Module {
    chain(quote! {
        open L provides { iter };
        open R provides { iter };
        open S provides { iter };
        l = pipe for (a) in scan L { emit (a) };
        r = let provides { seek @r[0] } for (a, b) in scan R { emit (a, b) };
        s = let provides { seek @s[0] } for (b, c) in scan S { emit (b, c) };
        j0 = pipe for (a) in scan l { for (b) in seek r[0](a) { emit (a, b) } };
        j1 = pipe for (a, b) in scan j0 { for (c) in seek s[0](b) { emit (a, b, c) } };
        p = pipe for (a, b, c) in scan j1 { support { emit (a, c) } };
        return p;
    })
}

#[test]
fn hash_chain_round_trips_and_exports_analysis() {
    let module = hash_chain();
    let analysis = stream::contract::check(&module).unwrap();
    assert_eq!(
        syn::parse2::<stream::Module>(module.to_token_stream()).unwrap(),
        module
    );
    assert_eq!(analysis.headings["j1"], ["a", "b", "c"]);
    assert_eq!(analysis.provided["r"], [Operation::Seek(vec![0])]);
    assert_eq!(
        analysis.required["j0"]["r"].iter().next(),
        Some(&Operation::Seek(vec![0]))
    );
    assert_eq!(analysis.readers["j0"], ["j1"]);
    assert_eq!(analysis.unique["j0"], "j1");
    assert_eq!(analysis.unique["r"], "j0");
}

#[test]
fn merge_join_uses_co_iteration_on_ordered_operands() {
    // Sorted L and R: the join on `a` co-iterates instead of seeking, and an
    // ordered capability discharges the IX seek demand by subsumption.
    let module = chain(quote! {
        open L provides { iter };
        open R provides { iter };
        open S provides { iter };
        l = let provides { ordered @l[0] } for (a) in scan L { emit (a) };
        r = let provides { ordered @r[0] } for (a, b) in scan R { emit (a, b) };
        s = let provides { seek @s[0] } for (b, c) in scan S { emit (b, c) };
        j0 = pipe co (() in order l[0], (b) in order r[0]) on (a) { emit (a, b) };
        j1 = pipe for (a, b) in scan j0 { for (c) in seek s[0](b) { emit (a, b, c) } };
        p = pipe for (a, b, c) in scan j1 { support { emit (a, c) } };
        return p;
    });
    let analysis = stream::contract::check(&module).unwrap();
    assert_eq!(
        analysis.required["j0"]["l"].iter().next(),
        Some(&Operation::Ordered(vec![0]))
    );
}

fn ya(stream: proc_macro2::TokenStream) -> stream::Module {
    syn::parse_quote! {
        struct YaDelivery;
        relation D(driver_id: i32, depot_id: i32);
        relation S(depot_id: i32, item_id: i32);
        relation N(job_id: i32, item_id: i32);
        answer(driver, job) :- D(driver, depot), S(depot, item), N(job, item);
        relational {
            d0 = rename D {driver_id -> driver, depot_id -> depot};
            s0 = rename S {depot_id -> depot, item_id -> item};
            n0 = rename N {job_id -> job, item_id -> item};
            s1 = semijoin s0 with d0;
            s2 = semijoin s1 with n0;
            d1 = semijoin d0 with s2;
            n1 = semijoin n0 with s2;
            sn = natural_join s2 with n1;
            live = project sn keep {depot, job};
            all_rows = natural_join live with d1;
            result = project all_rows keep {driver, job};
            output result as answer(driver, job);
        }
        indexes { i0 = @d0[0]; i1 = @n0[0]; i2 = @s2[0]; i3 = @s2[1]; i4 = @n1[0]; i5 = @d1[0]; }
        uses { s1.right = i0; s2.right = i1; d1.right = i2; n1.right = i3; sn.right = i4; all_rows.right = i5; }
        stream { #stream }
    }
}

// Headings (sorted): d0 {depot, driver}; s0 {depot, item}; n0 {item, job};
// s2 {depot, item}; n1 {item, job}; sn {depot, item, job}; live {depot, job};
// d1 {depot, driver}; all_rows {depot, driver, job}; result {driver, job}.
fn ya_eager() -> proc_macro2::TokenStream {
    quote! {
        open D provides { iter };
        open S provides { iter };
        open N provides { iter };
        d0 = let provides { iter, contains @d0[0], seek @d0[0] } for (driver, depot) in scan D { emit (depot, driver) };
        s0 = let provides { iter } for (depot, item) in scan S { emit (depot, item) };
        n0 = let provides { iter, contains @n0[0], seek @n0[0] } for (job, item) in scan N { emit (item, job) };
        s1 = pipe for (depot, item) in scan s0 { if contains d0[0](depot) { emit (depot, item) } };
        s2 = let provides { iter, contains @s2[0], contains @s2[1] } for (depot, item) in scan s1 { if contains n0[0](item) { emit (depot, item) } };
        d1 = let provides { seek @d1[0] } for (depot, driver) in scan d0 { if contains s2[0](depot) { emit (depot, driver) } };
        n1 = let provides { seek @n1[0] } for (item, job) in scan n0 { if contains s2[1](item) { emit (item, job) } };
        sn = pipe for (depot, item) in scan s2 { for (job) in seek n1[0](item) { emit (depot, item, job) } };
        live = pipe for (depot, item, job) in scan sn { emit (depot, job) };
        all_rows = pipe for (depot, job) in scan live { for (driver) in seek d1[0](depot) { emit (depot, driver, job) } };
        result = pipe for (depot, driver, job) in scan all_rows { support { emit (driver, job) } };
        return result;
    }
}

#[test]
fn yannakakis_reduction_is_eager_semijoins_over_contains() {
    let module = ya(ya_eager());
    let analysis = stream::contract::check(&module).unwrap();
    assert_eq!(analysis.readers["s2"], ["d1", "n1", "sn"]);
    assert!(!analysis.unique.contains_key("s2"));
    assert_eq!(analysis.unique["s1"], "s2");
}

#[test]
fn contract_rejects_each_ill_formed_program() {
    type Mutation = fn(&mut stream::Module);
    let cases: [(&str, Mutation, &str); 9] = [
        (
            "missing capability",
            |m| {
                streamed(&mut m.program.definitions[1]).mode =
                    syn::parse_quote!(let provides { iter });
            },
            "does not provide Seek",
        ),
        (
            "pipe read twice",
            |m| {
                streamed(&mut m.program.definitions[3]).mode = syn::parse_quote!(pipe);
                streamed(&mut m.program.definitions[4]).body = syn::parse_quote! {
                    for (a, b) in scan j0 { for (x, y) in scan j0 { emit (a, b, x) } }
                };
            },
            "read exactly once",
        ),
        (
            "unbound key",
            |m| {
                streamed(&mut m.program.definitions[3]).body = syn::parse_quote!(for (a) in scan l { for (b) in seek r[0](zz) { emit (a, b) } });
            },
            "is not bound",
        ),
        (
            "wrong arity",
            |m| {
                streamed(&mut m.program.definitions[0]).body =
                    syn::parse_quote!(for (a, q) in scan L { emit (a) });
            },
            "pattern binds 2 variables",
        ),
        (
            "emit arity",
            |m| {
                streamed(&mut m.program.definitions[3]).body =
                    syn::parse_quote!(for (a) in scan l { for (b) in seek r[0](a) { emit (a) } });
            },
            "has arity 2",
        ),
        (
            "missing support",
            |m| {
                streamed(&mut m.program.definitions[5]).body =
                    syn::parse_quote!(for (a, b, c) in scan j1 { emit (a, c) });
            },
            "set-valued",
        ),
        (
            "ix not discharged",
            |m| {
                streamed(&mut m.program.definitions[1]).mode =
                    syn::parse_quote!(let provides { iter, seek @r[0] });
                streamed(&mut m.program.definitions[3]).body = syn::parse_quote! {
                    for (a) in scan l { for (x, b) in scan r { if (x == a) { emit (a, b) } } }
                };
            },
            "as IX demands",
        ),
        (
            "lazy on join",
            |m| {
                streamed(&mut m.program.definitions[3]).lazy = Some(Default::default());
            },
            "only to semijoin",
        ),
        (
            "unopened input",
            |m| {
                m.program.inputs.remove(0);
            },
            "not opened",
        ),
    ];
    for (name, mutate, expected) in cases {
        let mut module = hash_chain();
        mutate(&mut module);
        let error = stream::contract::check(&module)
            .expect_err(name)
            .to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn lazy_semijoin_requires_its_consumer_to_retest_the_reducer() {
    // TreeTracker-style: s1 stays a pipe and its consumer must read d0 again.
    let mut tokens = ya_eager();
    let module = ya(std::mem::take(&mut tokens));
    let mut lazy = module.clone();
    streamed(&mut lazy.program.definitions[3]).lazy = Some(Default::default());
    let error = stream::contract::check(&lazy).unwrap_err().to_string();
    assert!(error.contains("also reads `d0`"), "{error}");
    // Making the consumer re-test d0 admits the strategy.
    streamed(&mut lazy.program.definitions[4]).body = syn::parse_quote! {
        for (depot, item) in scan s1 {
            if contains n0[0](item) { if contains d0[0](depot) { emit (depot, item) } }
        }
    };
    stream::contract::check(&lazy).unwrap();
}

#[test]
fn alias_inherits_host_capabilities_translated_through_the_rename() {
    // R(a, b) with host key on column 1 (`b`); r renames a -> x, b -> y so the
    // seek on r's sorted heading {x, y} is on ordinal 1.
    let module: stream::Module = syn::parse_quote! {
        struct Aliased;
        relation L(y: i32);
        relation R(a: i32, b: i32);
        answer(x, y) :- L(y), R(x, y);
        relational {
            l = rename L {y -> y};
            r = rename R {a -> x, b -> y};
            j = natural_join l with r;
            output j as answer(x, y);
        }
        indexes { i0 = @r[1]; }
        uses { j.right = i0; }
        stream {
            open L provides { iter };
            open R provides { seek @R[1] };
            l = pipe for (y) in scan L { emit (y) };
            r = alias R;
            j = pipe for (y) in scan l { for (x) in seek r[1](y) { emit (x, y) } };
            return j;
        }
    };
    let analysis = stream::contract::check(&module).unwrap();
    assert_eq!(analysis.provided["r"], [Operation::Seek(vec![1])]);
    assert_eq!(analysis.aliases["r"], "R");
    assert_eq!(analysis.readers["R"], ["r"]);

    let mut wrong_host = module.clone();
    wrong_host.program.definitions[1].realization = syn::parse_quote!(alias L);
    let error = stream::contract::check(&wrong_host)
        .unwrap_err()
        .to_string();
    assert!(error.contains("renames `R`"), "{error}");
    let mut not_rename = module.clone();
    not_rename.program.definitions[2].realization = syn::parse_quote!(alias R);
    let error = stream::contract::check(&not_rename)
        .unwrap_err()
        .to_string();
    assert!(error.contains("only a rename"), "{error}");
}
