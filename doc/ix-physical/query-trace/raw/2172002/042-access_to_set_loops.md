## 2172002/042-access_to_set_loops

[Raw input](042-access_to_set_loops.input.tokens) · [Raw output](042-access_to_set_loops.output.tokens)

Actual input:

```rust
answer(a, c) => ((a, c,)) : - r0(a) => bind a <- ((__cq_store_r0).iter()),
r1(a, b) => bind b <- ((__cq_index_i0).get(a).into_iter().flatten()), r3(b, c)
=> bind c <- ((__cq_index_i1).get(b).into_iter().flatten());
```

Actual output (formatted where Rust syntax permits):

```rust
{
    let mut __cq_output = :: std :: collections :: HashSet :: new(); for a in
    ((__cq_store_r0).iter())
    {
        for b in ((__cq_index_i0).get(a).into_iter().flatten())
        {
            let a = :: core :: clone :: Clone :: clone(& a); for c in
            ((__cq_index_i1).get(b).into_iter().flatten())
            {
                let a = :: core :: clone :: Clone :: clone(& a);
                __cq_output.insert((a, c,));
            }
        }
    } __cq_output
}
```
