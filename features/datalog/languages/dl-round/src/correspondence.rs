use super::*;
fn ensure(ok: bool, source: &storage::Program, message: &str) -> syn::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(source, message))
    }
}

fn identifiers(tokens: TokenStream, names: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(id) => {
                names.insert(symbol_name(&id));
            }
            TokenTree::Group(group) => identifiers(group.stream(), names),
            _ => {}
        }
    }
}

struct Fresh(BTreeSet<String>);
impl Fresh {
    fn new(source: &storage::Program) -> Self {
        let mut names = BTreeSet::new();
        identifiers(source.to_token_stream(), &mut names);
        Self(names)
    }
}

pub fn inputs(context: &RoundContext) -> Vec<RelationDecl> {
    let mut declarations = Vec::new();
    for relation in &context.relations {
        let mut declaration = relation.declaration.clone();
        let host = &relation.all;
        declaration.value = Some(if declaration.value.is_some() {
            syn::parse_quote!(= (#host))
        } else {
            syn::parse_quote!(= (&#host))
        });
        declarations.push(declaration);
        if let Some(delta) = &relation.delta {
            let mut declaration = relation.declaration.clone();
            declaration.name = delta.clone();
            let host = relation.delta_host.as_ref().expect("checked delta host");
            declaration.value = Some(syn::parse_quote!(= (&#host)));
            declarations.push(declaration);
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        let mut declaration = context.relations[layout.relation].declaration.clone();
        declaration.name = layout.name.clone();
        declaration.columns = layout
            .columns
            .iter()
            .map(|&i| declaration.columns[i].clone())
            .collect();
        let host = &layout.host;
        declaration.value = Some(syn::parse_quote!(= (&#host)));
        declarations.push(declaration);
    }
    declarations
}

// Layouts change only positional host representation, never Rust-step order.
pub fn use_layouts(query: &mut cq_lang_cq::Query, context: &RoundContext, storage: &storage::Program, variant: usize) -> syn::Result<()> {
    for read in storage.reads.iter().filter(|read|read.variant==variant) {
        let Some(BodyItem::Positive{atom})=query.body.iter_mut().nth(read.position) else {return Err(syn::Error::new_spanned(&*query,"recorded read is not a positive occurrence"));};
        if read.kind=="direct" {continue;}
        let layouts=if read.kind=="delta" {&context.delta_layouts} else {&context.layouts};
        let layout=layouts.get(read.index).ok_or_else(||syn::Error::new_spanned(&*atom,"missing recorded layout"))?;
        atom.args=read.columns.iter().map(|&i|atom.args[i].clone()).collect();
        atom.relation=layout.name.clone();
    }
    Ok(())
}

pub fn scan_source(query: &cq_lang_cq::Query, context: &RoundContext) -> Option<InputSource> {
    let BodyItem::Positive { atom } = query.body.first()? else {
        return None;
    };
    if let Some(index) = context
        .relations
        .iter()
        .position(|relation| relation.delta.as_ref() == Some(&atom.relation))
    {
        return Some(InputSource::Delta(index));
    }
    context
        .layouts
        .iter()
        .position(|layout| layout.name == atom.relation && layout.key == 0)
        .map(InputSource::Layout)
}

pub fn use_scan(
    query: &mut cq_lang_cq::Query,
    context: &RoundContext,
    scan: &ScanBinding,
) -> RelationDecl {
    let mut declaration = match scan.source {
        InputSource::Delta(index) => context.relations[index].declaration.clone(),
        InputSource::Layout(index) => {
            let layout = &context.layouts[index];
            let mut declaration = context.relations[layout.relation].declaration.clone();
            declaration.columns = layout
                .columns
                .iter()
                .map(|&column| declaration.columns[column].clone())
                .collect();
            declaration
        }
    };
    let binding = &scan.binding;
    declaration.name = binding.clone();
    declaration.value = Some(syn::parse_quote!(= (#binding)));
    let BodyItem::Positive { atom } = &mut query.body[0] else {
        unreachable!("checked scan source")
    };
    atom.relation = binding.clone();
    declaration
}

fn columns(atom: &cq_syntax::Atom) -> Vec<usize> {
    atom.args
        .iter()
        .enumerate()
        .filter_map(|(i, arg)| rust::variable(arg).map(|_| i))
        .collect()
}

fn filters(atom: &cq_syntax::Atom) -> Vec<(usize, Expr)> {
    atom.args
        .iter()
        .enumerate()
        .filter(|(_, arg)| {
            rust::variable(arg).is_none() && !matches!(rust::unparen(arg), Expr::Infer(_))
        })
        .map(|(i, arg)| (i, arg.clone()))
        .collect()
}

fn guard(atom: &cq_syntax::Atom, key: &KeySet) -> BodyItem {
    let name = &key.name;
    if key.columns.is_empty() {
        syn::parse_quote!(#[total] if !#name.contains(&((),)))
    } else {
        let args = key.columns.iter().map(|&i| &atom.args[i]);
        syn::parse_quote!(#[total] if !#name.contains(&(#(#args,)*)))
    }
}

fn key_representation(source: &storage::Program) -> syn::Result<KeyRepresentation> {
    match Some(symbol_name(&source.policy.negation_keys)).as_deref() {
        Some("hash_set_borrowed") => Ok(KeyRepresentation::HashSetBorrowed),
        _ => Err(syn::Error::new_spanned(
            source,
            "fork requires negation_keys hash_set_borrowed",
        )),
    }
}

/// Check complete CQ contracts, schedule preservation, key projections, and
/// variant erasure without rerunning the producer. Runtime names may be freely
/// renamed provided they remain fresh and all recorded references agree.
pub fn check_storage(source: &storage::Program, target: &Program) -> syn::Result<()> {
    let analysis = storage::contract::check(source)?;
    let variants = &source.incremental.variants;
    let representation = key_representation(source)?;
    let context = &target.context;
    ensure(
        context.relations.len() == source.incremental.strata.source.declarations.len()
            && context.strata.len() == analysis.components.len(),
        source,
        "fork changed relation or stratum coverage",
    )?;
    ensure(
        context.outputs
            == source.incremental.strata.source
                .outputs
                .iter()
                .map(|id| analysis.positions[&symbol_name(id)])
                .collect::<Vec<_>>()
            && context.observer == source.incremental.strata.source.observer,
        source,
        "fork changed output or observer",
    )?;
    let mut names = Fresh::new(source).0;
    let mut add_name = |id: &Ident| {
        ensure(
            names.insert(symbol_name(id)),
            source,
            "fork runtime names must be fresh and distinct",
        )
    };
    add_name(&context.observer_name)?;
    add_name(&context.round_name)?;
    for (relation, declaration) in context.relations.iter().zip(&source.incremental.strata.source.declarations) {
        ensure(
            relation.declaration == *declaration
                && relation.delta.as_ref() == analysis.deltas.get(&symbol_name(&declaration.name)),
            source,
            "fork changed relation schema or delta binding",
        )?;
        add_name(&relation.all)?;
        add_name(&relation.seen)?;
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
                && layout.key == expected.key,
            source,
            "fork changed a selected host layout",
        )?;
        add_name(&layout.name)?;
        add_name(&layout.host)?;
    }
    let declarations = inputs(context);
    let mut used_holes = BTreeSet::new();
    let mut used_variants = BTreeSet::new();
    for (stratum, members) in context.strata.iter().zip(&analysis.components) {
        ensure(
            stratum.members == *members,
            source,
            "fork changed component membership",
        )?;
        let expected_seeds:Vec<_>=variants.iter().enumerate().filter(|(_,v)|v.delta.is_none() && members.contains(&analysis.positions[&symbol_name(&v.query.head.relation)])).map(|(i,_)|i).collect();
        let expected_steps:Vec<_>=variants.iter().enumerate().filter(|(_,v)|v.delta.is_some() && members.contains(&analysis.positions[&symbol_name(&v.query.head.relation)])).map(|(i,_)|i).collect();
        ensure(stratum.seeds.iter().map(|r|r.variant).eq(expected_seeds) && stratum.steps.iter().map(|r|r.variant).eq(expected_steps),source,"fork changed written variant order")?;
        let mut projections = Vec::new();
        let mut used_keys = BTreeSet::new();
        for key in &stratum.keys {
            add_name(&key.name)?;
            ensure(
                key.representation == representation,
                source,
                "fork changed negation key representation",
            )?;
            let projection = (key.relation, key.columns.clone(), key.filters.clone());
            ensure(
                key.relation < source.incremental.strata.source.declarations.len() && !projections.contains(&projection),
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
            let hole = checked_hole(source, target, key.hole, &mut used_holes)?;
            add_name(&hole.name)?;
            ensure(
                hole.query.body.len() == 1 + key.filters.len(),
                source,
                "key hole must be a projection with exactly its constant filters",
            )?;
            let BodyItem::Positive { atom } = &hole.query.body[0] else {
                return Err(syn::Error::new_spanned(
                    hole,
                    "key hole must read its completed relation",
                ));
            };
            ensure(
                atom.relation == declaration.name && atom.args.len() == declaration.arity(),
                source,
                "key hole changed its input",
            )?;
            let variables: Vec<_> = atom.args.iter().map(rust::variable).collect();
            ensure(
                variables
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
                        let ty = &column.ty;
                        column.ty = syn::parse_quote!(&#ty);
                        column
                    })
                    .collect();
                key.columns.iter().map(|&i| atom.args[i].clone()).collect()
            };
            let mut expected_declarations = declarations.clone();
            expected_declarations.push(output);
            let name = &hole.name;
            let selections = key.filters.iter().map(|(column, expression)| {
                let variable = variables[*column].expect("checked filter variable");
                quote!(if *#variable == (#expression))
            });
            // A complete CQ parse recognizes literal equalities and total
            // predicates. Parsing isolated BodyItems would leave raw Filters
            // and compare different language representations of the same clause.
            let expected: CqProgram = syn::parse2(quote! {
                struct #name;
                #(#expected_declarations)*
                #name(#(#expected_args),*) :- #atom #(, #selections)*;
            })?;
            ensure(
                hole == &expected,
                source,
                "key hole changed its schema, projection or constant filters",
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
                let mut expected_declarations = declarations.clone();
                let mut query = variant.query.clone();
                query.head.relation = hole.name.clone();
                use_layouts(&mut query, context, source, rule.variant)?;
                ensure(
                    rule.scan.as_ref().map(|scan| &scan.source)
                        == scan_source(&query, context).as_ref(),
                    source,
                    "fork changed scan coverage or source",
                )?;
                if let Some(scan) = &rule.scan {
                    add_name(&scan.binding)?;
                    expected_declarations.push(use_scan(&mut query, context, scan));
                }
                expected_declarations.push(output);
                ensure(
                    hole.declarations == expected_declarations,
                    source,
                    "fork changed hole declarations",
                )?;
                for item in &mut query.body {
                    if let BodyItem::Negation(negative) = item {
                        let relation = analysis.positions[&symbol_name(&negative.atom.relation)];
                        let projection = columns(&negative.atom);
                        let selections = filters(&negative.atom);
                        let (index, key) = stratum
                            .keys
                            .iter()
                            .enumerate()
                            .find(|(_, key)| {
                                key.relation == relation
                                    && key.columns == projection
                                    && key.filters == selections
                            })
                            .ok_or_else(|| {
                                syn::Error::new_spanned(
                                    &negative.atom,
                                    "fork omitted a negative key set",
                                )
                            })?;
                        used_keys.insert(index);
                        *item = guard(&negative.atom, key);
                    }
                }
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

