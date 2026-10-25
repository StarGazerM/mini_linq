//! Target-owned planning on the checked Sequence pipeline protocol.
//!
//! Use `mini_linq::passes::plan(cq_sequence_tasks_macros::plan)` after authored
//! advice in `decompose`, or invoke `plan` directly in a checked pass queue.

use proc_macro::TokenStream;

struct PlanningOptions {
    threshold: usize,
    policies: Vec<cq_sequence_plan::KeyPartitionPolicy>,
}

impl syn::parse::Parse for PlanningOptions {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut threshold = None;
        let mut policies = Vec::new();
        while !input.is_empty() {
            let option: syn::Ident = input.parse()?;
            if option == "threshold" && threshold.is_none() {
                let value: syn::LitInt = input.parse()?;
                let value = value.base10_parse::<usize>()?;
                if value == 0 {
                    return Err(syn::Error::new_spanned(option, "provider partition threshold must be positive"));
                }
                threshold = Some(value);
            } else if option == "key_partition" {
                let policy;
                syn::parenthesized!(policy in input);
                policies.push(policy.parse()?);
            } else {
                return Err(syn::Error::new_spanned(option,
                    "expected threshold N or key_partition(name, select true|false)"));
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(Self {
            threshold: threshold.unwrap_or(cq_sequence_tasks::Config::default().key_partition_threshold),
            policies,
        })
    }
}

/// Fill only execution decomposition using the Tasks machine and catalog; the
/// relational order stays the authored or written one (no automatic reordering).
/// `threshold N` configures the machine's recorded adaptive-build guard.
/// `key_partition(name, select true|false)` forces or forbids routing.
#[proc_macro]
pub fn plan(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, arguments| {
        let options: PlanningOptions = syn::parse2(arguments)?;
        let machine = cq_sequence_tasks::machine_with(cq_sequence_tasks::Config {
            key_partition_threshold: options.threshold,
            ..Default::default()
        });
        let catalog = cq_sequence_tasks::catalog();
        let source = cq_sequence_tasks::select_providers(source, &catalog)?;
        Ok(cq_sequence_plan::decompose_with_key_partitions(
            &source,
            &machine,
            &catalog,
            &options.policies,
        )?
        .program)
    })
    .into()
}

/// Check the target-owned schedule before interpretation.
#[proc_macro]
pub fn tasks(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, _| {
        cq_lang_sequence::check_grading(source, &cq_sequence_tasks::machine())?;
        Ok(source.clone())
    }).into()
}

/// Return joined owned answers, selecting the runtime's plain/profile build.
#[proc_macro]
pub fn translate_tasks(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, _| {
        let analysis = cq_lang_sequence::check_grading(source, &cq_sequence_tasks::machine())?;
        let plain = cq_sequence_tasks::translate(source, &analysis)?;
        let profile = cq_sequence_tasks::translate_profile(source, &analysis)?;
        Ok(syn::parse_quote!(::cq_storage::profile_select! {
            plain { (#plain).into_iter() }
            counters { (#profile).into_iter() }
        }))
    }).into()
}

#[cfg(test)]
mod tests {
    use super::PlanningOptions;

    #[test]
    fn machine_threshold_and_author_overrides_parse() {
        let options: PlanningOptions = syn::parse_str(
            "threshold 8192, key_partition(small, select false), key_partition(big, select true)"
        ).unwrap();
        assert_eq!(options.threshold, 8192);
        assert!(!options.policies[0].select);
        assert!(options.policies[1].select);
        assert!(syn::parse_str::<PlanningOptions>("threshold 0").is_err());
        assert!(syn::parse_str::<PlanningOptions>("threshold 8, threshold 16").is_err());
    }
}
