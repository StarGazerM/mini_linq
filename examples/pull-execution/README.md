# Pull execution

The example explicitly binds `cq_example_providers::hash_distinct` as
`mini_linq::distinct_state`. Replace it with `ordered_distinct` to use ordered
state without changing the Access query or iterator lowering.
