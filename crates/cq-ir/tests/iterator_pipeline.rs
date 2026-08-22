use cq_ir::iterator_pipeline;
use quote::{ToTokens, quote};

fn triangle_pipeline_tokens() -> proc_macro2::TokenStream {
    quote! {
        iter0 = scan (x, y,) in (rows.iter()) yield (x, y,);
        iter1 = join iter0 as (x, y,) with (z,) in (
            by_y.get(&(y.clone(),)).into_iter().flatten()
        ) yield (x, y, z,);
        iter2 = filter iter1 as (x, y, z,) if (
            membership.contains(&(z.clone(), x.clone(),))
        );
        iter3 = project iter2 as (x, y, z,) yield ((x.clone(), y.clone(), z.clone(),));
        iter4 = distinct iter3;
        return iter4.
    }
}

#[test]
fn named_iterator_pipeline_round_trips_and_is_well_formed() {
    let first: iterator_pipeline::Pipeline = syn::parse2(triangle_pipeline_tokens()).unwrap();
    iterator_pipeline::contract::check(&first).unwrap();
    assert!(iterator_pipeline::contract::well_formed(&first));

    let second: iterator_pipeline::Pipeline = syn::parse2(first.to_token_stream()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn every_operator_names_its_stream_and_explicit_predecessor() {
    let pipeline: iterator_pipeline::Pipeline = syn::parse2(triangle_pipeline_tokens()).unwrap();
    let [scan, join, filter, project, distinct] = pipeline.definitions.as_slice() else {
        panic!("triangle has five named iterator definitions")
    };

    assert_eq!(scan.stream.to_string(), "iter0");
    let iterator_pipeline::Operator::Scan(scan_operator) = &scan.operator else {
        panic!("first definition is the scan")
    };
    assert_eq!(scan_operator.binding, syn::parse_quote!((x, y,)));

    assert_eq!(join.stream.to_string(), "iter1");
    let iterator_pipeline::Operator::Join(join_operator) = &join.operator else {
        panic!("second definition is the binary join")
    };
    assert_eq!(join_operator.input_stream.to_string(), "iter0");
    assert_eq!(join_operator.input_pattern, syn::parse_quote!((x, y,)));
    assert_eq!(join_operator.binding, syn::parse_quote!((x, y, z,)));

    assert_eq!(filter.stream.to_string(), "iter2");
    let iterator_pipeline::Operator::Filter(filter_operator) = &filter.operator else {
        panic!("third definition is the predicate")
    };
    assert_eq!(filter_operator.input_stream.to_string(), "iter1");
    assert_eq!(filter_operator.input_pattern, syn::parse_quote!((x, y, z,)));

    assert_eq!(project.stream.to_string(), "iter3");
    let iterator_pipeline::Operator::Project(project_operator) = &project.operator else {
        panic!("last definition is the projection")
    };
    assert_eq!(project_operator.input_stream.to_string(), "iter2");

    assert_eq!(distinct.stream.to_string(), "iter4");
    let iterator_pipeline::Operator::Distinct(distinct_operator) = &distinct.operator else {
        panic!("last definition is the set-result distinct operator")
    };
    assert_eq!(distinct_operator.input_stream.to_string(), "iter3");
    assert_eq!(pipeline.return_stream.stream.to_string(), "iter4");
}

#[test]
fn unit_is_an_explicit_named_one_row_source() {
    let pipeline: iterator_pipeline::Pipeline = syn::parse2(quote! {
        iter0 = unit yield ();
        iter1 = filter iter0 as () if (ready());
        iter2 = project iter1 as () yield ((0,));
        iter3 = distinct iter2;
        return iter3.
    })
    .unwrap();

    iterator_pipeline::contract::check(&pipeline).unwrap();
    assert!(matches!(
        pipeline.definitions[0].operator,
        iterator_pipeline::Operator::Unit(_)
    ));
}

#[test]
fn contract_rejects_missing_reordered_or_incomplete_stream_boundaries() {
    let invalid = [
        // Array/slice patterns belonged to the old homogeneous-row backend.
        quote! {
            iter0 = scan [x] in (rows.iter()) yield (x,);
            iter1 = project iter0 as (x,) yield ((x.clone(),));
            iter2 = distinct iter1;
            return iter2.
        },
        quote! {
            iter0 = scan (x, y,) in (rows.iter()) yield (y, x,);
            iter1 = project iter0 as (y, x,) yield ((x.clone(), y.clone(),));
            iter2 = distinct iter1;
            return iter2.
        },
        quote! {
            iter0 = scan (x, y,) in (rows.iter()) yield (x, y,);
            iter1 = join missing as (x, y,) with (z,) in (lookup(y.clone()))
                yield (x, y, z,);
            iter2 = project iter1 as (x, y, z,) yield ((x.clone(), y.clone(), z.clone(),));
            iter3 = distinct iter2;
            return iter3.
        },
        quote! {
            iter0 = scan (x, y,) in (rows.iter()) yield (x, y,);
            iter1 = join iter0 as (y, x,) with (z,) in (lookup(y.clone()))
                yield (x, y, z,);
            iter2 = project iter1 as (x, y, z,) yield ((x.clone(), y.clone(), z.clone(),));
            iter3 = distinct iter2;
            return iter3.
        },
        quote! {
            iter0 = scan (x, y,) in (rows.iter()) yield (x, y,);
            iter1 = join iter0 as (x, y,) with (z,) in (lookup(y.clone())) yield (x, z,);
            iter2 = project iter1 as (x, z,) yield ((x.clone(), z.clone(),));
            iter3 = distinct iter2;
            return iter3.
        },
        quote! {
            iter0 = scan (x, y,) in (rows.iter()) yield (x, y,);
            iter1 = filter iter0 as (y, x,) if (keep(x.clone(), y.clone()));
            iter2 = project iter1 as (x, y,) yield ((x.clone(), y.clone(),));
            iter3 = distinct iter2;
            return iter3.
        },
        quote! {
            iter0 = scan (x,) in (rows.iter()) yield (x,);
            iter1 = project iter0 as (x,) yield ((x.clone(),));
            iter2 = distinct iter1;
            return iter0.
        },
        quote! {
            iter0 = scan (x,) in (rows.iter()) yield (x,);
            iter1 = project iter0 as (x,) yield ((x.clone(),));
            return iter1.
        },
        quote! {
            scan0 = scan (x,) in (rows.iter()) yield (x,);
            iter1 = project scan0 as (x,) yield ((x.clone(),));
            iter2 = distinct iter1;
            return iter2.
        },
    ];

    for tokens in invalid {
        let pipeline: iterator_pipeline::Pipeline = syn::parse2(tokens).unwrap();
        assert!(iterator_pipeline::contract::check(&pipeline).is_err());
        assert!(!iterator_pipeline::contract::well_formed(&pipeline));
    }
}
