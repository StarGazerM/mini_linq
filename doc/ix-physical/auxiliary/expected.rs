let i0: std::collections::HashMap<i32, Vec<&[i32; 2]>> = {
    let mut index = std::collections::HashMap::new();
    for row in r {
        index.entry(row[0]).or_insert_with(Vec::new).push(row);
    }
    index
};
let i1: std::collections::BTreeMap<i32, Vec<&[i32; 2]>> = {
    let mut index = std::collections::BTreeMap::new();
    for row in r {
        index.entry(row[1]).or_insert_with(Vec::new).push(row);
    }
    index
};
let j0 = access! {
    out(a, row) => (row) :-
    L(a) => bind a <- (l.iter()),
    R(row) => bind row <- (i0.get(a).into_iter().flatten().copied());
};
let j1 = access! {
    out(b, row) => (row) :-
    M(b) => bind b <- (m.iter()),
    R(row) => bind row <- (i1.get(b).into_iter().flatten().copied());
};
