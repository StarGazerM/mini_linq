//! Recovered tests from xtask/src/runner.rs.
use crate::runner::*;
use quote::{ToTokens, quote};
use syn::visit_mut::VisitMut;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_runner_expands_both_typed_iterator_stages() {
        let mut file: syn::File = syn::parse2(quote! {
            fn query(left: &::std::vec::Vec<(i32,)>) {
                ::mini_linq::access_to_distinct_iter! {
                    answer(x) => ((::core::clone::Clone::clone(x),)) :-
                        Left(x) => for (x,) in (left.iter());
                }
            }
        })
        .unwrap();

        let mut expander = IteratorPipelineExpander::default();
        expander.visit_file_mut(&mut file);

        assert!(expander.error.is_none());
        assert_eq!(expander.pull_expansions, 1);
        assert_eq!(expander.pipeline_expansions, 1);
        let emitted = file.to_token_stream().to_string();
        assert!(!emitted.contains("mini_linq"));
        assert!(emitted.contains("once_with"));
        assert!(emitted.contains("Iterator :: map"));
        assert!(emitted.contains("HashSet :: new"));
    }
}
