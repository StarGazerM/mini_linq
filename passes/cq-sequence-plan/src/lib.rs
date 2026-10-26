//! Bounded, deterministic Sequence decomposition with replayable rewrite advice.
#![forbid(unsafe_code)]

use cq_lang_sequence::{Analysis, Catalog, Machine, OutputProtocol, Program, Term};
use cq_syntax::symbol_name;
use quote::quote;
use syn::Ident;

mod catalog;
pub use catalog::matches_type;
pub mod cost;
pub use cost::{Cost, cost};
mod advice;
mod decomposition;
mod key_partition;
mod placement;
mod planner;
pub use key_partition::{KeyPartitionPolicy, decompose_with_key_partitions};
pub use planner::{Planner, RuleBased};

pub use cq_sequence_rewrite_passes::select_from_catalog;

/// One invocation of the same checked library function exposed by a rewrite macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advice {
    Evidence(cq_lang_sequence::Evidence),
    LookupFirst(Box<cq_sequence_rewrite_passes::LookupFirst>),
    Adaptive(cq_sequence_rewrite_passes::AdaptiveCandidate),
    KeyPartition(cq_sequence_rewrite_passes::KeyPartition),
    GroupWithoutCanon(Ident),
    Store(Ident),
    StoreLevel(Ident),
    Publish {
        name: Ident,
        producer: Ident,
    },
    Tile {
        name: Ident,
        n: usize,
        items: Option<usize>,
    },
    Partition {
        name: Ident,
        n: usize,
        items: Option<usize>,
    },
    Segment {
        name: Ident,
        n: usize,
        items: Option<usize>,
    },
    Output(OutputProtocol),
    Grade {
        name: Ident,
        level: Vec<Ident>,
        machine: Ident,
    },
}

impl Advice {
    /// Replay a checked rewrite; this never bypasses its construction checker.
    pub fn apply(&self, source: &Program) -> syn::Result<Program> {
        self.apply_in(&mut Default::default(), source)
    }

    pub fn apply_in(
        &self,
        checker: &mut cq_lang_sequence::contract::Checker,
        source: &Program,
    ) -> syn::Result<Program> {
        use cq_sequence_rewrite_passes as rewrites;
        let result = match self {
            Self::Evidence(evidence) => rewrites::record_evidence_in(checker, source, evidence),
            Self::LookupFirst(arguments) => {
                rewrites::lookup_first_in(checker, source, quote!(#arguments))
            }
            Self::Adaptive(arguments) => rewrites::apply_adaptive_in(checker, source, arguments),
            Self::KeyPartition(arguments) => rewrites::key_partition_in(checker, source, arguments),
            Self::GroupWithoutCanon(name) => {
                rewrites::group_without_canon_in(checker, source, quote!(#name))
            }
            Self::Store(name) => rewrites::store_in(checker, source, quote!(#name)),
            Self::StoreLevel(name) => rewrites::store_level_in(checker, source, quote!(#name)),
            Self::Publish { name, producer } => {
                rewrites::publish_in(checker, source, name, producer)
            }
            Self::Tile { name, n, items }
            | Self::Partition { name, n, items }
            | Self::Segment { name, n, items } => {
                let n = syn::LitInt::new(&n.to_string(), name.span());
                let arguments = if let Some(items) = items {
                    let items = syn::LitInt::new(&items.to_string(), name.span());
                    quote!(#name, #n, items #items)
                } else {
                    quote!(#name, #n)
                };
                match self {
                    Self::Tile { .. } => rewrites::tile_in(checker, source, arguments),
                    Self::Partition { .. } => rewrites::partition_in(checker, source, arguments),
                    _ => rewrites::segment_in(checker, source, arguments),
                }
            }
            Self::Output(protocol) => rewrites::output_in(checker, source, quote!(#protocol)),
            Self::Grade {
                name,
                level,
                machine,
            } => {
                let mut target = source.clone();
                rewrites::grade_definitions_in(
                    checker,
                    &mut target,
                    machine,
                    &[cq_lang_sequence::Grade {
                        name: name.clone(),
                        level: level.clone(),
                    }],
                )
                .map(|()| target)
            }
        };
        result.map_err(|errors| {
            let mut annotated = None;
            for error in errors {
                let error = syn::Error::new(
                    error.span(),
                    format!("P-Decompose {}: {error}", quote!(#self)),
                );
                if let Some(previous) = &mut annotated {
                    syn::Error::combine(previous, error);
                } else {
                    annotated = Some(error);
                }
            }
            annotated.expect("syn::Error contains at least one diagnostic")
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub program: Program,
    pub trace: Vec<Advice>,
}

/// Fill unauthored scheduling decisions, returning the checked result and its proof trace.
pub fn plan(program: &Program, machine: &Machine, catalog: &Catalog) -> syn::Result<Planned> {
    plan_with_key_partitions(program, machine, catalog, &[])
}

/// Plan with explicit force/forbid decisions for key partitioning. Unmentioned
/// providers use a checked runtime count choice; collectives use PL-Cost.
pub fn plan_with_key_partitions(
    program: &Program,
    machine: &Machine,
    catalog: &Catalog,
    policies: &[KeyPartitionPolicy],
) -> syn::Result<Planned> {
    let started = std::env::var_os("CQ_SEQUENCE_PLAN_TIMING").map(|_| std::time::Instant::now());
    let checker = &mut cq_lang_sequence::contract::Checker::new(program)?;
    let mut prepared = Planned {
        program: program.clone(),
        trace: Vec::new(),
    };
    if let Some(advice) =
        cq_sequence_rewrite_passes::lookup_first_advice_in(checker, program, catalog)?
    {
        push(
            checker,
            &mut prepared,
            Advice::LookupFirst(Box::new(advice)),
        )?;
    }
    let ordering_elapsed = started.map(|started| started.elapsed());
    // Freeze every decision before the first choice changes the candidate graph.
    let adaptive = cq_sequence_rewrite_passes::adaptive_candidates_in(checker, &prepared.program)?;
    for advice in adaptive {
        push(checker, &mut prepared, Advice::Adaptive(advice))?;
    }
    let adaptive_elapsed = started.map(|started| started.elapsed());
    let mut planned = decompose_in(checker, &prepared.program, machine, catalog, policies)?;
    prepared.trace.append(&mut planned.trace);
    planned.trace = prepared.trace;
    if let Some(started) = started {
        eprintln!("Sequence plan: {:.6}s", started.elapsed().as_secs_f64());
        let ordering = ordering_elapsed.expect("timing enabled").as_secs_f64();
        let adaptive = adaptive_elapsed.expect("timing enabled").as_secs_f64();
        eprintln!(
            "Sequence phases: ordering {ordering:.6}s, adaptive {:.6}s, decomposition {:.6}s",
            adaptive - ordering,
            started.elapsed().as_secs_f64() - adaptive,
        );
        if started.elapsed() >= std::time::Duration::from_millis(100) {
            eprintln!("Sequence slow input: {}", quote!(#program));
        }
    }
    Ok(planned)
}

/// Fill only execution decomposition, preserving an explicitly authored join order.
/// Callers with an order-advice phase use this entry rather than selecting another
/// relational order. Its result has the same checked replay and grading contract.
pub fn decompose(program: &Program, machine: &Machine, catalog: &Catalog) -> syn::Result<Planned> {
    decompose_in(&mut Default::default(), program, machine, catalog, &[])
}

fn decompose_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    program: &Program,
    machine: &Machine,
    catalog: &Catalog,
    policies: &[KeyPartitionPolicy],
) -> syn::Result<Planned> {
    checker.check_shared(program)?;
    key_partition::validate(program, policies)?;
    let mut prepared = Planned {
        program: program.clone(),
        trace: Vec::new(),
    };
    fn has_group(term: &Term) -> bool {
        match term {
            Term::Group(_, _) => true,
            Term::Store { inner, .. } | Term::Canon(inner) => has_group(inner),
            _ => false,
        }
    }
    let groups: Vec<_> = program
        .sequence
        .definitions
        .iter()
        .filter(|definition| has_group(&definition.term))
        .map(|definition| definition.name.clone())
        .collect();
    let mut unique_groups = std::collections::BTreeSet::new();
    for name in groups {
        let advice = Advice::GroupWithoutCanon(name.clone());
        if let Ok(target) = advice.apply_in(checker, &prepared.program) {
            unique_groups.insert(symbol_name(&name));
            if target == prepared.program {
                continue;
            }
            prepared.program = target;
            prepared.trace.push(advice);
        }
    }
    let program = &prepared.program;
    let prefix_trace = prepared.trace.clone();
    if machine.items == 0 {
        return Err(syn::Error::new(
            machine.name.span(),
            "P-Decompose: machine items must be positive",
        ));
    }
    if machine.key_partition_threshold == 0 {
        return Err(syn::Error::new(
            machine.name.span(),
            "P-Decompose: machine key partition threshold must be positive",
        ));
    }
    if machine.key_partition_cache_threshold == 0 {
        return Err(syn::Error::new(
            machine.name.span(),
            "P-Decompose: machine key partition cache threshold must be positive",
        ));
    }
    if let Some(graded) = &program.graded
        && symbol_name(&graded.machine) != symbol_name(&machine.name)
    {
        return Err(syn::Error::new(
            graded.machine.span(),
            "PL-Precedence: cannot replace an authored machine",
        ));
    }
    // Formal rule PL-Det: Preserve preferred geometry while first trying nested
    // flatten segmentation. Reduce items, then strip width; split dispatches only
    // if no single-dispatch schedule fits. PL-Cost compares legal schedules.
    let parallel = !machine.root.children.is_empty();
    let widths = decomposition::widths(&machine.root);
    let width = widths.first().copied();
    let mut schedules = vec![(width, machine.items)];
    let mut items = machine.items;
    while items > 1 {
        items = (items / 2).max(1);
        schedules.push((width, items));
    }
    schedules.extend(widths.iter().skip(1).map(|&width| (Some(width), 1)));
    let mut error = None;
    let cuts = if parallel && machine.layout.is_some() {
        program
            .sequence
            .definitions
            .iter()
            .filter(|definition| {
                matches!(
                    definition.term,
                    Term::Flatten(_) | Term::Choose(_) | Term::Union(_)
                ) && !program
                    .graded
                    .iter()
                    .flat_map(|graded| &graded.grades)
                    .any(|grade| grade.name == definition.name)
            })
            .map(|definition| definition.name.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let stages = if parallel && machine.layout.is_some() {
        cuts.len() + 3
    } else {
        1
    };
    let mut split_best: Option<(Cost, Planned)> = None;
    for stage in 0..stages {
        if stage == cuts.len() + 1
            && let Some((_, planned)) = split_best.take()
        {
            return Ok(planned);
        }
        let prepared = (|| {
            let mut stores = if stage == 0 {
                Planned {
                    program: program.clone(),
                    trace: Vec::new(),
                }
            } else {
                let single = stage.checked_sub(1).and_then(|index| cuts.get(index));
                resource_stages(
                    checker,
                    program,
                    single.is_some() || stage == cuts.len() + 2,
                    single,
                )?
            };
            stores.trace.splice(0..0, prefix_trace.iter().cloned());
            let mut placed = prepare(checker, &stores.program, machine, catalog, parallel)?;
            stores.trace.append(&mut placed.trace);
            placed.trace = stores.trace;
            Ok::<_, syn::Error>(placed)
        })();
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(failure) => {
                candidate_error(&mut error, stage, None, None, None, failure);
                continue;
            }
        };
        let segmentable = prepared.program.sequence.definitions.iter().any(|definition| {
            matches!(&definition.term, Term::Flatten(inner) if matches!(inner.as_ref(), Term::Lookup { .. }))
        });
        let facts = checker.check_shared(&prepared.program)?;
        let partition_choices = key_partition::choices(&prepared.program, &facts, policies);
        let keyed_partials = key_partition::has_keyed_partials(&prepared.program, &unique_groups);
        for &(width, items) in &schedules {
            let mut best: Option<(Cost, Planned)> = None;
            for segmented in [true, false] {
                if segmented && (!parallel || width.is_none() || !segmentable) {
                    continue;
                }
                for (keyed, candidate_policies, partial_groups) in partition_choices
                    .iter()
                    .filter(|_| keyed_partials)
                    .map(|choice| (true, choice.as_slice(), true))
                    .chain(std::iter::once((false, policies, false)))
                    .chain(
                        partition_choices
                            .iter()
                            .map(|choice| (true, choice.as_slice(), false)),
                    )
                {
                    match candidate(
                        checker,
                        &prepared,
                        machine,
                        catalog,
                        width,
                        items,
                        segmented,
                        program,
                        candidate_policies,
                        keyed,
                        partial_groups,
                        &unique_groups,
                    ) {
                        Ok((analysis, planned)) => {
                            let cost = Cost::from_analysis(&planned.program, machine, &analysis)?;
                            if best
                                .as_ref()
                                .is_none_or(|(current, _)| cost.better_than(current))
                            {
                                best = Some((cost, planned));
                            }
                        }
                        Err(failure) => {
                            candidate_error(
                                &mut error,
                                stage,
                                width,
                                Some(items),
                                Some(segmented),
                                failure,
                            );
                        }
                    }
                }
            }
            if let Some((cost, planned)) = best {
                if stage > 0 && stage <= cuts.len() {
                    if split_best
                        .as_ref()
                        .is_none_or(|(current, _)| cost.better_than(current))
                    {
                        split_best = Some((cost, planned));
                    }
                    break;
                }
                return Ok(planned);
            }
        }
    }
    Err(error
        .unwrap_or_else(|| syn::Error::new(machine.name.span(), "P-Decompose: no legal candidate")))
}

fn candidate_error(
    errors: &mut Option<syn::Error>,
    stage: usize,
    width: Option<usize>,
    items: Option<usize>,
    segmented: Option<bool>,
    failure: syn::Error,
) {
    // Preserve every component: syn::Error's Display intentionally prints only
    // its first diagnostic, including when a checked rewrite combined failures.
    for failure in failure {
        let failure = syn::Error::new(
            failure.span(),
            format!(
                "P-Decompose candidate stage={stage} tile={width:?} items={items:?} segmented={segmented:?}: {failure}"
            ),
        );
        if let Some(errors) = errors {
            errors.combine(failure);
        } else {
            *errors = Some(failure);
        }
    }
}

fn resource_stages(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    split_choices: bool,
    single: Option<&Ident>,
) -> syn::Result<Planned> {
    let mut planned = Planned {
        program: source.clone(),
        trace: Vec::new(),
    };
    // Prepare the checked publication alternative once, only when the
    // unstaged schedules cannot satisfy this machine's obligations.
    for definition in &source.sequence.definitions {
        if (matches!(&definition.term, Term::Flatten(_))
            || (split_choices && matches!(&definition.term, Term::Choose(_) | Term::Union(_))))
            && single.is_none_or(|name| definition.name == *name)
            && !source
                .graded
                .iter()
                .flat_map(|graded| &graded.grades)
                .any(|grade| grade.name == definition.name)
        {
            push(
                checker,
                &mut planned,
                if matches!(definition.term, Term::Flatten(_)) {
                    Advice::StoreLevel(definition.name.clone())
                } else {
                    Advice::Store(definition.name.clone())
                },
            )?;
        }
    }
    Ok(planned)
}

fn prepare(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    machine: &Machine,
    catalog: &Catalog,
    parallel: bool,
) -> syn::Result<Planned> {
    let mut planned = Planned {
        program: source.clone(),
        trace: Vec::new(),
    };
    let source_facts = checker.check_shared(&planned.program)?;
    let boundaries = placement::boundaries(&planned.program, machine, &source_facts, catalog);
    push_all(checker, &mut planned, boundaries)?;
    let facts = checker.check_shared(&planned.program)?;
    let placement = placement::advise(&planned.program, machine, parallel, &facts, source, catalog);
    push_all(checker, &mut planned, placement)?;
    Ok(planned)
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Planned,
    machine: &Machine,
    catalog: &Catalog,
    width: Option<usize>,
    items: usize,
    segmented: bool,
    authored: &Program,
    policies: &[KeyPartitionPolicy],
    keyed: bool,
    partial_groups: bool,
    unique_groups: &std::collections::BTreeSet<String>,
) -> syn::Result<(Analysis, Planned)> {
    let mut planned = source.clone();
    key_partition::apply(
        checker,
        &mut planned,
        authored,
        machine,
        catalog,
        policies,
        keyed,
        false,
        partial_groups,
        unique_groups,
    )?;
    if !machine.root.children.is_empty() {
        let mut decomposed =
            decomposition::advise(checker, &planned.program, machine, width, items, segmented)?;
        planned.trace.append(&mut decomposed.trace);
        planned.program = decomposed.program;
    }
    key_partition::apply(
        checker,
        &mut planned,
        authored,
        machine,
        catalog,
        policies,
        keyed,
        true,
        partial_groups,
        unique_groups,
    )?;
    // Formal rule PL-Precedence: Recorded grades, tiles and output are never replaced by generated advice.
    if source.program.sequence.output.is_none() {
        let last =
            planned.program.sequence.definitions.last().ok_or_else(|| {
                syn::Error::new(machine.name.span(), "P-Decompose: empty Sequence")
            })?;
        let protocol = if matches!(
            last.term,
            Term::Store {
                capacity: Some(_),
                ..
            }
        ) {
            OutputProtocol::Bounded
        } else if variable_output(&last.term, &planned.program) {
            OutputProtocol::Reserve
        } else {
            OutputProtocol::Count
        };
        push(checker, &mut planned, Advice::Output(protocol))?;
    }
    // Formal rule PL-Closure: Each step uses a checked public rewrite; the completed trace satisfies machine grading.
    let analysis = checker.check_grading(&planned.program, machine)?;
    catalog::check(&planned.program, &analysis, catalog)?;
    Ok((analysis, planned))
}

fn push_all(
    checker: &mut cq_lang_sequence::contract::Checker,
    planned: &mut Planned,
    advice: Vec<Advice>,
) -> syn::Result<()> {
    let mut advice = advice.into_iter().peekable();
    while let Some(step) = advice.next() {
        let Advice::Grade {
            name,
            level,
            machine,
        } = &step
        else {
            push(checker, planned, step)?;
            continue;
        };
        let mut grades = vec![cq_lang_sequence::Grade {
            name: name.clone(),
            level: level.clone(),
        }];
        let mut trace = vec![step.clone()];
        while let Some(Advice::Grade { machine: next, .. }) = advice.peek() {
            if next != machine {
                break;
            }
            let next = advice.next().expect("peeked grade");
            let Advice::Grade { name, level, .. } = &next else {
                unreachable!()
            };
            grades.push(cq_lang_sequence::Grade {
                name: name.clone(),
                level: level.clone(),
            });
            trace.push(next);
        }
        // Grades are nonsemantic; the typed checked rewrite validates the
        // complete batch once while the replay trace retains individual steps.
        cq_sequence_rewrite_passes::grade_definitions_in(
            checker,
            &mut planned.program,
            machine,
            &grades,
        )?;
        planned.trace.extend(trace);
    }
    Ok(())
}

fn push(
    checker: &mut cq_lang_sequence::contract::Checker,
    planned: &mut Planned,
    advice: Advice,
) -> syn::Result<()> {
    if let Advice::Grade {
        name,
        level,
        machine,
    } = &advice
    {
        cq_sequence_rewrite_passes::grade_definitions_in(
            checker,
            &mut planned.program,
            machine,
            &[cq_lang_sequence::Grade {
                name: name.clone(),
                level: level.clone(),
            }],
        )
        .map_err(|error| {
            syn::Error::new(
                error.span(),
                format!("P-Decompose {}: {error}", quote!(#advice)),
            )
        })?;
    } else {
        planned.program = advice.apply_in(checker, &planned.program)?;
    }
    planned.trace.push(advice);
    Ok(())
}

fn variable_output(term: &Term, program: &Program) -> bool {
    match term {
        Term::Canon(_) | Term::Compact(_, _) => true,
        Term::Rows(_) => false,
        Term::Group(name, inner) => {
            program
                .structures
                .physical
                .program
                .definitions
                .iter()
                .any(|definition| {
                    definition.name == *name
                        && matches!(&definition.operator,
                cq_lang_sequence::structures::physical::Operator::Group(group)
                    if !group.keys.is_empty())
                })
                || variable_output(inner, program)
        }
        Term::Ref(name) => program
            .sequence
            .definitions
            .iter()
            .find(|d| symbol_name(&d.name) == symbol_name(name))
            .is_some_and(|d| variable_output(&d.term, program)),
        Term::Map(_, t)
        | Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::PartitionK { inner: t, .. }
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. }
        | Term::Lookup { inner: t, .. } => variable_output(t, program),
        Term::Union(terms) => terms.iter().any(|t| variable_output(t, program)),
        Term::Choose(arms) => arms.iter().any(|(_, t)| variable_output(t, program)),
    }
}
