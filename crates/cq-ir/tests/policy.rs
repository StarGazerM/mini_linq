use cq_ir::policy::Policy;
use quote::{ToTokens, quote};

#[test]
fn projection_strategy_defaults_to_keep_and_round_trips() {
    for (tokens, canon) in [
        (quote! { policy { R; } }, false),
        (quote! { policy { R; project = keep; } }, false),
        (quote! { policy { R; project = canon; } }, true),
        (quote! { policy { project = canon; } }, true),
        (
            quote! {
                policy {
                    R keyed [a] ordered canon;
                    join = merge;
                    semijoin = prefer_lazy;
                    project = canon;
                }
            },
            true,
        ),
    ] {
        let policy: Policy = syn::parse2(tokens).unwrap();
        assert_eq!(policy.canon_projections(), canon);
        let roundtrip: Policy = syn::parse2(policy.to_token_stream()).unwrap();
        assert_eq!(policy, roundtrip);
    }
}

#[test]
fn projection_strategy_rejects_unknown_duplicate_and_unterminated_choices() {
    for tokens in [
        quote! { policy { R; project = eager; } },
        quote! { policy { R; project = canon; project = keep; } },
        quote! { policy { R; project = canon } },
    ] {
        assert!(syn::parse2::<Policy>(tokens).is_err());
    }
}
