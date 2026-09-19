## 2172042/001-index_demands

[Raw input](001-index_demands.input.tokens) · [Raw output](001-index_demands.output.tokens)

Actual input:

```rust
struct Heterogeneous; relation L(a: Value); relation R(a: Value, b: Value);
relation S(b: Value, c: Value); answer(a, b, c) :- L(a), R(a, b), S(b, c);
relational
{
    l = rename L {a -> a}; r = rename R {a -> a, b -> b}; s = rename S
    {b -> b, c -> c}; j0 = natural_join l with r; j1 = natural_join j0 with s;
    output j1 as answer(a, b, c);
}
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::choose_physical! {
    struct Heterogeneous; relation L(a : Value); relation
    R(a : Value, b : Value); relation S(b : Value, c : Value); answer(a, b, c)
    : - L(a), R(a, b), S(b, c); relational
    {
        l = rename L { a - > a }; r = rename R { a - > a, b - > b }; s =
        rename S { b - > b, c - > c }; j0 = natural_join l with r; j1 =
        natural_join j0 with s; output j1 as answer(a, b, c);
    } indexes { i0 = @ r [0]; i1 = @ s [0]; } uses
    { j0.right = i0; j1.right = i1; }
}

```
