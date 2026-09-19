## 2172007/042-access_to_distinct_iter

[Raw input](042-access_to_distinct_iter.input.tokens) · [Raw output](042-access_to_distinct_iter.output.tokens)

Actual input:

```rust
answer(a, c) => ((a, c,)) : - r0(a) => bind a <-
(observe(0, (__cq_store_r0).iter())), r1(a, b) => bind b <-
(observe(1, (__cq_index_i0).get(a).into_iter().flatten())), r3(b, c) => bind c
<- (observe(2, (__cq_index_i1).get(b).into_iter().flatten()));
```

Actual output (formatted where Rust syntax permits):

```rust
mini_linq::pipeline_to_distinct_iter! {
    iter0 = scan a in (observe(0, (__cq_store_r0).iter())) yield (a,); iter1 =
    join iter0 as (a,) with b in
    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())) yield (a, b,);
    iter2 = join iter1 as (a, b,) with c in
    (observe(2, (__cq_index_i1).get(b).into_iter().flatten())) yield
    (a, b, c,); iter3 = project iter2 as (a, b, c,) yield ((a, c,)); iter4 =
    distinct iter3; return iter4;
}

```
