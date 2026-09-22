//! Recovered tests from crates/cq-index-policy/src/no_demands.rs.
use cq_index_policy::no_demands::*;
use cq_ir::relational_plan as ra;
#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> ra::Module {
        syn::parse_quote! {
            struct Shared;
            relation R(key: i32, value: String);
            relation S(key: i32);
            answer(v) :- R(k, v), S(k);
            relational {
                r = rename R { key -> k, value -> v };
                s = rename S { key -> k };
                identity = unit;
                seed = natural_join identity with r;
                left = semijoin seed with s;
                right = semijoin s with r;
                joined = natural_join left with right;
                result = project joined keep { v };
                output result as answer(v);
            }
        }
    }

    #[test]
    fn an_empty_overlay_preserves_the_complete_shared_relational_plan() {
        let source = source();
        let target = compile(&source).unwrap();
        ix::contract::check(&target).unwrap();
        assert_eq!(target.relational_plan, source);
        assert!(target.indexes.is_empty());
    }

    #[test]
    fn index_policy_does_not_assume_the_downstream_execution_domain() {
        let source = syn::parse_quote! {
            struct Selected;
            relation R(value: i32);
            relation answer(value: i32);
            answer(x + 1) :- R(x), if x > 0;
            relational {
                r = rename R { value -> x };
                selected = select r if (x > 0);
                output selected as answer(x + 1);
            }
        };
        let target = compile(&source).unwrap();
        ix::contract::check(&target).unwrap();
        assert_eq!(target.relational_plan, source);
        assert!(target.indexes.is_empty());
    }

    #[test]
    fn invalid_ra_is_rejected_before_adding_an_overlay() {
        let mut source = source();
        source.plan.output.input = syn::parse_quote!(missing);
        assert!(
            compile(&source)
                .unwrap_err()
                .to_string()
                .contains("not defined")
        );
    }
}
