//! Default Structures → Sequence lowering: one part, with only Physical stores.
//!
//! An authored probe chain
//! ```text
//! l = pipe rename L {a -> a};
//! r = let rename R {a -> a, b -> b};
//! j = pipe probe first [a] l,r;
//! out = pipe canon project j keep {b};
//! return out as answer(b);
//! ```
//! becomes (with Physical and Structures supplied separately)
//! ```text
//! sequence {
//!     l = rows l;
//!     r = store rows r;
//!     j = flatten lookup r [a] l;
//!     out = canon map out j;
//!     __sequence_output = store out;
//!     output count;
//! }
//! ```
//! Membership reads use `compact result lookup provider [keys] input`;
//! the named Physical semi/anti operator supplies the predicate's polarity.
//! A Probe with a recorded raw complete-row key or membership projection uses
//! the same flat Compact shape. Only payload-enumerating lookups introduce an
//! explicit Flatten; Rust provider types do not determine the read protocol.

use cq_lang_sequence::{Definition, OutputProtocol, Program, Sequence, Term};
use cq_lang_structures::physical::{ModeKind, Operator, symbol_name};

/// Lower separate Physical and Structures programs, retaining only stores selected by Physical modes.
pub fn compile(physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program) -> syn::Result<Program> {
    let analysis = cq_lang_structures::contract::check(physical, source)?;
    let physical_program = physical;
    let physical = &physical.program;
    let mut definitions = Vec::with_capacity(physical.definitions.len() + 1);
    // Formal rule L-Sequence: One part, Physical retention, explicit lookup expansion and final count store.
    for definition in &physical.definitions {
        let name = &definition.name;
        let key = symbol_name(name);
        let mut term = if analysis.physical.hosts.contains_key(&key) {
            Term::Rows(name.clone())
        } else {
            match &definition.operator {
                Operator::Unit(_) => Term::Rows(name.clone()),
                Operator::Union(union) => {
                    Term::Union(union.operands.iter().cloned().map(Term::Ref).collect())
                }
                Operator::Probe(_, keyed)
                | Operator::Merge(_, keyed)
                | Operator::Semi(_, keyed)
                | Operator::Anti(_, keyed) => {
                    let mut keys: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                    keys.sort();
                    let membership = matches!(
                        definition.operator,
                        Operator::Semi(_, _) | Operator::Anti(_, _)
                    ) || analysis
                        .membership
                        .contains(&(symbol_name(&keyed.right), keys));
                    let lookup = Box::new(Term::Lookup {
                        provider: keyed.right.clone(),
                        keys: keyed.columns.iter().cloned().collect(),
                        inner: Box::new(Term::Ref(keyed.left.clone())),
                    });
                    if membership {
                        Term::Compact(name.clone(), lookup)
                    } else {
                        Term::Flatten(lookup)
                    }
                }
                Operator::Group(group) => {
                    Term::Group(name.clone(), Box::new(Term::Ref(group.input.clone())))
                }
                Operator::Select(select) => {
                    Term::Compact(name.clone(), Box::new(Term::Ref(select.input.clone())))
                }
                operator => {
                    let input = operator.input().ok_or_else(|| {
                        syn::Error::new_spanned(name, "L-Sequence: operator has no Sequence input")
                    })?;
                    Term::Map(name.clone(), Box::new(Term::Ref(input.clone())))
                }
            }
        };
        if definition.canon.is_some() {
            term = Term::Canon(Box::new(term));
        }
        if retained(&analysis, definition) {
            term = Term::Store {
                capacity: None,
                inner: Box::new(term),
            };
        }
        definitions.push(Definition {
            name: name.clone(),
            term,
        });
    }
    let mut output_name = "__sequence_output".to_owned();
    while analysis.physical.headings.contains_key(&output_name) {
        output_name.push('_');
    }
    let mut output = Term::Ref(physical.returned.clone());
    if physical.return_canon.is_some() {
        output = Term::Canon(Box::new(output));
    }
    definitions.push(Definition {
        name: syn::Ident::new(&output_name, physical.returned.span()),
        term: Term::Store {
            capacity: None,
            inner: Box::new(output),
        },
    });
    let program = Program {
        sequence: Sequence {
            definitions,
            roots: Default::default(),
            output: Some(OutputProtocol::Count),
        },
        graded: None,
        evidence: Default::default(),
        representation: cq_lang_sequence::Representation {
            blocks: cq_lang_sequence::blocks::Execution::record(physical_program, source),
            ..Default::default()
        },
    };
    cq_lang_sequence::contract::check(physical_program, source, &program)?;
    Ok(program)
}

/// Check the supplied lowering's coverage, operators, retention and output
/// obligations directly. The compiler is not invoked; fresh output names are free.
pub fn check(physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program, target: &Program) -> syn::Result<()> {
    let analysis = cq_lang_structures::contract::check(physical, source)?;
    cq_lang_sequence::contract::check(physical, source, target)?;
    let invalid = || {
        syn::Error::new_spanned(
            target,
            "L-Sequence: target violates default lowering obligations",
        )
    };
    let physical = &physical.program;
    if target.graded.is_some()
        || target.evidence != Default::default()
        || target.sequence.output != Some(OutputProtocol::Count)
        || target.sequence.definitions.len() != physical.definitions.len() + 1
    {
        return Err(invalid());
    }
    let is_ref =
        |term: &Term, name: &syn::Ident| matches!(term, Term::Ref(reference) if reference == name);
    for (definition, lowered) in physical
        .definitions
        .iter()
        .zip(&target.sequence.definitions)
    {
        if definition.name != lowered.name {
            return Err(invalid());
        }
        let term = lowering_body(
            &lowered.term,
            retained(&analysis, definition),
            definition.canon.is_some(),
        )
        .ok_or_else(invalid)?;
        let name = &definition.name;
        let valid = if analysis.physical.hosts.contains_key(&symbol_name(name))
            || matches!(definition.operator, Operator::Unit(_))
        {
            matches!(term, Term::Rows(result) if result == name)
        } else {
            match (&definition.operator, term) {
                (Operator::Union(union), Term::Union(inputs)) => {
                    inputs.len() == union.operands.len()
                        && inputs
                            .iter()
                            .zip(&union.operands)
                            .all(|(input, source)| is_ref(input, source))
                }
                (Operator::Group(group), Term::Group(result, input)) => {
                    result == name && is_ref(input, &group.input)
                }
                (Operator::Select(select), Term::Compact(result, input)) => {
                    result == name && is_ref(input, &select.input)
                }
                (
                    Operator::Probe(_, keyed)
                    | Operator::Merge(_, keyed)
                    | Operator::Semi(_, keyed)
                    | Operator::Anti(_, keyed),
                    Term::Flatten(lookup) | Term::Compact(_, lookup),
                ) => {
                    let Term::Lookup {
                        provider,
                        keys,
                        inner,
                    } = lookup.as_ref()
                    else {
                        return Err(invalid());
                    };
                    let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                    columns.sort();
                    let membership =
                        matches!(definition.operator, Operator::Semi(..) | Operator::Anti(..))
                            || analysis
                                .membership
                                .contains(&(symbol_name(&keyed.right), columns));
                    let expansion = match term {
                        Term::Compact(result, _) => membership && result == name,
                        Term::Flatten(_) => !membership,
                        _ => unreachable!("matched lookup consumer"),
                    };
                    expansion
                        && provider == &keyed.right
                        && keys.iter().eq(keyed.columns.iter())
                        && is_ref(inner, &keyed.left)
                }
                (operator, Term::Map(result, input))
                    if !matches!(
                        operator,
                        Operator::Union(_)
                            | Operator::Group(_)
                            | Operator::Select(_)
                            | Operator::Probe(..)
                            | Operator::Merge(..)
                            | Operator::Semi(..)
                            | Operator::Anti(..)
                    ) =>
                {
                    result == name && operator.input().is_some_and(|source| is_ref(input, source))
                }
                _ => false,
            }
        };
        if !valid {
            return Err(invalid());
        }
    }
    let output = target.sequence.definitions.last().ok_or_else(invalid)?;
    if !lowering_body(&output.term, true, physical.return_canon.is_some())
        .is_some_and(|term| is_ref(term, &physical.returned))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Physical retention, except STR-RowsByValue: a canon `let` whose canonical
/// rows record is bound by value is not storage. Like a canon pipe, its
/// distinct-row build streams into its single reader.
fn retained(
    analysis: &cq_lang_structures::Analysis,
    definition: &cq_lang_structures::physical::Definition,
) -> bool {
    ModeKind::from(&definition.mode) != ModeKind::Pipe
        && !(definition.canon.is_some() && analysis.rows_by_value(&symbol_name(&definition.name)))
}

fn lowering_body(mut term: &Term, retained: bool, canonical: bool) -> Option<&Term> {
    if retained {
        let Term::Store {
            capacity: None,
            inner,
        } = term
        else {
            return None;
        };
        term = inner;
    }
    if canonical {
        let Term::Canon(inner) = term else {
            return None;
        };
        term = inner;
    }
    Some(term)
}
