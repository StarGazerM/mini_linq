# Stack an independent IR

The caller replaces the Physical → Structures continuation with
Physical → Trace → Physical → Structures. Trace extends Physical with an empty
`trace {}` annotation; its complete `Program` owns the Physical program.
Two ordinary passes add and erase the annotation. The bridge uses the same
`cq-stage::Policed` macro ABI as the compiler.

The reference test target points directly to the facade's `physical_query.rs`.
The stacked target executes the same scenarios through the bridge.
`python3 examples/stacked-ir/check_expansions.py` compares all ten expanded
`run` functions byte for byte. The regression runner includes this check.
