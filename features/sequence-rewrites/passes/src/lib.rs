//! Advice-driven Sequence rewrites. Every entry point checks source, target and
//! construction-based support equality; machine validation belongs to grading.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use cq_lang_sequence::{Grade, Graded, Guard, Len, OutputProtocol, Program, Term};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token};

mod decomposition;
pub use decomposition::{partition, partition_in, segment, segment_in};

mod selection;
use selection::Results;

mod catalog;
pub use catalog::select_from_catalog;

mod evidence;
pub use evidence::{evidence, record_evidence, record_evidence_in};

mod group;
pub use group::{group_without_canon, group_without_canon_in};

mod key_partition;
pub use key_partition::{KeyPartition, key_partition, key_partition_in};

mod adaptive;
pub use adaptive::{
    AdaptiveCandidate, adaptive, adaptive_candidates, adaptive_candidates_in, adaptive_choose,
    adaptive_filter, apply_adaptive, apply_adaptive_in, select_filter,
};

mod lookup_first;
pub use lookup_first::{
    LookupFirst, LookupPlan, lookup_first, lookup_first_advice, lookup_first_advice_in,
    lookup_first_in, lookup_plan,
};

mod kw {
    syn::custom_keyword!(items);
    syn::custom_keyword!(at);
}

fn checked_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    target: Program,
) -> syn::Result<Program> {
    checker.check_rewrite_in(source, &target)?;
    Ok(target)
}

fn definition(program: &Program, name: &Ident) -> syn::Result<usize> {
    program
        .sequence
        .definitions
        .iter()
        .position(|definition| symbol_name(&definition.name) == symbol_name(name))
        .ok_or_else(|| syn::Error::new_spanned(name, "unknown Sequence definition"))
}

/// Materialize one explicitly named definition without changing its producer.
/// The construction-based rewrite checker validates the storage boundary;
/// target-specific placement remains the caller's separate grading obligation.
// Materialization preserves the producer's support (EXT-SeqStore).
pub fn store(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    store_in(&mut Default::default(), source, arguments)
}

pub fn store_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let name: Ident = syn::parse2(arguments)?;
    let index = definition(source, &name)?;
    let mut target = source.clone();
    let term = &mut target.sequence.definitions[index].term;
    if !matches!(term, Term::Store { .. }) {
        *term = Term::Store {
            capacity: None,
            inner: Box::new(term.clone()),
        };
    }
    checked_in(checker, source, target)
}

/// Materialize selected flatten definitions. Empty advice or `all` selects all
/// flatten definitions; named selections must all exist and be flatten terms.
// Formal rule EXT-SeqStore: Materialize flatten levels or factor named publication producers with unchanged support.
pub fn store_level(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    store_level_in(&mut Default::default(), source, arguments)
}

pub fn store_level_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let selection = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    for definition in &mut target.sequence.definitions {
        if selection.includes(&definition.name) && matches!(definition.term, Term::Flatten(_)) {
            definition.term = Term::Store {
                capacity: None,
                inner: Box::new(definition.term.clone()),
            };
            matched.insert(symbol_name(&definition.name));
        }
    }
    selection.require_matches(&matched)?;
    checked_in(checker, source, target)
}

/// Name a store's producer so its execution grade can differ from publication.
/// Factoring a definition and inserting a store preserve support (EXT-SeqStore).
pub fn publish(source: &Program, name: &Ident, producer: &Ident) -> syn::Result<Program> {
    publish_in(&mut Default::default(), source, name, producer)
}

pub fn publish_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    name: &Ident,
    producer: &Ident,
) -> syn::Result<Program> {
    let index = definition(source, name)?;
    if source
        .sequence
        .definitions
        .iter()
        .any(|definition| definition.name == *producer)
    {
        return Err(syn::Error::new_spanned(
            producer,
            "publication producer name already exists",
        ));
    }
    let mut target = source.clone();
    let term = &mut target.sequence.definitions[index].term;
    let inner = match term {
        Term::Store { inner, .. } => std::mem::replace(inner.as_mut(), Term::Ref(producer.clone())),
        _ => std::mem::replace(
            term,
            Term::Store {
                capacity: None,
                inner: Box::new(Term::Ref(producer.clone())),
            },
        ),
    };
    target.sequence.definitions.insert(
        index,
        cq_lang_sequence::Definition {
            name: producer.clone(),
            term: inner,
        },
    );
    checked_in(checker, source, target)
}

struct Tile {
    name: Ident,
    n: usize,
    items: Option<usize>,
}

impl Parse for Tile {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let n = input.parse::<LitInt>()?.base10_parse()?;
        let items = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            input.parse::<kw::items>()?;
            Some(input.parse::<LitInt>()?.base10_parse()?)
        } else {
            None
        };
        Ok(Self { name, n, items })
    }
}

/// Wrap `d` in a positive contiguous tile: `d, n` or `d, n, items m`.
/// Unknown-length stores require a capacity before they can be tiled.
// Formal rule EXT-SeqTile: Positive contiguous tiles preserve the covered support.
pub fn tile(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    tile_in(&mut Default::default(), source, arguments)
}

pub fn tile_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let Tile { name, n, items } = syn::parse2(arguments)?;
    let index = definition(source, &name)?;
    let mut target = source.clone();
    let analysis = checker.check_shared(source)?;
    if matches!(
        source.sequence.definitions[index].term,
        Term::Store { capacity: None, .. }
    ) && matches!(analysis.lengths[&symbol_name(&name)], Len::Unknown(None))
    {
        return Err(syn::Error::new_spanned(
            name,
            "BE-Length: tiling an unknown-length store requires capacity",
        ));
    }
    let term = &mut target.sequence.definitions[index].term;
    *term = Term::Tile {
        n,
        items,
        inner: Box::new(term.clone()),
    };
    checked_in(checker, source, target)
}

struct Choose {
    name: Ident,
    guard: Guard,
    yes: Ident,
    no: Ident,
}

impl Parse for Choose {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let guard = input.parse()?;
        if !matches!(guard, Guard::Lt(Len::Input(_), _) | Guard::WorkersLt(_)) {
            return Err(input.error("expected len x < k or workers < k"));
        }
        input.parse::<Token![=>]>()?;
        let yes = input.parse()?;
        input.parse::<Token![,]>()?;
        input.parse::<Token![else]>()?;
        input.parse::<Token![=>]>()?;
        let no = input.parse()?;
        Ok(Self {
            name,
            guard,
            yes,
            no,
        })
    }
}

/// Replace `d` by a length- or worker-guarded choice with an `else` fallback.
/// The two earlier definitions must denote the same Structures result, not just its heading.
// Formal rule EXT-SeqChoose: Exhaustive scheduling guards select equal-support constructions.
pub fn choose(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    choose_in(&mut Default::default(), source, arguments)
}

pub fn choose_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let Choose {
        name,
        guard,
        yes,
        no,
    } = syn::parse2(arguments)?;
    let index = definition(source, &name)?;
    let mut target = source.clone();
    target.sequence.definitions[index].term =
        Term::Choose(vec![(guard, Term::Ref(yes)), (Guard::Else, Term::Ref(no))]);
    checked_in(checker, source, target)
}

/// Set `count`, `reserve` or `bounded` on the final store. Bounded output
/// requires the store's declared capacity; this pass never invents one.
// Formal rule EXT-SeqOutput: Output protocols change storage interpretation, not support.
pub fn output(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    output_in(&mut Default::default(), source, arguments)
}

pub fn output_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let protocol: OutputProtocol = syn::parse2(arguments)?;
    let mut target = source.clone();
    target.sequence.output = Some(protocol);
    checked_in(checker, source, target)
}

struct Grades {
    machine: Ident,
    grades: Vec<Grade>,
}

impl Parse for Grades {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let machine = input.parse()?;
        let mut grades = Vec::new();
        let mut names = BTreeSet::new();
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            let name: Ident = input.parse()?;
            if !names.insert(symbol_name(&name)) {
                return Err(syn::Error::new_spanned(name, "duplicate grade assignment"));
            }
            input.parse::<kw::at>()?;
            let mut level = vec![input.parse()?];
            while input.peek(Token![.]) {
                input.parse::<Token![.]>()?;
                level.push(input.parse()?);
            }
            grades.push(Grade { name, level });
        }
        Ok(Self { machine, grades })
    }
}

/// Add or update named placements on one machine. Unmentioned placements are
/// preserved. Paths and machine capabilities are checked only by the facade's
/// grade phase; unknown definitions, duplicate advice and machine changes fail.
// Formal rule EXT-SeqGrade: Record machine placements without changing the Sequence construction.
pub fn grade(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    grade_in(&mut Default::default(), source, arguments)
}

pub fn grade_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let Grades { machine, grades } = syn::parse2(arguments)?;
    let mut target = source.clone();
    grade_definitions_in(checker, &mut target, &machine, &grades)?;
    Ok(target)
}

/// Apply typed grade advice without copying unchanged syntax or rendering tokens.
/// Only the nonsemantic grading field is borrowed mutably after its complete
/// structural proof succeeds; machine grading remains a separate obligation.
pub fn grade_definitions_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &mut Program,
    machine: &Ident,
    grades: &[Grade],
) -> syn::Result<()> {
    for grade in grades {
        definition(source, &grade.name)?;
    }
    if let Some(graded) = &source.graded
        && symbol_name(&graded.machine) != symbol_name(machine)
    {
        return Err(syn::Error::new_spanned(
            machine,
            "cannot extend a different graded machine",
        ));
    }
    checker.check_shared(source)?;
    let graded = source.graded.get_or_insert_with(|| Graded {
        machine: machine.clone(),
        grades: Vec::new(),
    });
    for grade in grades {
        let name = symbol_name(&grade.name);
        if let Some(existing) = graded
            .grades
            .iter_mut()
            .find(|existing| symbol_name(&existing.name) == name)
        {
            existing.clone_from(grade);
        } else {
            graded.grades.push(grade.clone());
        }
    }
    Ok(())
}
