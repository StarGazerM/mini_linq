use cq_lang_sequence::{
    Catalog, CodecToken, LevelNode, Machine, Program, check_grading, check_rewrite, structures,
};
use cq_sequence_plan::{Advice, decompose, select_from_catalog};
use structures::Capability;

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
    let declared: structures::Providers = syn::parse_quote!(providers {
        range_provider = RangeProvider { reads [exact, range, membership], build inferred };
        host_index = HostIndex { reads [exact, range, membership], build inferred };
        sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
    });
    let mut declarations = declared.declarations.into_iter();
    let mut catalog = Catalog {
        codecs: vec![CodecToken {
            codec: None,
            codec_id: None,
            ordered: true,
            levels: vec![host.clone(), device.clone(), block.clone()],
        }],
        ..Catalog::default()
    };
    catalog.register(
        declarations.next().unwrap(),
        vec![device.clone(), block.clone()],
    );
    catalog.register(declarations.next().unwrap(), vec![host.clone()]);
    catalog.register(declarations.next().unwrap(), vec![host, device, block]);
    catalog
}

fn code_reader(retained: bool) -> structures::Program {
    let mut source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32,label:StringCode); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a,label -> label};
            r = pipe rename R@1 {a -> a,b -> b};
            answer = let canon probe first [a] l,r;
            return answer as answer(a,b,label);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r host result(answer) = @reader as Reader [a];
            answer rows = @sort as ::cq_storage::dedup::Sort; answer canon = rows;
        }
    };
    if retained {
        source.physical.program.definitions[1].mode =
            structures::physical::Mode::Let(Default::default());
        source.structures.entries[2] = syn::parse_quote!(r = @authored as Authored [a];);
        source
            .structures
            .declare(&[syn::parse_quote!(
                authored = Authored { reads [exact, range, membership], build inferred };
            )])
            .unwrap();
    }
    source
}

/// Row element types are not catalog facts: a code-carrying reader selects
/// the first token declaring its reads, and no token declaring them rejects.
#[test]
fn code_carrying_reader_selects_the_first_token_declaring_its_reads() {
    for retained in [false, true] {
        let source = code_reader(retained);
        let selected = select_from_catalog(&source.physical, &source, &catalog()).map(|(_, structures)| structures).unwrap();
        let facts = structures::contract::check(&selected).unwrap();
        assert_eq!(facts.layouts["r"], ["a", "b"]);
        assert_eq!(
            facts
                .built_on("r", &["a".into()])
                .map(|(provider, _)| provider),
            Some(&syn::parse_quote!(@range_provider as RangeProvider)),
        );
        structures::physical::check_rewrite(&source.physical, &selected.physical).unwrap();
        let mut unavailable = catalog();
        unavailable
            .providers
            .retain(|entry| entry.provider.token == "sort");
        assert!(select_from_catalog(&source.physical, &source, &unavailable).is_err());
    }
}

fn assert_replay(structures: &structures::Program, source: &Program, planned: &cq_sequence_plan::Planned) {
    let mut replay = (structures.physical.clone(), structures.clone(), source.clone());
    for advice in &planned.trace {
        replay = advice.apply(&replay.0, &replay.1, &replay.2).unwrap();
    }
    assert_eq!((&planned.physical, &planned.structures, &planned.program), (&replay.0, &replay.1, &replay.2));
    check_rewrite(&structures.physical, structures, source, &planned.physical, &planned.structures, &planned.program).unwrap();
    check_grading(&planned.physical, &planned.structures, &planned.program, &machine()).unwrap();
}

#[test]
fn numeric_lookup_is_published_before_host_code_expansion() {
    let structures: structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32,label:StringCode);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            t = let rename T@2 {b -> b,label -> label};
            numeric = pipe probe first [a] r,s;
            text = pipe probe first [b] numeric,t;
            selected = pipe select text if (label = "green");
            answer = let canon project selected keep {a,b};
            return answer as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; range_provider = RangeProvider { reads [exact, range, membership], build inferred }; host_index = HostIndex { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            r host scan = @reader as Reader []; s host scan = @reader as Reader []; t host scan = @reader as Reader [];
            s = @range_provider as RangeProvider [a]; t = @host_index as HostIndex [b];
            answer rows = @sort as ::cq_storage::dedup::Sort; answer canon = rows;
        }
    };
    let source = cq_sequence_lowering::compile(&structures.physical, &structures).unwrap();
    let planned = decompose(&structures.physical, &structures, &source, &machine(), &catalog()).unwrap();
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
    let facts = check_grading(&planned.physical, &planned.structures, &planned.program, &machine()).unwrap();
    assert_eq!(facts.levels["numeric"], ["host"]);
    assert_eq!(
        structures::contract::check(&planned.structures).unwrap()
            .built_on("s", &["a".into()])
            .map(|(provider, _)| provider),
        Some(&syn::parse_quote!(@range_provider as RangeProvider)),
    );
    assert_replay(&structures, &source, &planned);
}

#[test]
fn automatic_h09_uses_catalog_read_levels_after_opaque_prefix() {
    let physical = syn::parse_quote! {
        physical {
            relation Part(part_key:i32,part_name:StringCode);
            relation Orders(order_key:i32);
            relation Lineitem(order_key:i32,part_key:i32,supplier_key:i32);
            relation PartSupp(part_key:i32,supplier_key:i32);
            relation Supplier(supplier_key:i32,nation_key:i32);
            relation Nation(nation_key:i32);
            p = pipe rename Part@0 {part_key -> part_key,part_name -> part_name};
            green = pipe guard p if (accept(part_name));
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
            answer = let canon project joined keep {order_key,part_key,supplier_key};
            return answer as answer(order_key,part_key,supplier_key);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let selected = select_from_catalog(&structures.physical, &structures, &catalog()).map(|(_, structures)| structures).unwrap();
    let mut source = cq_sequence_lowering::compile(&selected.physical, &selected).unwrap();
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/catalog_host_reads.rs fixture") {
        count Part = 20; count Orders = 150; count Lineitem = 600;
        count PartSupp = 80; count Supplier = 10; count Nation = 2;
    });
    let planned = decompose(&selected.physical, &selected, &source, &machine(), &catalog()).unwrap();
    assert_replay(&selected, &source, &planned);
}

#[test]
fn automatic_h07_keeps_numeric_forwards_legal_before_code_readers() {
    let physical = syn::parse_quote! {
        physical {
            relation Supplier(supplier_key:i32,supplier_nation:i32);
            relation Lineitem(order_key:i32,supplier_key:i32,ship_date:i32);
            relation Orders(order_key:i32,customer_key:i32);
            relation Customer(customer_key:i32,customer_nation:i32);
            relation Nation(nation_key:i32,name:StringCode);
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
            answer = let canon project selected keep {customer_nation,order_key,supplier_nation};
            return answer as answer(order_key,supplier_nation,customer_nation);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let selected = select_from_catalog(&structures.physical, &structures, &catalog()).map(|(_, structures)| structures).unwrap();
    let mut source = cq_sequence_lowering::compile(&selected.physical, &selected).unwrap();
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/catalog_host_reads.rs fixture") {
        count Supplier = 10; count Lineitem = 600; count Orders = 150;
        count Customer = 15; count Nation = 2;
    });
    let planned = decompose(&selected.physical, &selected, &source, &machine(), &catalog()).unwrap();
    assert_eq!(planned.physical, selected.physical);
    assert_replay(&selected, &source, &planned);
}

#[test]
fn catalog_retention_and_canon_selection_cross_the_sequence_boundary() {
    let source = code_reader(false);
    let mut selected = select_from_catalog(&source.physical, &source, &catalog()).map(|(_, structures)| structures).unwrap();
    for entry in &mut selected.structures.entries {
        if entry.name == "answer"
            && let structures::Structure::Rows { provider, .. } = &mut entry.structure
        {
            *provider = syn::parse_quote!(@native_canon as NativeCanon);
        }
    }
    selected
        .structures
        .declare(&[syn::parse_quote!(
            native_canon = NativeCanon { reads [distinct], build inferred };
        )])
        .unwrap();
    // The ordinary Structures-only contract remains exact.
    assert!(structures::check_rewrite(&source, &selected).is_err());
    structures::check_retention_rewrite(&source, &selected).unwrap();
    let sequence = cq_sequence_lowering::compile(&source.physical, &source).unwrap();
    check_rewrite(&source.physical, &source, &sequence, &selected.physical, &selected, &sequence).unwrap();

    let mut changed_reader = selected.clone();
    if let structures::Structure::Host { keyed, .. } =
        &mut changed_reader.structures.entries[0].structure
    {
        keyed.provider = syn::parse_quote!(@different_reader as DifferentReader);
    }
    changed_reader
        .structures
        .declare(&[syn::parse_quote!(
            different_reader = DifferentReader { reads [exact, range, membership], build inferred };
        )])
        .unwrap();
    assert!(structures::check_retention_rewrite(&source, &changed_reader).is_err());
    let mut changed_order = selected.clone();
    changed_order.physical.program.definitions.swap(0, 1);
    assert!(structures::check_retention_rewrite(&source, &changed_order).is_err());
    let mut changed_mode = selected;
    changed_mode.physical.program.definitions[2].mode =
        structures::physical::Mode::Pipe(Default::default());
    assert!(structures::check_retention_rewrite(&source, &changed_mode).is_err());
}

fn full_key_host_membership() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r = pipe rename R@1 {a -> a};
            answer = let canon probe first [a] l,r;
            return answer as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; host_membership = HostMembership { reads [membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r host result(answer) = @host_membership as HostMembership [a] membership;
            answer rows = @sort as ::cq_storage::dedup::Sort;
            answer canon = rows;
        }
    }
}

fn membership_catalog(declaration: structures::Declaration) -> Catalog {
    let mut available = catalog();
    // Only the canonical rows provider survives beside the membership token.
    available
        .providers
        .retain(|entry| entry.provider.token == "sort");
    available
        .declarations
        .retain(|declaration| declaration.token == "sort");
    available.register(declaration, vec![vec!["host".into()]]);
    available
}

#[test]
fn catalog_preserves_admitted_raw_host_membership_without_enumeration() {
    let source = full_key_host_membership();
    let available = membership_catalog(syn::parse_quote!(
        host_membership = HostMembership { reads [membership], build inferred };
    ));
    let selected = select_from_catalog(&source.physical, &source, &available).map(|(_, structures)| structures).unwrap();
    assert_eq!(selected, source);
    let lowered = cq_sequence_lowering::compile(&selected.physical, &selected).unwrap();
    cq_sequence_lowering::check(&selected.physical, &selected, &lowered).unwrap();
    assert_eq!(
        lowered.sequence.definitions[2].term,
        syn::parse_quote!(canon compact answer lookup r [a] l)
    );
}

#[test]
fn catalog_retention_preserves_raw_membership_certificate_and_compact() {
    let source = full_key_host_membership();
    let available = membership_catalog(syn::parse_quote!(
        membership_index = MembershipIndex { reads [membership], build inferred };
    ));
    let selected = select_from_catalog(&source.physical, &source, &available).map(|(_, structures)| structures).unwrap();
    let facts = structures::contract::check(&selected).unwrap();
    assert!(facts.membership.contains(&("r".into(), vec!["a".into()])));
    assert_eq!(
        facts
            .built_on("r", &["a".into()])
            .map(|(provider, _)| provider),
        Some(&syn::parse_quote!(@membership_index as MembershipIndex))
    );
    assert!(selected.structures.entries.iter().any(|entry| {
        entry.name == "r"
            && matches!(&entry.structure, structures::Structure::Built { keyed, .. }
                if keyed.membership.is_some() && keyed.filtered.is_none())
    }));
    structures::check_retention_rewrite(&source, &selected).unwrap();
    let before = cq_sequence_lowering::compile(&source.physical, &source).unwrap();
    let after = cq_sequence_lowering::compile(&selected.physical, &selected).unwrap();
    cq_sequence_lowering::check(&selected.physical, &selected, &after).unwrap();
    check_rewrite(&source.physical, &source, &before, &selected.physical, &selected, &after).unwrap();
    assert_eq!(
        before.sequence.definitions[2].term,
        after.sequence.definitions[2].term
    );
    assert_eq!(
        cq_lang_sequence::contract::check(&selected.physical, &selected, &after)
            .unwrap()
            .capabilities[&("r".into(), vec!["a".into()])],
        Capability::Membership
    );

    let mut unavailable = available;
    let index = unavailable
        .declarations
        .iter_mut()
        .find(|declaration| declaration.token == "membership_index")
        .unwrap();
    index.facts.reads = vec![Capability::Exact, Capability::Range];
    assert!(select_from_catalog(&source.physical, &source, &unavailable).is_err());
}

#[test]
fn admitted_storage_type_does_not_move_unretained_host_membership_to_workers() {
    let structures = full_key_host_membership();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let mut available = membership_catalog(syn::parse_quote!(
        host_membership = HostMembership { reads [membership], build inferred };
    ));
    available.providers.last_mut().unwrap().levels = vec![
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

// PL-Resident: placement predicts a read level with the catalog's own
// admission. A resident native handle's anti-join membership feeding opaque
// host Rust is read below host and published, not left at an unadmitted host.
#[test]
fn resident_host_membership_before_opaque_host_rust_is_read_below_host() {
    let structures: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32);
            relation answer(c:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a,b -> b};
            r = pipe rename R@1 {a -> a};
            kept = pipe anti eager [a] l,r;
            bound = pipe bind kept with c : i32 = (*b);
            projected = pipe project bound keep {c};
            return projected as answer(c);
        }
        structures { providers { native = Native { reads [exact, range, membership], build inferred }; }
            l host scan = @native as Native []; r host scan = @native as Native [];
            r host result(kept) = @native as Native [a];
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    // PL-Catalog: the program declares the token exactly as the target does.
    let resident = |reads: proc_macro2::TokenStream| {
        let declaration: structures::Declaration =
            syn::parse_quote!(native = Native { reads [#reads], build inferred };);
        let mut available = Catalog {
            codecs: catalog().codecs,
            ..Catalog::default()
        };
        available.register(
            declaration.clone(),
            vec![
                vec!["host".into(), "device".into()],
                vec!["host".into(), "device".into(), "block".into()],
            ],
        );
        let mut source = source.clone();
        source.structures.structures.providers.declarations = vec![declaration];
        (source, available)
    };
    let (source, available) = resident(quote::quote!(exact, range, membership, resident));
    let facts = cq_lang_sequence::contract::check(&source).unwrap();
    assert!(facts.structures.host_read("r"));
    let planned = decompose(&source, &machine(), &available).unwrap();
    let facts = check_grading(&planned.program, &machine()).unwrap();
    let reader = facts
        .provider_reads
        .iter()
        .find(|(_, providers)| providers.contains("r"))
        .map(|(name, _)| name)
        .unwrap();
    assert!(
        facts.levels[reader].len() > 1,
        "{reader}: {:?}",
        facts.levels
    );
    assert_eq!(facts.levels["bound"], ["host"]);
    check_rewrite(&source, &planned.program).unwrap();
    // Without the explicit resident token the same handle is not admitted below host.
    let (source, native) = resident(quote::quote!(exact, range, membership));
    assert!(decompose(&source, &machine(), &native).is_err());
}
