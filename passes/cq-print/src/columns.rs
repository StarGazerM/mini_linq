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

impl Context<'_> {
    /// Expression accessing one logical column in the current environment.
    pub fn access(&self, result: &str, column: &str) -> TokenStream {
        self.ident(result, column).to_token_stream()
    }

    /// Sorted columns carried between adapters, computed once from consumers.
    pub fn carried(&self, result: &str) -> &[String] {
        &self.carried_columns[result]
    }

    /// Bind selected columns of a tuple carrier; unread fields are wildcards.
    /// `borrowed` borrows owned computed values rather than moving them.
    pub fn bind_columns(&self, result: &str, columns: &[String], borrowed: bool) -> TokenStream {
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
            quote!(&(#(#fields,)*))
        } else {
            quote!((#(#fields,)*))
        }
    }

    /// Borrow only Rust's free lexical inputs from a carrier.
    pub fn rust_borrowed_pattern(
        &self,
        result: &str,
        expression: &Expr,
        environment: Option<&Environment>,
    ) -> TokenStream {
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
        quote!(&(#(#fields,)*))
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
        let fields = self.carried(result).iter().map(|c| expression(c));
        quote!((#(#fields,)*))
    }

    /// Forward a subset after a filtering adapter without retaining dead columns.
    pub fn forward(&self, result: &str, input: &str, rows: TokenStream) -> TokenStream {
        if self.carried(result) == self.carried(input) {
            return rows;
        }
        let pattern = self.bind_columns(input, self.carried(result), false);
        let row = self.carrier(result, |c| self.access(input, c));
        quote!(::core::iter::Iterator::map(#rows, move |#pattern| #row))
    }
}
