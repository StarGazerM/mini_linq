# Branch and release policy

The public repository uses progressive homework branches so students receive a
small current task without losing a stable starting point.

## Common homework branches

| Branch | Starting state | Student-owned work |
|---|---|---|
| `hw1` | All three common pass bodies unfinished | CQ → RA (`cq-elaborate`) only |
| `hw2` | HW1 reference complete | RA → IX (`cq-index-policy`) only |
| `hw3` | HW1 and HW2 references complete | IX + policy → Physical (`cq-physical-lowering`) only; supplied structure selection and realization continue to Rust |

The `main` branch advances to the latest announced common starter. Numbered
branches do not move after release. Students create their own submission branch
from the assigned numbered branch rather than committing directly to it.

Later branches are not published early: their history would reveal prerequisite
answers. Each public release is a clean student-facing snapshot and does not
contain the private staff-answer history.

## Pick-one extension branches

After the common core, every team completes one approved advanced extension.
Each extension is released from the completed common reference under a branch
named `pick-one/<name>`, for example:

```text
pick-one/negation-aggregation
pick-one/tokio
pick-one/incremental
pick-one/rayon
pick-one/shared-indexes
```

The list is a planned menu, not a promise that every branch will be offered.
An option becomes assignable only after staff has released and checked its
complete scaffold, tests, and reference solution.

Teams rank three preferences. The instructor balances assignments across the
class, aiming for different extensions when practical. When an extension must
be shared, teams receive different workloads, performance claims, or
adversarial cases. Accessibility, team composition, and a workable staff
checkpoint take priority over uniqueness.

## Release checklist

Before publishing any new starter branch, staff verifies that:

1. prerequisite answers are allowed to be released;
2. no later answer or private repository history is reachable;
3. student-owned sites are explicitly marked;
4. default tests pass and the assigned focused test fails only at its intended
   unfinished boundary;
5. the handout, branch name, and public project page agree; and
6. the branch contains `AI-USE.md` and the required evidence instructions.
