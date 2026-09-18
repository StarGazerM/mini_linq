use cq_ir::{index_requirements, relational_plan};
use quote::quote;
type IndexSignature = (String, Vec<usize>);
mod tests {
    use syn::visit::Visit;

    use super::*;
    use cq_compose::compile_cq;
    use cq_index_policy::base_demands::compile as compile_input_indexes;
    use cq_ir::cq;
    use cq_relation_lowering::base_hash::compile;
    use proc_macro2::TokenStream;

    fn index_signature(index: &index_requirements::IndexRequirement) -> IndexSignature {
        (
            cq_ir::symbol_name(&index.relation),
            index
                .key_columns
                .iter()
                .map(|column| column.index as usize)
                .collect(),
        )
    }

    #[test]
    fn triangle_relational_plan_and_index_overlay_are_locally_valid() {
        let source: cq::Module = syn::parse2(quote! {
            pub struct TriangleProgram;
            relation R(src: i32, dst: i32);
            relation S(src: i32, dst: i32);
            relation T(src: i32, dst: i32);
            triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
        })
        .unwrap();
        cq::contract::check(&source).unwrap();

        let logical = compile_cq(&source).unwrap();
        relational_plan::contract::check(&logical).unwrap();
        let indexed = compile_input_indexes(&logical).unwrap();
        index_requirements::contract::check(&indexed).unwrap();
    }

    #[test]
    fn indexes_and_physical_labels_follow_relational_order_not_cq_text_order() {
        let logical: relational_plan::Module = syn::parse2(quote! {
            struct Reordered;
            relation R(src: i32, dst: i32);
            relation S(src: i32, dst: i32);
            relation T(src: i32, dst: i32);
            answer(x, w) :- R(x, y), S(y, z), T(z, w);
            relational {
                r0 = rename T {src -> z, dst -> w};
                r1 = rename S {src -> y, dst -> z};
                r2 = natural_join r0 with r1;
                r3 = rename R {src -> x, dst -> y};
                r4 = natural_join r2 with r3;
                r5 = project r4 keep {x, w};
                output r5 as answer(x, w);
            }
        })
        .unwrap();
        relational_plan::contract::check(&logical).unwrap();

        let indexed = compile_input_indexes(&logical).unwrap();
        let signatures = indexed
            .indexes
            .iter()
            .map(index_signature)
            .collect::<Vec<_>>();
        assert_eq!(
            signatures,
            vec![("S".to_owned(), vec![1]), ("R".to_owned(), vec![1])]
        );

        let file = compile(
            &indexed,
            &syn::parse_quote!(::mini_linq::access_to_distinct_iter),
        )
        .unwrap();
        #[derive(Default)]
        struct PullFinder {
            tokens: Option<TokenStream>,
        }
        impl<'ast> Visit<'ast> for PullFinder {
            fn visit_macro(&mut self, node: &'ast syn::Macro) {
                if node
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "access_to_distinct_iter")
                {
                    self.tokens = Some(node.tokens.clone());
                }
                syn::visit::visit_macro(self, node);
            }
        }
        let mut finder = PullFinder::default();
        finder.visit_file(&file);
        let access: cq_ir::rust_access_plan::Plan = syn::parse2(finder.tokens.unwrap()).unwrap();
        let labels = access
            .body
            .iter()
            .map(|clause| match &clause.item {
                cq::BodyItem::Positive { atom } => cq_ir::symbol_name(&atom.relation),
                _ => panic!("positive input-index fixture"),
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, ["T", "S", "R"]);
        assert_eq!(access.head.atom, logical.program.query.head);
    }

    #[test]
    fn basic_backend_rejects_missing_or_nonfinal_project() {
        let unsupported = [
            quote! {
                struct MissingProject;
                relation R(src: i32, dst: i32);
                answer(x, y) :- R(x, y);
                relational {
                    r0 = rename R {src -> x, dst -> y};
                    output r0 as answer(x, y);
                }
            },
            quote! {
                struct NonfinalProject;
                relation R(src: i32, dst: i32);
                answer(x) :- R(x, y);
                relational {
                    r0 = rename R {src -> x, dst -> y};
                    r1 = project r0 keep {x, y};
                    r2 = project r1 keep {x};
                    output r2 as answer(x);
                }
            },
        ];

        for source in unsupported {
            let logical: relational_plan::Module = syn::parse2(source).unwrap();
            relational_plan::contract::check(&logical).unwrap();
            assert!(compile_input_indexes(&logical).is_err());
        }
    }
}
