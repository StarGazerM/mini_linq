//! Fork a complete semi-naive Datalog schedule into independently checked CQ
//! programs and a round context. This pass emits no Rust loops and knows no
//! pipeline protocol. A bridge expands every hole through its own continuation.
//! Negative atoms become total membership predicates over recorded key sets;
//! each distinct projection and constant filter is built once per stratum.
//! Key-hole correspondence compares complete parsed CQ programs, including
//! CQ's schema-aware guard recognition, rather than raw isolated predicates.
//! Full hosts use keys bound by earlier positive atoms plus constant arguments.
//! Delta probes use separately recorded layouts; an initial unkeyed delta scans
//! its ordinary host. The checked variant driver determines positive body order;
//! layout realization never moves an atom or a Rust guard.

use cq_lang_cq::{BodyItem, Program as CqProgram};
use cq_lang_datalog::{Program, contract};
use cq_syntax::{RelationDecl, rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use std::collections::BTreeSet;
use syn::{Expr, Ident};

/// Runtime names and schema for one source relation.
#[derive(Clone, Debug)]
pub struct Relation {
    pub declaration: RelationDecl,
    pub all: Ident,
    pub seen: Ident,
    pub next: Ident,
    pub delta: Option<Ident>,
    /// Fresh Rust binding; `delta` remains the recorded CQ relation name.
    pub delta_host: Option<Ident>,
}

/// A selected full/delta permutation with its context-recorded realization.
#[derive(Clone, Debug)]
pub struct Layout {
    pub relation: usize,
    pub columns: Vec<usize>,
    pub key: usize,
    pub name: Ident,
    pub host: Ident,
}

/// Checked materialization policy recorded by the semi-naive IR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRepresentation {
    HashSetBorrowed,
    DeviceSetOwned,
}

/// Concrete layout lifecycle, checked together with admission/key selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutRealization {
    MaintainedGroupedHost,
    /// Frozen full layouts are indexed before each stratum; changing layouts
    /// retain their per-round resident projection/build realization.
    MaintainedFrozenDeviceIndex,
}

impl KeyRepresentation {
    fn layouts(self) -> LayoutRealization {
        match self {
            Self::HashSetBorrowed => LayoutRealization::MaintainedGroupedHost,
            Self::DeviceSetOwned => LayoutRealization::MaintainedFrozenDeviceIndex,
        }
    }
}

/// A key projection, materialized once before a stratum starts.
#[derive(Clone, Debug)]
pub struct KeySet {
    pub relation: usize,
    pub columns: Vec<usize>,
    /// Constant column selections applied before projecting keys.
    pub filters: Vec<(usize, Expr)>,
    pub name: Ident,
    pub representation: KeyRepresentation,
    pub hole: usize,
}

/// One rule invocation and the relation receiving its new rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleHole {
    pub variant: usize,
    pub head: usize,
    pub hole: usize,
}

/// Immutable snapshots are shared by all step holes before publication.
#[derive(Clone, Debug)]
pub struct Stratum {
    pub members: Vec<usize>,
    pub keys: Vec<KeySet>,
    /// Indices into `RoundContext::layouts`, built once before this stratum.
    /// Each names a keyed full read of an EDB or a completed lower stratum.
    pub frozen_layouts: Vec<usize>,
    pub seeds: Vec<RuleHole>,
    pub steps: Vec<RuleHole>,
}

/// All information a round printer needs, without a pipeline dependency.
#[derive(Clone, Debug)]
pub struct RoundContext {
    /// Admission and negative-key policy selected by the checked schedule.
    pub representation: KeyRepresentation,
    pub layout_realization: LayoutRealization,
    pub relations: Vec<Relation>,
    pub layouts: Vec<Layout>,
    pub delta_layouts: Vec<Layout>,
    pub strata: Vec<Stratum>,
    pub outputs: Vec<usize>,
    pub observer: Option<Expr>,
    pub observer_name: Ident,
    pub round_name: Ident,
}

/// Complete CQ programs paired with their surrounding fixed-point context.
#[derive(Clone, Debug)]
pub struct Fork {
    pub context: RoundContext,
    pub holes: Vec<CqProgram>,
}

/// The same context after a caller has expanded every CQ hole.
#[derive(Clone, Debug)]
pub struct ExpandedFork {
    pub context: RoundContext,
    pub holes: Vec<TokenStream>,
}

fn ensure(ok: bool, source: &Program, message: &str) -> syn::Result<()> {
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
    fn new(source: &Program) -> Self {
        let mut names = BTreeSet::new();
        identifiers(source.to_token_stream(), &mut names);
        Self(names)
    }
    fn id(&mut self, role: &str) -> Ident {
        for n in 0.. {
            let name = format!("__cq_datalog_{role}_{n}");
            if self.0.insert(name.clone()) {
                return Ident::new(&name, Span::mixed_site());
            }
        }
        unreachable!()
    }
}

fn inputs(context: &RoundContext) -> Vec<RelationDecl> {
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
fn use_layouts(query: &mut cq_lang_cq::Query, context: &RoundContext) -> syn::Result<()> {
    let mut bound = BTreeSet::new();
    for (position, item) in query.body.iter_mut().enumerate() {
        let BodyItem::Positive { atom } = item else {
            continue;
        };
        let relation = context
            .relations
            .iter()
            .position(|r| {
                r.declaration.name == atom.relation || r.delta.as_ref() == Some(&atom.relation)
            })
            .ok_or_else(|| syn::Error::new_spanned(&*atom, "undeclared positive host"))?;
        let is_delta = context.relations[relation].delta.as_ref() == Some(&atom.relation);
        let keys: Vec<_> = atom
            .args
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| {
                let is_key = match rust::variable(arg) {
                    Some(id) => bound.contains(&symbol_name(id)),
                    None => !matches!(rust::unparen(arg), Expr::Infer(_)),
                };
                is_key.then_some(i)
            })
            .collect();
        let key = keys.len();
        if is_delta && key == 0 && position == 0 {
            bound.extend(atom.variables().map(symbol_name));
            continue;
        }
        let columns: Vec<_> = keys
            .iter()
            .copied()
            .chain((0..atom.args.len()).filter(|i| !keys.contains(i)))
            .collect();
        let layouts = if is_delta {
            &context.delta_layouts
        } else {
            &context.layouts
        };
        let layout = layouts
            .iter()
            .find(|layout| {
                layout.relation == relation && layout.columns == columns && layout.key == key
            })
            .ok_or_else(|| {
                syn::Error::new_spanned(&*atom, "fork omitted a positive host layout")
            })?;
        bound.extend(atom.variables().map(symbol_name));
        atom.args = columns.iter().map(|&i| atom.args[i].clone()).collect();
        atom.relation = layout.name.clone();
    }
    Ok(())
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
    if key.representation == KeyRepresentation::DeviceSetOwned {
        let args = key.columns.iter().map(|&i| &atom.args[i]);
        return syn::parse_quote!(
            #[total] if !::cq_gpu_runtime::datalog::contains(&#name, &(#(#args,)*))
        );
    }
    if key.columns.is_empty() {
        syn::parse_quote!(#[total] if !#name.contains(&((),)))
    } else {
        let args = key.columns.iter().map(|&i| &atom.args[i]);
        syn::parse_quote!(#[total] if !#name.contains(&(#(#args,)*)))
    }
}

fn key_representation(source: &Program) -> syn::Result<KeyRepresentation> {
    match source.negation_keys.as_ref().map(symbol_name).as_deref() {
        Some("hash_set_borrowed") => Ok(KeyRepresentation::HashSetBorrowed),
        Some("device_set_owned") => Ok(KeyRepresentation::DeviceSetOwned),
        _ => Err(syn::Error::new_spanned(
            source,
            "fork requires a checked negation key representation",
        )),
    }
}

fn parse_hole(tokens: TokenStream, representation: KeyRepresentation) -> syn::Result<CqProgram> {
    let mut program: CqProgram = syn::parse2(tokens)?;
    if representation == KeyRepresentation::DeviceSetOwned {
        // Device binders evaluate declared hosts. Keep precisely the source
        // scans and output schema of this checked hole, not unrelated layouts.
        let mut used = BTreeSet::from([symbol_name(&program.query.head.relation)]);
        for item in &program.query.body {
            if let BodyItem::Positive { atom } = item {
                used.insert(symbol_name(&atom.relation));
            }
        }
        program
            .declarations
            .retain(|declaration| used.contains(&symbol_name(&declaration.name)));
    }
    Ok(program)
}

/// Lower variants and key projections, retaining the recorded schedule.
pub fn compile(source: &Program) -> syn::Result<Fork> {
    let analysis = contract::check(source)?;
    let variants = source
        .variants
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(source, "fork requires seminaive variants"))?;
    let representation = key_representation(source)?;
    let mut fresh = Fresh::new(source);
    let mut context = RoundContext {
        representation,
        layout_realization: representation.layouts(),
        relations: source
            .declarations
            .iter()
            .map(|declaration| Relation {
                declaration: declaration.clone(),
                all: fresh.id("all"),
                seen: fresh.id("seen"),
                next: fresh.id("next"),
                delta: analysis
                    .deltas
                    .get(&symbol_name(&declaration.name))
                    .cloned(),
                delta_host: analysis
                    .deltas
                    .contains_key(&symbol_name(&declaration.name))
                    .then(|| fresh.id("delta")),
            })
            .collect(),
        layouts: source
            .layouts
            .iter()
            .map(|layout| Layout {
                relation: analysis.positions[&symbol_name(&layout.relation)],
                columns: layout.columns.clone(),
                key: layout.key,
                name: fresh.id("layout"),
                host: fresh.id("host"),
            })
            .collect(),
        delta_layouts: source
            .delta_layouts
            .iter()
            .map(|layout| Layout {
                relation: analysis.positions[&symbol_name(&layout.relation)],
                columns: layout.columns.clone(),
                key: layout.key,
                name: fresh.id("delta_layout"),
                host: fresh.id("delta_host"),
            })
            .collect(),
        strata: Vec::new(),
        outputs: source
            .outputs
            .iter()
            .map(|id| analysis.positions[&symbol_name(id)])
            .collect(),
        observer: source.observer.clone(),
        observer_name: fresh.id("observer"),
        round_name: fresh.id("round"),
    };
    let declarations = inputs(&context);
    let mut holes = Vec::new();
    for members in &analysis.components {
        let mut stratum = Stratum {
            members: members.clone(),
            keys: Vec::new(),
            frozen_layouts: Vec::new(),
            seeds: Vec::new(),
            steps: Vec::new(),
        };
        for (variant_index, variant) in variants.iter().enumerate() {
            let head = analysis.positions[&symbol_name(&variant.query.head.relation)];
            if !members.contains(&head) {
                continue;
            }
            let mut query = variant.query.clone();
            use_layouts(&mut query, &context)?;
            for item in &mut query.body {
                let BodyItem::Negation(negative) = item else {
                    continue;
                };
                let relation = analysis.positions[&symbol_name(&negative.atom.relation)];
                let projection = columns(&negative.atom);
                let selections = filters(&negative.atom);
                let key_index = if let Some(index) = stratum.keys.iter().position(|key| {
                    key.relation == relation
                        && key.columns == projection
                        && key.filters == selections
                }) {
                    index
                } else {
                    let key = KeySet {
                        relation,
                        columns: projection,
                        filters: selections,
                        name: fresh.id("keys"),
                        representation,
                        hole: holes.len(),
                    };
                    let name = fresh.id("key_hole");
                    let source_name = &source.declarations[relation].name;
                    let variables: Vec<_> = source.declarations[relation]
                        .columns
                        .iter()
                        .map(|_| fresh.id("column"))
                        .collect();
                    let (selected, output_columns): (Vec<_>, Vec<_>) = if key.columns.is_empty() {
                        let column = &variables[0];
                        match representation {
                            KeyRepresentation::HashSetBorrowed => {
                                (vec![quote!(())], vec![quote!(#column: ())])
                            }
                            KeyRepresentation::DeviceSetOwned => {
                                (vec![quote!(0u8)], vec![quote!(#column: u8)])
                            }
                        }
                    } else {
                        (
                            key.columns
                                .iter()
                                .map(|&i| {
                                    let variable = &variables[i];
                                    match representation {
                                        KeyRepresentation::HashSetBorrowed => quote!(#variable),
                                        KeyRepresentation::DeviceSetOwned => quote!(*#variable),
                                    }
                                })
                                .collect(),
                            key.columns
                                .iter()
                                .map(|&i| {
                                    let variable = &variables[i];
                                    let ty = &source.declarations[relation].columns[i].ty;
                                    match representation {
                                        KeyRepresentation::HashSetBorrowed => {
                                            quote!(#variable: &#ty)
                                        }
                                        KeyRepresentation::DeviceSetOwned => quote!(#variable: #ty),
                                    }
                                })
                                .collect(),
                        )
                    };
                    // Keep owned constant expressions in post-scan predicates.
                    // An atom expression becomes an owned CQ prefix binding;
                    // carrying that binding through scan fanout can move it out
                    // of an FnMut closure. Comparison borrows both operands.
                    let selections = key.filters.iter().map(|(column, expression)| {
                        let variable = &variables[*column];
                        quote!(if *#variable == (#expression))
                    });
                    holes.push(parse_hole(quote!(struct #name; #(#declarations)* relation #name(#(#output_columns),*); #name(#(#selected),*) :- #source_name(#(#variables),*) #(, #selections)*;), representation)?);
                    stratum.keys.push(key);
                    stratum.keys.len() - 1
                };
                *item = guard(&negative.atom, &stratum.keys[key_index]);
            }
            let name = fresh.id("hole");
            query.head.relation = name.clone();
            let mut output = source.declarations[head].clone();
            output.name = name.clone();
            let hole = holes.len();
            holes.push(parse_hole(
                quote!(struct #name; #(#declarations)* #output #query),
                representation,
            )?);
            let rule = RuleHole {
                variant: variant_index,
                head,
                hole,
            };
            if variant.delta.is_some() {
                stratum.steps.push(rule);
            } else {
                stratum.seeds.push(rule);
            }
        }
        if representation == KeyRepresentation::DeviceSetOwned {
            for (index, layout) in context.layouts.iter().enumerate() {
                if layout.key != 0 && !members.contains(&layout.relation)
                    && stratum.seeds.iter().chain(&stratum.steps).any(|rule| {
                        holes[rule.hole].query.body.iter().any(|item| {
                            matches!(item, BodyItem::Positive { atom } if atom.relation == layout.name)
                        })
                    })
                {
                    stratum.frozen_layouts.push(index);
                }
            }
        }
        context.strata.push(stratum);
    }
    let target = Fork { context, holes };
    check(source, &target)?;
    Ok(target)
}

/// Check complete CQ contracts, schedule preservation, key projections, and
/// variant erasure without rerunning the producer. Runtime names may be freely
/// renamed provided they remain fresh and all recorded references agree.
pub fn check(source: &Program, target: &Fork) -> syn::Result<()> {
    let analysis = contract::check(source)?;
    let variants = source
        .variants
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(source, "fork requires seminaive variants"))?;
    let representation = key_representation(source)?;
    let context = &target.context;
    ensure(
        context.representation == representation
            && context.layout_realization == representation.layouts(),
        source,
        "fork changed the recorded representation policy",
    )?;
    ensure(
        context.relations.len() == source.declarations.len()
            && context.strata.len() == analysis.components.len(),
        source,
        "fork changed relation or stratum coverage",
    )?;
    ensure(
        context.outputs
            == source
                .outputs
                .iter()
                .map(|id| analysis.positions[&symbol_name(id)])
                .collect::<Vec<_>>()
            && context.observer == source.observer,
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
    for (relation, declaration) in context.relations.iter().zip(&source.declarations) {
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
        let mut projections = Vec::new();
        let mut used_keys = BTreeSet::new();
        let mut frozen_layouts = BTreeSet::new();
        for key in &stratum.keys {
            add_name(&key.name)?;
            ensure(
                key.representation == representation,
                source,
                "fork changed negation key representation",
            )?;
            let projection = (key.relation, key.columns.clone(), key.filters.clone());
            ensure(
                key.relation < source.declarations.len() && !projections.contains(&projection),
                source,
                "fork has invalid or duplicate key projection",
            )?;
            projections.push(projection);
            let declaration = &source.declarations[key.relation];
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
                match representation {
                    KeyRepresentation::HashSetBorrowed => {
                        output.columns = syn::parse_quote!(#column: ());
                        vec![syn::parse_quote!(())]
                    }
                    KeyRepresentation::DeviceSetOwned => {
                        output.columns = syn::parse_quote!(#column: u8);
                        vec![syn::parse_quote!(0u8)]
                    }
                }
            } else {
                output.columns = key
                    .columns
                    .iter()
                    .map(|&i| {
                        let mut column = declaration.columns[i].clone();
                        column.name = variables[i].expect("checked key variable").clone();
                        let ty = &column.ty;
                        if representation == KeyRepresentation::HashSetBorrowed {
                            column.ty = syn::parse_quote!(&#ty);
                        }
                        column
                    })
                    .collect();
                key.columns
                    .iter()
                    .map(|&i| {
                        let arg = &atom.args[i];
                        match representation {
                            KeyRepresentation::HashSetBorrowed => arg.clone(),
                            KeyRepresentation::DeviceSetOwned => syn::parse_quote!(*#arg),
                        }
                    })
                    .collect()
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
            let expected = parse_hole(
                quote! {
                    struct #name;
                    #(#expected_declarations)*
                    #name(#(#expected_args),*) :- #atom #(, #selections)*;
                },
                representation,
            )?;
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
                let mut output = source.declarations[rule.head].clone();
                output.name = hole.name.clone();
                let mut expected_declarations = declarations.clone();
                expected_declarations.push(output);
                let mut query = variant.query.clone();
                query.head.relation = hole.name.clone();
                use_layouts(&mut query, context)?;
                if representation == KeyRepresentation::DeviceSetOwned {
                    for item in &query.body {
                        if let BodyItem::Positive { atom } = item {
                            if let Some((index, layout)) = context
                                .layouts
                                .iter()
                                .enumerate()
                                .find(|(_, layout)| layout.name == atom.relation)
                            {
                                if layout.key != 0 && !members.contains(&layout.relation) {
                                    frozen_layouts.insert(index);
                                }
                            }
                        }
                    }
                }
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
                let expected = parse_hole(
                    quote!(struct #name; #(#expected_declarations)* #query),
                    representation,
                )?;
                ensure(
                    hole == &expected,
                    source,
                    "fork hole does not erase to its recorded variant",
                )?;
            }
        }
        ensure(
            stratum.frozen_layouts.iter().copied().eq(frozen_layouts),
            source,
            "fork frozen device indexes must exactly cover keyed EDB/lower-stratum layouts in layout order",
        )?;
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
    source: &Program,
    target: &'a Fork,
    index: usize,
    used: &mut BTreeSet<usize>,
) -> syn::Result<&'a CqProgram> {
    ensure(used.insert(index), source, "fork reuses a hole")?;
    let hole = target
        .holes
        .get(index)
        .ok_or_else(|| syn::Error::new_spanned(source, "fork references a missing hole"))?;
    cq_lang_cq::contract::check(hole)?;
    Ok(hole)
}

/// Invoke the caller's continuation exactly once for each checked CQ hole.
pub fn fork(
    source: &Program,
    mut expand: impl FnMut(CqProgram) -> syn::Result<TokenStream>,
) -> syn::Result<ExpandedFork> {
    let Fork { context, holes } = compile(source)?;
    let holes = holes
        .into_iter()
        .map(&mut expand)
        .collect::<syn::Result<_>>()?;
    Ok(ExpandedFork { context, holes })
}
