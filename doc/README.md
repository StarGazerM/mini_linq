# Documentation map

The current compiler architecture is `CQ → RA → IX → Physical → Rust[Access]`.
Start with the [design and implemented domain](IX-PHYSICAL-REDESIGN.md), then read
the [actual expansion review](ix-physical/realization-trace/review.md).

- [Language syntax and contracts](DSL.md)
- [Stage boundaries](IR-STAGE-REVIEW.md)
- [Crate and module ownership](COMPILER-STRUCTURE.md)
- [Macro names, implementation names and alias slots](NAME-AUDIT.md)
- [Provider callback protocol](PROVIDER-PROTOCOL.md)
- [Representation and access obligations](ACCESS-REFINEMENT.md)
- [Borrowed execution and remaining coverage](STAGED-BORROWED-EXECUTION.md)
- [Logical planning](PLANNING.md) and [verification](PLANNING-VERIFICATION.md)
- [Rust expression semantics](RUST-EXPRESSIONS.md)
- [Formal papers and validation](formal/README.md)

The [workload catalog](../workloads/README.md) and its SQL/data fixtures remain
independent of physical-provider coverage. The executable
[composition example](../examples/composition/README.md) uses current bindings.

Teaching material is separate from implementation conformance:

- [Homework 1](HW1-START-HERE.md): CQ → RA only.
- [Basic checkpoint](R1-START-HERE.md) and [optional HW4](HW4-OPTIONAL.md): later
  exercise specifications, including obligations not implemented by current providers.
- [Project tracks](OPTIONAL-PROJECT-TRACKS.md), [semester plan](semester.md), and
  [Rust tutorial design](RUST-TUTORIAL-DESIGN.md).
- [YA teaching extraction status](../teaching/ya/README.md): logical foundation
  retained; the obsolete backend exporter is retired.

The older generated host API and physical routes were removed. Their historical
snapshots and passed tests do not establish acceptance of the new architecture;
unported behavior is listed explicitly in the design note.
