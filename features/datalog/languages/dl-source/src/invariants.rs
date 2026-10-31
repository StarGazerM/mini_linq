//! Independent checking and erasure of bounded frozen-filter certificates.
//! DESIGN contract7-invariant-projection
//! Contract 7, doc/PLANNING-REDESIGN.md and DL-InvariantProjection:
//! helpers project relational variables; DL-AnswerOwnership owns realization.
//! DO NOT choose copy syntax from host types or recognize Rust copy expressions.
use super::*;

/// Erase helpers and restore the certified source rules. Callers must check first.
pub fn erase(program: &Program) -> syn::Result<Program> {
    let mut original = program.clone();
    let helpers: BTreeSet<_> = program
        .invariants
        .iter()
        .map(|h| symbol_name(&h.helper))
        .collect();
    for entry in &program.invariants {
        ensure(
            entry.rule < original.rules.len(),
            program,
            "invalid invariant rule",
        )?;
        original.rules[entry.rule] = entry.source.clone();
    }
    let kept: Vec<_> = original
        .rules
        .iter()
        .map(|q| !helpers.contains(&symbol_name(&q.head.relation)))
        .collect();
    let mut keep = kept.iter().copied();
    original.rules.retain(|_| keep.next().unwrap_or(true));
    let mut keep = kept.iter().copied();
    original.labels.retain(|_| keep.next().unwrap_or(true));
    original
        .declarations
        .retain(|d| !helpers.contains(&symbol_name(&d.name)));
    original.models.retain(|name, _| !helpers.contains(name));
    original.invariants.clear();
    Ok(original)
}

pub fn check(program: &Program) -> syn::Result<()> {
    if program.invariants.is_empty() {
        return Ok(());
    }
    let original = erase(program)?;
    let analysis = contract::check(&original)?;
    let mut rules = BTreeSet::new();
    let mut helpers = BTreeSet::new();
    for entry in &program.invariants {
        ensure(
            entry.rule < original.rules.len()
                && rules.insert(entry.rule)
                && helpers.insert(symbol_name(&entry.helper)),
            program,
            "duplicate invariant certificate",
        )?;
        ensure(
            !analysis.positions.contains_key(&symbol_name(&entry.helper))
                && !program.outputs.contains(&entry.helper),
            program,
            "invariant helper is not private and fresh",
        )?;
        let head = analysis.positions[&symbol_name(&entry.source.head.relation)];
        // An input outside the consumer SCC is frozen in the source fixpoint.
        // DL-Strata separately checks that its recorded component finishes first.
        let frozen = |id: &Ident| {
            analysis.positions.get(&symbol_name(id)).is_some_and(|&i| {
                original.declarations[i].value.is_some()
                    || (i != head && !analysis.graph.reach[i][head])
            })
        };
        let changing_positions: Vec<_> = entry
            .source
            .body
            .iter()
            .enumerate()
            .filter_map(|(i, item)| match item {
                BodyItem::Positive { atom } if !frozen(&atom.relation) => Some(i),
                _ => None,
            })
            .collect();
        ensure(
            !changing_positions.is_empty(),
            program,
            "invariant consumer must be recursive",
        )?;
        let definitions: Vec<_> = program
            .rules
            .iter()
            .filter(|q| q.head.relation == entry.helper)
            .collect();
        ensure(
            definitions.len() == 1,
            program,
            "invariant requires exactly one definition",
        )?;
        let definition = definitions[0];
        let Some(BodyItem::Positive { atom: input }) = definition.body.first() else {
            return Err(syn::Error::new_spanned(
                program,
                "invariant must start with one frozen positive atom",
            ));
        };
        let vars: Vec<_> = input.variables().collect();
        let names: BTreeSet<_> = vars.iter().map(|v| symbol_name(v)).collect();
        ensure(
            vars.len() == names.len() && !vars.is_empty() && frozen(&input.relation),
            program,
            "invariant positive input must be frozen with distinct named columns",
        )?;
        ensure(
            definition.body.len() > 1,
            program,
            "invariant must perform a filter",
        )?;
        for item in definition.body.iter().skip(1) {
            let BodyItem::Negation(negative) = item else {
                return Err(syn::Error::new_spanned(
                    program,
                    "bounded invariant permits only one positive input",
                ));
            };
            ensure(
                frozen(&negative.atom.relation)
                    && negative
                        .atom
                        .variables()
                        .all(|v| names.contains(&symbol_name(v))),
                program,
                "invariant negation is not frozen and locally bound",
            )?;
        }
        let input_decl = &original.declarations[analysis.positions[&symbol_name(&input.relation)]];
        // The helper records relational projection, not a Rust copy expression.
        // DL-AnswerOwnership realizes the declared-owned result downstream.
        // Formal rule DL-InvariantProjection: preserve positive-input variables
        // in column order, as relational head variables rather than Rust copies.
        let expected_head = input.args.iter().filter(|arg| rust::variable(arg).is_some());
        ensure(
            definition.head.args.iter().eq(expected_head),
            program,
            "invariant must preserve all positive bindings",
        )?;
        let mut rewritten = entry.source.clone();
        let mut positions = Vec::new();
        for item in &definition.body {
            let candidates: Vec<_> = entry
                .source
                .body
                .iter()
                .enumerate()
                .filter(|(i, source)| !positions.contains(i) && *source == item)
                .map(|(i, _)| i)
                .collect();
            ensure(
                !candidates.is_empty(),
                program,
                "invariant body is not a source sub-body",
            )?;
            positions.push(candidates[0]);
        }
        ensure(
            positions[1..].windows(2).all(|p| p[0] < p[1]),
            program,
            "invariant changed filter order",
        )?;
        let first = positions[0];
        let frozen_positives = entry
            .source
            .body
            .iter()
            .filter(|item| matches!(item, BodyItem::Positive { atom } if frozen(&atom.relation)))
            .count();
        ensure(
            frozen_positives == 1,
            program,
            "invariant must contain the maximal single-positive frozen side",
        )?;
        ensure(
            entry
                .source
                .body
                .iter()
                .enumerate()
                .all(|(i, item)| match item {
                    BodyItem::Negation(n)
                        if frozen(&n.atom.relation)
                            && n.atom.variables().all(|v| names.contains(&symbol_name(v))) =>
                    {
                        positions.contains(&i)
                    }
                    _ => true,
                }),
            program,
            "invariant omitted a locally bound frozen negative filter",
        )?;
        let mut prior = BTreeSet::new();
        for item in entry.source.body.iter().take(first) {
            if let BodyItem::Positive { atom } = item {
                prior.extend(atom.variables().map(symbol_name));
            }
        }
        ensure(
            changing_positions.iter().any(|&i| i < first)
                && names.iter().any(|v| prior.contains(v)),
            program,
            "invariant must be a keyed side after a changing driver",
        )?;
        // No opaque clauses may be crossed, and the maximal frozen positive side
        // must be this single input (otherwise there is no cardinality bound).
        ensure(
            entry.source.body.iter().all(|item| match item {
                BodyItem::Positive { atom } => !frozen(&atom.relation) || atom == input,
                BodyItem::Negation(_) => true,
                _ => false,
            }),
            program,
            "invariant would cross an opaque clause or expand a frozen join",
        )?;
        let helper = &entry.helper;
        let replacement: BodyItem = syn::parse2(quote!(#helper(#(#vars),*)))?;
        rewritten.body = rewritten
            .body
            .into_iter()
            .enumerate()
            .filter_map(|(i, item)| {
                if i == first {
                    Some(replacement.clone())
                } else if positions.contains(&i) {
                    None
                } else {
                    Some(item)
                }
            })
            .collect();
        ensure(
            program.rules[entry.rule] == rewritten,
            program,
            "invariant replacement changed variable bindings or residual body",
        )?;
        // Check exact projected schema, independently of Rust's later type checking.
        let columns: Vec<_> = input
            .args
            .iter()
            .zip(input_decl.columns.iter())
            .filter_map(|(arg, col)| {
                rust::variable(arg).map(|var| {
                    let ty = &col.ty;
                    quote!(#var: #ty)
                })
            })
            .collect();
        let expected: RelationDecl = syn::parse2(quote!(relation #helper(#(#columns),*);))?;
        ensure(
            program
                .declarations
                .iter()
                .filter(|d| d.name == *helper)
                .eq(std::iter::once(&expected)),
            program,
            "invariant helper schema changed",
        )?;
        ensure(
            program.rules.iter().enumerate().all(|(i, q)| {
                i == entry.rule
                    || q.body.iter().all(|item| match item {
                        BodyItem::Positive { atom } => atom.relation != *helper,
                        BodyItem::Negation(n) => n.atom.relation != *helper,
                        _ => true,
                    })
            }),
            program,
            "invariant helper has an uncertified consumer",
        )?;
    }
    // The enclosing language contract checks the factored source independently;
    // DL-Strata additionally checks helper order and the erased partition.
    Ok(())
}
