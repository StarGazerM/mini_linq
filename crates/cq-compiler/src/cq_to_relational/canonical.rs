//! Written-order CQ lowering: canonical atoms or an expression-preserving prefix.

use cq_ir::{cq, relational_plan};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeMap;

/// Lower one positive CQ to the canonical named left-deep relational plan.
#[contracts::contract(
    debug_ensures(
        cq::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| crate::pass_contracts::canonical::relational_plan_is_exact(source, target))
    ),
)]
pub fn compile_cq(source: &cq::Module) -> syn::Result<relational_plan::Module> {
    cq::contract::check(source)?;
    if !source.program.query.is_basic() {
        let analysis = crate::cq_to_relational::dependencies::ClauseDependencies::analyze(source)?;
        return crate::cq_to_relational::clause_rules::plan(
            source,
            &analysis,
            crate::PlanningOptions {
                strategy: crate::JoinPolicy::Written,
                max_arity: None,
            },
        )
        .map(|(plan, _)| plan);
    }
    let declarations = source
        .program
        .inputs()
        .map(|input| (cq_ir::symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut definitions = Vec::<TokenStream>::new();
    let mut next_result = 0usize;
    let mut current = None::<syn::Ident>;

    for item in &source.program.query.body {
        let atom = item.positive_atom()?;
        let declaration = declarations
            .get(&cq_ir::symbol_name(&atom.relation))
            .expect("the CQ contract requires every body relation to be declared");
        let rename = local_ident("r", next_result);
        next_result += 1;
        let mapping = declaration
            .columns
            .iter()
            .zip(atom.variables())
            .map(|(column, variable)| {
                let source = &column.name;
                quote!(#source -> #variable)
            })
            .collect::<Vec<_>>();
        let relation = &atom.relation;
        definitions.push(quote! {
            #rename = rename #relation { #(#mapping),* };
        });

        if let Some(left) = current {
            let result = local_ident("r", next_result);
            next_result += 1;
            definitions.push(quote! {
                #result = natural_join #left with #rename;
            });
            current = Some(result);
        } else {
            current = Some(rename);
        }
    }

    let input = current.expect("a CQ contract requires a nonempty body");
    let project = local_ident("r", next_result);
    let head = &source.program.query.head;
    let attributes = head.variables();
    definitions.push(quote! {
        #project = project #input keep { #(#attributes),* };
    });

    let program = &source.program;
    syn::parse2(quote! {
        #program
        relational {
            #(#definitions)*
            output #project as #head;
        }
    })
}

fn local_ident(prefix: &str, number: usize) -> syn::Ident {
    format_ident!("{prefix}{number}", span = Span::mixed_site())
}
