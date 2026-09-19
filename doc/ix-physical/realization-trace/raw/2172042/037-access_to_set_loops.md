## 2172042/037-access_to_set_loops

[Raw input](037-access_to_set_loops.input.tokens) · [Raw output](037-access_to_set_loops.output.tokens)

Actual input:

```rust
answer(a, b, c) => ((a, b, c,)) : - l(a) => bind a <- ((__cq_store_l).iter()),
r(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten()), s(b, c)
=> bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
```

Actual output (formatted where Rust syntax permits):

```rust
{
    let mut __cq_output = :: std :: collections :: HashSet :: new(); for a in
    ((__cq_store_l).iter())
    {
        for b in ((__cq_index_i0).get(a).into_iter().flatten())
        {
            let a = :: core :: clone :: Clone :: clone(& a); for c in
            ((__cq_index_i1).get(b).into_iter().flatten())
            {
                let a = :: core :: clone :: Clone :: clone(& a); let b = ::
                core :: clone :: Clone :: clone(& b);
                __cq_output.insert((a, b, c,));
            }
        }
    } __cq_output
}
```
