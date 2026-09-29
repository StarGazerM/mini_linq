use cq_lang_physical as physical;
use cq_lang_structures as structures;
use cq_structure_selection::{check, compile};
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[test]
fn scenarios_discharge_every_physical_read_and_reject_changed_targets() {
    for case in scenarios::SCENARIOS {
        let source = syn::parse_str(case.physical).unwrap();
        let policy = syn::parse_str(case.policy).unwrap();
        let target = compile(&source, &policy).unwrap();
        check(&source, &target, &policy).unwrap();
        assert_eq!(
            target.structures,
            syn::parse_str::<structures::Structures>(case.structures).unwrap(),
            "{}",
            case.name
        );
        let mut changed = target;
        changed.structures.entries.pop();
        assert!(check(&source, &changed, &policy).is_err());
    }
}
fn chain(advice: &str) -> physical::Program {
    syn::parse_str(&format!("physical {{ relation L(a:i32); {advice} relation R(b:i32,a:i32); relation S(b:i32,c:i32);
        l = let rename L {{a -> a}}; r = alias rename R {{b -> b,a -> a}}; s = let rename S {{b -> b,c -> c}};
        j0 = pipe probe[a] l,r; j1 = pipe probe[b] j0,s; x = pipe semi[a] j1,l;
        p = pipe canon project x keep {{a,c}}; return p as answer(a,c); }}")).unwrap()
}

fn assert_selection(source: &physical::Program, policy: &str, expected: &str) {
    let policy = syn::parse_str(policy).unwrap();
    let target = compile(source, &policy).unwrap();
    check(source, &target, &policy).unwrap();
    assert_eq!(
        target.structures,
        syn::parse_str::<structures::Structures>(expected).unwrap()
    );
}

#[test]
fn host_facts_and_keys_take_precedence_over_index_advice() {
    let source = chain("#[index(MyIndex)]");
    assert_selection(
        &source,
        "policy { L canon; R keyed [a] ordered canon; S; }",
        "structures {
        L canon; R keyed [a] ordered canon; S;
        l = ::std::collections::HashSet<_> [a];
        s = ::cq_storage::HashIndex<_, _> [b];
    }",
    );
    let mut built = source.clone();
    built.program.definitions[1].mode = syn::parse_quote!(let);
    assert_selection(
        &built,
        "policy { L; R; S; }",
        "structures {
        L; R; S;
        l = ::std::collections::HashSet<_> [a];
        r = MyIndex [a];
        s = ::cq_storage::HashIndex<_, _> [b];
    }",
    );
    let error = compile(&source, &syn::parse_quote!(policy { L; R; S; }))
        .unwrap_err()
        .to_string();
    assert!(error.contains("`r`"), "{error}");
}

#[test]
fn wrong_column_host_keys_report_the_alias_and_reader() {
    let error = compile(
        &chain(""),
        &syn::parse_quote!(policy { L; R keyed [b]; S; }),
    )
    .unwrap_err()
    .to_string();
    for construct in ["`j0`", "`r`", "\"a\""] {
        assert!(error.contains(construct), "{error}");
    }
}

#[test]
fn merge_storage_also_answers_membership_and_sorted_pipes_need_no_build() {
    let mut source = chain("");
    source.program.definitions[1].mode = syn::parse_quote!(let);
    source.program.definitions[3] = syn::parse_quote!(j0 = let merge[a] l, r;);
    source.program.definitions[4] = syn::parse_quote!(j1 = pipe merge[b] j0, s;);
    assert_selection(
        &source,
        "policy { L; R; S; join = merge; }",
        "structures {
        L; R; S;
        l = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
        r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
        s = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b];
        j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b];
    }",
    );
    let star: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32, b: i32); relation T(a: i32, c: i32);
        l = alias rename L {a -> a};
        r = alias rename R {a -> a, b -> b};
        t = let rename T {a -> a, c -> c};
        j0 = pipe merge[a] l, r;
        j1 = pipe canon merge[a] j0, t; return j1 as answer(a, b, c);
    }};
    assert_selection(
        &star,
        "policy { L keyed [a] ordered; R keyed [a] ordered; T; join = merge; }",
        "structures {
        L keyed [a] ordered; R keyed [a] ordered; T;
        t = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
    }",
    );
}

#[test]
fn range_builds_only_when_the_host_key_is_not_ordered() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: i32, b: i32);
        r = let rename R {a -> a, b -> b};
        f = pipe range[a] r if (a > 1); return f as answer(a, b);
    }};
    assert_selection(
        &source,
        "policy { R keyed [a] canon; }",
        "structures {
        R keyed [a] canon;
        r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
    }",
    );
    assert_selection(
        &source,
        "policy { R keyed [a] ordered canon; }",
        "structures {
        R keyed [a] ordered canon;
    }",
    );
}

#[test]
fn invalid_policy_and_noncanonical_return_name_the_construct() {
    for (policy, names) in [
        ("policy { L; R keyed [z]; S; }", &["R", "z"][..]),
        ("policy { L; R keyed [a]; }", &["S"][..]),
    ] {
        let error = compile(&chain(""), &syn::parse_str(policy).unwrap())
            .unwrap_err()
            .to_string();
        for name in names {
            assert!(error.contains(&format!("`{name}`")), "{error}");
        }
    }
    let mut source = chain("");
    source.program.definitions[6] = syn::parse_quote!(p = pipe project x keep {a, c};);
    physical::contract::check(&source).unwrap();
    let error = compile(&source, &syn::parse_quote!(policy { L; R keyed [a]; S; }))
        .unwrap_err()
        .to_string();
    assert!(error.contains("`p`"), "{error}");
}

#[test]
fn fine_triangle_pins_trie_levels_and_smallest_candidate_storage() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "fine_triangle")
        .unwrap();
    let source = syn::parse_str(case.physical).unwrap();
    assert_selection(
        &source,
        case.policy,
        "structures {
        R; S; T;
        r0 = ::cq_storage::HashIndex<_, _> [a];
        r2 = ::cq_storage::Trie<'_, _, ::cq_storage::HashIndex<_, _>> [a][c];
        r5 = ::cq_storage::HashIndex<_, _> [b];
    }",
    );
    assert_selection(
        &source,
        "policy { R; S; T; intersect = smallest; }",
        "structures {
        R; S; T;
        r0 = ::cq_storage::HashIndex<_, _> [a];
        r2 = ::cq_storage::Trie<'_, _, ::cq_storage::HashIndex<_, _>> [a][c];
        r5 = ::cq_storage::Trie<'_, _, ::std::collections::HashSet<_>> [b][c];
    }",
    );
}

#[test]
fn merge_and_range_choose_ordered_storage() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: i32); relation S(a: i32);
        r = let rename R {a -> x}; s = let rename S {a -> x};
        j = let merge[x] r, s; w = pipe canon range[x] j if (x > 1);
        return w as answer(x);
    }};
    assert_selection(
        &source,
        "policy { R; S; }",
        "structures {
        R; S;
        r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [x];
        s = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [x];
        j = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [x];
    }",
    );
}
