use cq_lang_physical::policy::Policy;
use quote::{ToTokens, quote};

#[test]
fn projection_strategy_defaults_to_keep_and_round_trips() {
    for (tokens, canon) in [
        (quote! { policy {  } }, false),
        (quote! { policy {  project = keep; } }, false),
        (quote! { policy {  project = canon; } }, true),
        (quote! { policy { project = canon; } }, true),
        (
            quote! {
                policy {
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
        quote! { policy {  project = eager; } },
        quote! { policy {  project = canon; project = keep; } },
        quote! { policy {  project = canon } },
    ] {
        assert!(syn::parse2::<Policy>(tokens).is_err());
    }
}

#[test]
fn policy_rejects_removed_host_entries() {
    for text in ["policy { R; }", "policy { R supports [a]; }"] {
        assert!(syn::parse_str::<Policy>(text).is_err());
    }
}
