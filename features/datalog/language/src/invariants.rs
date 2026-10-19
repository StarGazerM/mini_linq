//! Independent checking and erasure of bounded frozen-filter certificates.
use super::*;

/// Ownership spelling of a projected declared column, not a target rewrite.
/// Primitive Copy scalars use the same dereference as authored Datalog heads;
/// generic columns retain the ordinary value-preserving Clone requirement.
pub fn owned_column(variable: &Ident, ty: &syn::Type) -> Expr {
    fn primitive(ty: &syn::Type, names: &[&str]) -> bool {
        let syn::Type::Path(path) = ty else {
            return false;
        };
        if path.qself.is_some() || path.path.segments.iter().any(|s| !s.arguments.is_empty()) {
            return false;
        }
        let segments = &path.path.segments;
        let canonical_path = segments.len() == 1
            || (segments.len() == 3
                && (segments[0].ident == "std" || segments[0].ident == "core")
                && segments[1].ident == "primitive");
        canonical_path
            && segments
                .last()
                .is_some_and(|s| names.iter().any(|name| s.ident == *name))
    }
    let copy = match ty {
        syn::Type::Paren(p) => return owned_column(variable, &p.elem),
        syn::Type::Group(g) => return owned_column(variable, &g.elem),
        syn::Type::Reference(r) => {
            r.mutability.is_none()
                && r.lifetime.as_ref().is_some_and(|l| l.ident == "static")
                && primitive(&r.elem, &["str"])
        }
        _ => primitive(
            ty,
            &[
                "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
                "usize", "bool", "char",
            ],
        ),
    };
    if copy {
        syn::parse_quote!(*#variable)
    } else {
        syn::parse_quote!((*#variable).clone())
    }
}

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
    original
        .rules
        .retain(|q| !helpers.contains(&symbol_name(&q.head.relation)));
    original
        .declarations
        .retain(|d| !helpers.contains(&symbol_name(&d.name)));
    if let Some(strata) = &mut original.strata {
        for component in strata.iter_mut() {
            component.retain(|r| !helpers.contains(&symbol_name(r)));
        }
        strata.retain(|c| !c.is_empty());
    }
    original.invariants.clear();
    // Delta/layout decisions belong to the factored program, not its erasure.
    original.variants = None;
    original.deltas.clear();
    original.layouts.clear();
    original.delta_layouts.clear();
    original.insertion = None;
    original.negation_keys = None;
    Ok(original)
}

pub fn check(program: &Program) -> syn::Result<()> {
    if program.invariants.is_empty() {
        return Ok(());
    }
    ensure(
        program.strata.is_some(),
        program,
        "invariants require strata",
    )?;
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
        let component = analysis.component[head];
        let frozen = |id: &Ident| {
            analysis.positions.get(&symbol_name(id)).is_some_and(|&i| {
                analysis.component[i].is_none() || analysis.component[i] < component
            })
        };
        ensure(
            !analysis.changing_positions[entry.rule].is_empty(),
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
        let expected_head: Vec<Expr> = input
            .args
            .iter()
            .zip(input_decl.columns.iter())
            .filter_map(|(arg, column)| rust::variable(arg).map(|v| owned_column(v, &column.ty)))
            .collect();
        ensure(
            definition.head.args.iter().eq(expected_head.iter()),
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
            analysis.changing_positions[entry.rule]
                .iter()
                .any(|&i| i < first)
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
    // The ordinary graph checker validates helper order and exact SCCs. Erasure
    // above independently validates the unchanged original SCC partition.
    Ok(())
}
