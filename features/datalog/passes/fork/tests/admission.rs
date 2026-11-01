//! ADM-InPlan: FK-Known decisions, the synthetic committed read, its fork
//! correspondence (erasure and mutations), DL-AdmitOnce and the schedule read.
use cq_datalog_fork::{check, compile, compile_with};
use cq_lang_datalog_round::{self as round, Admission, cq::BodyItem, incremental, storage};
use quote::{ToTokens, quote};

fn strata() -> cq_lang_datalog_round::incremental::strata::Program {
    let source = syn::parse2(quote! {
        struct Admission;
        relation E(x:i32,y:i32) = (e);
        relation Flag(x:i32);
        relation Pair(x:i32,y:i32);
        relation Twice(a:i32,b:i32,c:i32);
        output Flag, Pair, Twice;
        // Advised known(at(level 1)): `x` binds at the first of two reads.
        Flag: Flag(*x) :- E(x,y), E(y,_);
        // Unadvised: admit.
        Pair: Pair(*x,*z) :- E(x,y), E(y,z);
        // Repeated borrowed and computed head terms bind at the first read.
        Twice: Twice(x, x, (*x + 1)) :- E(x,y), E(y,_);
    })
    .unwrap();
    cq_datalog_stratify::compile(&source).unwrap()
}

/// Flag and Twice advised `known(at(level 1))`; Pair left at the default.
fn program() -> storage::Program {
    let strata = strata();
    let plans = ["Flag", "Twice"].map(|rule| cq_datalog_incremental::RulePlan {
        rule: syn::Ident::new(rule, proc_macro2::Span::call_site()),
        delta: None,
        order: Vec::new(),
        smallest: Vec::new(),
        known: Some(incremental::Known::At(1)),
    });
    let incremental = cq_datalog_incremental::compile_with_advice(
        &strata,
        cq_datalog_incremental::DriverPolicy::WrittenOrder,
        &plans,
    )
    .unwrap();
    cq_datalog_storage::compile(&incremental).unwrap()
}

#[test]
fn default_admission_is_admit() {
    let strata = strata();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let source = cq_datalog_storage::compile(&incremental).unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    for rule in rules(&target) {
        assert_eq!(rule.known, Admission::Admit);
        assert!(target.holes[rule.hole].pipeline.passes[0].arguments.is_empty());
    }
}

fn rules(target: &round::Program) -> Vec<&round::RuleHole> {
    target
        .context
        .strata
        .iter()
        .flat_map(|stratum| stratum.seeds.iter().chain(&stratum.steps))
        .collect()
}

fn rule<'a>(target: &'a round::Program, head: &str) -> &'a round::RuleHole {
    rules(target)
        .into_iter()
        .find(|rule| target.context.relations[rule.head].declaration.name == head)
        .unwrap()
}

fn canonical(target: &round::Program, head: usize) -> &round::Layout {
    &target.context.layouts[target.context.relations[head].providers.canonical]
}

#[test]
fn advised_admission_records_the_committed_read() {
    let source = program();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let reparsed: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(reparsed, target);
    assert_eq!(rule(&target, "Flag").known, Admission::InPlan);
    assert_eq!(rule(&target, "Pair").known, Admission::Admit);
    assert_eq!(rule(&target, "Twice").known, Admission::InPlan);
    for rule in rules(&target) {
        let hole = &target.holes[rule.hole].cq;
        let name = &canonical(&target, rule.head).name;
        let last = matches!(
            hole.query.body.iter().last(),
            Some(BodyItem::Negation(negative)) if &negative.atom.relation == name
        );
        assert_eq!(last, rule.known == Admission::InPlan);
        // IX-Stable: an in-plan hole records its committed head read.
        let stratum = target
            .context
            .strata
            .iter()
            .position(|stratum| stratum.seeds.iter().chain(&stratum.steps).any(|r| r == rule))
            .unwrap();
        let reads = target.schedule.strata[stratum]
            .reads
            .iter()
            .find(|reads| {
                let phase = &target.context.strata[stratum];
                let holes = if reads.phase == "seed" { &phase.seeds } else { &phase.steps };
                holes[reads.index] == *rule
            })
            .unwrap();
        let committed = reads
            .reads
            .iter()
            .any(|read| read.relation == rule.head && read.version == "full");
        assert_eq!(committed, rule.known == Admission::InPlan);
    }
}

#[test]
fn computed_and_repeated_head_terms_bind_before_the_committed_read() {
    let source = program();
    let target = compile(&source).unwrap();
    let rule = rule(&target, "Twice");
    let hole = &target.holes[rule.hole].cq;
    let name = &canonical(&target, rule.head).name;
    let head = &hole.query.head.args;
    // The borrowed repeated variable stays; the computed term gets one binding.
    assert_eq!(head[0], syn::parse_quote!(x));
    assert_eq!(head[1], syn::parse_quote!(x));
    let computed = &head[2];
    let body: Vec<_> = hole.query.body.iter().cloned().collect();
    let expected_let: BodyItem = syn::parse_quote!(let #computed: i32 = (*x + 1));
    let expected_negation: BodyItem = syn::parse_quote!(!#name(x, x, #computed));
    assert_eq!(body[body.len() - 2], expected_let);
    assert_eq!(body[body.len() - 1], expected_negation);
    // The hole keeps its answer ownership: computed columns are owned.
    assert_eq!(
        target.holes[rule.hole].answer,
        [
            round::AnswerColumn::Borrowed,
            round::AnswerColumn::Borrowed,
            round::AnswerColumn::Owned,
        ]
    );
}

#[test]
fn fork_correspondence_erases_exactly_the_committed_read() {
    let source = program();
    let target = compile(&source).unwrap();
    let original = target.clone();
    let flag = rule(&original, "Flag");
    let twice = rule(&original, "Twice");
    let pair = rule(&original, "Pair");
    let flag_layout = canonical(&target, flag.head).name.clone();
    let other = target
        .context
        .layouts
        .iter()
        .find(|layout| layout.name != flag_layout && layout.relation != flag.head)
        .unwrap()
        .name
        .clone();
    let pair_layout = canonical(&target, pair.head).name.clone();
    let body = |p: &mut round::Program, hole: usize| -> Vec<BodyItem> {
        p.holes[hole].cq.query.body.iter().cloned().collect()
    };
    let set = |p: &mut round::Program, hole: usize, items: Vec<BodyItem>| {
        p.holes[hole].cq.query.body = items.into_iter().collect();
    };
    let changes: Vec<(&str, Box<dyn Fn(&mut round::Program) + '_>)> = vec![
        ("wrong layout", {
            let other = other.clone();
            Box::new(move |p| {
                let mut items = body(p, flag.hole);
                items.pop();
                items.push(syn::parse_quote!(!#other(x, x)));
                set(p, flag.hole, items);
            })
        }),
        ("missing head variable", {
            let name = flag_layout.clone();
            Box::new(move |p| {
                let mut items = body(p, flag.hole);
                items.pop();
                items.push(syn::parse_quote!(!#name(_)));
                set(p, flag.hole, items);
            })
        }),
        ("wrong head variable", {
            let name = flag_layout.clone();
            Box::new(move |p| {
                let mut items = body(p, flag.hole);
                items.pop();
                items.push(syn::parse_quote!(!#name(y)));
                set(p, flag.hole, items);
            })
        }),
        ("extra negation", {
            let name = flag_layout.clone();
            Box::new(move |p| {
                let mut items = body(p, flag.hole);
                items.push(syn::parse_quote!(!#name(x)));
                set(p, flag.hole, items);
            })
        }),
        ("negation on an admit hole", {
            let name = pair_layout.clone();
            Box::new(move |p| {
                let mut items = body(p, pair.hole);
                items.push(syn::parse_quote!(!#name(x, z)));
                set(p, pair.hole, items);
            })
        }),
        (
            "missing negation",
            Box::new(move |p| {
                let mut items = body(p, flag.hole);
                items.pop();
                set(p, flag.hole, items);
            }),
        ),
        (
            "record without the read",
            Box::new(move |p| {
                for stratum in &mut p.context.strata {
                    for rule in stratum.seeds.iter_mut().chain(&mut stratum.steps) {
                        if rule.hole == pair.hole {
                            rule.known = Admission::InPlan;
                        }
                    }
                }
            }),
        ),
        (
            "read without the record",
            Box::new(move |p| {
                for stratum in &mut p.context.strata {
                    for rule in stratum.seeds.iter_mut().chain(&mut stratum.steps) {
                        if rule.hole == flag.hole {
                            rule.known = Admission::Admit;
                        }
                    }
                }
            }),
        ),
        (
            "changed computed binding",
            Box::new(move |p| {
                let mut items = body(p, twice.hole);
                let at = items.len() - 2;
                let BodyItem::Let(binding) = &mut items[at] else { panic!("binding") };
                binding.expression = syn::parse_quote!((*x + 2));
                set(p, twice.hole, items);
            }),
        ),
        (
            "changed computed type",
            Box::new(move |p| {
                let mut items = body(p, twice.hole);
                let at = items.len() - 2;
                let BodyItem::Let(binding) = &mut items[at] else { panic!("binding") };
                binding.annotation.as_mut().unwrap().ty = syn::parse_quote!(i64);
                set(p, twice.hole, items);
            }),
        ),
    ];
    for (name, change) in changes {
        let mut changed = target.clone();
        change(&mut changed);
        assert!(check(&source, &changed).is_err(), "{name}");
    }
}

#[test]
fn non_identity_canonical_order_permutes_the_committed_key() {
    let mut source = program();
    // Host certificates describe the default layouts; this test authors one.
    source.certificates.clear();
    let pair = source
        .incremental
        .strata
        .source
        .declarations
        .iter()
        .position(|declaration| declaration.name == "Twice")
        .unwrap();
    // An authored complete layout in column order (2, 0, 1) as canonical.
    let index = source.layouts.len();
    source.layouts.push(storage::Layout {
        relation: syn::parse_quote!(Twice),
        columns: vec![2, 0, 1],
        key: 3,
        provider: syn::parse_quote!(
            @hash_set as ::std::collections::HashSet<(i32, i32, i32,), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
        ),
        hasher: storage::Hasher::Fx,
        maintenance: syn::parse_quote!(admitted_rows),
        protocol: storage::LayoutProtocol::Set,
    });
    source.providers[pair].canonical = index;
    storage::contract::check(&source).unwrap();
    let target = compile(&source).unwrap();
    let rule = rule(&target, "Twice");
    assert_eq!(rule.known, Admission::InPlan);
    let hole = &target.holes[rule.hole].cq;
    let layout = &target.context.layouts[index];
    let name = &layout.name;
    let computed = &hole.query.head.args[2];
    let expected: BodyItem = syn::parse_quote!(!#name(#computed, x, x));
    assert_eq!(hole.query.body.iter().last().unwrap(), &expected);
    check(&source, &target).unwrap();
    // The identity order over the same layout is rejected.
    let mut changed = target.clone();
    let mut items: Vec<BodyItem> = changed.holes[rule.hole].cq.query.body.iter().cloned().collect();
    items.pop();
    items.push(syn::parse_quote!(!#name(x, x, #computed)));
    changed.holes[rule.hole].cq.query.body = items.into_iter().collect();
    assert!(check(&source, &changed).is_err());
}

fn advised(known: incremental::Known, order: bool) -> syn::Result<round::Program> {
    let source = syn::parse2(quote! {
        struct Advice;
        relation E(x:i32,y:i32) = (e);
        relation Flag(x:i32);
        relation Pair(x:i32,y:i32);
        output Flag, Pair;
        Flag: Flag(*x) :- E(x,y), E(y,w), E(w,_);
        Pair: Pair(*x,*z) :- E(x,y), E(y,z), E(z,_);
    })
    .unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let plans = ["Flag", "Pair"].map(|rule| cq_datalog_incremental::RulePlan {
        rule: syn::Ident::new(rule, proc_macro2::Span::call_site()),
        delta: None,
        order: if order {
            vec![syn::parse_quote!(E@0), syn::parse_quote!(E@1)]
        } else {
            Vec::new()
        },
        smallest: Vec::new(),
        known: Some(known),
    });
    let incremental = cq_datalog_incremental::compile_with_advice(
        &strata,
        cq_datalog_incremental::DriverPolicy::WrittenOrder,
        &plans,
    )?;
    compile(&cq_datalog_storage::compile(&incremental)?)
}

#[test]
fn known_advice_pins_admission_and_transports_the_level() {
    // known(admit) keeps the committed probe at admission.
    let admit = advised(incremental::Known::Admit, false).unwrap();
    assert!(rules(&admit).iter().all(|rule| rule.known == Admission::Admit));
    for rule in rules(&admit) {
        assert!(admit.holes[rule.hole].pipeline.passes[0].arguments.is_empty());
    }
    // known(at(level 2)): Pair's `z` binds at read 2, Flag's `x` at read 1.
    for order in [false, true] {
        let at = advised(incremental::Known::At(2), order).unwrap();
        for rule in rules(&at) {
            assert_eq!(rule.known, Admission::InPlan);
            let hole = &at.holes[rule.hole];
            let name = &canonical(&at, rule.head).name;
            let clause = syn::Index::from(hole.cq.query.body.len() - 1);
            let arguments = hole.pipeline.passes[0].arguments.to_string();
            let known = quote!(known(#name @ #clause at 2)).to_string();
            assert!(arguments.ends_with(&known), "{arguments}");
            assert_eq!(arguments.contains("order"), order, "{arguments}");
        }
    }
    // Level 1 precedes Pair's last head binding.
    let error = advised(incremental::Known::At(1), false).unwrap_err().to_string();
    assert!(error.contains("known(at(level 1)) must lie between"), "{error}");
}

#[test]
fn advised_in_plan_requires_a_planning_owner() {
    let queue = quote!(cq = pipeline { custom::lower, ::mini_linq::passes::translate_pull }; lower = 0;);
    // The default (admit) carries no known option: no planning owner needed.
    let incremental = cq_datalog_incremental::compile(&strata()).unwrap();
    let default = cq_datalog_storage::compile(&incremental).unwrap();
    let target = compile_with(&default, queue.clone()).unwrap();
    for hole in &target.holes {
        assert!(hole.plan.is_none());
    }
    // Advised in plan, the known placement targets a planning invocation.
    let error = compile_with(&program(), queue).unwrap_err().to_string();
    assert!(error.contains("requires a recorded planning invocation"), "{error}");
}
