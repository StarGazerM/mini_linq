use crate::Data;
use std::collections::BTreeSet;

pub fn check(data: &Data) {
    // Include Unicode, punctuation, and empty strings; no integer coercion.
    let label = |x: i32| if x == 0 { String::new() } else { format!("node,{x}:λ") };
    let edge: Vec<_> = data.edge.iter().map(|&(x,y)| (label(x),label(y))).collect();
    let seed: Vec<_> = data.seed.iter().map(|&(x,)| (label(x),)).collect();
    let oracle = ascent::ascent_run! {
        relation edge(String, String) = edge.clone();
        relation seed(String) = seed.clone();
        relation answer(String);
        answer(x.clone()) <-- seed(x);
        answer(y.clone()) <-- answer(x), edge(x,y);
    };
    let mini: BTreeSet<_> = cq_datalog::datalog! {
            struct Owned;
            relation edge(x:String,y:String) = (edge);
            relation seed(x:String) = (seed);
            relation answer(x:String);
            output answer;
            answer((*x).clone()) :- seed(x);
            answer((*y).clone()) :- answer(x), edge(x,y);
    }.collect();
    assert_eq!(mini, oracle.answer.into_iter().collect());
    let naive: BTreeSet<_> = crate::naive("reach", data).unwrap().into_iter()
        .map(|row| (label(row[0]),)).collect();
    assert_eq!(mini, naive);
}
