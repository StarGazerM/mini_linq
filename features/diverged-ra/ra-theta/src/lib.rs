//! An independent theta-join surface lowered to a complete IX program.
//!
//! Its denotation is Set: the join is relational and its Rust guard is opaque,
//! pure and total. The guard must follow the binders of its free variables;
//! written order, panic positions and evaluation counts are not constraints.
//! Core relational predicates use `where`, rather than interpreting Rust guards.
//!
//! The public surface accepts declarations, `theta_join L { ... } with
//! R { ... } if expression;`, and `output answer(...);`. [`Query::indexes`]
//! authors exact demands in IX without invoking a core planner. A complete
//! `relational { ... }` input also forwards union/anti, block paths and origin
//! metadata. Anti requests its exact F key; union adds no demand. Meaning and
//! free-variable placement contracts remain the core language's.
//! Core blocks preserve union and ordered fallback through this consumer.
//! Their laws are join distribution over union and `B1 else B2 = B1 union
//! (B2 anti_F B1)`. Common interface bindings escape; locals do not. The complete
//! IR retains union/anti, nested occurrence paths and branch-local bindings.
//! Rust placement follows free-variable dependencies in each branch.
//!
use cq_lang_ix::{
    self as ix, ra,
    syntax::{self, symbol_name},
};
use quote::quote;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};
mod kw {
    syn::custom_keyword!(theta_join);
    syn::custom_keyword!(with);
    syn::custom_keyword!(output);
}
struct Binding {
    host: Ident,
    mapping: syntax::CommaList<ra::AttributeRename>,
}
impl Parse for Binding {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let host = input.parse()?;
        let content;
        syn::braced!(content in input);
        let mapping = syntax::CommaList::parse_terminated(&content)?;
        Ok(Self { host, mapping })
    }
}
pub struct Query {
    ra: ra::Program,
}

impl Parse for Query {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(ra::kw::relational) {
            let ra = input.parse()?;
            ra::contract::check(&ra)?;
            return Ok(Self { ra });
        }
        let declarations = syntax::parse_declarations(input)?;
        input.parse::<kw::theta_join>()?;
        let left: Binding = input.parse()?;
        input.parse::<kw::with>()?;
        let right: Binding = input.parse()?;
        input.parse::<Token![if]>()?;
        let condition: syn::Expr = input.parse()?;
        input.parse::<Token![;]>()?;
        input.parse::<kw::output>()?;
        let head: syntax::Atom = input.parse()?;
        input.parse::<Token![;]>()?;
        let (lh, lm, rh, rm) = (&left.host, &left.mapping, &right.host, &right.mapping);
        let kept = head.variables();
        let ra: ra::Program = syn::parse2(quote! {
            relational {
                #(#declarations)*
                l = rename #lh@0 { #lm };
                r = rename #rh@1 { #rm };
                j = natural_join l with r;
                s = guard j if (#condition);
                p = project s keep { #(#kept),* };
                output p as #head;
            }
        })?;
        let info = ra::contract::check(&ra)?;
        for (column, ty) in &info["l"].types {
            if info["r"].types.get(column).is_some_and(|right| right != ty) {
                return Err(syn::Error::new_spanned(
                    &head,
                    format!("theta_join attribute `{column}` has unequal types"),
                ));
            }
        }
        Ok(Self { ra })
    }
}
impl Query {
    /// Name the language-owned demands; preserve the theta dialect's index names.
    pub fn indexes(&self) -> syn::Result<ix::Program> {
        let info = ra::contract::check(&self.ra)?;
        let mut indexes = syntax::SemicolonList::new();
        for demand in ix::demands(&self.ra, &info) {
            let right = demand.relation;
            let (_, columns, supply) = demand.signature;
            indexes.push(ix::IndexDefinition {
                name: syn::Ident::new(&format!("theta_key_{}", indexes.len()), right.span()),
                eq_token: Default::default(),
                requirement: ix::IndexRequirement {
                    at_token: Default::default(),
                    relation: right.clone(),
                    bracket_token: Default::default(),
                    columns: columns.into_iter().map(syn::Index::from).collect(),
                    supply: supply.into(),
                },
            });
            indexes.push_punct(Default::default());
        }
        let program = ix::Program {
            ra: self.ra.clone(),
            indexes_token: Default::default(),
            indexes_brace: Default::default(),
            indexes,
        };
        ix::contract::check(&program)?;
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::{Database, PassCase, Rows, semantics};

    #[test]
    fn complete_union_and_anti_program_keeps_nested_occurrences() {
        let source: Query = syn::parse_quote! {relational {
            relation R(a:i32); relation S(a:i32);
            r = rename R@0.0.0 {a -> x}; s = rename S@0.1.0 {a -> x};
            a = anti [] s,r;
            u = union r,a block @0 free[] interface[x] fallback;
            output u as answer(x);
        }};
        let target = source.indexes().unwrap();
        assert_eq!(target.ra, source.ra);
        assert_eq!(
            ix::signature(&target.indexes[0].requirement),
            ("r".into(), vec![], syntax::SupplyKind::Stream)
        );
        assert!(matches!(
            target.ra.plan.definitions[3].operator,
            ra::Operator::Union(_)
        ));
    }

    #[test]
    fn complete_ix_fixture_and_invalid_descriptor_mutations() {
        let source: Query = syn::parse_str(include_str!("../../query.theta")).unwrap();
        let expected: ix::Program = syn::parse_quote! {
            relational {
                relation L(key: i32, value: i32);
                relation R(key: i32, value: i32);
                l = rename L@0 {key -> k, value -> x};
                r = rename R@1 {key -> k, value -> y};
                j = natural_join l with r;
                s = guard j if (x < y);
                p = project s keep {x, y};
                output p as answer(x, y);
            }
            indexes { theta_key_0 = @r[0] stream; }
        };
        let pass = PassCase {
            source,
            expected_next: Some(expected),
            compile: Box::new(Query::indexes),
            check: Box::new(|source: &Query, target: &ix::Program| {
                ix::contract::check(target)?;
                if source.ra != target.ra
                    || target.indexes.len() != 1
                    || ix::signature(&target.indexes[0].requirement)
                        != ("r".to_owned(), vec![0], syntax::SupplyKind::Stream)
                {
                    return Err(syn::Error::new_spanned(target, "theta demand mismatch"));
                }
                Ok(())
            }),
            mutations: vec![
                Box::new(|target: &ix::Program| {
                    let mut target = target.clone();
                    target.indexes[0].requirement.columns[0] = syn::Index::from(99);
                    target
                }),
                Box::new(|target: &ix::Program| {
                    let mut target = target.clone();
                    target.indexes.clear();
                    target
                }),
            ],
            answers: None,
        };
        let target = pass.run(None).unwrap();
        let database: Database = [
            (
                "L".to_owned(),
                Rows::from([vec![1, 2], vec![1, 9], vec![2, 3]]),
            ),
            ("R".to_owned(), Rows::from([vec![1, 5], vec![2, 7]])),
        ]
        .into();
        assert_eq!(
            semantics::evaluate_ra(&target.ra, &database).output,
            Rows::from([vec![2, 5], vec![3, 7]])
        );
    }

    #[test]
    fn demand_ordinals_follow_the_logical_right_heading() {
        let query: Query = syn::parse_quote! {
            relation L(key: i32, value: i32);
            relation R(key: i32, value: i32);
            theta_join L {key -> k, value -> z}
                with R {key -> k, value -> a} if z < a;
            output answer(z, a);
        };
        let indexes = query.indexes().unwrap();
        assert_eq!(
            ix::signature(&indexes.indexes[0].requirement),
            ("r".to_owned(), vec![1], syntax::SupplyKind::Stream)
        );
    }

    #[test]
    fn cartesian_theta_join_records_the_unit_key() {
        let query: Query = syn::parse_quote! {
            relation L(value: i32); relation R(value: i32);
            theta_join L {value -> x} with R {value -> y} if x < y;
            output answer(x, y);
        };
        let indexes = query.indexes().unwrap();
        assert_eq!(
            ix::signature(&indexes.indexes[0].requirement),
            ("r".to_owned(), vec![], syntax::SupplyKind::Stream)
        );
    }

    #[test]
    fn invalid_surfaces_return_errors_without_panicking() {
        let valid = include_str!("../../query.theta");
        for invalid in [
            valid.replace("R { key", "Unknown { key"),
            valid.replace("value -> y", "missing -> y"),
            valid.replace("answer(x, y)", "answer(unbound, y)"),
            valid.replace("R(key: i32", "R(key: String"),
            valid.trim_end().trim_end_matches(';').to_owned(),
        ] {
            assert!(syn::parse_str::<Query>(&invalid).is_err(), "{invalid}");
        }
    }
}
