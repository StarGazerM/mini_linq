let i0 = r;
let i1 = s;
let j0 = access! {
    out(a, b) => ((a, b)) :-
    L(a) => bind a <- (l.iter()),
    R(a, b) => bind b <- (i0.get(a).into_iter().flatten());
};
let j1 = access! {
    out(c, d) => ((c, d)) :-
    M(c) => bind c <- (m.iter()),
    S(c, d) => bind d <- (i1.get(c).into_iter().flatten());
};
