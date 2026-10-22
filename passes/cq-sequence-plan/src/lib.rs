//! Bounded, deterministic Sequence decomposition with replayable rewrite advice.
#![forbid(unsafe_code)]

use cq_lang_sequence::{Analysis, Catalog, Machine, OutputProtocol, Program, Term};
use cq_syntax::symbol_name;
use quote::quote;
use syn::Ident;

mod catalog;
pub mod cost;
pub use cost::{Cost, cost};
mod decomposition;
mod placement;

/// One invocation of the same checked library function exposed by a rewrite macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advice {
    Store(Ident),
    StoreLevel(Ident),
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
        use cq_sequence_rewrite_passes as rewrites;
        let result = match self {
            Self::Store(name) => rewrites::store(source, quote!(#name)),
            Self::StoreLevel(name) => rewrites::store_level(source, quote!(#name)),
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
                    Self::Tile { .. } => rewrites::tile(source, arguments),
                    Self::Partition { .. } => rewrites::partition(source, arguments),
                    _ => rewrites::segment(source, arguments),
                }
            }
            Self::Output(protocol) => rewrites::output(source, quote!(#protocol)),
            Self::Grade {
                name,
                level,
                machine,
            } => rewrites::grade(source, quote!(#machine, #name at #(#level).*)),
        };
        result.map_err(|error| {
            syn::Error::new(error.span(), format!("P-Decompose {self:?}: {error}"))
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
    cq_lang_sequence::contract::check(program)?;
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
        match candidate(program, machine, catalog, parallel, items, segmented) {
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
    for advice in placement::advise(source, machine, parallel) {
        push(&mut planned, advice)?;
    }
    if parallel {
        for advice in decomposition::advise(&planned.program, machine, items, segmented)? {
            push(&mut planned, advice)?;
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
        push(&mut planned, Advice::Output(protocol))?;
    }
    // Formal rule PL-Closure: Each step uses a checked public rewrite; the completed trace satisfies machine grading.
    let analysis = cq_lang_sequence::check_grading(&planned.program, machine)?;
    catalog::check(&planned.program, &analysis, catalog)?;
    Ok((analysis, planned))
}

fn push(planned: &mut Planned, advice: Advice) -> syn::Result<()> {
    planned.program = advice.apply(&planned.program)?;
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
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. }
        | Term::Lookup { inner: t, .. } => variable_output(t, program),
        Term::Union(terms) => terms.iter().any(|t| variable_output(t, program)),
        Term::Choose(arms) => arms.iter().any(|(_, t)| variable_output(t, program)),
    }
}
