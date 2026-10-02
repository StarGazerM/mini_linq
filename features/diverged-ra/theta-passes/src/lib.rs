//! External macro adapters for IX → Physical → Structures → Rust.
//!
//! Each adapter uses the public checked cq-stage protocol. The merge rewrite
//! writes ordered joins into Physical and checks its correspondence; selection
//! then records the concrete ordered readers in Structures. No planner or
//! facade entry point participates in this pipeline.

use cq_lang_physical as physical;
use proc_macro::TokenStream;

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "this example pass accepts no arguments",
        ))
    }
}

fn lower_stage(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    cq_stage::pass::<cq_lang_ix::Program, physical::Program>(input, |source, arguments| {
        no_arguments(arguments)?;
        cq_physical_lowering::compile(source)
    })
}

/// Start at IX with an explicit remaining pass queue.
#[proc_macro]
pub fn lower(input: TokenStream) -> TokenStream {
    lower_stage(input.into()).into()
}

fn compile_merge(source: &physical::Program) -> syn::Result<physical::Program> {
    physical::contract::check(source)?;
    let mut target = source.clone();
    for definition in &mut target.program.definitions {
        if let physical::Operator::Probe(_, keyed) = &definition.operator {
            let mut ordered = keyed.clone();
            ordered.choice = None;
            definition.operator = physical::Operator::Merge(Default::default(), ordered);
        }
    }
    physical::contract::check(&target)?;
    Ok(target)
}

/// Record merge decisions in Physical, preserving RA erasure and Rust order.
#[proc_macro]
pub fn merge(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        compile_merge(source)
    })
    .into()
}

/// Select concrete default representations from the written Physical program.
#[proc_macro]
pub fn select(input: TokenStream) -> TokenStream {
    cq_stage::pass::<physical::Program, cq_lang_structures::Program>(
        input.into(),
        |source, arguments| {
            no_arguments(arguments)?;
            cq_structure_selection::compile(source)
        },
    )
    .into()
}

/// End the queue with a Rust pull expression.
#[proc_macro]
pub fn print_pull(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_physical_realization::compile(source)
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::PassCase;
    use quote::quote;

    fn indexes() -> cq_lang_ix::Program {
        syn::parse_str::<ra_theta::Query>(include_str!("../../query.theta"))
            .unwrap()
            .indexes()
            .unwrap()
    }

    #[test]
    fn merge_is_a_checked_written_decision() {
        let source = cq_physical_lowering::compile(&indexes()).unwrap();
        let expected: physical::Program = syn::parse_quote! {
            physical {
                relation L(key: i32, value: i32);
                relation R(key: i32, value: i32);
                l = pipe rename L { key -> k, value -> x };
                r = pipe rename R { key -> k, value -> y };
                j = pipe merge [k] l, r;
                s = pipe guard j if (x < y);
                p = pipe canon project s keep {x, y};
                return p as answer(x, y);
            }
        };
        let pass = PassCase {
            source,
            expected_next: Some(expected),
            compile: Box::new(compile_merge),
            check: Box::new(physical::check_rewrite),
            mutations: vec![Box::new(|target: &physical::Program| {
                let mut target = target.clone();
                let physical::Operator::Guard(guard) = &mut target.program.definitions[3].operator
                else {
                    panic!("fixture guard");
                };
                guard.condition = syn::parse_quote!(x > y);
                target
            })],
            answers: None,
        };
        let target = pass.run(None).unwrap();
        cq_physical_lowering::check(&indexes(), &target).unwrap();
        let selected = cq_structure_selection::compile(&target).unwrap();
        assert_eq!(selected.physical, target);
        cq_structure_selection::check(&target, &selected).unwrap();
        cq_physical_realization::compile(&selected).unwrap();
    }

    #[test]
    fn lowering_continues_only_with_an_explicit_printer_queue() {
        let source = indexes();
        let output = lower_stage(quote! {
            pipeline { theta_passes::select, theta_passes::print_pull }; #source
        });
        assert!(output.to_string().starts_with("theta_passes :: select !"));
        let missing = lower_stage(quote!(pipeline {}; #source)).to_string();
        assert!(missing.contains("pipeline ended before its Rust printer"));
        let arguments = lower_stage(quote! {
            arguments { unexpected }; pipeline { theta_passes::select }; #source
        })
        .to_string();
        assert!(arguments.contains("this example pass accepts no arguments"));
    }

    #[test]
    fn ordered_route_rejects_a_cartesian_join() {
        let source: physical::Program = syn::parse_quote! {
            physical {
                relation L(x: i32); relation R(y: i32);
                l = pipe rename L {x -> x};
                r = pipe rename R {y -> y};
                j = pipe canon probe first [] l, r;
                return j as answer(x, y);
            }
        };
        assert!(compile_merge(&source).is_err());
    }
}
