//! ADM-InPlan: FK-Known decisions from advice and DL-Fanout, the synthetic
//! committed read, its fork correspondence (erasure and mutations),
//! DL-AdmitOnce and the schedule read.
use cq_datalog_fork::{check, compile, compile_with};
use cq_lang_datalog_round::{
    self as round, Admission, Fanout, cq::BodyItem, incremental, incremental::strata::source,
    storage,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn strata() -> cq_lang_datalog_round::incremental::strata::Program {
    strata_with(quote!())
}

fn source_with(evidence: TokenStream) -> source::Program {
    syn::parse2(quote! {
        struct Admission;
        relation E(x:i32,y:i32) = (e);
        relation Flag(x:i32);
        relation Pair(x:i32,y:i32);
        relation Twice(a:i32,b:i32,c:i32);
        #evidence
        output Flag, Pair, Twice;
        // `x` binds at the first of two reads; `E(y,_)` enumerates a witness.
        Flag: Flag(*x) :- E(x,y), E(y,_);
        // `z` binds at the last read.
        Pair: Pair(*x,*z) :- E(x,y), E(y,z);
        // Repeated borrowed and computed head terms bind at the first read.
        Twice: Twice(x, x, (*x + 1)) :- E(x,y), E(y,_);
    })
    .unwrap()
}

fn strata_with(evidence: TokenStream) -> cq_lang_datalog_round::incremental::strata::Program {
    cq_datalog_stratify::compile(&source_with(evidence)).unwrap()
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

fn unadvised(evidence: TokenStream) -> (storage::Program, round::Program) {
    let incremental = cq_datalog_incremental::compile(&strata_with(evidence)).unwrap();
    let source = cq_datalog_storage::compile(&incremental).unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    (source, target)
}

/// Without advice FK-Known follows DL-Fanout: Flag and Twice bind their head
/// at read 1 and then enumerate `E(y,_)` (`multiply`), so they are in plan at
/// level 1; Pair binds its head at its last read and stays `admit`.
#[test]
fn default_admission_follows_the_recorded_fanout() {
    let (source, target) = unadvised(quote!());
    assert_eq!(rule(&target, "Flag").known, Admission::InPlan);
    assert_eq!(rule(&target, "Twice").known, Admission::InPlan);
    assert_eq!(rule(&target, "Pair").known, Admission::Admit);
    for rule in rules(&target) {
        assert_eq!(rule.fanout, [Fanout::Multiply, Fanout::Multiply]);
        let arguments = target.holes[rule.hole]
            .invocation_pipeline()
            .unwrap()
            .passes[0]
            .arguments
            .to_string();
        let name = &canonical(&target, rule.head).name;
        let clause = syn::Index::from(target.holes[rule.hole].cq.query.body.len() - 1);
        match rule.known {
            Admission::InPlan => {
                assert_eq!(arguments, quote!(known(#name @ #clause at 1)).to_string())
            }
            Admission::Admit => assert!(arguments.is_empty(), "{arguments}"),
        }
    }
    let reparsed: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(reparsed, target);
    // The record is checked: a changed fan-out fact, or an admit record that
    // contradicts a recorded `multiply` read, is rejected.
    let flag = rule(&target, "Flag").clone();
    let mut changed = target.clone();
    for stratum in &mut changed.context.strata {
        for rule in stratum.seeds.iter_mut().chain(&mut stratum.steps) {
            if rule.hole == flag.hole {
                rule.fanout[1] = Fanout::Filter;
            }
        }
    }
    assert!(check(&source, &changed).is_err());
    let mut changed = target.clone();
    for stratum in &mut changed.context.strata {
        for rule in stratum.seeds.iter_mut().chain(&mut stratum.steps) {
            if rule.hole == flag.hole {
                rule.known = Admission::Admit;
            }
        }
    }
    let hole = &mut changed.holes[flag.hole];
    let mut items: Vec<BodyItem> = hole.cq.query.body.iter().cloned().collect();
    items.pop();
    hole.cq.query.body = items.into_iter().collect();
    hole.planning.as_mut().unwrap().known = None;
    assert!(check(&source, &changed).is_err());
}

/// A recorded EDB key bound at a read makes it a filter (at most one row).
#[test]
fn recorded_keys_make_keyed_reads_filters() {
    let (_, target) = unadvised(quote!(evidence from "test: E has one y per x" { key E [x]; }));
    for rule in rules(&target) {
        assert_eq!(rule.known, Admission::Admit);
        assert_eq!(rule.fanout, [Fanout::Multiply, Fanout::Filter]);
    }
    // WF-DL-Evidence: provenance, EDB subjects and distinct declared columns.
    for bad in [
        quote!(evidence from "" { key E [x]; }),
        quote!(evidence from "m" { key Missing [x]; }),
        quote!(evidence from "m" { key E [w]; }),
        quote!(evidence from "m" { key E [x, x]; }),
        quote!(evidence from "m" { key E [x]; key E [x]; }),
    ] {
        assert!(
            source::contract::check(&source_with(bad.clone())).is_err(),
            "{bad}"
        );
    }
    let program = source_with(quote!(evidence from "m" { key E [x, y]; }));
    source::contract::check(&program).unwrap();
    let reparsed: source::Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(reparsed, program);
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
            .position(|stratum| {
                stratum
                    .seeds
                    .iter()
                    .chain(&stratum.steps)
                    .any(|r| r == rule)
            })
            .unwrap();
        let reads = target.schedule.strata[stratum]
            .reads
            .iter()
            .find(|reads| {
                let phase = &target.context.strata[stratum];
                let holes = if reads.phase == "seed" {
                    &phase.seeds
                } else {
                    &phase.steps
                };
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
                let BodyItem::Let(binding) = &mut items[at] else {
                    panic!("binding")
                };
                binding.expression = syn::parse_quote!((*x + 2));
                set(p, twice.hole, items);
            }),
        ),
        (
            "changed computed type",
            Box::new(move |p| {
                let mut items = body(p, twice.hole);
                let at = items.len() - 2;
                let BodyItem::Let(binding) = &mut items[at] else {
                    panic!("binding")
                };
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
    let mut items: Vec<BodyItem> = changed.holes[rule.hole]
        .cq
        .query
        .body
        .iter()
        .cloned()
        .collect();
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
    assert!(
        rules(&admit)
            .iter()
            .all(|rule| rule.known == Admission::Admit)
    );
    for rule in rules(&admit) {
        assert!(
            admit.holes[rule.hole].pipeline.passes[0]
                .arguments
                .is_empty()
        );
    }
    // known(at(level 2)): Pair's `z` binds at read 2, Flag's `x` at read 1.
    for order in [false, true] {
        let at = advised(incremental::Known::At(2), order).unwrap();
        for rule in rules(&at) {
            assert_eq!(rule.known, Admission::InPlan);
            let hole = &at.holes[rule.hole];
            let name = &canonical(&at, rule.head).name;
            let clause = syn::Index::from(hole.cq.query.body.len() - 1);
            let arguments = hole.invocation_pipeline().unwrap().passes[0]
                .arguments
                .to_string();
            let known = quote!(known(#name @ #clause at 2)).to_string();
            assert!(arguments.ends_with(&known), "{arguments}");
            assert_eq!(arguments.contains("order"), order, "{arguments}");
        }
    }
    // Level 1 precedes Pair's last head binding.
    let error = advised(incremental::Known::At(1), false)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("known(at(level 1)) must lie between"),
        "{error}"
    );
}

#[test]
fn advised_in_plan_requires_a_planning_owner() {
    let queue =
        quote!(cq = pipeline { custom::lower, ::mini_linq::passes::translate_pull }; lower = 0;);
    // Without a recorded planning invocation the default is admit even where
    // a `multiply` read follows the head binding: no known option is placed.
    let incremental = cq_datalog_incremental::compile(&strata()).unwrap();
    let default = cq_datalog_storage::compile(&incremental).unwrap();
    let target = compile_with(&default, queue.clone()).unwrap();
    for hole in &target.holes {
        assert!(hole.plan.is_none());
    }
    for rule in rules(&target) {
        assert_eq!(rule.known, Admission::Admit);
    }
    // Advised in plan, the known placement targets a planning invocation.
    let error = compile_with(&program(), queue).unwrap_err().to_string();
    assert!(
        error.contains("requires a recorded planning invocation"),
        "{error}"
    );
}

#[test]
fn recorded_planning_admits_legal_levels_but_rewrites_preserve_the_fact() {
    let (storage, original) = unadvised(quote!());
    let hole = rule(&original, "Flag").hole;
    assert_eq!(
        original.holes[hole].planning.as_ref().unwrap().known,
        Some(1)
    );
    let mut alternative = original.clone();
    alternative.holes[hole].planning.as_mut().unwrap().known = Some(2);
    check(&storage, &alternative).unwrap();
    assert!(round::check_rewrite(&original, &alternative).is_err());
    for level in [0, 3] {
        let mut invalid = original.clone();
        invalid.holes[hole].planning.as_mut().unwrap().known = Some(level);
        assert!(check(&storage, &invalid).is_err());
    }
    let authored = advised(incremental::Known::At(2), true).unwrap();
    let hole = rule(&authored, "Flag").hole;
    let mutations: &[fn(&mut round::Planning)] = &[
        |p| p.order.swap(0, 1),
        |p| p.authored = false,
        |p| {
            p.steps.push(round::PlanningStep {
                occurrence: usize::MAX,
                columns: None,
            })
        },
        |p| p.smallest.push(vec![0, 1]),
        |p| p.known = Some(3),
    ];
    for mutate in mutations {
        let mut invalid = authored.clone();
        mutate(invalid.holes[hole].planning.as_mut().unwrap());
        assert!(check(&authored.storage, &invalid).is_err());
    }
    let mut missing = authored.clone();
    missing.holes[hole].planning = None;
    assert!(check(&authored.storage, &missing).is_err());
    let mut raw = authored.clone();
    raw.holes[hole].pipeline.passes[0].arguments = quote!(known(C@0 at 1));
    assert!(check(&authored.storage, &raw).is_err());
}
