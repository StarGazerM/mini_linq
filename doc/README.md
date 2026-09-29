# Documentation map

The current compiler architecture is `CQ → NF → VO → RA → IX → Physical → Γ → Rust`.
IX records demands; the Structures program records the bindings Γ answering
each keyed read.
Start with the [crate and module ownership](COMPILER-STRUCTURE.md), then the
[language syntax and contracts](DSL.md).

- [Language syntax and contracts](DSL.md)
- [Semantics of the chain](SEMANTICS.md)
- [Crate and module ownership](COMPILER-STRUCTURE.md)
- [Logical planning](PLANNING.md) and [verification](PLANNING-VERIFICATION.md)
- [Rust expression semantics](RUST-EXPRESSIONS.md)
- [Formal paper and validation](formal/README.md)

The [workload catalog](../workloads/README.md) and its SQL/data fixtures remain
independent of physical coverage. The executable
[composition example](../examples/composition/README.md) binds every macro slot.

Teaching material is separate from implementation conformance:

- [Homework 1](HW1-START-HERE.md): CQ → RA only.
- [Basic checkpoint](R1-START-HERE.md) and [optional HW4](HW4-OPTIONAL.md): later
  exercise specifications, including obligations the compiler does not implement.
- [Project tracks](OPTIONAL-PROJECT-TRACKS.md), [semester plan](semester.md), and
  [Rust tutorial design](RUST-TUTORIAL-DESIGN.md).
- [YA teaching extraction status](../teaching/ya/README.md): logical foundation
  retained; the obsolete backend exporter is retired.
