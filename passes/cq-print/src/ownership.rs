//! Move shared dispatch state into each closure that needs it. Nested closures
//! get their own handles, so repeated probes never move out of an outer closure.
use super::*;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

pub fn own_captures(expression: &mut Expr, owned: &[Ident]) {
    struct Captures<'a> {
        owned: &'a [Ident],
        used: Vec<Ident>,
    }
    impl<'ast> Visit<'ast> for Captures<'_> {
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            if let Some(ident) = path.path.get_ident() {
                if self.owned.contains(ident) && !self.used.contains(ident) {
                    self.used.push(ident.clone());
                }
            }
        }
    }
    struct Owner<'a>(&'a [Ident]);
    impl VisitMut for Owner<'_> {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            syn::visit_mut::visit_expr_mut(self, expression);
            if let Expr::Closure(closure) = expression {
                if closure.capture.is_none() {
                    return;
                }
                let mut captures = Captures {
                    owned: self.0,
                    used: Vec::new(),
                };
                captures.visit_expr(&closure.body);
                if !captures.used.is_empty() {
                    let used = captures.used;
                    *expression = syn::parse_quote!({
                        #(let #used = ::core::clone::Clone::clone(&#used);)*
                        #closure
                    });
                }
            }
        }
    }
    Owner(owned).visit_expr_mut(expression);
}

/// Keep captured generator Rust intact outside fused `move` closures. Prepare
/// its computed predecessors too, so rustc can infer their item types before
/// checking a later callable's body. Type witnesses never evaluate a binding.
pub(super) fn binding_captures(
    module: &structures::Program,
    analysis: &structures::Analysis,
    statements: &mut Vec<Stmt>,
    used: &mut std::collections::BTreeSet<String>,
) -> BTreeMap<String, (Ident, Vec<Ident>)> {
    let definitions = &module.physical.program.definitions;
    let mut origins: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut required = std::collections::BTreeSet::new();
    for (index, definition) in definitions.iter().enumerate() {
        let name = symbol_name(&definition.name);
        let row = &analysis.physical.rows[&name];
        let mut inherited = BTreeMap::new();
        for operand in definition.operator.operands() {
            if let Some(columns) = origins.get(&symbol_name(operand)) {
                for (column, origin) in columns {
                    if row.computed.contains(column) {
                        inherited.entry(column.clone()).or_insert(*origin);
                    }
                }
            }
        }
        if let Operator::Group(group) = &definition.operator {
            for aggregate in &group.aggregates {
                inherited.remove(&symbol_name(&aggregate.name));
            }
        }
        if let Operator::Bind(bind) = &definition.operator {
            for binding in structures::syntax::rust::bindings(&bind.pattern) {
                inherited.insert(symbol_name(&binding), index);
            }
            let input = symbol_name(&bind.input);
            let columns = columns_used_rust(
                &bind.expression,
                &analysis.layouts[&input],
                bind.environment.as_ref(),
            );
            let free = structures::syntax::rust::free_variables(&bind.expression);
            if bind.for_token.is_some() && (free.opaque || !free.names.is_subset(&columns)) {
                required.insert(index);
            }
        }
        origins.insert(name, inherited);
    }
    // Definitions are dependency-ordered. Close the preparation set over the
    // bindings that supply each callable's computed input types.
    for index in (0..definitions.len()).rev() {
        if !required.contains(&index) {
            continue;
        }
        let Operator::Bind(bind) = &definitions[index].operator else {
            unreachable!();
        };
        let input = symbol_name(&bind.input);
        for column in columns_used_rust(
            &bind.expression,
            &analysis.layouts[&input],
            bind.environment.as_ref(),
        ) {
            if let Some(origin) = origins[&input].get(&column) {
                required.insert(*origin);
            }
        }
    }
    let mut functions = BTreeMap::new();
    let mut witnesses: BTreeMap<(usize, String), Ident> = BTreeMap::new();
    for index in required {
        let definition = &definitions[index];
        let Operator::Bind(bind) = &definition.operator else {
            unreachable!();
        };
        let input = symbol_name(&bind.input);
        let columns = columns_used_rust(
            &bind.expression,
            &analysis.layouts[&input],
            bind.environment.as_ref(),
        );
        let row = &analysis.physical.rows[&input];
        let arguments: Vec<_> = columns
            .iter()
            .map(|column| row.bindings[column].clone())
            .collect();
        let types = columns.iter().map(|column| &row.types[column]);
        let mut n = functions.len();
        while used.contains(&format!("__cq_binding_{n}")) {
            n += 1;
        }
        let name = format_ident!("__cq_binding_{n}", span = proc_macro2::Span::mixed_site());
        used.insert(name.to_string());
        let expression = &bind.expression;
        let generic_types: Vec<_> = (0..arguments.len())
            .map(|index| format_ident!("T{index}"))
            .collect();
        let inputs: Vec<_> = columns
            .iter()
            .zip(&generic_types)
            .map(|(column, ty)| {
                if row.computed.contains(column) {
                    quote!(&#ty)
                } else {
                    quote!(&'a #ty)
                }
            })
            .collect();
        let input_witnesses: Vec<_> = columns
            .iter()
            .map(|column| {
                if let Some(origin) = origins[&input].get(column) {
                    let witness = &witnesses[&(*origin, column.clone())];
                    quote!(#witness)
                } else {
                    let ty = &row.types[column];
                    quote!(&::core::marker::PhantomData::<#ty>)
                }
            })
            .collect();
        let pattern = &bind.pattern;
        let annotation = &bind.annotation;
        let bindings = structures::syntax::rust::bindings(pattern);
        let output_types: Vec<_> = (0..bindings.len())
            .map(|index| format_ident!("O{index}"))
            .collect();
        let output_witnesses: Vec<_> = bindings
            .iter()
            .enumerate()
            .map(|(ordinal, binding)| {
                let mut suffix = ordinal;
                while used.contains(&format!("__cq_binding_{n}_type_{suffix}")) {
                    suffix += 1;
                }
                let witness = format_ident!(
                    "__cq_binding_{n}_type_{suffix}",
                    span = proc_macro2::Span::mixed_site()
                );
                used.insert(witness.to_string());
                witnesses.insert((index, symbol_name(binding)), witness.clone());
                witness
            })
            .collect();
        let (result_bound, item) = if bind.for_token.is_some() {
            (
                quote!(R: ::core::iter::IntoIterator,),
                quote!(<R as ::core::iter::IntoIterator>::Item),
            )
        } else {
            (TokenStream::new(), quote!(R))
        };
        statements.push(syn::parse_quote!(
            let (#name, (#(#output_witnesses,)*)) = &{
                // Host arguments may lend values to the result. Computed
                // arguments are borrowed independently for each call.
                fn #name<'a, #(#generic_types: 'a,)* R, F, #(#output_types,)* P>(
                    _: (#(&::core::marker::PhantomData<#generic_types>,)*),
                    function: F,
                    _: P,
                ) -> (F, (#(::core::marker::PhantomData<#output_types>,)*))
                where
                    F: ::core::ops::Fn(#(#inputs),*) -> R,
                    #result_bound
                    P: ::core::ops::FnOnce(#item) -> (#(#output_types,)*),
                {
                    (function, (#(::core::marker::PhantomData::<#output_types>,)*))
                }
                #name(
                    (#(#input_witnesses,)*),
                    |#(#arguments: &#types),*| #expression,
                    |__cq_item| {
                        #[allow(unused_variables)]
                        let #pattern #annotation = __cq_item;
                        (#(#bindings,)*)
                    },
                )
            };
        ));
        functions.insert(symbol_name(&definition.name), (name, arguments));
    }
    functions
}
