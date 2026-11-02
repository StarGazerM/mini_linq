use std::collections::BTreeSet;

#[test]
fn ranges_fan_out_and_keep_borrowed_prefix_columns() {
    let rows = [(1_i32, vec![1_i64]), (3, vec![3_i64])];
    let actual: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation rows(n:i32,label:Vec<i64>);
        relation answer(label:Vec<i64>,value:i32);
        answer(label,value) :- rows(n,label), for value in 0..*n;
    }
    .collect();
    assert_eq!(
        actual,
        BTreeSet::from([
            (&rows[0].1, 0),
            (&rows[1].1, 0),
            (&rows[1].1, 1),
            (&rows[1].1, 2)
        ])
    );
}

#[test]
fn vec_columns_borrow_items_and_empty_iterators_remove_rows() {
    let rows = [(1_i32, vec![vec![1_i64], vec![2_i64]]), (2, vec![])];
    let actual: Vec<(&i32, &Vec<i64>)> = mini_linq::mini_linq! {
        struct Q; relation rows(key:i32,values:Vec<Vec<i64>>);
        relation answer(key:i32,value:&Vec<i64>);
        answer(key,value) :- rows(key,values), for value in values;
    }
    .collect();
    assert_eq!(
        actual.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(&rows[0].0, &rows[0].1[0]), (&rows[0].0, &rows[0].1[1]),])
    );
}

#[test]
fn captured_collections_remain_borrowed_and_duplicates_are_unobservable() {
    let rows = [(1_i32,), (2,)];
    let values = vec![vec![2_i64], vec![1_i64], vec![2_i64]];
    let actual: Vec<(&i32, &Vec<i64>)> = mini_linq::mini_linq! {
        struct Q; relation rows(key:i32);
        relation answer(key:i32,value:&Vec<i64>);
        answer(key,value) :- rows(key), for value in &values;
    }
    .collect();
    assert_eq!(values.len(), 3);
    assert_eq!(
        actual.len(),
        4,
        "generator duplicates must be canonicalized"
    );
    assert_eq!(
        actual.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            (&rows[0].0, &values[0]),
            (&rows[0].0, &values[1]),
            (&rows[1].0, &values[0]),
            (&rows[1].0, &values[1]),
        ])
    );
}

#[test]
fn generated_values_join_host_keys_and_feed_opaque_suffixes() {
    let limits = [(3_i32,)];
    let labels = [(0_i32, vec![0_i64]), (2, vec![2_i64]), (4, vec![4_i64])];
    let actual: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation limits(n:i32); relation labels(key:i32,label:Vec<i64>);
        relation answer(value:i32,label:Vec<i64>,next:i32);
        answer(value,label,next) :- limits(n), for value in 0..*n, labels(value,label),
            if *value % 2 == 0, let next:i32 = *value + 1;
    }
    .collect();
    assert_eq!(
        actual,
        BTreeSet::from([(0, &labels[0].1, 1), (2, &labels[1].1, 3)])
    );
}

#[test]
fn nested_generators_keep_owned_prefixes_and_move_tuple_items() {
    let rows = [(2_i32,)];
    let actual: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation rows(n:i32);
        relation answer(prefix:Vec<i64>,label:Vec<i32>,index:i32,other:i32);
        answer(prefix,label,index,other) :- rows(n), let prefix:Vec<i64> = vec![-1_i64],
            for (label,index) in (0..*n).map(|i| (vec![i], i)),
            for other in 0..(*index + 1);
    }
    .collect();
    assert_eq!(
        actual,
        BTreeSet::from([
            (vec![-1_i64], vec![0_i32], 0, 0),
            (vec![-1_i64], vec![1_i32], 1, 0),
            (vec![-1_i64], vec![1_i32], 1, 1),
        ])
    );
}

#[test]
fn captured_generators_borrow_computed_inputs_only_for_each_call() {
    let rows = [(vec![vec![1_i64], vec![2_i64, 2]],)];
    let captured = vec![vec![9_i64]];
    let actual: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation rows(values:Vec<Vec<i64>>);
        relation answer(limit:usize,value:&Vec<i64>,length:usize);
        answer(limit,value,length) :- rows(values),
            for limit in 0..=values.len(),
            for value in values.iter().take(*limit).chain(captured.iter()),
            for length in Some(value.len());
    }
    .collect();
    assert_eq!(
        actual,
        BTreeSet::from([
            (0, &captured[0], 1),
            (1, &rows[0].0[0], 1),
            (1, &captured[0], 1),
            (2, &rows[0].0[0], 1),
            (2, &rows[0].0[1], 2),
            (2, &captured[0], 1),
        ])
    );
    assert_eq!(captured.len(), 1);
}

#[test]
fn captured_generators_infer_destructured_items_and_unannotated_scalar_inputs() {
    let seed = [(0_i32,)];
    let captured = vec![vec![1_i64], vec![2_i64, 2]];
    let actual: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32);
        relation answer(index:usize,length:usize);
        answer(index,length) :- seed(_),
            for (index,label) in captured.iter().enumerate(),
            let owned = label.clone(),
            for length in Some(owned.len());
    }
    .collect();
    assert_eq!(actual, BTreeSet::from([(0, 1), (1, 2)]));
    assert_eq!(captured.len(), 2);
}

#[test]
fn empty_and_dead_generators_still_obey_iteration_semantics() {
    let seed = [(0_i32,)];
    let empty: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32); relation answer(value:i32);
        answer(value) :- seed(_), for value in 0..0_i32;
    }
    .collect();
    assert!(empty.is_empty());
    let exists: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32); relation answer(x:i32);
        answer(x) :- seed(x), for _value in [1_i32, 1, 2];
    }
    .collect();
    assert_eq!(exists, [(&seed[0].0,)]);
    let absent: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32); relation answer(x:i32);
        answer(x) :- seed(x), for _value in 0..0_i32;
    }
    .collect();
    assert!(absent.is_empty());
}

#[test]
fn total_generator_expressions_and_iteration_preserve_values() {
    let seed = [(0_i32,)];
    fn expression() -> std::ops::Range<i32> {
        0..2
    }
    fn iteration() -> impl Iterator<Item = i32> {
        (0..2).map(|value| value + 1)
    }
    let direct: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32); relation answer(value:i32);
        answer(value) :- seed(_), for value in expression();
    }
    .collect();
    assert_eq!(direct, [(0,), (1,)].into());
    let mapped: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32); relation answer(value:i32);
        answer(value) :- seed(_), for value in iteration();
    }
    .collect();
    assert_eq!(mapped, [(1,), (2,)].into());
}

#[test]
fn generator_iteration_order_does_not_change_answers() {
    fn run(values: &[i32]) -> BTreeSet<i32> {
        let seed = [(0_i32,)];
        let answers: Vec<_> = mini_linq::mini_linq! {
            struct Q; relation seed(x:i32); relation answer(value:i32);
            answer(value) :- seed(_), for value in values.iter().copied();
        }
        .map(|(value,)| value)
        .collect();
        assert_eq!(answers.len(), 3);
        answers.into_iter().collect()
    }
    assert_eq!(run(&[3, 1, 3, 2]), run(&[2, 3, 1, 2]));
}

#[test]
fn generator_capture_borrowing_preserves_local_shadowing_and_generic_callees() {
    let seed = [(0_i32,)];
    let captured = vec![String::from("ab"), String::from("abc")];
    let actual: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation seed(x:i32); relation answer(value:usize);
        answer(value) :- seed(_), for value in captured.iter().map(|captured| captured.len())
            .chain({ let captured = [4_usize]; captured.into_iter() })
            .chain(Some(5_usize))
            .chain(Some("abcdef").into_iter().map(str::len));
    }
    .map(|(value,)| value)
    .collect();
    assert_eq!(actual, BTreeSet::from([2, 3, 4, 5, 6]));
    assert_eq!(captured.len(), 2);
}

#[test]
fn generator_captures_survive_fused_union_and_fallback_closures() {
    let rows = [(0_i32,), (1,)];
    let captured = vec![vec![1_i64]];
    let alternate = vec![vec![2_i64]];
    let union: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32);
        relation answer(x:i32,y:&Vec<i64>);
        answer(x,y) :- rows(x),
            { for y in captured.iter().take(*x as usize) } |
            { for y in alternate.iter() };
    }
    .collect();
    let fallback: BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32);
        relation answer(x:i32,y:&Vec<i64>);
        answer(x,y) :- rows(x),
            { for y in captured.iter().take(*x as usize) } else
            { for y in alternate.iter() };
    }
    .collect();
    assert_eq!(
        union,
        BTreeSet::from([
            (&rows[0].0, &alternate[0]),
            (&rows[1].0, &alternate[0]),
            (&rows[1].0, &captured[0]),
        ])
    );
    assert_eq!(
        fallback,
        BTreeSet::from([(&rows[0].0, &alternate[0]), (&rows[1].0, &captured[0]),])
    );
    assert_eq!(captured.len(), 1);
    assert_eq!(alternate.len(), 1);
}
