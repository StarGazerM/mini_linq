# Rule conformance: printers

Scope: pull translation (`passes/cq-sequence-pull`), Rayon (`features/sequence-tasks`), CUDA (`experiments/gpu/sequence`), R/E/C realization rules.

Produced 2026-10-28 by a read-only audit pairing each `\Rule` in `doc/formal/*.tex`
(and its `doc/formal/rules.tsv` row) with the code anchor that implements it.
Status legend: `exact`, `weaker`, `stronger`, `unimplemented`, `no-rule`; see
`doc/formal/CONFORMANCE.md`. Already-listed emitter decisions (grid, deferred
range, offset scan, whole fold, host sync) are in `doc/SEQUENCE-PLAN.md` [6].

## Rule conformance

This section extends the printer audit to the requested R/E/C and BE rules. All code anchors below were opened. `rules.tsv:N` means `doc/formal/rules.tsv:N`; that inventory distinguishes implementation rules from abstract obligations. **Exact** means the inspected checker/emitter implements the stated conditional obligation—not that runtime storage laws, user-supplied monoid laws, or semantic refinement have been mechanically proved.

Host capture ownership is implemented by `passes/cq-print/src/ownership.rs`:
each consuming move closure clones the values it reads at creation, including
nested closures. Retired type-dependent IF-Capture handles are not the current
mechanism; GPU-Capture below instead records closed expression evaluation and
codec/domain obligations.

### Non-exact rules and decisions without a rule

| Rule | TeX file:line | Opened code anchor | Premises as written | What the code checks/does | Status | Invariant |
|---|---|---|---|---|---|---|
| GPU-Launch | `doc/formal/backend_ir.tex` GPU-Launch | `languages/cq-lang-sequence/src/gpu.rs` `derive`; `languages/cq-lang-sequence/src/resources.rs` `dispatch_shape`; `experiments/gpu/sequence/core/src/emit.rs` | Checked dispatch and GPU machine policy. | Records execution before BE-Resource; accounting and emission consume the same publication kind. The checker validates partition, share and metadata records by legality predicates, never by rerunning their selection. Host references stop at completed stores rather than following already-published ancestry. | **exact** (was no-rule; duplicate resource derivation removed after H05 integration) | Launch decisions are checked once |
| GPU-LAUNCH-OCCUPANCY | `doc/formal/backend_ir.tex` GPU-LAUNCH-OCCUPANCY | `experiments/gpu/printer-runtime/src/lib.rs` `launch_grid_for_kernel`; `experiments/gpu/datalog-runtime/src/runs.rs`; `experiments/gpu/datalog-runtime/src/pack.rs` | Domain tile count and prepared kernel. | Queries actual device SM count and kernel resident blocks; caps launch by tiles. | **exact** (was no-rule) | No literal device grid |
| GPU-ProgramLaunch | `doc/formal/backend_ir.tex` GPU-ProgramLaunch | `experiments/gpu/sequence/core/src/realize.rs` `select`, `check`; `experiments/gpu/sequence/core/src/emit.rs` `KernelAbi::check`, `CheckedKernel`; `experiments/gpu/printer-runtime/src/lib.rs` `ProgramLaunch` | One preparation per dispatch; one-block preparation has one root part; concrete entry, context and ordinary 1-D envelope agree. | The checker validates coverage and one-part legality independently. Each kernel has one optional preparation for its fixed specialization. Symbols resolve only on a miss; hits neither allocate nor compare names. The checked final ABI records live scalar positions, parameter-to-witness dependencies and the preparation entry; definitions and calls print it directly without kernel reparsing or method rewrites. Context replacement and query-scope release discard slots. Runtime rejects foreign contexts or grids outside the envelope without attribute/occupancy queries. Grid caps and readbacks are unchanged. | **exact** implementation; gates not rerun | No Rust-type inspection, launch-policy selection or hidden synchronization |
| GPU-OffsetScan | `doc/formal/backend_ir.tex` GPU-OffsetScan | `experiments/gpu/sequence-runtime/src/offsets.rs` `scan` | Initialized range lengths plus zero sentinel. | CUB device-wide exclusive scan, parallel wrap checks, recorded total/failure readback before consumption. | **exact** (was no-rule) | Deferred ranges do not force one block |
| GPU-FoldCompletion | `doc/formal/backend_ir.tex` GPU-FoldCompletion | `experiments/gpu/sequence/core/src/emit.rs` `partition_rows` | Recorded whole fold and monoid laws. | Block partials followed by a one-part combine kernel; empty input publishes identity. | **exact** (was no-rule) | Only combined results are published |
| ST-Total (`rules.tsv:174`) | `doc/formal/backend.tex:95-102` | `languages/cq-lang-structures/src/stage.rs:4-35,145-196` | \(S=(o_1,\ldots,o_m)\); every \(o_i\in\mathcal T_{\rm stage}\); admissibleKeys(S); totalInputs(S). The written \(\mathcal T_{\rm stage}\) excludes Probe/Merge. | `total()` includes Probe/Merge; traversal subsequently permits recorded expansion, certified full-key membership probes, and Smallest forcing reads. The latter two are extra cases beyond the written stage-step set. This is an explicit implementation extension, not evidence that those cases are unsafe. | **weaker** than the literal operator-set premise | Every lookahead step is justified as total |
| T-Gpu | `doc/formal/backend_ir.tex` T-Gpu | `experiments/gpu/sequence/core/src/emit.rs`; `experiments/gpu/sequence/core/src/reduce.rs` | Checked host > device > block > lane machine; one query kernel plus recorded completion steps per BE-Boundary dispatch. | Dispatches consume checked launch/strip/publication/completion analysis; terminal groups, sources, uploads, unions and host outputs consume GPU-Realization records (GPU-TerminalGroup, GPU-SourceAccess, GPU-Transfer, GPU-ProviderDomain). | **exact** for recorded execution, input realization and terminal grouping | Emission follows checked placement and boundaries |
| GPU-Runs | `doc/formal/backend_ir.tex` (GPU-Runs) | `experiments/gpu/sequence/core/src/runs.rs`; `experiments/gpu/sequence/core/src/emit.rs`; `experiments/gpu/datalog-runtime/src/layout.rs` | Checked radix_relation / NativeAbi::RadixRelation binding, complete-key schema and packing; DL-DevicePacking and STR-RunRead. | Encodes logical prefix probes once, searches inclusive per-run complete-key intervals, expands block-uniformly and decodes demanded key fields only; membership ORs all runs. | **exact**, conditional on packing and full-key set laws; not a new execution result | Every published run contributes its matching complete keys; no logical global offset is an item address |
| PH-ConstantGuard (`rules.tsv` PH-ConstantGuard) | `doc/formal/backend_ir.tex` PH-ConstantGuard; `doc/formal/passes.tex` | `experiments/gpu/sequence/core/src/constants.rs` (producer); `languages/cq-lang-physical/src/lib.rs` `check_rewrite` (checker) | \(a=\mathrm{rename}(R,(\ldots,c,\ldots))\); \(c\) closed; \(x\) fresh. | Generates fresh columns, literal equality guards and projection; the shared Physical rewrite checker inspects the recorded rename/select/project expansion without re-deriving it. Rejects nonliteral equality values; code-domain operands already arrive as A-Domain capture references. | **stronger**: implements the closed-literal subcase | Exposing atom constants preserves its bag |
| GPU-Arena (`rules.tsv:232`) | `doc/formal/backend_ir.tex:1026-1028` | `experiments/gpu/sequence/core/src/arena.rs:9-18,30-57,73-105`; `experiments/gpu/sequence/core/src/emit.rs:2550-2564` | run_gpu(t) as T-Gpu; output protocol discharges BE-Length. | Implements a restricted raw-candidate terminal whose scalar packing (head columns at recorded carriers) is recorded in publication storage, pool checks and validated lease completion; does not accept every terminal covered by general GPU interpretation. | **stronger**: restricted terminal/scalar subset | Arena publication is an initialized complete candidate bag |
| GPU-CodecIdentity | `doc/formal/backend_ir.tex` GPU-CodecIdentity | `experiments/gpu/sequence/core/src/runs.rs` `Codecs::record` | One recorded `Option<CodecId>` for each recorded `LogicalTypeId` (`None`: identity). | Looks up codecs by logical id and compares codec ids across every native layout before lowering captures; rejects conflicts even for equal codec spellings. Source/provider logical-id mismatches reject. Logical and codec types are retained only for printing. | **exact** | Captures use the compared column’s checked codec identity |
| GPU-Capture | `doc/formal/backend_ir.tex` GPU-Capture | `experiments/gpu/sequence/core/src/capture.rs` `Captures::value`; `experiments/gpu/sequence/core/src/rows.rs` `capture_arguments` | Recorded consuming definitions and nonempty enabled launch domains. | Records capture IDs while lowering expressions (every closed subexpression, literals included, with no `Expr::Lit` exemption) and consumes explicit domain conditions; no generated-name/launch-ABI recovery. | **exact** (was no-rule) | Captures execute only in their required invocation domain |
| GPU-ProviderLayout | `doc/formal/backend_ir.tex` GPU-ProviderLayout | `experiments/gpu/sequence/core/src/bindings.rs` `select`, `check`; `languages/cq-lang-sequence/src/gpu/bindings.rs` | Recorded provider/catalog ABI, key reconstruction, payload, native permutation, download adapter and dense policy. | GPU binding selects facts upstream; independent checking rejects stale records; both construction paths consume them. | **exact** for recorded provider realization | Provider reconstruction and allocation are checked facts |
| GPU-NativeAbi | `doc/formal/backend_ir.tex` GPU-NativeAbi | `languages/cq-lang-structures/src/provider.rs`; `experiments/gpu/sequence/core/src/runs.rs`; `passes/cq-sequence-plan/src/placement.rs` (`rebuilds_native_provider`, `rebuilding_a_declared_native_provider_publishes_device_rows`, `native_abi_omitted_from_a_catalog_token_is_rejected`) | Declared token ABI, contiguous-range and bounds facts; instance logical/storage columns and optional codec. | Native schemas and residency consume recorded facts; no Rust path or generic-argument recognition. Placement's native-rebuild preference reads the declared `abi`, not catalog level shape. | **exact** implementation; gates not rerun | ABI and encoding follow checked provider/input records |
| GPU-RadixAdvice | `doc/formal/backend_ir.tex` GPU-RadixAdvice | `languages/cq-lang-structures/src/provider.rs` `RadixAdvice`, `check_instance`; `experiments/gpu/sequence/core/src/lib.rs` `catalog` | Radix-ABI token; authored per-instance word/offset/ratio/radix_bits/merge_tile. | Parses and prints the advice; rejects missing advice on radix instances, advice on other ABIs and illegal parameters; the token is registered after existing catalog tokens. | **exact** | Radix parameters are authored, never chosen by printer or runtime |
| GPU-RadixProvider | `doc/formal/backend_ir.tex` GPU-RadixProvider | `experiments/gpu/sequence/core/src/bindings.rs` `radix_realization`, `check_radix`; `languages/cq-lang-sequence/src/gpu/bindings.rs` `RadixRealization` | Recorded column types, advice and Sequence/PH-Smallest reads. | Records K/V carriers (dictionary codes as u32), W/O/params and Group/Prefix/Candidates read forms; the checker recomputes each from its source records and rejects stale facts. | **exact** for recorded radix realization | The printer instantiates exactly the recorded RadixLayout ABI |
| GPU-RadixCandidates | `doc/formal/backend_ir.tex` GPU-RadixCandidates | `experiments/gpu/datalog-runtime/src/radix_run.rs` `device::distinct`, `device::distinct_at`; `experiments/gpu/datalog-runtime/tests/radix_candidates.rs` | Keys distinct within a run; encoded inclusive interval. | Counts each interval key once, in its oldest run, and enumerates the same keys; runtime law tests over duplicate-heavy multi-run sides against a candidate-set oracle. | **exact** (runtime law tests) | Exact union cardinality, never a per-run sum |
| GPU-RadixEncodedProbe | `doc/formal/backend_ir.tex` GPU-RadixEncodedProbe | `experiments/gpu/datalog-runtime/src/radix_run.rs` `device::encode_prefix`, `device::interval_items`; `experiments/gpu/sequence/core/src/emit.rs` `radix_probe` | Most-significant-first checked packing; bound leading fields. | Encodes once per bound row with low/high suffix fill and no `high+1`; out-of-domain values match nothing; per-run item intervals equal CPU RadixRun groups in tests. | **exact** | Whole-word compares replace per-field decode |
| GPU-RadixSourceBuild | `doc/formal/backend_ir.tex` GPU-RadixSourceBuild | `experiments/gpu/datalog-runtime/src/radix_run.rs` `RadixLayout::from_sources`; `experiments/gpu/sequence-runtime/src/lib.rs` `radix_layout`; `experiments/gpu/sequence/core/src/rows.rs` | Resident sources share pool and live rows; optional recorded selection. | Packs key fields and fills value lanes (decoding FOR sources: DECODE V, P4) and publishes by the ordinary grouped build with one group-count readback. | **exact**; encoded-key packing remains P4 work | The build reads exactly the recorded rows |
| GPU-Smallest | `doc/formal/backend_ir.tex` GPU-Smallest | `experiments/gpu/sequence/core/src/realize.rs` `smallest`, `check_smallest`; `experiments/gpu/sequence/core/src/emit.rs` `smallest_lookup`, `smallest_payload` | PH-Smallest sides, each a two-level radix layout with a recorded Candidates read. | Records column, ordered sides, exact union counts and first-in-order ties; rejects any other side shape (no First fallback); emits per-row counts, first-minimum enumeration and membership in every other side. Retained witnesses (Retain/Expand) remain unregistered. | **exact** for the recorded non-witness case | Smallest enumerates exactly the intersection candidates |
| GPU-GroupProvider | `doc/formal/backend_ir.tex` GPU-GroupProvider | `experiments/gpu/sequence/core/src/grouping.rs`; `experiments/gpu/sequence/core/src/reduce.rs` | Keyed accumulator provider recorded in Structures. | GPU binding selects `SegmentedGroup`; terminal keyed emission invokes that recorded provider. | **exact** for provider ownership; execution recorded by GPU-TerminalGroup | Group accumulator selection is not a printer decision |
| GPU-Realization | `doc/formal/backend_ir.tex` GPU-Realization | `experiments/gpu/sequence/core/src/realize.rs` `select`, `check` | Graded program, provider bindings and publication storage. | Records sources, per-definition steps, transfers, descriptor bounds, element geometry and captures once; checker validates union arms, transfer coverage, provider origins and single provider ownership, captures and supported geometry without selection; a selection failure is recorded and raised only at printing. | **exact** | Printers consume one realization record |
| GPU-TerminalGroup | `doc/formal/backend_ir.tex` GPU-TerminalGroup | `experiments/gpu/sequence/core/src/realize.rs` `Steps::group`; `experiments/gpu/sequence/core/src/emit.rs` | Final grouped publication and its Physical group. | Records aliases, canonical/stored/source/host input with upload, and segmented or grid-partials completion; the emitter verifies the record against its chain. | **exact** | No unrecorded group rerouting |
| GPU-GroupGridFold | `doc/formal/backend_ir.tex` GPU-GroupGridFold | `experiments/gpu/sequence/core/src/reduce.rs` `global`; `experiments/gpu/sequence-runtime/src/group.rs` `GlobalFold` | Keyless canonical valuations; checked arithmetic. | Fused per-block typed partials with extremum presence over the occupancy grid at recorded block geometry, then one combine block per aggregate on the producing stream at recorded combine geometry. | **exact** | One global row with identity on empty input |
| GPU-SourceAccess | `doc/formal/backend_ir.tex` GPU-SourceAccess | `experiments/gpu/sequence/core/src/realize.rs` `sources`; `experiments/gpu/sequence/core/src/rows.rs` `Rows::new` | Physical sources read by dispatches. | Origin, access (scan/positions/provider interval) and phase are recorded; the checker verifies recorded interval bounds as a set against the source chain's selections. | **exact** | Interval access equals its selections |
| GPU-Transfer | `doc/formal/backend_ir.tex` GPU-Transfer | `experiments/gpu/sequence/core/src/realize.rs` `Steps::evaluation`; `experiments/gpu/sequence/core/src/emit.rs` `publish_host` | Host evaluations over completed publications. | Bound publications and complete-row transfers are recorded; printers download exactly those. | **exact** | Each transfer downloads once per invocation |
| GPU-ProviderDomain | `doc/formal/backend_ir.tex` GPU-ProviderDomain | `experiments/gpu/sequence/core/src/rows.rs` `descriptor_bounds` | Provider builds over completed publications. | Descriptor bounds and measured keys are recorded per column; no printer-owned bounds map. | **exact** | Descriptor bounds enclose the published column |
| BE-TaskProducer | `doc/formal/backend_ir.tex` | `languages/cq-lang-sequence/src/task_execution.rs`; `features/sequence-tasks/src/lib.rs` | Checked term ancestry and stable definition/preorder identity. | Joins, Group drivers and partition lifetime consume recorded producer identities; token scanning is removed. | **exact** (was no-rule) | Join ownership follows checked dataflow |
| BE-CarrierPublication | `doc/formal/backend_ir.tex` | `languages/cq-lang-sequence/src/task_execution.rs`; `passes/cq-sequence-plan/src/cost.rs`; `passes/cq-sequence-pull/src/lib.rs` | Non-borrowable task driver or runtime choice. | Records TileInput/Choice publications and charges independent symbolic retained-row volumes; only selected choice arms execute. | **exact** (was no-rule) | Materialization and retained traffic are explicit |
| BE-Blocks | `doc/formal/backend_ir.tex` BE-Blocks | `languages/cq-lang-sequence/src/blocks.rs`; `passes/cq-print/src/blocks.rs`; `features/sequence-tasks/src/lib.rs` | Checked Structures source/predicate/read IDs, native snapshot positions, exact phase order and single-consumer total predicate runs. | Independently checks coverage, run order, bounded vector/mask geometry, materialization consumers and aligned borrowed task ranges. Both targets emit recorded shared-mask and routing protocols; canonical ordinal keys are gathered once and external ignored witnesses retain value codes. | **exact** for the authored unary S1 slice; pull/Tasks and duplicate-value execution fixtures pass | No full-row adapter, semantic-code inverse for ignored fields, input staging or printer fallback |

BE-Blocks also covers printable `route` field-to-representation references for
`PartitionK`. `check_routing` validates their exact key demand, lineage and
equality preservation independently of `record_routing`. Pull and
`features/sequence-tasks/src/keyed.rs` consume those ordinals directly; borrowed
blocked routing uses native vector ranges and metadata lengths. The Tasks
fixture includes explicit routed execution and missing/wrong-field mutations.

### R/E/C rules and their shared realization

| Rule | TeX file:line | Opened code anchor | Premises as written | What the code checks/does | Status | Invariant |
|---|---|---|---|---|---|---|
| STR-Repr (`rules.tsv:173`) | `doc/formal/backend.tex:31-35` | `languages/cq-lang-structures/src/repr.rs:213-291`; `passes/cq-print/src/positional.rs:274-342` | For every H, lineageConnected(r,H,λH); validOwned(r,Or); unambiguous(Lr,Or); heading(r) ⊆ Lr ∪ Or. | Checks truthful injective lineage against derived occurrences, valid computed owned columns and full heading coverage; printers consume recorded positions/carriers. Storage forward simulation remains a runtime premise. | **exact** for structural obligations | Every logical column has a truthful physical origin |
| STR-Witness (`rules.tsv:178`) | `doc/formal/backend.tex:56-60` | `languages/cq-lang-structures/src/repr.rs:30-112`; `passes/cq-physical-realization/src/operators.rs:603-637,775-808` | share(r)=(b,K,s,p); α_witness(w)=lookup_b(k); allPathsConsumeBeforeEscape(r,p). | Checks witness/share identity and every forwarding path; rejects return, canonicalization, materialization and scope escapes before force. Emission carries and reuses the cursor; Rust checks its written type/Copy use. | **exact**, conditional on lookup storage law | A retained cursor cannot escape before its force |
| ST-Stage | `doc/formal/backend.tex` (`ST-Stage`) | `languages/cq-lang-structures/src/stage.rs`; `passes/cq-physical-realization/src/stage.rs` | Indexable producer, positive batch size and represented segment. | Consumes recorded batch representation without a Rust partiality distinction. | **unverified** | Lookahead preserves the recorded plan. |
| ST-Share (`rules.tsv:177`) | `doc/formal/backend.tex:138-143` | `languages/cq-lang-structures/src/share.rs:62-172`; `passes/cq-physical-realization/src/operators.rs:775-808` | b<s<p; letPayload(b,K) or sameSnapshotLayout(b,K,s,p); s=semi_eager[K](l,b); p=probe[K](l′,b); preservedKeys(s,l′,K). | Checks ordering, qualifying payload/snapshot, eager semi, force probe, matching keys and preserving chain; excludes membership-only providers. Emission performs the first demand and retains its cursor. | **exact**, conditional on immutable provider law | Shared demands denote the same bucket |
| ST-Expand | `doc/formal/backend.tex` (`ST-Expand`) | `languages/cq-lang-structures/src/stage.rs`; `passes/cq-physical-realization/src/operators.rs` | Exact range cursor, represented segment and positive batch size. | Emits the recorded segmented expansion. | **unverified** | Every bucket occurrence is preserved. |
| BK-Dual | `doc/formal/backend.tex` (`BK-Dual`) | Abstract control law | Same stream algebra, representations, plan and answer-determined stopping. | Pure-total Rust allows omitted unneeded work, not changed answers. | **unimplemented**, intentionally abstract | Pull and push preserve the answer set. |

### Sequence and machine-interpretation rules

| Rule | TeX file:line | Opened code anchor | Premises as written | What the code checks/does | Status | Invariant |
|---|---|---|---|---|---|---|
| BE-Alpha (`rules.tsv:221`) | `doc/formal/backend_ir.tex:101-104` | `languages/cq-lang-sequence/src/contract.rs:640-713`; `passes/cq-sequence-pull/src/lib.rs:506-568` | None written; conclusion gives erasure equations for Map/Flatten/Tile/Compact/Store. | Checks result provenance and constructor/operator correspondence; pull emits maps, filtering, flattening and contiguous tiles. | **exact** structurally | Shape erasure preserves the occurrence bag |
| BE-Bind (`rules.tsv:222`) | `doc/formal/backend_ir.tex:106-107` | `languages/cq-lang-sequence/src/contract.rs:1170-1220`; `passes/cq-sequence-pull/src/lib.rs:669-807` | None written; probe equals flattened per-row lookup/join. | Records/checks provider requirements; emission implements buckets and flattening, consuming recorded membership/range/ordered capabilities. | **exact**, under provider laws | Every probe expands its matching bucket |
| BE-Partition (`rules.tsv:223`) | `doc/formal/backend_ir.tex:109-111` | `languages/cq-lang-sequence/src/contract.rs:804-821`; `features/sequence-tasks/src/combine.rs:19-40,77-87,171-190` | Parts disjoint; union of parts=[0,|t|); monoid commutative. | Positive tiles describe contiguous coverage; indexed task dispatch consumes each tile, preserves encounter order and joins buffers. Monoid laws remain semantic premises. | **exact**, conditional on monoid laws | No position is lost or duplicated |
| BE-Offsets (`rules.tsv:228`) | `doc/formal/backend_ir.tex:113-114` | `languages/cq-lang-sequence/src/contract.rs:922-942`; `passes/cq-sequence-pull/src/lib.rs:506-518` | o=scan_(+,0)(|t0|,…,|t|t|−1|). | Checker requires nesting and derives scan lengths; pull uses iterator concatenation. GPU collective offsets realize the same owner relation. | **exact** at the inspected shape/iterator level | Flatten preserves bucket coverage and ownership |
| BE-Canon; BE-CanonSort; BE-CanonOrdered | `doc/formal/backend_ir.tex` | `languages/cq-lang-sequence/src/contract.rs`; `passes/cq-physical-realization/src/lib.rs`; `passes/cq-sequence-pull/src/lib.rs`; `runtime/cq-storage/src/dedup.rs`; `experiments/gpu/sequence-runtime/src/canon.rs`; `features/own-dialect/tests/queries.rs` | Complete-row identity/order laws; a `host` reference additionally requires a direct immutable verified host carrier. | Pull emits `<T>::sort` of the canonical rows provider or the host carrier's ordered runs, Tasks `<T>::par_sort`, GPU retains packed complete-row sort/unique as the canonical rows provider. Push callback timing follows the whole-stream provider-build barrier; single-consumer lets publish by value like pipes, with no printer-local hash exception. | **exact** under order laws for focused host coverage; GPU acceptance and final combined gates remain unverified | Every canonical boundary removes duplicates of complete rows without assuming prefix uniqueness |
| BE-Code (**abstract**) | `doc/formal/backend_ir.tex` BE-Code | `languages/cq-lang-sequence/src/contract.rs`; `passes/cq-sequence-plan/src/catalog.rs`; `experiments/gpu/sequence/core/src/rows.rs` | dec injective on K, both sides; enc monotone on c. | Looks up codecs by recorded `LogicalTypeId`, then records codec ids and requirements; catalog admission matches ids, ordering and levels, not codec types. GPU-CodecIdentity checks cross-layout compatibility. Runtime encoding laws are not compiler-proved. | **weaker** locally: semantic laws remain external premises | Codes preserve logical equality and order under the stated laws |
| BE-Choose (`rules.tsv:238`) | `doc/formal/backend_ir.tex:124-126` | `languages/cq-lang-sequence/src/contract.rs:1124-1166`; `passes/cq-sequence-pull/src/lib.rs:1331-1377` | Alternatives checked equal-support; guards over lengths/target-wide workers; exhaustive. | Checks exhaustiveness and equal meanings, joins size facts; host pull may select Else, runtime path evaluates guards. The rule explicitly permits any support-equivalent arm. | **exact** | Choice cannot change relational support |
| BE-KeyPart (`rules.tsv:224`) | `doc/formal/backend_ir.tex:163-168` | `languages/cq-lang-sequence/src/grading.rs:473-517`; `features/sequence-tasks/src/keyed.rs:51-137`; `features/sequence-tasks/src/combine.rs:213-263` | P>0; K⊆V; ancestor-store input at a strict ancestor of separated pool; P∈{1,width(pool)}. | Checks pool width and ancestor Store; runtime routes each row to one of exactly P owners and retains empty owners; P=1 avoids hashing. | **exact** | Routing is a complete key-disjoint bag cover |
| BE-KeyPartCanon (`rules.tsv:225`) | `doc/formal/backend_ir.tex:170-173` | `languages/cq-lang-sequence/src/contract.rs:1089-1106`; `features/sequence-tasks/src/keyed.rs:139-156` | [Bp] = partition_K,P(t); V→K. | Checks routing columns belong to complete valuations; realizes canon per checked part. | **exact**, under canon laws | Equal complete valuations meet in one part |
| BE-KeyPartGroup (`rules.tsv:226`) | `doc/formal/backend_ir.tex:175-179` | `languages/cq-lang-sequence/src/contract.rs:989-1001`; `features/sequence-tasks/src/keyed.rs:158-175` | [Bp]=partition_K,P(t); G≠∅; G→K. | Checks routing keys are determined by nonempty group keys; groups each checked part using the shared native Group adapter. | **exact** | A logical group never crosses routing parts |
| BE-KeyPartProvider (`rules.tsv:227`) | `doc/formal/backend_ir.tex:187-190` | `languages/cq-lang-sequence/src/contract.rs:1177-1195`; `passes/cq-print/src/lib.rs:686-737` | [Bp]=partition_K,P(t); L→K; K=h∘L; Ip=build_L(Bp). | Checks lookup keys cover routing keys; constructs per-part providers and the lookup-key projection router from checked routing data. | **exact**, under provider laws | Build and lookup use the same routing projection |
| BE-GroupPartial (`rules.tsv:273`) | `doc/formal/backend_ir.tex:181-185` | `languages/cq-lang-sequence/src/contract.rs:1002-1050`; `features/sequence-tasks/src/keyed.rs:178-220,270-320,322-436` | D=ι(supp αt); disjoint union of Dp=D; declared aggregates form commutative monoids. | Requires global uniqueness for partial folds; emits local finite maps/scalars and host merges with declared aggregate identities. | **exact**, conditional on monoid laws | Partial aggregation counts each valuation once |
| BE-Group (`rules.tsv:272`) | `doc/formal/backend_ir.tex` (BE-Group) | `languages/cq-lang-sequence/src/contract.rs`; `experiments/gpu/sequence-runtime/src/group.rs` | α(S)=D; S[ok:ok+1] enumerates Dk exactly once. | Checks complete canonical valuation input; GPU runtime sorts key codes, selects owner boundaries and performs typed per-owner reductions. Execution and terminal completion consume GPU-Realization records described above. | **exact** for grouping-domain realization | Every aggregate consumes the same distinct valuation domain |
| BE-GroupKey (`rules.tsv:274`) | `doc/formal/backend_ir.tex:1130-1132` | `languages/cq-lang-sequence/src/contract.rs:423-467,1002-1028` | Evidence entails unique physical keys KR→R for every positive atom; V determines every KR. | Transfers asserted/inherited keys and requires uniqueness before accepting canon-free/partial Group inputs. Physical-key truth remains user evidence. | **exact**, conditional on key evidence | Canon omission cannot duplicate valuations |
| BE-Scope (`rules.tsv:239`) | `doc/formal/backend_ir.tex:288-289` | `languages/cq-lang-sequence/src/grading.rs:320-357` | Store t at ℓ; read at ℓ′≤ℓ lexically inside extent. | Checks definition ordering, ancestor path compatibility and required materialization for cross-grade access. | **exact** in grading; terminal group aliases are recorded by GPU-TerminalGroup | Reads stay inside published value extents |
| BE-Collective (`rules.tsv:240`) | `doc/formal/backend_ir.tex:291-293` | `languages/cq-lang-sequence/src/grading.rs:759-790`; `experiments/gpu/sequence/core/src/reduce.rs:54-72` | Fold/scan/flatten at ℓ; ℓ has members; collaborative; whole ℓ participates; operator associative. | Rejects noncollaborative and nonuniform/tile-nested collective invocations; block reduction emits full-thread barriers. | **exact**, conditional on supplied algebraic laws | Collective participation is whole-level |
| BE-Separate (`rules.tsv:241`) | `doc/formal/backend_ir.tex:295-296` | `languages/cq-lang-sequence/src/grading.rs:333-357`; `features/sequence-tasks/src/lib.rs:967-1007` | ℓ1 and ℓ2 separated siblings. | Rejects direct sibling exchanges; task joins publish owned buffers before host consumption. | **exact** | Separated siblings exchange only through publication |
| BE-Length (`rules.tsv:242`) | `doc/formal/backend_ir.tex:298-299` | `languages/cq-lang-sequence/src/contract.rs:1108-1122`; `experiments/gpu/sequence/core/src/emit.rs:2451-2465,2539-2557` | Store at a separated level; host stores need no bound. | Tracks Store length/capacity and output protocol; emitted writes are capacity-guarded and insufficient capacity retries before publication. | **exact** for inspected output protocol | No truncated output is published |
| BE-Publication (`rules.tsv:243`) | `doc/formal/backend_ir.tex:301-302` | `languages/cq-lang-sequence/src/publication.rs`; `languages/cq-lang-sequence/src/task_execution.rs` | d=store t; checked provenance and consumers. | Derives borrow/alias/owned/part publication and validates routed-provider ABI/key coverage upstream; carrier storage is separately recorded by BE-CarrierPublication. | **exact** | Store ownership is derived once |
| BE-Boundary (`rules.tsv:244`) | `doc/formal/backend_ir.tex:304-305` | `languages/cq-lang-sequence/src/grading.rs:906-982`; `experiments/gpu/sequence/core/src/emit.rs:1609-1618,1955-1963` | Maximal segment of total terms between Stores at a separated level. | Derives connected components cut by publication barriers and their outer domains; ordinary emitter dispatches consume that analysis. Terminal group aliases are recorded steps (GPU-TerminalGroup). | **exact** | Dispatch boundaries and domains are derived |
| BE-RustRepresentation | `doc/formal/backend_ir.tex` (`BE-RustRepresentation`) | `languages/cq-lang-sequence/src/grading.rs`; `experiments/gpu/sequence/core/src/capture.rs` | Identity representation for read columns and closed captures otherwise. | GPU emits original Rust for cuda-oxide; unknown/encoded row reads execute at host. | **unverified** | No expression recognition or rewriting. |
| BE-Abort | `doc/formal/backend_ir.tex` (`BE-Abort`) | `languages/cq-lang-sequence/src/grading.rs`; `experiments/gpu/sequence/core/src/emit.rs` | Runtime failure below host. | Failure channels must prevent truncated success; Rust expression panics violate totality. | **unverified** | Failure never publishes incomplete answers. |
| BE-Resource | `doc/formal/backend_ir.tex` BE-Resource | `languages/cq-lang-sequence/src/resources.rs:70` `dispatch_shape`; `languages/cq-lang-sequence/src/grading.rs`; `experiments/gpu/sequence/core/src/layout.rs` | Shape consumes recorded publication refinement; target layout must fit producer-level capacity. | Ordinary range publications charge live-parent payload scratch; the emitter carries exactly the recorded parents. Only explicit deferred-range execution suppresses it. Completed publications do not charge producer scratch again. | **exact** for modeled shared storage | Generic machines do not silently acquire GPU range deferral |
| T-Pull (`rules.tsv:260`) | `doc/formal/backend_ir.tex` T-Pull | `passes/cq-sequence-pull/src/lib.rs` | t checked; every grade host. | Host-only grading, checked capabilities and final count-store laziness; recorded ordinary choices select Else, worker-guarded choices use first-match guards, Tile groups contiguous rows. | **exact**; operational documentation aligned | Host interpretation preserves source support |
| T-Tasks (`rules.tsv:248`) | `doc/formal/backend_ir.tex` T-Tasks | `features/sequence-tasks/src/lib.rs`; `languages/cq-lang-sequence/src/task_execution.rs` | t checked against host > pool(separated,dynamic) > task. | Consumes recorded distributing tiles, producer identities and carrier publications; joins owned buffers in order and preserves lifted dimensions. | **exact** | Task scheduling preserves the checked cover |
| GPU-RadixSet | `doc/formal/backend_ir.tex` GPU-RadixSet | `languages/cq-lang-sequence/src/gpu/publication.rs`; `experiments/gpu/datalog-runtime/src/radix_run.rs` | Injective complete-heading FOR codec; explicit declared/runtime width provenance; full-key/unit-item constructor; initialized stream-ordered input. | Checker validates construction coverage, width provenance and parameters; runtime compares every lane, retains resident count and certified adopted owners. Printer emits the recorded constructor. | **exact**, under encoding and readiness premises | Ordinary full-row set construction, not occurrence multiplicity |
| GPU-Arena-Timing (`rules.tsv:233`) | `doc/formal/backend_ir.tex:1037-1039` | `experiments/gpu/sequence/core/src/emit.rs:2548-2557,2762-2768,2809-2827` | Metadata and BE-Abort checked before publication; all uses ordered on producing stream. | Validates metadata; arena timing is opt-in; terminal fence occurs only in enabled timing branch. | **exact**, under stream-order premise | Measurement fences do not affect candidate contents |
| GPU-ReservePrevious (`rules.tsv:234`) | `doc/formal/backend_ir.tex:1050-1052` | `experiments/gpu/sequence/core/src/emit.rs:2453-2459,2539-2560` | Inputs immutable during invocation; count≤capacity checked before publication. | Starts from prior count, retries at full required capacity, checks failure and remembers only completed count. | **exact**, under snapshot immutability | Prior capacity never acts as a truncation bound |
| GPU-Capture | `doc/formal/backend_ir.tex` GPU-Capture | `experiments/gpu/sequence/core/src/capture.rs` `Captures`; `experiments/gpu/sequence/core/src/rows.rs` `capture_arguments` | Closed deterministic total expression, injective checked codec identity, recorded consuming definitions/domains. | Caches encoded host values and guards arguments using explicit nonempty enabled domains. Encoding laws remain premises. | **exact** conditional transformation | Closed capture substitution preserves row predicates |

Codec identity compatibility is now checked by GPU-CodecIdentity; injectivity and determinism remain semantic premises rather than compiler-proved laws.

## Findings

### Resolved: symbol codec identity and capture domains

GPU-CodecIdentity compares the recorded `codec_id: Option<CodecId>` keyed by each recorded `LogicalTypeId`, not by the
logical type spelling, across native layouts and rejects conflicting ids before
translation, even when the codec spellings match. At the Structures declaration
boundary, omitted ids are interned once by opaque token spelling: automatic
equal spellings share an id, while explicit different ids remain distinct.
Later checking and admission use only ids; logical and codec types are printing payloads. Source/provider logical-id mismatches reject.
This contract update does not claim fresh cutover gate evidence.
GPU-Capture records capture parameter IDs used by each lowered physical
expression; launch construction supplies the consuming definitions and their
nonempty enabled domains. Cached values survive retries. The generated-name
visitor and positional launch-argument recovery were deleted.

### Terminal Group syntax triggers an unrecorded GPU execution pipeline [resolved, P3e]
GPU-Realization records the terminal group's store aliases, input realization
(canonical, stored, source or host upload) and completion before printing; the
emitter verifies the record against its chain and reroutes nothing.

### Resolved: GPU fuzz source and scalar ownership

The GPU catalog macro uses PL-Select's checked retention correspondence:
materializing an unsupported keyed host read is not a strict Structures rewrite.
GPU-SourceAccess includes whole-relation lookup operands in its required source
set, and independently checks that each read has a source or preceding completed
publication. Filtered Cartesian operands therefore publish their filtered count.
Only single-column contiguous restrictions fold into intervals; independent
literal-column restrictions remain predicates. Range descriptors record the
signed key base used by directory addressing, with checked domain/sentinel size.
GPU carrier selection takes an inferred answer binder's declared head constraint
before recording its bounds and arena scalar metadata; the checker verifies the
constraint without asking the printer to recover a scalar type.
GPU-Capture records query capture order separately from per-row closed captures.
Answer gates short-circuit to empty row/arena results before later bindings and
inputs; host upload contexts reuse those bindings rather than repeating them.
Closed constraints need no row column: their host Boolean result is recorded.
Grouped outputs with an answer gate are checked unsupported before printing.
For mixed ranges, GPU-Capture records row-column context from the subject or
either bound, so a closed subject does not erase a row-valued bound's context.
ST-Context reports checked failures as one diagnostic expression; grouped-gate
rejection no longer leaks additional Rust syntax errors from concatenated
diagnostics.


### Resolved: provider layout and native ABI identity

GPU-ProviderLayout now records catalog identity/ABI, reconstruction, payload,
directory policy, native complete permutations and download adapters during
GPU binding/grading, not in `Rows::new`. Its independent checker never reruns
selection. Both resident and computed paths consume the same records, and
unsupported constructors are rejected rather than replaced by RangeProvider.
Host subplans use separately recorded host collection/index adapters at exact
Structures entry positions. Coverage and the adapted host Structures contract
are checked before printing; device-native range/run/radix rows become host
vectors and keyed builds become host sorted runs only in this recorded host
view. Device read/build records are unchanged.
`RangeDescriptor.directory_entries` carries the checked allocation to the
runtime; validation checks the sentinel instead of inventing allocation size.
Sparse codes still use the recorded dense policy, with explicit resource demand.

GPU-NativeAbi consumes the declared token's `abi range|radix_relation|columns|radix`,
`contiguous_range` and `bounds resident_columns`. Native radix-relation/column Host scans
carry `input { keys n; columns [Logical => Storage codec [0] Codec, ...] }`;
source ids may be omitted for declaration-local interning. `codec_id: None`
explicitly denotes identity representation. Catalog templates may
omit the instance schema. Structures checks key count, required Host schemas and
ABI/contiguous/bounds consistency; the binder copies input facts to Built
instances. Every Rust spelling is opaque, including `Symbol` and provider
constructors. Placement treats a scanned Host provider as native row work exactly
when its token declares an `abi` other than `columns`. Columns scans retain a host
publication boundary for descriptor-based index builds unless other row work
requires a descendant grade; catalog read levels do not confer nativeness. This is an implementation claim,
not fresh gate evidence.

The GPU `decompose` argument grammar is
`codecs [CodecToken { codec_id: Some(0), codec: Some(Type), ordered: true,
levels: [[host], [host, device]] }, ...]`.
Fields occur in that order. `codec_id` accepts `Some(u32)` or `None`;
identity uses `codec_id: None, codec: None`. `codec` is an opaque type payload,
`ordered` is an explicit Boolean, and each `levels` entry is an explicit level
path. The adapter extends its catalog with these Sequence `CodecToken` records;
it derives no capabilities from schema types. Builtin syntactic `Dense` codec
tokens are removed: custom codecs, including `TestCodec` in the Datalog symbols
fixture, require explicit tokens naming their declaration-local ids.
PL-Catalog matches the recorded id, required ordering and all graded levels.
The 26-output DOOP pipeline explicitly pairs storage codec id 0 with its
ordered `DoopSymbols` catalog token at host/device/block/lane levels. SRD DOOP
uses prepared integer IDs and needs no symbol codec authorization.
Closed symbolic answer binders take their logical identity from the authored
answer slot even though hoisting removes them from row layouts. Carrier and
arena metadata therefore retain the declared codec's u32 representation.
Row carriers resolve each recorded `LogicalTypeId` to its declaration's opaque
Rust payload, or retain the row's authored payload when no identity is recorded.
Neither producer nor checker recognizes `_` or any other Rust type syntax to
choose a carrier. Capture annotations come from the syntax layer's authored
whole-variable binding annotation, not a test of the generated type payload.

### Resolved: task ownership, carrier publications and pull choice

BE-TaskProducer records stable producer identities through term ancestry and
references. Rayon consumes these records for joins, partial Group drivers and
worker-local lifetime; generated token spellings no longer decide ownership.

BE-CarrierPublication records non-borrowable Tile inputs and runtime Choice
carriers. Collection remains an explicit carrier publication, with symbolic
retained-row traffic charged by cost; conditional volumes are conservative
bounds, not execution of unselected arms. Borrowed host drivers are recorded
separately and incur no such copy.

Pull's operational interpretation is now documented consistently: choices
without worker guards select Else; worker-guarded pull and target hooks use
first-match guards. Both preserve BE-Choose support equivalence.