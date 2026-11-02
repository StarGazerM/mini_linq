use super::*;
use cq_datalog_support::{Fresh, columns, equalities, filters};

// Independently derive the exact input schema from the hole's relational reads.
// Do not use the fork producer's declaration builder here. Layout and key-set
// inputs declare their token's host reads on their recorded key (IF-HostDecl);
// a key set collected by a non-batch key hole stores borrowed key columns.
fn required_inputs(
    tokens: &[storage::provider::Token],
    holes: &[Hole],
    context: &RoundContext,
    keys: &[KeySet],
    query: &cq::Query,
) -> syn::Result<Vec<RelationDecl>> {
    let required: BTreeSet<_> = query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Positive { atom } => Some(symbol_name(&atom.relation)),
            BodyItem::Negation(negative) => Some(symbol_name(&negative.atom.relation)),
            _ => None,
        })
        .collect();
    let mut declarations = Vec::new();
    for relation in &context.relations {
        if let Some(delta) = &relation.delta {
            if !required.contains(&symbol_name(delta)) {
                continue;
            }
            let mut declaration = relation.declaration.clone();
            declaration.name = delta.clone();
            let host = relation.delta_host.as_ref().expect("checked delta host");
            declaration.value = Some(syn::parse_quote!(= (&#host)));
            declaration.hosts = Vec::new();
            declarations.push(declaration);
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        if !required.contains(&symbol_name(&layout.name)) {
            continue;
        }
        let mut declaration = context.relations[layout.relation].declaration.clone();
        declaration.name = layout.name.clone();
        declaration.columns = layout
            .columns
            .iter()
            .map(|&i| declaration.columns[i].clone())
            .collect();
        let host = &layout.host;
        declaration.value = Some(syn::parse_quote!(= (&#host)));
        declaration.hosts =
            declared_hosts(tokens, &layout.provider, &declaration, layout.key, false)?;
        declarations.push(declaration);
    }
    for key in keys {
        if !required.contains(&symbol_name(&key.name)) {
            continue;
        }
        let mut declaration = context.relations[key.relation].declaration.clone();
        declaration.name = key.name.clone();
        declaration.columns = if key.columns.is_empty() {
            let column = &declaration.columns[0].name;
            syn::parse_quote!(#column: ())
        } else {
            key.columns
                .iter()
                .map(|&i| declaration.columns[i].clone())
                .collect()
        };
        let host = &key.name;
        declaration.value = Some(syn::parse_quote!(= (#host)));
        // A unit key (no projected columns) stores an owned `()`.
        let borrowed = !key.columns.is_empty()
            && holes
                .get(key.hole)
                .is_some_and(|hole| hole.interface != HoleInterface::Batch);
        let arity = declaration.arity();
        declaration.hosts = declared_hosts(tokens, &key.provider, &declaration, arity, borrowed)?;
        declarations.push(declaration);
    }
    Ok(declarations)
}

fn declared_hosts(
    tokens: &[storage::provider::Token],
    provider: &storage::Provider,
    declaration: &RelationDecl,
    key: usize,
    borrowed: bool,
) -> syn::Result<Vec<cq_syntax::HostDecl>> {
    let token = storage::provider::lookup(tokens, &provider.token)?;
    let key: Vec<Ident> = declaration
        .columns
        .iter()
        .take(key)
        .map(|c| c.name.clone())
        .collect();
    Ok(storage::provider::host_decl(token, &key, borrowed)
        .into_iter()
        .collect())
}

/// Check complete CQ contracts, schedule preservation, key projections, and
/// variant erasure without rerunning the producer. Runtime names may be freely
/// renamed provided they remain fresh and all recorded references agree.
pub fn check_storage(source: &storage::Program, target: &Program) -> syn::Result<()> {
    let analysis = storage::contract::check(source)?;
    check_storage_with_analysis(source, target, &analysis)
}

pub(crate) fn check_storage_with_analysis(
    source: &storage::Program,
    target: &Program,
    analysis: &storage::Analysis,
) -> syn::Result<()> {
    let variants = &source.incremental.variants;
    let context = &target.context;
    ensure(
        context.relations.len() == source.incremental.strata.source.declarations.len()
            && context.strata.len() == analysis.components.len(),
        source,
        "fork changed relation or stratum coverage",
    )?;
    ensure(
        context
            .outputs
            .iter()
            .map(|output| output.relation)
            .collect::<Vec<_>>()
            == source
                .incremental
                .strata
                .source
                .outputs
                .iter()
                .map(|id| analysis.positions[&symbol_name(id)])
                .collect::<Vec<_>>()
            && context.observer == source.incremental.strata.source.observer,
        source,
        "fork changed output or observer",
    )?;
    let mut names = Fresh::new(source);
    let mut add_name = |id: &Ident| {
        ensure(
            names.reserve(id),
            source,
            "fork runtime names must be fresh and distinct",
        )
    };
    add_name(&context.observer_name)?;
    add_name(&context.round_name)?;
    for ((relation, declaration), providers) in context
        .relations
        .iter()
        .zip(&source.incremental.strata.source.declarations)
        .zip(&source.providers)
    {
        ensure(
            relation.declaration == *declaration
                && relation.delta.as_ref() == analysis.deltas.get(&symbol_name(&declaration.name))
                && relation.providers == *providers,
            source,
            "fork changed relation schema or delta binding",
        )?;
        add_name(&relation.next_canonical)?;
        add_name(&relation.next)?;
        ensure(
            relation.delta_host.is_some() == relation.delta.is_some(),
            source,
            "fork changed delta host coverage",
        )?;
        if let Some(host) = &relation.delta_host {
            add_name(host)?;
        }
    }
    ensure(
        context.layouts.len() == source.layouts.len()
            && context.delta_layouts.len() == source.delta_layouts.len(),
        source,
        "fork changed layout coverage",
    )?;
    for (layout, expected) in context
        .layouts
        .iter()
        .zip(&source.layouts)
        .chain(context.delta_layouts.iter().zip(&source.delta_layouts))
    {
        ensure(
            layout.relation == analysis.positions[&symbol_name(&expected.relation)]
                && layout.columns == expected.columns
                && layout.key == expected.key
                && layout.provider == expected.provider
                && layout.hasher == expected.hasher
                && layout.maintenance == expected.maintenance
                && layout.protocol == expected.protocol,
            source,
            "fork changed a selected host layout",
        )?;
        add_name(&layout.name)?;
        add_name(&layout.host)?;
    }
    let mut used_holes = BTreeSet::new();
    let mut used_variants = BTreeSet::new();
    for (stratum, members) in context.strata.iter().zip(&analysis.components) {
        ensure(
            stratum.members == *members,
            source,
            "fork changed component membership",
        )?;
        let expected_seeds: Vec<_> = variants
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                v.delta.is_none()
                    && members.contains(&analysis.positions[&symbol_name(&v.query.head.relation)])
            })
            .map(|(i, _)| i)
            .collect();
        let expected_steps: Vec<_> = variants
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                v.delta.is_some()
                    && members.contains(&analysis.positions[&symbol_name(&v.query.head.relation)])
            })
            .map(|(i, _)| i)
            .collect();
        ensure(
            stratum.seeds.iter().map(|r| r.variant).eq(expected_seeds)
                && stratum.steps.iter().map(|r| r.variant).eq(expected_steps),
            source,
            "fork changed written variant order",
        )?;
        let mut projections = Vec::new();
        let mut used_keys = BTreeSet::new();
        for key in &stratum.keys {
            add_name(&key.name)?;
            // A key set is a declared token over the inferred row `_` (no payload).
            storage::provider::check_instance(
                &source.tokens,
                &key.provider,
                &syn::parse_quote!(_),
                &syn::parse_quote!(()),
                key.hasher,
            )?;
            ensure(
                source.negations.iter().any(|negative| {
                    analysis.components.get(negative.component) == Some(members)
                        && negative.relation == key.relation
                        && negative.columns == key.columns
                        && negative.filters == key.filters
                        && negative.equalities == key.equalities
                        && negative.provider == key.provider
                        && negative.hasher == key.hasher
                }),
                source,
                "fork key set provider differs from its Storage negation record",
            )?;
            let projection = (
                key.relation,
                key.columns.clone(),
                key.filters.clone(),
                key.equalities.clone(),
            );
            ensure(
                key.relation < source.incremental.strata.source.declarations.len()
                    && !projections.contains(&projection),
                source,
                "fork has invalid or duplicate key projection",
            )?;
            projections.push(projection);
            let declaration = &source.incremental.strata.source.declarations[key.relation];
            ensure(
                key.columns.windows(2).all(|w| w[0] < w[1])
                    && key.columns.iter().all(|&i| i < declaration.arity()),
                source,
                "fork key columns must be an ordered subset",
            )?;
            ensure(
                key.filters.windows(2).all(|w| w[0].0 < w[1].0)
                    && key.filters.iter().all(|(i, expression)| {
                        *i < declaration.arity()
                            && !key.columns.contains(i)
                            && rust::variable(expression).is_none()
                            && !matches!(rust::unparen(expression), Expr::Infer(_))
                    }),
                source,
                "fork constant filters must select distinct non-key columns",
            )?;
            ensure(
                key.equalities.windows(2).all(|w| w[0].1 < w[1].1)
                    && key.equalities.iter().all(|&(first, repeated)| {
                        first < repeated
                            && repeated < declaration.arity()
                            && key.columns.contains(&first)
                            && !key.columns.contains(&repeated)
                            && !key.filters.iter().any(|(column, _)| *column == repeated)
                    }),
                source,
                "fork repeated columns must refer to distinct projected first occurrences",
            )?;
            let hole = checked_hole(source, target, key.hole, &mut used_holes)?;
            ensure(
                target.holes[key.hole].planning.is_none(),
                hole,
                "key holes have no Incremental planning advice",
            )?;
            let batch = target.holes[key.hole].interface == HoleInterface::Batch;
            add_name(&hole.name)?;
            ensure(
                hole.query.body.len() == 1 + key.filters.len() + key.equalities.len(),
                source,
                "key hole must retain its constant and repeated-variable filters",
            )?;
            let BodyItem::Positive { atom } = &hole.query.body[0] else {
                return Err(syn::Error::new_spanned(
                    hole,
                    "key hole must read its completed relation",
                ));
            };
            // A completed relation is scanned through its canonical layout.
            let canonical = &context.layouts[source.providers[key.relation].canonical];
            ensure(
                atom.relation == canonical.name && atom.args.len() == declaration.arity(),
                source,
                "key hole changed its input",
            )?;
            let mut args: Vec<&Expr> = atom.args.iter().collect();
            for (position, &column) in canonical.columns.iter().enumerate() {
                args[column] = &atom.args[position];
            }
            let variables: Vec<_> = args.iter().map(|arg| rust::variable(arg)).collect();
            ensure(
                variables.iter().all(Option::is_some)
                    && variables
                        .iter()
                        .flatten()
                        .map(|id| symbol_name(id))
                        .collect::<BTreeSet<_>>()
                        .len()
                        == declaration.arity(),
                source,
                "key projection requires distinct variables",
            )?;
            let mut output = declaration.clone();
            output.name = hole.name.clone();
            output.value = None;
            let expected_args: Vec<Expr> = if key.columns.is_empty() {
                let column = &hole.declarations.last().expect("checked output").columns[0].name;
                output.columns = syn::parse_quote!(#column: ());
                vec![syn::parse_quote!(())]
            } else {
                output.columns = key
                    .columns
                    .iter()
                    .map(|&i| {
                        let mut column = declaration.columns[i].clone();
                        column.name = variables[i].expect("checked key variable").clone();
                        if !batch {
                            let ty = &column.ty;
                            column.ty = syn::parse_quote!(&#ty);
                        }
                        column
                    })
                    .collect();
                key.columns.iter().map(|&i| args[i].clone()).collect()
            };
            let mut expected_declarations =
                required_inputs(&source.tokens, &target.holes, context, &[], &hole.query)?;
            expected_declarations.push(output);
            let name = &hole.name;
            let selections = key.filters.iter().map(|(column, expression)| {
                let variable = variables[*column].expect("checked filter variable");
                quote!(where #variable == (#expression))
            });
            let repeated = key.equalities.iter().map(|&(first, repeated)| {
                let first = variables[first].expect("checked key variable");
                let repeated = variables[repeated].expect("checked repeated variable");
                quote!(where #first == #repeated)
            });
            // Parse complete CQ to resolve the relational predicate's terms.
            let expected: CqProgram = syn::parse2(quote! {
                struct #name;
                #(#expected_declarations)*
                #name(#(#expected_args),*) :- #atom #(, #selections)* #(, #repeated)*;
            })?;
            ensure(
                hole == &expected,
                source,
                "key hole changed its schema, projection or filters",
            )?;
        }
        for (delta, rules) in [(false, &stratum.seeds), (true, &stratum.steps)] {
            for rule in rules {
                ensure(
                    rule.variant < variants.len() && used_variants.insert(rule.variant),
                    source,
                    "fork duplicates or invents a variant",
                )?;
                let variant = &variants[rule.variant];
                ensure(
                    rule.head == analysis.positions[&symbol_name(&variant.query.head.relation)]
                        && members.contains(&rule.head)
                        && delta == variant.delta.is_some(),
                    source,
                    "fork changed variant scheduling",
                )?;
                let hole = checked_hole(source, target, rule.hole, &mut used_holes)?;
                add_name(&hole.name)?;
                let mut output = source.incremental.strata.source.declarations[rule.head].clone();
                output.name = hole.name.clone();
                let mut query = variant.query.clone();
                query.head.relation = hole.name.clone();
                // Check the recorded bindings, independently of fork's renaming.
                for read in source
                    .reads
                    .iter()
                    .filter(|read| read.variant == rule.variant)
                {
                    let BodyItem::Positive { atom } = &mut query.body[read.position] else {
                        return Err(syn::Error::new_spanned(
                            source,
                            "read is not a positive occurrence",
                        ));
                    };
                    if read.kind != "direct" {
                        let layouts = if read.kind == "delta" {
                            &context.delta_layouts
                        } else {
                            &context.layouts
                        };
                        let layout = &layouts[read.index];
                        let original = atom.args.clone();
                        atom.args = read
                            .columns
                            .iter()
                            .map(|&column| original[column].clone())
                            .collect();
                        atom.relation = layout.name.clone();
                    }
                }
                let scan_declaration = if let Some(scan) = &rule.scan {
                    add_name(&scan.binding)?;
                    let Some(BodyItem::Positive { atom }) = query.body.first_mut() else {
                        return Err(syn::Error::new_spanned(
                            source,
                            "scan must bind the first positive occurrence",
                        ));
                    };
                    let mut declaration = match scan.source {
                        InputSource::Delta(index) => {
                            let relation = context.relations.get(index).ok_or_else(|| {
                                syn::Error::new_spanned(source, "scan delta is out of range")
                            })?;
                            ensure(
                                relation.delta.as_ref() == Some(&atom.relation),
                                source,
                                "scan must read the same delta",
                            )?;
                            relation.declaration.clone()
                        }
                        InputSource::Layout(index) => {
                            let layout = context.layouts.get(index).ok_or_else(|| {
                                syn::Error::new_spanned(source, "scan layout is out of range")
                            })?;
                            let original = context
                                .layouts
                                .iter()
                                .find(|candidate| candidate.name == atom.relation);
                            ensure(
                                layout.key == 0
                                    && original.is_some_and(|original| {
                                        original.relation == layout.relation
                                            && original.columns == layout.columns
                                    }),
                                source,
                                "scan requires a complete key-zero layout of the same relation and columns",
                            )?;
                            let mut declaration =
                                context.relations[layout.relation].declaration.clone();
                            declaration.columns = layout
                                .columns
                                .iter()
                                .map(|&column| declaration.columns[column].clone())
                                .collect();
                            declaration
                        }
                    };
                    declaration.name = scan.binding.clone();
                    let binding = &scan.binding;
                    declaration.value = Some(syn::parse_quote!(= (#binding)));
                    atom.relation = binding.clone();
                    Some(declaration)
                } else {
                    None
                };
                for (position, item) in query.body.iter_mut().enumerate() {
                    if let BodyItem::Negation(negative) = item {
                        let relation = analysis.positions[&symbol_name(&negative.atom.relation)];
                        let projection = columns(&negative.atom);
                        let selections = filters(&negative.atom);
                        let repeated = equalities(&negative.atom);
                        let (index, key) = stratum
                            .keys
                            .iter()
                            .enumerate()
                            .find(|(_, key)| {
                                key.relation == relation
                                    && key.columns == projection
                                    && key.filters == selections
                                    && key.equalities == repeated
                            })
                            .ok_or_else(|| {
                                syn::Error::new_spanned(
                                    &negative.atom,
                                    "fork omitted a negative key set",
                                )
                            })?;
                        used_keys.insert(index);
                        let Some(BodyItem::Negation(recorded)) =
                            hole.query.body.iter().nth(position)
                        else {
                            return Err(syn::Error::new_spanned(
                                hole,
                                "negative occurrence was lost",
                            ));
                        };
                        let arguments_match = if key.columns.is_empty() {
                            recorded.atom.args.len() == 1
                                && matches!(&recorded.atom.args[0], Expr::Infer(_))
                        } else {
                            recorded.atom.args.len() == key.columns.len()
                                && recorded.atom.args.iter().zip(&key.columns).all(
                                    |(argument, &column)| argument == &negative.atom.args[column],
                                )
                        };
                        ensure(
                            recorded.atom.relation == key.name && arguments_match,
                            hole,
                            "negative occurrence must probe its recorded projected key",
                        )?;
                        *item = BodyItem::Negation(recorded.clone());
                    }
                }
                // DL-Fanout: the recorded fact agrees with the variant's reads
                // and recorded keys. FK-Known: advice decides; otherwise a hole
                // with a recorded planning invocation is in plan iff a later
                // read is recorded `multiply`.
                let strata_source = &source.incremental.strata.source;
                ensure(
                    rule.fanout
                        == crate::fanout(&variant.query, &|relation| strata_source.keys(relation)),
                    source,
                    "fork must record the fan-out of every variant read",
                )?;
                let expected = match variant.known {
                    Some(incremental::Known::At(_)) => Admission::InPlan,
                    Some(incremental::Known::Admit) => Admission::Admit,
                    None if source.device.is_none()
                        && target.holes[rule.hole].plan.is_some()
                        && crate::fanout_follows(&rule.fanout, crate::head_level(&query).0) =>
                    {
                        Admission::InPlan
                    }
                    None => Admission::Admit,
                };
                ensure(
                    rule.known == expected,
                    source,
                    "fork admission placement disagrees with its advice or recorded fan-out",
                )?;
                let planning = target.holes[rule.hole].planning.as_ref().ok_or_else(|| {
                    syn::Error::new_spanned(hole, "rule hole must record planning facts")
                })?;
                let order: Vec<_> = variant
                    .query
                    .body
                    .iter()
                    .enumerate()
                    .filter_map(|(position, item)| {
                        matches!(item, BodyItem::Positive { .. }).then_some(variant.order[position])
                    })
                    .collect();
                ensure(
                    planning.order == order
                        && planning.authored == variant.authored
                        && planning.smallest == variant.smallest
                        && planning.steps.len() == variant.steps.len()
                        && planning
                            .steps
                            .iter()
                            .zip(&variant.steps)
                            .all(|(recorded, authored)| {
                                recorded.occurrence == authored.occurrence
                                    && recorded.columns == authored.columns
                            }),
                    hole,
                    "planning facts must preserve Incremental occurrence advice",
                )?;
                let (last, reads) = crate::head_level(&query);
                ensure(
                    match (rule.known, planning.known) {
                        (Admission::Admit, None) => true,
                        (Admission::InPlan, Some(level)) => {
                            level >= last.max(1)
                                && level <= reads
                                && match variant.known {
                                    Some(incremental::Known::At(authored)) => level == authored,
                                    _ => true,
                                }
                        }
                        _ => false,
                    },
                    hole,
                    "known level must respect head binding and authored admission advice",
                )?;
                if rule.known == Admission::InPlan {
                    admission_suffix(&mut query, hole, context, rule.head, &mut add_name)?;
                }
                let mut expected_declarations = required_inputs(
                    &source.tokens,
                    &target.holes,
                    context,
                    &stratum.keys,
                    &query,
                )?;
                expected_declarations.extend(scan_declaration);
                expected_declarations.push(output);
                ensure(
                    hole.declarations == expected_declarations,
                    source,
                    "fork changed hole declarations",
                )?;
                let name = &hole.name;
                let expected: CqProgram =
                    syn::parse2(quote!(struct #name; #(#expected_declarations)* #query))?;
                ensure(
                    hole == &expected,
                    source,
                    "fork hole does not erase to its recorded variant",
                )?;
            }
        }
        ensure(
            used_keys.len() == stratum.keys.len(),
            source,
            "fork contains an unused key set",
        )?;
    }
    ensure(
        used_holes.len() == target.holes.len() && used_variants.len() == variants.len(),
        source,
        "fork omitted a variant or contains an unused hole",
    )
}

/// Fork correspondence of an FK-Known record: an `in_plan` hole is its variant followed by exactly
/// one synthetic suffix: optional `let h: T = e;` bindings naming computed head
/// terms (`T` the declared head column type, `e` the variant's head term), then
/// `!C(x̄)` over the head's committed canonical layout with the complete head
/// key in canonical column order. Extend `query` by the checked suffix; exact
/// equality with the recorded hole then rejects any other change.
fn admission_suffix(
    query: &mut cq::Query,
    hole: &CqProgram,
    context: &RoundContext,
    head: usize,
    add_name: &mut dyn FnMut(&Ident) -> syn::Result<()>,
) -> syn::Result<()> {
    let declaration = &context.relations[head].declaration;
    let recorded: Vec<_> = hole.query.body.iter().collect();
    let base = query.body.len();
    ensure(
        recorded.len() > base && hole.query.head.args.len() == query.head.args.len(),
        hole,
        "in-plan hole lacks its committed admission read",
    )?;
    let mut bound = BTreeSet::new();
    for item in &recorded[base..recorded.len() - 1] {
        let BodyItem::Let(binding) = item else {
            return Err(syn::Error::new_spanned(
                item,
                "in-plan admission suffix admits only computed head bindings",
            ));
        };
        let syn::Pat::Ident(pattern) = &binding.pattern else {
            return Err(syn::Error::new_spanned(
                item,
                "head binding must name one variable",
            ));
        };
        let position = hole
            .query
            .head
            .args
            .iter()
            .position(|argument| rust::variable(argument) == Some(&pattern.ident));
        let Some(position) = position.filter(|_| {
            pattern.by_ref.is_none() && pattern.mutability.is_none() && pattern.subpat.is_none()
        }) else {
            return Err(syn::Error::new_spanned(
                item,
                "head binding must bind exactly one head variable",
            ));
        };
        ensure(
            rust::variable(&query.head.args[position]).is_none()
                && bound.insert(position)
                && binding.expression == query.head.args[position]
                && binding
                    .annotation
                    .as_ref()
                    .is_some_and(|annotation| annotation.ty == declaration.columns[position].ty),
            item,
            "head binding must carry its variant's computed head term at the declared column type",
        )?;
        add_name(&pattern.ident)?;
        query.head.args[position] = hole.query.head.args[position].clone();
        query.body.push((*item).clone());
    }
    let args: Vec<Expr> = query.head.args.iter().cloned().collect();
    ensure(
        args.iter()
            .all(|argument| rust::variable(argument).is_some()),
        hole,
        "in-plan admission requires a complete head key of variables",
    )?;
    let Some(BodyItem::Negation(negative)) = recorded.last().copied() else {
        return Err(syn::Error::new_spanned(
            hole,
            "admission suffix must end with a negative canonical read",
        ));
    };
    let canonical = &context.layouts[context.relations[head].providers.canonical];
    ensure(
        negative.atom.relation == canonical.name
            && negative.atom.args.len() == canonical.columns.len()
            && negative
                .atom
                .args
                .iter()
                .zip(&canonical.columns)
                .all(|(argument, &column)| argument == &args[column]),
        hole,
        "admission suffix must probe the complete canonical head key",
    )?;
    query.body.push(BodyItem::Negation(negative.clone()));
    Ok(())
}

fn checked_hole<'a>(
    source: &storage::Program,
    target: &'a Program,
    index: usize,
    used: &mut BTreeSet<usize>,
) -> syn::Result<&'a CqProgram> {
    ensure(used.insert(index), source, "fork reuses a hole")?;
    let hole = target
        .holes
        .get(index)
        .ok_or_else(|| syn::Error::new_spanned(source, "fork references a missing hole"))?;
    cq_lang_cq::contract::check(&hole.cq)?;
    Ok(&hole.cq)
}
