## 2172007/043-pipeline_to_distinct_iter

[Raw input](043-pipeline_to_distinct_iter.input.tokens) · [Raw output](043-pipeline_to_distinct_iter.output.tokens)

Actual input:

```rust
iter0 = scan a in (observe(0, (__cq_store_r0).iter())) yield (a,); iter1 =
join iter0 as (a,) with b in
(observe(1, (__cq_index_i0).get(a).into_iter().flatten())) yield (a, b,);
iter2 = join iter1 as (a, b,) with c in
(observe(2, (__cq_index_i1).get(b).into_iter().flatten())) yield (a, b, c,);
iter3 = project iter2 as (a, b, c,) yield ((a, c,)); iter4 = distinct iter3;
return iter4;
```

Actual output (formatted where Rust syntax permits):

```rust
:: core :: iter :: Iterator ::
flatten(:: core :: iter ::
once_with(move | |
{
    let iter0 = :: core :: iter :: Iterator ::
    map(:: core :: iter :: IntoIterator ::
    into_iter(observe(0, (__cq_store_r0).iter())), move | a | (a,),); let
    iter1 = :: core :: iter :: Iterator ::
    flat_map(iter0, move | (a,) |
    {
        :: core :: iter :: Iterator ::
        map(:: core :: iter :: IntoIterator ::
        into_iter(observe(1, (__cq_index_i0).get(a).into_iter().flatten())),
        move | b |
        { let a = :: core :: clone :: Clone :: clone(& a); (a, b,) },)
    },); let iter2 = :: core :: iter :: Iterator ::
    flat_map(iter1, move | (a, b,) |
    {
        :: core :: iter :: Iterator ::
        map(:: core :: iter :: IntoIterator ::
        into_iter(observe(2, (__cq_index_i1).get(b).into_iter().flatten())),
        move | c |
        {
            let a = :: core :: clone :: Clone :: clone(& a); let b = :: core
            :: clone :: Clone :: clone(& b); (a, b, c,)
        },)
    },); let iter3 = :: core :: iter :: Iterator ::
    map(iter2, move | #[allow(unused_variables)] (a, b, c,) | (a, c,),); let
    iter4 =
    {
        let mut seen = :: std :: collections :: HashSet :: new(); :: core ::
        iter :: Iterator ::
        filter(iter3, move | row |
        seen.insert(:: core :: clone :: Clone :: clone(row)),)
    }; iter4
}))
```
