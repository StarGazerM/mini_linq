use cq_lang_sequence::{
    Analysis, DispatchShape, LevelNode, Machine, Program, ResourceAllocation, check_grading,
};
use quote::quote;

fn machine() -> Machine {
    fn node(name: &str, children: Vec<LevelNode>) -> LevelNode {
        LevelNode {
            name: syn::parse_str(name).unwrap(),
            collaborative: name == "team",
            dynamic: name == "engine",
            width: (name == "team").then_some(8),
            shared_bytes: (name == "team").then_some(1024),
            children,
        }
    }
    Machine {
        name: syn::parse_quote!(target),
        items: 3,
        root: node(
            "host",
            vec![node(
                "engine",
                vec![node("team", vec![node("item", vec![])])],
            )],
        ),
        // Deliberately not a GPU layout: the callback sees only structural facts.
        layout: Some(|shape| Ok(shape.chains.iter().map(Vec::len).sum())),
    }
}

fn lookup_program(body: proc_macro2::TokenStream, grades: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation L(a:u32); relation R(a:u32,b:u32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe canon probe first [a] l,r;
            return j as answer(a,b);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Provider [a];
            j canon = Canon;
        } sequence { #body }
        graded target { #grades }
    })
    .unwrap()
}

fn shape<'a>(analysis: &'a Analysis, publication: &str) -> &'a DispatchShape {
    &analysis
        .resources
        .values()
        .find(|usage| usage.shape.publication == publication)
        .expect("recorded publication resources")
        .shape
}

fn parents(allocation: &ResourceAllocation) -> Vec<&str> {
    let ResourceAllocation::Payload { parents } = allocation else {
        panic!("expected payload allocation")
    };
    parents.iter().map(|column| column.name.as_str()).collect()
}

#[test]
fn predicates_survive_projection_without_carrying_dead_columns() {
    let program: Program = syn::parse_quote! {
        physical {
            relation L(a:u32,x:i64,dead:i64); relation R(a:u32,b:u32);
            l = pipe rename L@0 {a -> a,x -> x,dead -> dead};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            selected = pipe select j if (x > 0);
            projected = pipe canon project selected keep {b};
            return projected as answer(b);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Provider [a];
            projected canon = Canon;
        } sequence {
            result = store map projected compact selected flatten lookup r [a] rows l;
        } graded target { result at engine; }
    };
    let analysis = check_grading(&program, &machine()).unwrap();
    let shape = shape(&analysis, "result");
    assert_eq!(shape.level, ["host", "engine", "team"]);
    assert_eq!(shape.width, Some(8));
    assert_eq!(shape.chains.len(), 1);
    assert_eq!(shape.chains[0].len(), 1);
    assert_eq!(parents(&shape.chains[0][0]), ["x"]);
    let ResourceAllocation::Payload { parents } = &shape.chains[0][0] else {
        unreachable!()
    };
    assert_eq!(parents[0].types, vec![syn::parse_quote!(i64)]);
    assert!(parents[0].coded);
}

#[test]
fn every_choice_arm_contributes_in_order() {
    let program = lookup_program(
        quote! {
            result = store capacity 128 choose [
                len l < 8 => flatten lookup r [a] rows l,
                else => flatten lookup r [a] rows l
            ];
        },
        quote!(result at engine;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let shape = shape(&analysis, "result");
    assert_eq!(shape.chains.len(), 2);
    for chain in &shape.chains {
        assert_eq!(chain.len(), 1);
        assert_eq!(parents(&chain[0]), ["a"]);
    }
}

#[test]
fn fold_scope_precedes_its_input_payload_and_uses_only_the_sum_column() {
    let program = lookup_program(
        quote! {
            result = store fold sum b flatten lookup r [a] rows l;
        },
        quote!(result at engine;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let chain = &shape(&analysis, "result").chains[0];
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0], ResourceAllocation::Fold);
    assert!(parents(&chain[1]).is_empty());
}

#[test]
fn a_fold_inside_a_child_tile_follows_the_parent_payload() {
    let program = lookup_program(
        quote! {
            result = store flatten fold sum b tile 8 items 3 flatten lookup r [a] rows l;
        },
        quote!(result at engine;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let chain = &shape(&analysis, "result").chains[0];
    assert_eq!(chain.len(), 2);
    assert!(parents(&chain[0]).is_empty());
    assert_eq!(chain[1], ResourceAllocation::Fold);
}

#[test]
fn terminal_ranges_and_published_stores_do_not_allocate_payload_scratch() {
    let program = lookup_program(
        quote! {
            view = flatten lookup r [a] rows l;
            first = store capacity 128 view;
            result = store capacity 128 canon first;
        },
        quote!(view at engine.team; first at engine; result at engine;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    assert!(shape(&analysis, "first").chains[0].is_empty());
    assert!(shape(&analysis, "result").chains[0].is_empty());
}

#[test]
fn membership_reads_do_not_allocate_payload_scratch() {
    for operation in [quote!(semi), quote!(anti)] {
        let program: Program = syn::parse2(quote! {
            physical {
                relation L(a:u32); relation R(a:u32);
                l = pipe rename L@0 {a -> a};
                r = let rename R@1 {a -> a};
                j = pipe canon #operation eager [a] l,r;
                return j as answer(a);
            } structures {
                l host scan = Reader [];
                r host scan = Reader [];
                r = Provider [a];
                j canon = Canon;
            } sequence {
                result = store tile 8 compact j lookup r [a] rows l;
            } graded target { result at engine; }
        })
        .unwrap();
        let analysis = check_grading(&program, &machine()).unwrap();
        let shape = shape(&analysis, "result");
        assert_eq!(shape.tile, Some(8));
        assert_eq!(shape.items, 1);
        assert!(shape.chains[0].is_empty());
    }
}

#[test]
fn later_lookup_keys_are_live_across_an_earlier_payload() {
    let program: Program = syn::parse_quote! {
        physical {
            relation L(a:u32,x:u32); relation R(a:u32,b:u32); relation S(x:u32,c:u32);
            l = pipe rename L@0 {a -> a,x -> x};
            r = let rename R@1 {a -> a,b -> b};
            s = let rename S@2 {x -> x,c -> c};
            first = pipe probe first [a] l,r;
            second = pipe probe first [x] first,s;
            projected = pipe canon project second keep {c};
            return projected as answer(c);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            s host scan = Reader [];
            r = Provider [a];
            s = Provider [x];
            projected canon = Canon;
        } sequence {
            result = store map projected flatten lookup s [x] flatten lookup r [a] rows l;
        } graded target { result at engine; }
    };
    let analysis = check_grading(&program, &machine()).unwrap();
    let chain = &shape(&analysis, "result").chains[0];
    assert_eq!(chain.len(), 2);
    assert_eq!(parents(&chain[0]), ["x"]);
    assert!(parents(&chain[1]).is_empty());
}

#[test]
fn union_retains_every_arm_without_reusing_allocations() {
    let program: Program = syn::parse_quote! {
        physical {
            relation L(a:u32); relation R(a:u32,b:u32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = let probe first [a] l,r;
            out = pipe canon union j,j;
            return out as answer(a,b);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Provider [a];
            j rows = Rows;
            out canon = Canon;
        } sequence {
            result = store capacity 128 union [
                flatten lookup r [a] rows l,
                flatten lookup r [a] rows l
            ];
        } graded target { result at engine; }
    };
    let analysis = check_grading(&program, &machine()).unwrap();
    let shape = shape(&analysis, "result");
    assert_eq!(shape.chains.len(), 2);
    for chain in &shape.chains {
        assert_eq!(chain.len(), 1);
        assert_eq!(parents(&chain[0]), ["a"]);
    }
}

#[test]
fn a_host_consumer_requires_expanded_rows_instead_of_virtual_ranges() {
    let program: Program = syn::parse_quote! {
        physical {
            relation L(a:u32); relation R(a:u32,b:u32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            guarded = pipe guard j if (a % 2 == 0);
            return guarded as answer(a,b);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Provider [a];
            j rows = Rows;
            j canon = Canon;
        } sequence {
            view = flatten lookup r [a] rows l;
            first = store view;
            result = store map guarded first;
        } graded target {
            view at engine.team;
            first at host;
            result at host;
        }
    };
    let analysis = check_grading(&program, &machine()).unwrap();
    assert!(
        !analysis
            .dispatches
            .iter()
            .any(|dispatch| { dispatch.inner_stores.iter().any(|name| name == "result") })
    );
    let chain = &shape(&analysis, "first").chains[0];
    assert_eq!(chain.len(), 1);
    assert_eq!(parents(&chain[0]), ["a"]);
}
