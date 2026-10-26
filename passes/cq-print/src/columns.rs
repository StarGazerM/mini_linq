//! Column demands for the checked Physical DAG and tuple adapter boundaries.
use super::*;
use std::collections::BTreeSet;
use structures::syntax::{algebra::Environment, algebra::Theta, rust};

/// Logical columns read by a declarative predicate.
pub fn columns_used(theta: &Theta) -> BTreeSet<String> {
    theta
        .constraints
        .iter()
        .flat_map(structures::syntax::Constraint::variables)
        .chain(
            theta
                .equalities
                .iter()
                .flat_map(structures::syntax::Equality::variables),
        )
        .map(symbol_name)
        .collect()
}

/// Logical columns read by Rust, restricted to its written environment.
/// Opaque syntax conservatively reads the whole lexical environment.
pub fn columns_used_rust(
    expression: &Expr,
    heading: &[String],
    environment: Option<&Environment>,
) -> BTreeSet<String> {
    let names = environment
        .map(|e| e.variables.iter().map(symbol_name).collect())
        .unwrap_or_else(|| heading.iter().cloned().collect());
    rust::free_variables(expression).dependencies(&names)
}

pub(super) fn carried_columns(
    module: &structures::Program,
    analysis: &structures::Analysis,
) -> BTreeMap<String, Vec<String>> {
    let mut used: BTreeMap<String, BTreeSet<String>> = analysis
        .layouts
        .keys()
        .map(|n| (n.clone(), BTreeSet::new()))
        .collect();
    let returned = symbol_name(&module.physical.program.returned);
    for expression in &module.physical.program.head.args {
        used.get_mut(&returned).unwrap().extend(columns_used_rust(
            expression,
            &analysis.layouts[&returned],
            None,
        ));
    }
    // Canonical boundaries compare the complete logical valuation. Recorded
    // storage keys/payloads likewise remain independent of downstream projection.
    for definition in &module.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if definition.canon.is_some() {
            used.get_mut(&name)
                .unwrap()
                .extend(analysis.layouts[&name].iter().cloned());
        }
        if let Some(builds) = analysis.built.get(&name) {
            for (levels, _) in builds {
                used.get_mut(&name).unwrap().extend(levels.concat());
            }
        }
        if let Some(filtered) = analysis.filtered.get(&name) {
            used.get_mut(&name)
                .unwrap()
                .extend(filtered.payload.columns.iter().map(symbol_name));
        }
        // Native host keyed protocols expose the declared payload layout. Their
        // lazy fallback must implement that identical layout, even for a reader
        // that ultimately projects some fields away.
        if analysis.host_read(&name)
            && (analysis.physical.key_reads.contains_key(&name)
                || analysis.physical.merge_reads.contains_key(&name))
        {
            used.get_mut(&name)
                .unwrap()
                .extend(analysis.layouts[&name].iter().cloned());
        }
    }
    for (name, key) in analysis.keysets.keys() {
        used.get_mut(name).unwrap().extend(key.iter().cloned());
    }
    propagate_columns(module, analysis, used)
}

fn propagate_columns(
    module: &structures::Program,
    analysis: &structures::Analysis,
    mut used: BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    for definition in module.physical.program.definitions.iter().rev() {
        let name = symbol_name(&definition.name);
        let output = used[&name].clone();
        let mut demand = |input: &Ident, extra: BTreeSet<String>| {
            let input = symbol_name(input);
            let heading = &analysis.layouts[&input];
            used.get_mut(&input).unwrap().extend(
                output
                    .iter()
                    .chain(&extra)
                    .filter(|c| heading.contains(c))
                    .cloned(),
            );
        };
        match &definition.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                demand(&s.input, columns_used(&s.condition))
            }
            Operator::Guard(g) => demand(
                &g.input,
                columns_used_rust(
                    &g.condition,
                    &analysis.layouts[&symbol_name(&g.input)],
                    g.environment.as_ref(),
                ),
            ),
            Operator::Bind(b) => demand(
                &b.input,
                columns_used_rust(
                    &b.expression,
                    &analysis.layouts[&symbol_name(&b.input)],
                    b.environment.as_ref(),
                ),
            ),
            Operator::Probe(_, k) | Operator::Merge(_, k) => {
                let keys: BTreeSet<_> = k.columns.iter().map(symbol_name).collect();
                demand(&k.left, keys.clone());
                demand(&k.right, keys);
            }
            Operator::Semi(_, k) | Operator::Anti(_, k) => {
                let keys: BTreeSet<_> = k.columns.iter().map(symbol_name).collect();
                demand(&k.left, keys.clone());
                used.get_mut(&symbol_name(&k.right)).unwrap().extend(keys);
            }
            Operator::Project(p) => demand(&p.input, BTreeSet::new()),
            Operator::Group(group) => demand(
                &group.input,
                analysis.layouts[&symbol_name(&group.input)]
                    .iter()
                    .cloned()
                    .collect(),
            ),
            Operator::Union(u) => {
                for operand in &u.operands {
                    demand(operand, BTreeSet::new());
                }
            }
            Operator::Rename(_) | Operator::Unit(_) => {}
        }
    }
    used.into_iter()
        .map(|(n, cs)| (n, cs.into_iter().collect()))
        .collect()
}

/// Witness demand ends at its forcing probe; it never enters the logical heading.
pub(super) fn carried_witnesses(
    module: &structures::Program,
    analysis: &structures::Analysis,
) -> BTreeMap<String, Vec<String>> {
    let definitions: BTreeMap<_, _> = module
        .physical
        .program
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), definition))
        .collect();
    let mut carried: BTreeMap<String, Vec<String>> = definitions
        .keys()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    for origin in analysis.witnesses.keys() {
        let share = &analysis.shares[origin];
        let Operator::Probe(_, probe) = &definitions[&share.force].operator else {
            unreachable!("checked witness force");
        };
        let mut current = symbol_name(&probe.left);
        loop {
            carried.get_mut(&current).unwrap().push(origin.clone());
            if current == *origin {
                break;
            }
            current = match &definitions[&current].operator {
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                    symbol_name(&s.input)
                }
                Operator::Project(p) => symbol_name(&p.input),
                Operator::Semi(_, k) | Operator::Anti(_, k) => symbol_name(&k.left),
                Operator::Bind(b) => symbol_name(&b.input),
                Operator::Guard(g) => symbol_name(&g.input),
                _ => unreachable!("checked witness forwarding chain"),
            };
        }
    }
    carried
}

impl Context<'_> {
    /// Demand an intermediate publication's columns in addition to final-answer
    /// columns. Propagate them through the same checked DAG rules before any
    /// row templates are emitted, so producer and consumer carriers agree.
    pub fn demand_columns(
        &mut self,
        module: &structures::Program,
        result: &str,
        columns: &[String],
    ) -> syn::Result<()> {
        let heading = self.analysis.layouts.get(result).ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "unknown publication demand")
        })?;
        if columns.iter().any(|column| !heading.contains(column)) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "publication demand contains a column outside its checked heading",
            ));
        }
        let mut used: BTreeMap<_, BTreeSet<_>> = self
            .carried_columns
            .iter()
            .map(|(name, columns)| (name.clone(), columns.iter().cloned().collect()))
            .collect();
        used.get_mut(result)
            .expect("checked publication demand")
            .extend(columns.iter().cloned());
        self.carried_columns = propagate_columns(module, self.analysis, used);
        Ok(())
    }

    /// Stable non-column binding for a retained lookup cursor.
    pub fn witness_ident(&self, origin: &str) -> Ident {
        format_ident!(
            "__cq_witness_{}",
            origin,
            span = proc_macro2::Span::mixed_site()
        )
    }

    /// Cursor consumed for this exact build and prefix at a recorded force.
    /// A Smallest force can consume independent cursors from both candidate sides.
    pub fn forcing_witness(&self, result: &str, build: &str, keys: &[String]) -> Option<Ident> {
        self.analysis
            .witnesses
            .keys()
            .find(|origin| {
                let share = &self.analysis.shares[*origin];
                share.force == result && share.build == build && share.keys == keys
            })
            .map(|origin| self.witness_ident(origin))
    }

    fn witness_fields(&self, result: &str) -> Vec<Ident> {
        self.carried_witnesses[result]
            .iter()
            .map(|origin| self.witness_ident(origin))
            .collect()
    }

    /// Whether this result carries base-relative row IDs rather than values.
    pub fn carries_positions(&self, result: &str) -> bool {
        matches!(
            self.analysis.repr.get(result),
            Some(structures::Repr::Positions { .. })
        )
    }

    pub(super) fn position_ident(&self, result: &str, index: usize) -> Ident {
        let mut occurrence = result;
        loop {
            occurrence = match &self.physical[occurrence].operator {
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                    self.physical
                        .get_key_value(&symbol_name(&s.input))
                        .unwrap()
                        .0
                }
                Operator::Project(p) => {
                    self.physical
                        .get_key_value(&symbol_name(&p.input))
                        .unwrap()
                        .0
                }
                Operator::Semi(_, k) | Operator::Anti(_, k) => {
                    self.physical
                        .get_key_value(&symbol_name(&k.left))
                        .unwrap()
                        .0
                }
                _ => break,
            };
        }
        format_ident!(
            "__cq_position_{}_{}",
            occurrence,
            index,
            span = proc_macro2::Span::mixed_site()
        )
    }

    /// Bind fetched logical values for predicate templates that use lexical names.
    pub fn position_aliases(&self, result: &str, columns: &[String]) -> Vec<TokenStream> {
        if !self.carries_positions(result) {
            return Vec::new();
        }
        columns
            .iter()
            .map(|column| {
                let ident = self.ident(result, column);
                let value = self.access(result, column);
                quote!(#[allow(unused_variables)] let #ident = #value;)
            })
            .collect()
    }

    /// Expression accessing one logical column in the current environment.
    /// Positional carriers project the host's borrowed row so String and scalar
    /// columns keep their exact `&T` type; key markers may normalize or copy keys.
    pub fn access(&self, result: &str, column: &str) -> TokenStream {
        assert!(
            column != "<witness>",
            "a lookup witness is not a logical column"
        );
        if let Some(structures::Repr::Positions { bases, .. }) = self.analysis.repr.get(result) {
            for (i, base) in bases.iter().enumerate() {
                if let Some(lineage) = base
                    .columns
                    .iter()
                    .find(|lineage| symbol_name(&lineage.logical) == column)
                {
                    let base_name = symbol_name(&base.base);
                    let host = Self::positional_host(&base_name);
                    let column = symbol_name(&lineage.column);
                    let index = self.analysis.host_layouts[&base_name]
                        .iter()
                        .position(|c| c == &column)
                        .expect("checked lineage column");
                    let id = self.position_ident(result, i);
                    let index = crate::index(index);
                    return quote!(::cq_storage::PositionalRows::row_at(#host, #id).expect("host-issued live ID").#index);
                }
            }
        }
        self.ident(result, column).to_token_stream()
    }

    /// Sorted columns carried between adapters, computed once from consumers.
    pub fn carried(&self, result: &str) -> &[String] {
        &self.carried_columns[result]
    }

    /// Bind selected columns of a tuple carrier; unread fields are wildcards.
    /// `borrowed` borrows owned computed values rather than moving them.
    pub fn bind_columns(&self, result: &str, columns: &[String], borrowed: bool) -> TokenStream {
        let witnesses = self
            .witness_fields(result)
            .into_iter()
            .map(|id| if borrowed { quote!(_) } else { quote!(#id) })
            .collect::<Vec<_>>();
        if let Some(structures::Repr::Positions { bases, owned, .. }) =
            self.analysis.repr.get(result)
        {
            let ids = (0..bases.len()).map(|i| self.position_ident(result, i));
            let values = owned.columns.iter().map(|column| {
                let column = symbol_name(column);
                if !columns.contains(&column) {
                    return quote!(_);
                }
                let id = self.ident(result, &column);
                if borrowed {
                    quote!(ref #id)
                } else {
                    quote!(#id)
                }
            });
            return if borrowed {
                quote!(&(#(#ids,)* #(#values,)* #(#witnesses,)*))
            } else {
                quote!((#(#ids,)* #(#values,)* #(#witnesses,)*))
            };
        }
        let fields = self.carried(result).iter().map(|c| {
            if !columns.contains(c) {
                return quote!(_);
            }
            let id = self.ident(result, c);
            if borrowed && self.infos[result].computed.contains(c) {
                quote!(ref #id)
            } else {
                quote!(#id)
            }
        });
        if borrowed {
            quote!(&(#(#fields,)* #(#witnesses,)*))
        } else {
            quote!((#(#fields,)* #(#witnesses,)*))
        }
    }

    /// Borrow only Rust's free lexical inputs from a carrier.
    pub fn rust_borrowed_pattern(
        &self,
        result: &str,
        expression: &Expr,
        environment: Option<&Environment>,
    ) -> TokenStream {
        if self.carries_positions(result) {
            let columns = columns_used_rust(expression, self.heading(result), environment);
            return self.bind_columns(result, &columns.into_iter().collect::<Vec<_>>(), true);
        }
        let used = columns_used_rust(expression, self.heading(result), environment);
        let fields = self.carried(result).iter().map(|c| {
            if !used.contains(c) {
                return quote!(_);
            }
            let id = &self.infos[result].bindings[c];
            if self.infos[result].computed.contains(c) {
                quote!(ref #id)
            } else {
                quote!(#id)
            }
        });
        let witnesses = self.carried_witnesses[result].iter().map(|_| quote!(_));
        quote!(&(#(#fields,)* #(#witnesses,)*))
    }

    /// Introduce lexical aliases only for Rust's free inputs.
    pub fn rust_aliases(
        &self,
        result: &str,
        expression: &Expr,
        environment: Option<&Environment>,
    ) -> Vec<TokenStream> {
        columns_used_rust(expression, self.heading(result), environment)
            .iter()
            .map(|c| {
                let logical = &self.infos[result].bindings[c];
                let value = self.access(result, c);
                if self.infos[result].computed.contains(c) {
                    quote!(#[allow(unused_variables)] let #logical = &#value;)
                } else {
                    quote!(#[allow(unused_variables)] let #logical = #value;)
                }
            })
            .collect()
    }

    /// Build the selected adapter carrier from column expressions.
    pub fn carrier(&self, result: &str, expression: impl Fn(&str) -> TokenStream) -> TokenStream {
        let witnesses = self.witness_fields(result);
        if let Some(structures::Repr::Positions { bases, owned, .. }) =
            self.analysis.repr.get(result)
        {
            let ids = (0..bases.len()).map(|i| self.position_ident(result, i));
            let values = owned
                .columns
                .iter()
                .map(|column| expression(&symbol_name(column)));
            return quote!((#(#ids,)* #(#values,)* #(#witnesses,)*));
        }
        let fields = self.carried(result).iter().map(|c| expression(c));
        quote!((#(#fields,)* #(#witnesses,)*))
    }

    /// Forward a subset after a filtering adapter without retaining dead columns.
    pub fn forward(&self, result: &str, input: &str, rows: TokenStream) -> TokenStream {
        if self.carried(result) == self.carried(input)
            && self.carries_positions(result) == self.carries_positions(input)
            && self.carried_witnesses[result] == self.carried_witnesses[input]
        {
            return rows;
        }
        let pattern = self.bind_columns(input, self.carried(result), false);
        let row = self.carrier(result, |c| self.access(input, c));
        quote!(::core::iter::Iterator::map(#rows, move |#pattern| #row))
    }
}
