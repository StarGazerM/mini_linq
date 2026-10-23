use cq_lang_ra as ra;
use cq_lang_vo::cq;
use quote::ToTokens;
use std::collections::BTreeSet;

fn elaborate(source: &str) -> (cq_lang_vo::Program, ra::Program) {
    let source: cq::Program = syn::parse_str(source).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let ordered = cq_order::compile(&normal, &Default::default()).unwrap();
    let target = cq_elaborate::compile(&ordered).unwrap();
    cq_elaborate::check(&ordered, &target).unwrap();
    (ordered, target)
}

#[test]
fn grouped_heads_elaborate_to_one_checked_group_over_complete_named_valuations() {
    let (source, target) = elaborate(
        "struct P; relation R(k:i32,id:i32,v:i64,hidden:i32);
         relation answer(k:i32,n:usize,total:i64,lo:Option<i64>,hi:Option<i64>);
         answer(k,count(),sum(v),min(v),max(v)) :- R(k,id,v,_);",
    );
    let analysis = ra::contract::check(&target).unwrap();
    let groups: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|definition| {
            if let ra::Operator::Group(group) = &definition.operator {
                Some((definition, group))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(groups.len(), 1);
    let (definition, group) = groups[0];
    assert_eq!(
        analysis[&group.input.to_string()].heading,
        BTreeSet::from(["k".into(), "id".into(), "v".into()])
    );
    assert_eq!(group.aggregates.len(), 4);
    assert_eq!(target.plan.output.input, definition.result);
    assert_eq!(target.plan.output.head, source.cq.query.head);
    assert_eq!(
        syn::parse2::<ra::Program>(target.to_token_stream()).unwrap(),
        target
    );
    ra::check_rewrite(&target, &target).unwrap();
}

#[test]
fn count_retains_nonargument_named_variables_and_excludes_only_anonymous_witnesses() {
    let (_, target) = elaborate(
        "struct P; relation R(id:i32,hidden:i32); relation answer(n:usize);
         answer(count()) :- R(id,_);",
    );
    let ra::Operator::Group(group) = &target.plan.definitions.last().unwrap().operator else {
        panic!("group")
    };
    assert!(group.keys.is_empty());
    assert_eq!(
        ra::contract::check(&target).unwrap()[&group.input.to_string()].heading,
        BTreeSet::from(["id".into()])
    );
}

#[test]
fn computed_aggregate_arguments_are_bound_before_the_group() {
    let (_, target) = elaborate(
        "struct P; relation R(id:i32,v:i64); relation answer(total:i64,lo:Option<i64>);
         answer(sum(first(v)),min(second(v))) :- R(id,v), if guard(v);",
    );
    let expressions: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|definition| {
            if let ra::Operator::Bind(bind) = &definition.operator {
                Some(bind.expression.to_token_stream().to_string())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(expressions, ["first (v)", "second (v)"]);
    assert!(matches!(
        target.plan.definitions.last().unwrap().operator,
        ra::Operator::Group(_)
    ));
}

#[test]
fn aggregate_kind_and_valuation_domain_changes_fail_correspondence() {
    let (source, target) = elaborate(
        "struct P; relation R(id:i32,v:i64); relation answer(n:usize,total:i64);
         answer(count(),sum(v)) :- R(id,v);",
    );
    let mut changed = target.clone();
    let ra::Operator::Group(group) = &mut changed.plan.definitions.last_mut().unwrap().operator
    else {
        panic!("group")
    };
    group.aggregates[1].expression = Some(syn::parse_quote!(id));
    assert!(cq_elaborate::check(&source, &changed).is_err());

    let mut changed = target.clone();
    let projection = changed
        .plan
        .definitions
        .iter_mut()
        .rev()
        .find_map(|definition| {
            if let ra::Operator::Project(project) = &mut definition.operator {
                Some(project)
            } else {
                None
            }
        })
        .unwrap();
    projection.attributes = projection
        .attributes
        .iter()
        .filter(|id| *id != "id")
        .cloned()
        .collect();
    ra::contract::check(&changed).unwrap();
    assert!(cq_elaborate::check(&source, &changed).is_err());
    assert!(ra::check_rewrite(&target, &changed).is_err());
}
