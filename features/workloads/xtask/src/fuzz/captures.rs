//! Transparent runtime observation of query-closed expression evaluation.
use cq_lang_cq::{BodyItem, CommaList, Program, rust};
use quote::ToTokens;
use std::collections::BTreeSet;

pub fn instrument(source: &str) -> (String, usize) {
    let mut program: Program = syn::parse_str(source).expect("generated CQ");
    let mut bound = BTreeSet::new();
    for item in cq_lang_cq::blocks::walk(&program.query.body) {
        match item {
            BodyItem::Positive { atom } => {
                bound.extend(atom.variables().map(cq_lang_cq::symbol_name));
            }
            BodyItem::Let(binding) => {
                bound.extend(
                    rust::bindings(&binding.pattern)
                        .iter()
                        .map(cq_lang_cq::symbol_name),
                );
            }
            BodyItem::For(binding) => {
                bound.extend(
                    rust::bindings(&binding.pattern)
                        .iter()
                        .map(cq_lang_cq::symbol_name),
                );
            }
            _ => {}
        }
    }
    if program.query.group.is_some() {
        bound.extend(program.query.head.variables().map(cq_lang_cq::symbol_name));
    }
    let mut slots = 0;
    fn wrap(expression: &mut syn::Expr, bound: &BTreeSet<String>, slots: &mut usize) {
        let free = rust::free_variables(expression);
        if free.opaque || !free.dependencies(bound).is_empty() {
            return;
        }
        let slot = *slots;
        *slots += 1;
        *expression = syn::parse_quote!(self::count_capture(#slot, #expression));
    }
    fn body(items: &mut CommaList<BodyItem>, bound: &BTreeSet<String>, slots: &mut usize) {
        for item in items {
            match item {
                BodyItem::Let(binding) => wrap(&mut binding.expression, bound, slots),
                BodyItem::Filter(filter) => wrap(&mut filter.condition, bound, slots),
                BodyItem::Block(block) => {
                    for branch in &mut block.branches {
                        body(&mut branch.body, bound, slots);
                    }
                }
                _ => {}
            }
        }
    }
    body(&mut program.query.body, &bound, &mut slots);
    for expression in &mut program.query.head.args {
        // A fold head (`sum(c)`) must stay an aggregate call; wrapping it would
        // turn it into a group key.
        if !matches!(cq_lang_cq::group::aggregate(expression), Ok(None)) {
            continue;
        }
        wrap(expression, &bound, &mut slots);
    }
    (program.into_token_stream().to_string(), slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_query_closed_let_guard_and_head_sites_are_counted() {
        let (_, slots) = instrument(
            "struct Q; relation R(x:i32); answer(x, host) :- R(x),
             let c:i32 = host, let d:i32 = *x + 1,
             if host > 0, if *x > 0;",
        );
        assert_eq!(slots, 3);
    }

    #[test]
    fn branch_local_binders_are_not_mistaken_for_host_captures() {
        let (_, slots) = instrument(
            "struct Q; relation R(x:i32); answer(x) :-
             { R(x), let y:i32 = *x + 1, if y > 0 }
             | { R(x), let z:i32 = host, if *x > 0 };",
        );
        assert_eq!(slots, 1);
    }

    #[test]
    fn closed_fold_heads_stay_aggregates() {
        let (instrumented, slots) = instrument(
            "struct Q; relation R(x:i32); relation answer(x:i32, s:i32);
             answer(x, sum(host)) :- R(x);",
        );
        assert_eq!(slots, 0);
        assert!(!instrumented.contains("count_capture"));
    }
}
