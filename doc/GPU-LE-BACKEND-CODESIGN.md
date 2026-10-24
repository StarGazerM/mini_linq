# Lookup&Expand on the R/E/C back-end model

The active GPU backend translates explicitly graded Sequence through
`cq_gpu_sequence::translate`; decomposition records evaluation and storage
decisions before `machine_grade` checks GPU placements. See
[the active workspace](../experiments/gpu/README.md).

The former Structures GPU printer and its maintained-index Datalog backend
have been removed. Their frozen/leveled indexes and frontier scheduling have
not been reinterpreted as Sequence terms. The removed `p via cursor s;`
implementation and proposed `cq-lang-device` layer are not compatibility paths.

## Sources of truth

- [Back-end rules](formal/backend.tex): ST-Share, STR-Witness, ST-Expand and
  the R/E/C refinement obligations.
- [Lookup-expand status and CPU measurements](STATUS.md#lookup-expand-route-cq_plannersorderle).
- [GPU design and evidence boundaries](GPU-DESIGN.md).

Historical timing anecdotes from the pre-integration proposal are superseded
by those linked records; they are not evidence that sharing alone caused a
speedup or that the hand-written and generated kernels have equal performance.
This note summarizes the implementation and does not change the mathematical
core specification.
