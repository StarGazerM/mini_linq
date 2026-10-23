//! Bounded, deterministic Sequence decomposition with replayable rewrite advice.
#![forbid(unsafe_code)]

use cq_lang_sequence::{Analysis, Catalog, Machine, OutputProtocol, Program, Term};
use cq_syntax::symbol_name;
use quote::quote;
use syn::Ident;

mod catalog;
pub mod cost;
pub use cost::{Cost, cost};
mod advice;
mod decomposition;
mod placement;
mod planner;
pub use planner::{Planner, RuleBased};

pub use cq_sequence_rewrite_passes::select_from_catalog;

/// One invocation of the same checked library function exposed by a rewrite macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advice {
    Evidence(cq_lang_sequence::Evidence),
    LookupFirst(Box<cq_sequence_rewrite_passes::LookupFirst>),
    Adaptive(cq_sequence_rewrite_passes::AdaptiveCandidate),
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
            } => rewrites::grade_in(checker, source, quote!(#machine, #name at #(#level).*)),
        };
        result.map_err(|error| {
            syn::Error::new(
                error.span(),
                format!("P-Decompose {}: {error}", quote!(#self)),
            )
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
    let started = std::env::var_os("CQ_SEQUENCE_PLAN_TIMING").map(|_| std::time::Instant::now());
    let checker = &mut cq_lang_sequence::contract::Checker::default();
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
    // Freeze every decision before the first choice changes the candidate graph.
    let adaptive = cq_sequence_rewrite_passes::adaptive_candidates_in(checker, &prepared.program)?;
    for advice in adaptive {
        push(checker, &mut prepared, Advice::Adaptive(advice))?;
    }
    let mut planned = decompose_in(checker, &prepared.program, machine, catalog)?;
    prepared.trace.append(&mut planned.trace);
    planned.trace = prepared.trace;
    if let Some(started) = started {
        eprintln!("Sequence plan: {:.6}s", started.elapsed().as_secs_f64());
    }
    Ok(planned)
}

/// Fill only execution decomposition, preserving an explicitly authored join order.
/// Callers with an order-advice phase use this entry rather than selecting another
/// relational order. Its result has the same checked replay and grading contract.
pub fn decompose(program: &Program, machine: &Machine, catalog: &Catalog) -> syn::Result<Planned> {
    decompose_in(&mut Default::default(), program, machine, catalog)
}

fn decompose_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    program: &Program,
    machine: &Machine,
    catalog: &Catalog,
) -> syn::Result<Planned> {
    checker.check(program)?;
    if machine.items == 0 {
        return Err(syn::Error::new(
            machine.name.span(),
            "P-Decompose: machine items must be positive",
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
    // Formal rule PL-Det: Fixed candidate order and stable ties depend only on the three inputs.
    let parallel = !machine.root.children.is_empty();
    let mut choices = vec![(1, false)];
    if parallel && decomposition::width(&machine.root).is_some() {
        if machine.items != 1 {
            choices.push((machine.items, false));
        }
        choices.push((1, true));
        if machine.items != 1 {
            choices.push((machine.items, true));
        }
    }
    let mut best: Option<(Cost, Planned)> = None;
    let mut error = None;
    for (items, segmented) in choices {
        match candidate(
            checker, program, machine, catalog, parallel, items, segmented,
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
                if let Some(error) = &mut error {
                    syn::Error::combine(error, failure);
                } else {
                    error = Some(failure);
                }
            }
        }
    }
    best.map(|(_, planned)| planned).ok_or_else(|| {
        error.unwrap_or_else(|| {
            syn::Error::new(machine.name.span(), "P-Decompose: no legal candidate")
        })
    })
}

fn candidate(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    machine: &Machine,
    catalog: &Catalog,
    parallel: bool,
    items: usize,
    segmented: bool,
) -> syn::Result<(Analysis, Planned)> {
    let mut planned = Planned {
        program: source.clone(),
        trace: Vec::new(),
    };
    let source_facts = checker.check(source)?;
    for advice in placement::boundaries(source, machine, &source_facts) {
        push(checker, &mut planned, advice)?;
    }
    let facts = checker.check(&planned.program)?;
    for advice in placement::advise(&planned.program, machine, parallel, &facts) {
        push(checker, &mut planned, advice)?;
    }
    if parallel {
        for advice in decomposition::advise(checker, &planned.program, machine, items, segmented)? {
            push(checker, &mut planned, advice)?;
        }
    }
    // Formal rule PL-Precedence: Recorded grades, tiles and output are never replaced by generated advice.
    if source.sequence.output.is_none() {
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

fn push(
    checker: &mut cq_lang_sequence::contract::Checker,
    planned: &mut Planned,
    advice: Advice,
) -> syn::Result<()> {
    planned.program = advice.apply_in(checker, &planned.program)?;
    planned.trace.push(advice);
    Ok(())
}

fn variable_output(term: &Term, program: &Program) -> bool {
    match term {
        Term::Canon(_) | Term::Compact(_, _) => true,
        Term::Rows(_) => false,
        Term::Ref(name) => program
            .sequence
            .definitions
            .iter()
            .find(|d| symbol_name(&d.name) == symbol_name(name))
            .is_some_and(|d| variable_output(&d.term, program)),
        Term::Map(_, t)
        | Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::Group(_, t)
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. }
        | Term::Lookup { inner: t, .. } => variable_output(t, program),
        Term::Union(terms) => terms.iter().any(|t| variable_output(t, program)),
        Term::Choose(arms) => arms.iter().any(|(_, t)| variable_output(t, program)),
    }
}
