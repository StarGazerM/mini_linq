# Design migration status

The audit at `a054555` identified coupled whole-query backend selection, repeated
index analysis, misleading boundaries, and undocumented execution/ownership
choices. That implementation has been retired. The historical detail is in Git;
it is not a specification of the current compiler.

Current syntax, rules and acceptance evidence are maintained in
[IX-PHYSICAL-REDESIGN.md](IX-PHYSICAL-REDESIGN.md) and the [formal contract](formal/ix_physical_contract.tex).

Completed corrections:

- IX owns index-column analysis and explicit per-use references.
- Physical freezes relation/index providers and each RA lowering separately.
- Generic realization dispatches opaque external paths without a backend enum.
- Typed callbacks live within realization; the common Native dialect and old
  bypass routes have been removed.
- Borrowed mixed-storage execution, local rebinding and an external output
  provider are checked against independent results and actual macro traces.
- Logical proof/oracle tests remain in the logical compiler; storage and Access
  tests establish their own component laws.

Remaining implementation gaps require new provider/rule specifications and
acceptance examples: stored intermediate versions, general index construction,
additional RA operators, block/YA access, generated host APIs, and execution
scheduling. The [domain table](IX-PHYSICAL-REDESIGN.md#implementation-domain)
is authoritative. Missing coverage is not replaced by an old fallback route.
