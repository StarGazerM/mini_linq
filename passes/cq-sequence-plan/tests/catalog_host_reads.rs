use cq_lang_sequence::{
    CanonToken, Capability, Catalog, CodecToken, LevelNode, Machine, Program, ProviderToken, Term,
    check_grading, check_rewrite, structures,
};
use cq_sequence_plan::rewrites::{
    adaptive_candidates, apply_adaptive, lookup_first, lookup_first_advice,
};
use cq_sequence_plan::{Advice, decompose, plan, select_from_catalog};
use quote::ToTokens;

fn machine() -> Machine {
    fn node(name: &str, children: Vec<LevelNode>) -> LevelNode {
        LevelNode {
            name: syn::parse_str(name).unwrap(),
            host_rust: name == "host",
            collaborative: name == "block",
            dynamic: name == "device",
            width: (name == "block").then_some(32),
            shared_bytes: None,
            children,
        }
    }
    Machine {
        gpu: None,
        name: syn::parse_quote!(gpu),
        items: 1,
        key_partition_threshold: 65536,
        key_partition_cache_threshold: 6 * 1024 * 1024,
        layout: None,
        root: node("host", vec![node("device", vec![node("block", vec![])])]),
    }
}

fn catalog() -> Catalog {
    let host = vec!["host".into()];
    let device = vec!["host".into(), "device".into()];
    let block = vec!["host".into(), "device".into(), "block".into()];
    let capabilities = vec![Capability::Exact, Capability::Range, Capability::Membership];
    Catalog {
        providers: vec![
            ProviderToken {
                supports: Vec::new(),
                ty: syn::parse_quote!(RangeProvider),
                elements: Some(vec![syn::parse_quote!(i32)]),
                capabilities: capabilities.clone(),
                levels: vec![device.clone(), block.clone()],
            },
            ProviderToken {
                supports: Vec::new(),
                ty: syn::parse_quote!(HostIndex),
                elements: None,
                capabilities,
                levels: vec![host.clone()],
            },
        ],
        canons: vec![CanonToken {
            ty: syn::parse_quote!(Canon),
            levels: vec![host.clone(), device.clone(), block.clone()],
            max_packed_bits: None,
        }],
        codecs: vec![CodecToken {
            ty: syn::parse_quote!(Identity),
            ordered: true,
            levels: vec![host, device, block],
        }],
    }
}

fn string_reader(retained: bool) -> structures::Program {
    let mut source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32,label:String); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a,label -> label};
            r = pipe rename R@1 {a -> a,b -> b};
            answer = pipe canon probe first [a] l,r;
            return answer as answer(a,b,label);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r host result(answer) = Reader [a]; answer canon = Canon;
        }
    };
    if retained {
        source.physical.program.definitions[1].mode =
            structures::physical::Mode::Let(Default::default());
        source.structures.entries[2] = syn::parse_quote!(r = Authored [a];);
    }
    source
}

#[test]
fn numeric_provider_of_string_carrying_reader_selects_host_level() {
    for retained in [false, true] {
        let source = string_reader(retained);
        let selected = select_from_catalog(&source, &catalog()).unwrap();
        let facts = structures::contract::check(&selected).unwrap();
        assert_eq!(facts.layouts["r"], ["a", "b"]);
        assert_eq!(
            facts.built_on("r", &["a".into()]).map(|(ty, _)| ty),
            Some(&syn::parse_quote!(HostIndex)),
        );
        structures::physical::check_rewrite(&source.physical, &selected.physical).unwrap();
        let mut unavailable = catalog();
        unavailable.providers.truncate(1);
        assert!(select_from_catalog(&source, &unavailable).is_err());
    }
}

#[test]
fn automatic_lookup_records_host_reader_token_without_overriding_authored_tokens() {
    let source = cq_sequence_lowering::compile(&string_reader(false)).unwrap();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    assert!(
        advice
            .providers
            .iter()
            .any(|(name, _, ty, _)| name == "r" && *ty == syn::parse_quote!(HostIndex))
    );
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    check_rewrite(&source, &target).unwrap();
    assert_eq!(
        target,
        lookup_first(&source, advice.to_token_stream()).unwrap()
    );

    let source = cq_sequence_lowering::compile(&string_reader(true)).unwrap();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    assert!(!advice.providers.iter().any(|(name, _, _, _)| name == "r"));
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    let facts = structures::contract::check(&target.structures).unwrap();
    assert_eq!(
        facts.built_on("r", &["a".into()]).map(|(ty, _)| ty),
        Some(&syn::parse_quote!(Authored)),
    );
    check_rewrite(&source, &target).unwrap();
}

fn assert_replay(source: &Program, planned: &cq_sequence_plan::Planned) {
    let mut replay = source.clone();
    for advice in &planned.trace {
        replay = advice.apply(&replay).unwrap();
    }
    assert_eq!(planned.program, replay);
    check_rewrite(source, &planned.program).unwrap();
    check_grading(&planned.program, &machine()).unwrap();
}

#[test]
fn numeric_lookup_is_published_before_host_string_expansion() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32,label:String);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            t = let rename T@2 {b -> b,label -> label};
            numeric = pipe probe first [a] r,s;
            text = pipe probe first [b] numeric,t;
            selected = pipe select text if (label = "green");
            answer = pipe canon project selected keep {a,b};
            return answer as answer(a,b);
        }
        structures {
            r host scan = Reader []; s host scan = Reader []; t host scan = Reader [];
            s = RangeProvider [a]; t = HostIndex [b]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let planned = decompose(&source, &machine(), &catalog()).unwrap();
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::Store(name) if name == "numeric"))
    );
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::Publish { name, .. } if name == "numeric"))
    );
    let facts = check_grading(&planned.program, &machine()).unwrap();
    assert_eq!(facts.levels["numeric"], ["host"]);
    assert_eq!(
        facts
            .structures
            .built_on("s", &["a".into()])
            .map(|(ty, _)| ty),
        Some(&syn::parse_quote!(RangeProvider)),
    );
    assert_replay(&source, &planned);
}

#[test]
fn automatic_h09_uses_catalog_read_levels_after_opaque_prefix() {
    let physical = syn::parse_quote! {
        physical {
            relation Part(part_key:i32,part_name:String);
            relation Orders(order_key:i32);
            relation Lineitem(order_key:i32,part_key:i32,supplier_key:i32);
            relation PartSupp(part_key:i32,supplier_key:i32);
            relation Supplier(supplier_key:i32,nation_key:i32);
            relation Nation(nation_key:i32);
            p = pipe rename Part@0 {part_key -> part_key,part_name -> part_name};
            green = pipe guard p if (part_name.contains("green"));
            o = let rename Orders@2 {order_key -> order_key};
            po = pipe probe first [] green,o;
            l = let rename Lineitem@3 {
                order_key -> order_key,part_key -> part_key,supplier_key -> supplier_key
            };
            pol = pipe probe first [order_key,part_key] po,l;
            ps = let rename PartSupp@4 {part_key -> part_key,supplier_key -> supplier_key};
            valid = pipe probe first [part_key,supplier_key] pol,ps;
            s = let rename Supplier@5 {supplier_key -> supplier_key,nation_key -> nation_key};
            supplied = pipe probe first [supplier_key] valid,s;
            n = let rename Nation@6 {nation_key -> nation_key};
            joined = pipe probe first [nation_key] supplied,n;
            answer = pipe canon project joined keep {order_key,part_key,supplier_key};
            return answer as answer(order_key,part_key,supplier_key);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let selected = select_from_catalog(&structures, &catalog()).unwrap();
    let mut source = cq_sequence_lowering::compile(&selected).unwrap();
    source.evidence = syn::parse_quote!(evidence {
        count Part = 20; count Orders = 150; count Lineitem = 600;
        count PartSupp = 80; count Supplier = 10; count Nation = 2;
    });
    let planned = plan(&source, &machine(), &catalog()).unwrap();
    assert_replay(&source, &planned);
}

#[test]
fn automatic_h07_keeps_numeric_forwards_legal_before_string_readers() {
    let physical = syn::parse_quote! {
        physical {
            relation Supplier(supplier_key:i32,supplier_nation:i32);
            relation Lineitem(order_key:i32,supplier_key:i32,ship_date:i32);
            relation Orders(order_key:i32,customer_key:i32);
            relation Customer(customer_key:i32,customer_nation:i32);
            relation Nation(nation_key:i32,name:String);
            s = pipe rename Supplier@2 {
                supplier_key -> supplier_key,supplier_nation -> supplier_nation
            };
            sn = pipe rename Nation@5 {nation_key -> supplier_nation,name -> supplier_name};
            sf = let select sn if ((supplier_name = "FRANCE") or (supplier_name = "GERMANY"));
            ss = let semi eager [supplier_nation] s,sf;
            c = pipe rename Customer@4 {
                customer_key -> customer_key,customer_nation -> customer_nation
            };
            cn = pipe rename Nation@6 {nation_key -> customer_nation,name -> customer_name};
            cf = let select cn if ((customer_name = "GERMANY") or (customer_name = "FRANCE"));
            cs = let semi eager [customer_nation] c,cf;
            o = pipe rename Orders@3 {order_key -> order_key,customer_key -> customer_key};
            os = let semi eager [customer_key] o,cs;
            l = pipe rename Lineitem@0 {
                order_key -> order_key,supplier_key -> supplier_key,ship_date -> ship_date
            };
            dated = pipe range select l if (ship_date between 1i32 and 20i32);
            ls = pipe probe first [supplier_key] dated,ss;
            lo = pipe probe first [order_key] ls,os;
            lc = pipe probe first [customer_key] lo,cs;
            lsn = pipe probe first [supplier_nation] lc,sf;
            lcn = pipe probe first [customer_nation] lsn,cf;
            selected = pipe select lcn if (
                (supplier_name = "FRANCE" and customer_name = "GERMANY")
                or (supplier_name = "GERMANY" and customer_name = "FRANCE")
            );
            answer = pipe canon project selected keep {customer_nation,order_key,supplier_nation};
            return answer as answer(order_key,supplier_nation,customer_nation);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let selected = select_from_catalog(&structures, &catalog()).unwrap();
    let mut source = cq_sequence_lowering::compile(&selected).unwrap();
    source.evidence = syn::parse_quote!(evidence {
        count Supplier = 10; count Lineitem = 600; count Orders = 150;
        count Customer = 15; count Nation = 2;
    });
    let planned = plan(&source, &machine(), &catalog()).unwrap();
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::LookupFirst(_)))
    );
    assert_replay(&source, &planned);
}

#[test]
fn catalog_retention_and_canon_retyping_cross_the_sequence_boundary() {
    let source = string_reader(false);
    let mut selected = select_from_catalog(&source, &catalog()).unwrap();
    for entry in &mut selected.structures.entries {
        if let structures::Structure::Dedup { ty, .. } = &mut entry.structure {
            *ty = syn::parse_quote!(NativeCanon);
        }
    }
    // The ordinary Structures-only contract remains exact.
    assert!(structures::check_rewrite(&source, &selected).is_err());
    structures::check_retention_rewrite(&source, &selected).unwrap();
    let sequence = cq_sequence_lowering::compile(&source).unwrap();
    let mut target = sequence.clone();
    target.structures = selected.clone();
    check_rewrite(&sequence, &target).unwrap();

    let mut changed_reader = selected.clone();
    if let structures::Structure::Host { keyed, .. } =
        &mut changed_reader.structures.entries[0].structure
    {
        keyed.ty = syn::parse_quote!(DifferentReader);
    }
    assert!(structures::check_retention_rewrite(&source, &changed_reader).is_err());
    let mut changed_order = selected.clone();
    changed_order.physical.program.definitions.swap(0, 1);
    assert!(structures::check_retention_rewrite(&source, &changed_order).is_err());
    let mut changed_mode = selected;
    changed_mode.physical.program.definitions[2].mode =
        structures::physical::Mode::Let(Default::default());
    assert!(structures::check_retention_rewrite(&source, &changed_mode).is_err());
}

#[test]
fn catalog_retyping_composes_with_checked_join_reexpression() {
    let mut original = string_reader(false);
    for entry in &mut original.structures.entries {
        if let structures::Structure::Dedup { ty, .. } = &mut entry.structure {
            *ty = syn::parse_quote!(AuthoredCanon);
        }
    }
    let source = cq_sequence_lowering::compile(&original).unwrap();
    let selected = select_from_catalog(&original, &catalog()).unwrap();
    let intermediate = cq_sequence_lowering::compile(&selected).unwrap();
    check_rewrite(&source, &intermediate).unwrap();
    let target = lookup_first(&intermediate, quote::quote!(using HostIndex, order [r,l])).unwrap();
    check_rewrite(&intermediate, &target).unwrap();
    assert_ne!(source.structures.physical, target.structures.physical);
    // This is a composition, not merely the retention-only special case.
    assert!(structures::check_retention_rewrite(&source.structures, &target.structures).is_err());
    check_rewrite(&source, &target).unwrap();

    let mut changed_reader = target;
    let scan = changed_reader.structures.structures.entries.iter_mut().find(|entry|
        matches!(&entry.structure, structures::Structure::Host { reader, .. } if matches!(reader, structures::ReaderRef::Scan))
    ).unwrap();
    if let structures::Structure::Host { keyed, .. } = &mut scan.structure {
        keyed.ty = syn::parse_quote!(DifferentReader);
    }
    assert!(check_rewrite(&source, &changed_reader).is_err());
}

fn full_key_host_membership() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r = pipe rename R@1 {a -> a};
            answer = pipe canon probe first [a] l,r;
            return answer as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r host result(answer) = HostMembership [a] membership;
            answer canon = Canon;
        }
    }
}

fn membership_catalog(ty: syn::Type) -> Catalog {
    let mut available = catalog();
    available.providers = vec![ProviderToken {
        ty,
        supports: Vec::new(),
        elements: None,
        capabilities: vec![Capability::Membership],
        levels: vec![vec!["host".into()]],
    }];
    available
}

#[test]
fn catalog_preserves_admitted_raw_host_membership_without_enumeration() {
    let source = full_key_host_membership();
    let available = membership_catalog(syn::parse_quote!(HostMembership));
    let selected = select_from_catalog(&source, &available).unwrap();
    assert_eq!(selected, source);
    let lowered = cq_sequence_lowering::compile(&selected).unwrap();
    cq_sequence_lowering::check(&selected, &lowered).unwrap();
    assert_eq!(
        lowered.sequence.definitions[2].term,
        syn::parse_quote!(canon compact answer lookup r [a] l)
    );
}

#[test]
fn catalog_retention_preserves_raw_membership_certificate_and_compact() {
    let source = full_key_host_membership();
    let available = membership_catalog(syn::parse_quote!(MembershipIndex));
    let selected = select_from_catalog(&source, &available).unwrap();
    let facts = structures::contract::check(&selected).unwrap();
    assert!(facts.membership.contains(&("r".into(), vec!["a".into()])));
    assert_eq!(
        facts.built_on("r", &["a".into()]).map(|(ty, _)| ty),
        Some(&syn::parse_quote!(MembershipIndex))
    );
    assert!(selected.structures.entries.iter().any(|entry| {
        entry.name == "r"
            && matches!(&entry.structure, structures::Structure::Built { keyed, .. }
                if keyed.membership.is_some() && keyed.filtered.is_none())
    }));
    structures::check_retention_rewrite(&source, &selected).unwrap();
    let before = cq_sequence_lowering::compile(&source).unwrap();
    let after = cq_sequence_lowering::compile(&selected).unwrap();
    cq_sequence_lowering::check(&selected, &after).unwrap();
    check_rewrite(&before, &after).unwrap();
    assert_eq!(
        before.sequence.definitions[2].term,
        after.sequence.definitions[2].term
    );
    assert_eq!(
        cq_lang_sequence::contract::check(&after)
            .unwrap()
            .capabilities[&("r".into(), vec!["a".into()])],
        Capability::Membership
    );

    let mut unavailable = available;
    unavailable.providers[0].capabilities = vec![Capability::Exact, Capability::Range];
    assert!(select_from_catalog(&source, &unavailable).is_err());
}

#[test]
fn adaptive_candidates_clone_raw_host_membership_read_certificates() {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32); relation T(b:i32,c:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r = pipe rename R@1 {a -> a};
            t = pipe rename T@2 {b -> b,c -> c};
            tf = let select t if (c > 0);
            member = pipe probe first [a] l,r;
            optional = pipe semi eager [b] member,tf;
            joined = pipe probe first [b] optional,tf;
            answer = pipe canon project joined keep {a,b,c};
            return answer as answer(a,b,c);
        }
        structures {
            l host scan = Reader []; r host scan = Reader []; t host scan = Reader [];
            r host result(member) = HostMembership [a] membership;
            tf = Index [b];
            answer canon = Canon;
        }
    };
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source
        .sequence
        .optional_filters
        .push(syn::parse_quote!(optional));
    let candidates = adaptive_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].filter, "optional");
    for select in [None, Some(false), Some(true)] {
        let mut candidate = candidates[0].clone();
        candidate.select = select;
        let target = apply_adaptive(&source, &candidate).unwrap();
        check_rewrite(&source, &target).unwrap();
        assert!(adaptive_candidates(&target).unwrap().is_empty());
        let facts = cq_lang_sequence::contract::check(&target).unwrap();
        assert!(
            facts
                .structures
                .membership
                .contains(&("r".into(), vec!["a".into()]))
        );
        let reads = &facts.structures.physical.keyed_reads["r"];
        assert_eq!(
            reads.len(),
            2,
            "both arms retain their mandatory membership read"
        );
        for (reader, keys, _) in reads {
            assert!(target.structures.structures.entries.iter().any(|entry| {
                entry.name == "r"
                    && matches!(&entry.structure,
                        structures::Structure::Host { reader: adapter, keyed, .. }
                        if adapter.result().is_some_and(|name| name.to_string() == *reader)
                            && keyed.membership.is_some()
                            && keyed.ty == syn::parse_quote!(HostMembership)
                            && keyed.level_names().concat() == *keys)
            }));
            assert!(target.sequence.definitions.iter().any(|definition| {
                matches!(&definition.term, Term::Compact(result, inner)
                    if result.to_string() == *reader
                        && matches!(inner.as_ref(), Term::Lookup { provider, .. } if provider == "r"))
            }));
        }

        let mut missing_adapter = target.clone();
        missing_adapter
            .structures
            .structures
            .entries
            .retain(|entry| {
                !matches!(&entry.structure, structures::Structure::Host { reader, .. }
                if entry.name == "r"
                    && reader.result().is_some_and(|name| name != "member"))
            });
        assert!(cq_lang_sequence::contract::check(&missing_adapter).is_err());
        assert_eq!(target, apply_adaptive(&source, &candidate).unwrap());
    }
}

#[test]
fn lookup_first_rebuilds_raw_membership_when_prefix_reads_expand_its_domain() {
    let physical = syn::parse_quote! {
        physical {
            relation L(x:i32,y:i32); relation R(y:i32,z:i32); relation S(x:i32,z:i32);
            l = pipe rename L@0 {x -> x,y -> y};
            r = let rename R@1 {y -> y,z -> z};
            s = let rename S@2 {x -> x,z -> z};
            joined = pipe probe first [y] l,r;
            answer = pipe canon probe first [x,z] joined,s;
            return answer as answer(x,y,z);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let before = structures::contract::check(&source.structures).unwrap();
    assert!(
        before
            .membership
            .contains(&("s".into(), vec!["x".into(), "z".into()]))
    );
    let advice = quote::quote!(using HostIndex, order [l,r,s], transfer [], early []);
    let target = lookup_first(&source, advice.clone()).unwrap();
    check_rewrite(&source, &target).unwrap();
    let after = structures::contract::check(&target.structures).unwrap();
    assert!(after.membership.iter().all(|(name, _)| name != "s"));
    assert!(target.structures.structures.entries.iter().any(|entry| {
        matches!(&entry.structure, structures::Structure::Built { keyed, .. }
            if keyed.level_names() == vec![vec!["x".to_owned()]]
                && keyed.membership.is_none() && keyed.ty == syn::parse_quote!(HostIndex))
    }));
    assert_eq!(target, lookup_first(&source, advice).unwrap());
}

#[test]
fn admitted_storage_type_does_not_move_unretained_host_membership_to_workers() {
    let structures = full_key_host_membership();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let mut available = membership_catalog(syn::parse_quote!(HostMembership));
    available.providers[0].levels = vec![
        vec!["host".into()],
        vec!["host".into(), "device".into()],
        vec!["host".into(), "device".into(), "block".into()],
    ];
    let planned = decompose(&source, &machine(), &available).unwrap();
    let facts = check_grading(&planned.program, &machine()).unwrap();
    assert_eq!(facts.levels["answer"], ["host"]);
    assert!(facts.structures.host_read("r"));
    check_rewrite(&source, &planned.program).unwrap();

    let authored =
        cq_sequence_plan::rewrites::grade(&source, quote::quote!(gpu, answer at device)).unwrap();
    assert!(decompose(&authored, &machine(), &available).is_err());
}
