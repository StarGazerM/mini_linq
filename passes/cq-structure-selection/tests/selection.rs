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

fn reject(
    source: &physical::Program,
    target: &structures::Program,
    policy: &physical::policy::Policy,
    names: &[&str],
) {
    let error = check(source, target, policy).unwrap_err().to_string();
    for name in names {
        assert!(error.contains(name), "{error}");
    }
}

#[test]
fn independent_selection_checks_contracts_preservation_and_exact_host_facts() {
    let source = chain("");
    let policy: physical::policy::Policy =
        syn::parse_quote!(policy { L canon; R keyed [a] ordered canon; S; });
    let target = compile(&source, &policy).unwrap();
    let mut bad_source = source.clone();
    bad_source.program.definitions[3] = syn::parse_quote!(j0 = pipe probe[a] missing,r;);
    reject(&bad_source, &target, &policy, &["j0", "missing"]);
    let mut missing = target.clone();
    missing.structures.entries.pop();
    reject(&source, &missing, &policy, &["j1", "s", "no host key"]);
    let mut changed = target.clone();
    changed.physical.program.definitions[1].mode = syn::parse_quote!(let);
    structures::contract::check(&changed).unwrap();
    reject(&source, &changed, &policy, &["r", "preserve"]);
    for (entry, replacement, name) in [
        (0, "L;", "L"),
        (1, "R keyed [a] canon;", "R"),
        (0, "L keyed [a] canon;", "L"),
    ] {
        let mut changed = target.clone();
        changed.structures.entries[entry] = syn::parse_str(replacement).unwrap();
        structures::contract::check(&changed).unwrap();
        reject(&source, &changed, &policy, &[name, "host facts"]);
    }
    for (text, names) in [
        (
            "policy { L canon; R keyed [a] ordered canon; }",
            &["S", "missing"][..],
        ),
        (
            "policy { L canon; R keyed [a] ordered canon; S; Extra; }",
            &["Extra", "extra"],
        ),
        (
            "policy { L canon; R keyed [a] ordered canon; S; L; }",
            &["L", "duplicate"],
        ),
    ] {
        reject(&source, &target, &syn::parse_str(text).unwrap(), names);
    }
    let mut source = source;
    source.program.definitions[1].mode = syn::parse_quote!(let);
    let mut redundant = compile(&source, &policy).unwrap();
    redundant
        .structures
        .entries
        .push(syn::parse_quote!(r = ::cq_storage::HashIndex<_,_> [a];));
    structures::contract::check(&redundant).unwrap();
    reject(
        &source,
        &redundant,
        &policy,
        &["r", "host key already answers"],
    );
}

#[test]
fn independent_selection_checks_advice_and_each_read_default() {
    let policy = syn::parse_quote!(policy { L; R; S; });
    let mut source = chain("#[index(MyIndex)]");
    source.program.definitions[1].mode = syn::parse_quote!(let);
    let target = compile(&source, &policy).unwrap();
    for (name, ty, diagnostic) in [
        ("r", "::cq_storage::HashIndex<_,_>", "#[index(T)]"),
        ("l", "::cq_storage::HashIndex<_,_>", "HashSet"),
        ("s", "::std::collections::HashSet<_>", "HashIndex"),
    ] {
        let mut changed = target.clone();
        let entry = changed
            .structures
            .entries
            .iter_mut()
            .find(|e| e.name == name)
            .unwrap();
        let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
            unreachable!()
        };
        keyed.ty = syn::parse_str(ty).unwrap();
        structures::contract::check(&changed).unwrap();
        reject(&source, &changed, &policy, &[name, diagnostic]);
    }
    let ordered: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); relation S(a:i32);
        r = let rename R {a -> a}; s = let rename S {a -> a};
        j = let merge[a] r,s; w = pipe canon range[a] j if (a > 0);
        return w as answer(a);
    }};
    let policy = syn::parse_quote!(policy { R; S; });
    let target = compile(&ordered, &policy).unwrap();
    for name in ["r", "s", "j"] {
        let mut changed = target.clone();
        let entry = changed
            .structures
            .entries
            .iter_mut()
            .find(|e| e.name == name)
            .unwrap();
        let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
            unreachable!()
        };
        keyed.ty = syn::parse_quote!(::cq_storage::HashIndex<_,_>);
        structures::contract::check(&changed).unwrap();
        reject(&ordered, &changed, &policy, &[name, "BTreeMap"]);
    }
}

#[test]
fn independent_selection_checks_chain_levels_and_smallest_membership() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "fine_triangle")
        .unwrap();
    let source = syn::parse_str(case.physical).unwrap();
    let policy = syn::parse_str(case.policy).unwrap();
    let target = compile(&source, &policy).unwrap();
    let position = target
        .structures
        .entries
        .iter()
        .position(|e| e.name == "r2")
        .unwrap();
    let mut wrong_type = target.clone();
    wrong_type.structures.entries[position] =
        syn::parse_quote!(r2 = ::cq_storage::Trie<'_,_,::std::collections::HashSet<_>> [a][c];);
    reject(&source, &wrong_type, &policy, &["r2", "HashIndex"]);
    let mut separate = target.clone();
    separate.structures.entries[position] =
        syn::parse_quote!(r2 = ::std::collections::HashSet<_> [a];);
    separate
        .structures
        .entries
        .push(syn::parse_quote!(r2 = ::cq_storage::HashIndex<_,_> [a,c];));
    structures::contract::check(&separate).unwrap();
    reject(&source, &separate, &policy, &["r2", "Trie levels"]);
    let smallest = syn::parse_quote!(policy { R; S; T; intersect = smallest; });
    // All physical reads hold, but smallest's extra membership key is absent.
    reject(&source, &target, &smallest, &["r5", "Trie levels"]);
    let mut wrong_leaf = compile(&source, &smallest).unwrap();
    let position = wrong_leaf
        .structures
        .entries
        .iter()
        .position(|e| e.name == "r5")
        .unwrap();
    wrong_leaf.structures.entries[position] =
        syn::parse_quote!(r5 = ::cq_storage::Trie<'_,_,::cq_storage::HashIndex<_,_>> [b][c];);
    reject(&source, &wrong_leaf, &smallest, &["r5", "HashSet"]);
}

#[test]
fn hand_written_entry_order_is_free() {
    let source = chain("");
    let policy = syn::parse_quote!(policy { L canon; R keyed [a]; S; });
    let alternative: structures::Program = syn::parse_quote! {
        #source structures {
            s = ::cq_storage::HashIndex<_,_> [b];
            S; R keyed [a];
            l = ::std::collections::HashSet<_> [a];
            L canon;
        }
    };
    check(&source, &alternative, &policy).unwrap();
    assert_ne!(compile(&source, &policy).unwrap(), alternative);
}

#[test]
fn host_key_tuple_order_is_a_preserved_policy_fact() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = alias canon rename R {a -> a,b -> b}; return r as answer(a,b);
    }};
    let policy = syn::parse_quote!(policy { R keyed [b,a]; });
    let mut target = compile(&source, &policy).unwrap();
    check(&source, &target, &policy).unwrap();
    target.structures.entries[0] = syn::parse_quote!(R keyed [a,b];);
    structures::contract::check(&target).unwrap();
    reject(&source, &target, &policy, &["R", "host facts"]);
}
