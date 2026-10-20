//! Rayon candidate evaluation with ordered, sequential admission/publication.
//! Storage, observers, timing, output conversion and CQ splicing remain owned
//! by the reusable base printer. No mutable relation is shared with a worker.
use cq_lang_datalog_round::{
    self as round, Execution, InputSource, Program, RoundContext, RuleHole,
};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

pub fn compile(program: &Program) -> syn::Result<syn::Expr> {
    let source = round::erase(program)?;
    cq_datalog_partition::check(&source, program)?;
    cq_datalog_print::compile_with(program, evaluate)
}

fn evaluate(
    rules: &[RuleHole],
    context: &RoundContext,
    holes: &[TokenStream],
    execution: &[Execution],
) -> syn::Result<(TokenStream, TokenStream)> {
    let mut declarations = Vec::new();
    let mut evaluations = Vec::new();
    let mut admissions = Vec::new();
    let scope = Ident::new("__cq_scope", Span::mixed_site());
    let source_name = Ident::new("__cq_source", Span::mixed_site());
    let evaluator = Ident::new("__cq_evaluate", Span::mixed_site());
    let grain = Ident::new("__cq_grain", Span::mixed_site());
    let bucket = Ident::new("__cq_bucket", Span::mixed_site());
    let admitted_chunk = Ident::new("__cq_admitted_chunk", Span::mixed_site());
    for rule in rules {
        let candidates = format_ident!("__cq_candidates_{}", rule.hole, span = Span::mixed_site());
        let invocation = &holes[rule.hole];
        let head = &context.relations[rule.head];
        let types = head.declaration.columns.iter().map(|column| &column.ty);
        let consume = quote!((#invocation).collect::<::std::vec::Vec<(#(#types,)*)>>());
        let evaluation = if let Some(scan) = &rule.scan {
            let annotation = execution
                .iter()
                .find(|annotation| annotation.hole == rule.hole)
                .expect("checked execution coverage");
            let chunk = &annotation.binding;
            let (source, relation, columns) = match scan.source {
                InputSource::Delta(index) => {
                    let relation = &context.relations[index];
                    let host = relation.delta_host.as_ref().expect("checked delta host");
                    (
                        quote!(#host.as_slice()),
                        relation,
                        (0..relation.declaration.arity()).collect::<Vec<_>>(),
                    )
                }
                InputSource::Layout(index) => {
                    let layout = &context.layouts[index];
                    let host = &layout.host;
                    (
                        quote!(#host.get(&()).map_or(&[][..], |#bucket| #bucket.as_slice())),
                        &context.relations[layout.relation],
                        layout.columns.clone(),
                    )
                }
            };
            let input_types = columns
                .iter()
                .map(|&column| &relation.declaration.columns[column].ty);
            quote!({
                let #source_name = #source;
                let #evaluator = |#chunk: &[(#(#input_types,)*)]| { #consume };
                let #grain = ::core::cmp::max(1024, #source_name.len() / (4 * ::rayon::current_num_threads()));
                if #source_name.len() <= #grain {
                    ::std::vec![#evaluator(#source_name)]
                } else {
                    use ::rayon::prelude::*;
                    #source_name.par_chunks(#grain).map(#evaluator).collect::<::std::vec::Vec<_>>()
                }
            })
        } else {
            quote!(::std::vec![#consume])
        };
        declarations.push(quote!(let mut #candidates = ::std::vec::Vec::new();));
        evaluations.push(quote!(#scope.spawn(|_| { #candidates = #evaluation; });));
        let insert = cq_datalog_print::insert_new(&head.seen, &head.next, &quote!(#admitted_chunk));
        admissions.push(quote!(for #admitted_chunk in #candidates { #insert }));
    }
    let evaluation = if rules.is_empty() {
        TokenStream::new()
    } else {
        quote!(#(#declarations)* ::rayon::scope(|#scope| { #(#evaluations)* });)
    };
    Ok((evaluation, quote!(#(#admissions)*)))
}
