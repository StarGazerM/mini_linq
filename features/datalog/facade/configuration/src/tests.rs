use super::*;
use quote::ToTokens;

#[test]
fn defaults_include_surface_and_every_checked_stage() {
    let queue = DatalogConfiguration::default().resolve(DatalogConfiguration::defaults()).unwrap();
    let names: Vec<_> = queue.passes.iter().map(|pass|
        pass.path.segments.last().unwrap().ident.to_string()).collect();
    assert_eq!(names, ["surface_if_let", "stratify", "incremental", "storage", "fork", "print_rounds"]);
    assert!(queue.passes[4].arguments.to_string().contains("cq = pipeline"));
}

#[test]
fn named_phases_preserve_order_and_opaque_arguments() {
    let config: DatalogConfiguration = syn::parse_quote! { pipeline {
        surface = [sugar(opaque(tokens)), sugar_two];
        stratify = components(one);
        incremental = variants(two);
        storage = stores(three);
        storage_rewrites = [rewrite_a(four), rewrite_b];
        fork = holes(five;);
        round = [round_a(six), round_b];
        print = terminal(seven);
        cq = pipeline {
            surface = [cq_surface(opaque(tokens))];
            plan = planner(no_reduce);
            physical = [physical_a(eight), physical_b];
            structures = [structures(nine)];
        };
    } };
    let queue = config.resolve(DatalogConfiguration::defaults()).unwrap();
    let cq: Pipeline = syn::parse_quote! { pipeline {
        cq_surface(opaque(tokens)), planner(no_reduce),
        ::mini_linq::passes::index_demands, ::mini_linq::passes::lower,
        physical_a(eight), physical_b, ::mini_linq::passes::select,
        structures(nine), ::mini_linq::passes::represent,
        ::mini_linq::passes::stage, ::mini_linq::passes::print_pull
    } };
    let expected: Pipeline = syn::parse_quote! { pipeline {
        sugar(opaque(tokens)), sugar_two, components(one), variants(two), stores(three),
        rewrite_a(four), rewrite_b, holes(five; cq = #cq;),
        round_a(six), round_b, terminal(seven)
    } };
    assert_eq!(queue.to_token_stream().to_string(), expected.to_token_stream().to_string());
}

#[test]
fn explicit_empty_surface_disables_the_default_expander() {
    let config: DatalogConfiguration = syn::parse_quote!(pipeline { surface = []; });
    let queue = config.resolve(DatalogConfiguration::defaults()).unwrap();
    assert_eq!(queue.passes[0].path.segments.last().unwrap().ident, "stratify");
    assert_eq!(queue.passes.len(), 5);
}

#[test]
fn malformed_or_duplicate_phases_are_rejected() {
    for tokens in [
        quote!(pipeline { surface = []; surface = []; }),
        quote!(pipeline { cq = pipeline {}; cq = pipeline {}; }),
        quote!(pipeline { unknown = pass; }),
        quote!(pipeline { plan = pass; }),
        quote!(pipeline { cq = pipeline { plan = p; plan = q; }; }),
        quote!(pipeline { round = rewrite; }),
    ] {
        assert!(syn::parse2::<DatalogConfiguration>(tokens).is_err());
    }
    assert!(DatalogConfiguration::default().resolve(DatalogConfiguration::default()).is_err());
}

#[test]
fn explicit_defaults_and_timing_are_recorded_in_fork_arguments() {
    let queue = DatalogConfiguration::default().resolve(DatalogConfiguration::defaults()).unwrap();
    let args = queue.passes[4].arguments.to_string();
    assert_eq!(args.contains("timing = phases"), cfg!(feature = "phase-timing"));
    let custom: DatalogConfiguration = syn::parse_quote!(pipeline { fork = custom(timing = none;); });
    let queue = custom.resolve(DatalogConfiguration::defaults()).unwrap();
    assert!(queue.passes[4].arguments.to_string().starts_with("timing = none ; cq = pipeline"));
}
