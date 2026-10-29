# Rule conformance: sequence

Scope: Structures→Sequence lowering, cq-sequence-plan rewrites/placement/cost, cq-lang-sequence contract/grading/resources.

Produced 2026-10-28 by a read-only audit pairing each `\Rule` in `doc/formal/*.tex`
(and its `doc/formal/rules.tsv` row) with the code anchor that implements it.
Status legend: `exact` (code checks the written premises), `weaker` (checks
less), `stronger` (checks more), `unimplemented` (abstract or no checker),
`no-rule` (a code decision with no governing rule; the invariant column states
the rule that is missing). "Exact" is structural conformance under the rule's
stated runtime premises, not a proof. Items marked [INFERENCE] were not
executed. `doc/formal/check.py` verifies that every rule named here exists in
`rules.tsv` and every cited file exists.

## Rule conformance

Read-only inspection; no builds or tests were executed. Scope excludes target translators and the already documented GPU emitter violations.

The relevant inventory entries are `doc/formal/rules.tsv:221-229,237-247,249-259,268-270,272-274`. They are marked `implementation`, except **BE-Code**, which is explicitly `abstract`. “Exact” below means the inspected **static implementation** matches the rule’s structural obligation; it does not establish runtime trait laws or emitter correctness.

### Deviations and decisions without a corresponding rule

| Rule | TeX file:line | Code anchor | Premises as written | What the code checks/does | Status | INVARIANT |
|---|---|---|---|---|---|---|
| BE-Length | `doc/formal/backend_ir.tex:298-300` | `languages/cq-lang-sequence/src/contract.rs:26`; `languages/cq-lang-sequence/src/grading.rs:412-418` | Separated writes consume checked pre-write lengths or bounds. | Grading reads `Analysis.stores`; structural checking propagates Map/Choose lengths and turns completed unknown publications into runtime length inputs. | **exact**, structural | Every separated write has a pre-write size or bound |
| PL-Select | `doc/formal/backend_ir.tex:465-471` | `passes/cq-sequence-plan/src/rewrites/catalog.rs:16,187-219` | First eligible token in explicitly stable-partitioned catalog order (recorded representation first). | Constructs that order explicitly and uses `find`; native keyed ABI preference is now part of the rule. | **exact** | Explicit catalog policy determines representation selection |
| PL-Resident | `doc/formal/backend_ir.tex` (`PL-Resident`) | `passes/cq-sequence-plan/src/catalog.rs` (`provider_available`, `resident_host_reads_require_explicit_complete_catalog_facts`); `experiments/gpu/sequence/core/src/lib.rs` (`catalog`) | A pipe-host handle is read below host. | Requires an explicitly recorded Resident token, plus matching type, elements, read capabilities and read levels; only the native DeviceLayout GPU token declares residency. Positive native and negative ordinary-adapter/missing-capability/wrong-level/type/element tests exercise the boundary. | **exact**, structural | Residency is target-authored evidence, not Rust type-name inference |
| PL-Cost | `doc/formal/backend_ir.tex:473-478` | `passes/cq-sequence-plan/src/cost.rs:166,800-829` | Admissible candidates preserving authored decisions; compare `(W,I,S,B,stores,dispatches)` using checked Group mode. | Group evaluation consumes constructor-preorder `Analysis.partial_groups`; layer facts supply geometry only. | **exact**, structural | Cost describes the checked candidate |
| BE-KeyPartProvider | `doc/formal/backend_ir.tex` (`BE-KeyPartProvider`) | `languages/cq-lang-sequence/src/contract.rs`; `passes/cq-sequence-plan/src/key_partition.rs`; `passes/cq-sequence-plan/src/rewrites/key_partition.rs` | A key partition; lookup key determines routing key, with routing projection `h`; each part builds its provider. | Language checking accepts routing-key containment. The provider rewrite additionally requires the complete outer key in order. Selection and explicit rewriting exclude fused-arm providers whose real prefix-local constructor does not consume the rewritten Store. | **stronger** in the rewrite | Build and lookup use identical routing; a provider type cannot claim routing absent from its actual constructor |
| EXT-SeqAdaptive | `doc/formal/backend_ir.tex:850-853` | `passes/cq-sequence-plan/src/rewrites/adaptive.rs:93-133` | Published filtered store; equal-support alternatives; threshold is advice. | Also requires a positive threshold, earlier publication and alternatives with different work. | **stronger** | Runtime selection cannot change support |
| EXT-SeqChoose | `doc/formal/backend_ir.tex:763` | `passes/cq-sequence-plan/src/rewrites/mod.rs:234-275` | Equal-support alternatives; exhaustive guards. | Public advice parser only accepts a single `len x < k` or worker guard plus `else`, naming earlier definitions. General `Term::Choose` is broader. | **stronger** | Choice arms preserve support |
| EXT-SeqOutput | `doc/formal/backend_ir.tex:797` | `passes/cq-sequence-plan/src/rewrites/mod.rs:281-294` | Protocol belongs to `{count,reserve,reserve_previous,bounded}`; bounded requires declared capacity. | Accepts the explicit protocol enum; arena obligations remain target-owned. | Original **weaker** finding was specification drift; rule corrected, runtime behavior unchanged | Output strategy is explicit |
| BE-Code | `doc/formal/backend_ir.tex:120-122` | `languages/cq-lang-sequence/src/contract.rs:1253-1269` | Injective codec on equality keys; monotone encoding on ordered columns. | Records `Obligation::Codec`; explicitly does not prove these laws statically. Inventory marks this abstract. | **unimplemented** statically, intentionally delegated | Encoded operations preserve row semantics |
| BE-Abort | `doc/formal/backend_ir.tex:310-311` | `languages/cq-lang-sequence/src/grading.rs:216-220,1015-1018` | Failure below host. | Records a failure-channel obligation. This layer does not establish once-set flag behavior or prohibit runtime truncated success. | **weaker** locally, intentionally delegated to translations/runtime | Failure never becomes truncated success |
| PL-OptionalFilter | `doc/formal/backend_ir.tex:887-889` | `languages/cq-lang-sequence/src/lib.rs:83`; `passes/cq-sequence-plan/src/rewrites/adaptive_candidates.rs:84-122` | Checked positive pipelined semijoin provenance; preparation proves published provider and covering joins. | lookup_first records printable optional-filter identities; adaptive discovery reads them without generated-prefix or default-lowering equality gates. | **exact**, structural | Semantic eligibility does not depend on spelling |
| No formal rule: placement candidate topology | —; governing prose at `doc/formal/backend_ir.tex:665-667` | `passes/cq-sequence-plan/src/placement.rs:181-204,345-360` | Prose promises deepest admissible placement. | Considers the first root child, its first child and host, rather than all admissible machine levels. | **no-rule** | Machine data determines legal placement alternatives |
| BE-RustRepresentation | `doc/formal/backend_ir.tex` (`BE-RustRepresentation`) | `passes/cq-sequence-plan/src/placement.rs`; `languages/cq-lang-sequence/src/grading.rs` | Machine host_rust capability; recorded identity read columns and closed captures. | Representation facts replace type-spelling policy. | **unverified** | Target eligibility is not a Rust semantic classification. |

### Remaining relevant rules

| Rule | TeX file:line | Code anchor | Premises as written | What the code checks/does | Status | INVARIANT |
|---|---|---|---|---|---|---|
| BE-Alpha | `doc/formal/backend_ir.tex:101-104` | `languages/cq-lang-sequence/src/contract.rs:471-472,638-774,822-847` | No additional premise in the displayed erasure equations. | Checks named results, Physical segment provenance, Compact operators and lookup headings; derives shape and meaning. | exact, structural | Erasure preserves constructor support |
| BE-Bind | `doc/formal/backend_ir.tex:106-107` | `passes/cq-sequence-lowering/src/lib.rs:47-70`; `languages/cq-lang-sequence/src/contract.rs:1170-1269` | No additional premise in the displayed probe equation. | Records Lookup plus expansion or membership Compact; derives provider capability obligations. | exact, structural | Probe expansion preserves join support |
| BE-Partition | `doc/formal/backend_ir.tex:109-111` | `languages/cq-lang-sequence/src/contract.rs:804-820,1053-1071` | Disjoint, covering parts; commutative monoid. | Requires positive tile/items sizes; derives covering tile shape; uses the closed built-in monoid syntax. | exact, structural | Every input position belongs to one part |
| BE-Offsets | `doc/formal/backend_ir.tex:113-114` | `languages/cq-lang-sequence/src/contract.rs:922-944` | Offsets equal the exclusive scan of inner lengths. | Requires nested positions and records symbolic scan length; actual scan realization is a translation obligation. | exact, structural | Flatten indexes the concatenated nested domain |
| BE-Canon | `doc/formal/backend_ir.tex:116-118` | `languages/cq-lang-sequence/src/contract.rs:1073-1105` | `αt=B`. | Records canon/codec obligations and checked uniqueness at valid global/key-disjoint boundaries. | exact, structural | Canon produces support of complete valuations |
| BE-Choose | `doc/formal/backend_ir.tex:124-126` | `languages/cq-lang-sequence/src/contract.rs:1124-1165,1297-1331` | Equal support; exhaustive length/worker guards. | Checks meaning equivalence, guard availability/exhaustiveness and positive worker bounds; joins shape and size facts. | exact, structural | Every selected alternative has equal support |
| BE-KeyPart | `doc/formal/backend_ir.tex:163-168` | `languages/cq-lang-sequence/src/contract.rs:774-802`; `languages/cq-lang-sequence/src/grading.rs:473-517` | Positive parts; routing columns available; strict ancestor store; count is one or pool width. | Checks all those structural premises, flat input, distinct routing columns and a suitable separated pool. | exact, structural | Key-disjoint parts cover every occurrence |
| BE-KeyPartCanon | `doc/formal/backend_ir.tex:170-173` | `languages/cq-lang-sequence/src/contract.rs:1089-1105` | A key partition; complete valuation determines routing key. | Requires routing columns in the complete heading before certifying global uniqueness. | exact, structural | Equal valuations cannot survive in different parts |
| BE-KeyPartGroup | `doc/formal/backend_ir.tex:175-179` | `languages/cq-lang-sequence/src/contract.rs:989-1000` | A key partition; nonempty group keys determine routing key. | Requires every routing column to belong to group keys. | exact, structural | A keyed group never crosses parts |
| BE-GroupPartial | `doc/formal/backend_ir.tex:181-185` | `languages/cq-lang-sequence/src/contract.rs:1002-1049` | Globally distinct valuations; covering parts; commutative aggregate monoids. | Requires global uniqueness and admissible one-level parts, then records partial mode and flat merged shape. | exact, structural | Partial aggregation merges each valuation once |
| BE-Scope | `doc/formal/backend_ir.tex:288-289` | `languages/cq-lang-sequence/src/grading.rs:315-360` | Read occurs within the store’s lexical extent at an admissible descendant level. | Rejects forward references, upward escapes and unstored cross-grade references. | exact, structural | Reads remain inside stored extents |
| BE-Collective | `doc/formal/backend_ir.tex:291-293` | `languages/cq-lang-sequence/src/grading.rs:759-789` | Level has members, is collaborative and participates wholly; operation associative. | Leaf execution is sequential; otherwise checks collaboration and participation/uniformity, using built-in monoids. | exact for the inspected static checks | Collective participation is uniform |
| BE-Separate | `doc/formal/backend_ir.tex:295-296` | `languages/cq-lang-sequence/src/grading.rs:333-360` | Separated sibling levels. | Rejects sibling-local exchange without ancestor publication. | exact, structural | Siblings exchange only through ancestor storage |
| BE-Publication | `doc/formal/backend_ir.tex:301-302` | `languages/cq-lang-sequence/src/publication.rs:27-189,193-245` | Store with checked provenance and consumers. | Derives borrow/alias/owned/parts ownership and constrained deferred fusion; validates native routed-provider ABI. | exact, structural | Publication ownership has checked provenance |
| BE-Boundary | `doc/formal/backend_ir.tex:304-305` | `languages/cq-lang-sequence/src/grading.rs:906-1040` | Maximal total segment between separated stores. | Forms components separated by store barriers; checks one outer domain; records producer level, parts and failure channel. | exact, structural | Each dispatch has one explicit stored domain |
| BE-RustRepresentation | `doc/formal/backend_ir.tex` (`BE-RustRepresentation`) | `languages/cq-lang-sequence/src/grading.rs` (`BE-RustRepresentation`) | Every read column has identity device representation; other inputs are closed captures. | Unknown/encoded reads require host Rust support; host and CPU task levels support it. | **unverified** | Device expressions remain verbatim. |
| BE-Resource | `doc/formal/backend_ir.tex:313-315` | `languages/cq-lang-sequence/src/grading.rs:222-273`; `languages/cq-lang-sequence/src/resources.rs:70-184` | Checked dispatch; target layout fits level capacity. | Requires layout model for bounded levels, derives demands, invokes layout and rejects excess bytes. | exact, structural | Dispatch resource demand fits its machine |
| PL-Closure | `doc/formal/backend_ir.tex:446-449` | `passes/cq-sequence-plan/src/lib.rs:572-575,623-648`; `passes/cq-sequence-plan/src/rewrites/mod.rs:46-53` | Planner returns program and rewrite trace. | Applies checked rewrite entry points and checks completed machine grading/catalog obligations. | exact, structural | Trace reconstructs a checked final program |
| PL-Precedence | `doc/formal/backend_ir.tex:451-452` | `passes/cq-sequence-plan/src/lib.rs:263-272,551-571`; `passes/cq-sequence-plan/src/decomposition.rs:64-65` | Planner invocation with authored choices. | Rejects machine replacement, skips existing tiles and preserves recorded output protocol. | exact in inspected scheduling paths | Authored scheduling decisions survive planning |
| PL-Det | `doc/formal/backend_ir.tex:454-456` | `passes/cq-sequence-plan/src/lib.rs:273-375,399-414` | Fixed program, machine, catalog and enumeration order. | Enumerates fixed geometry/staging alternatives; replaces a winner only on strict cost improvement. | exact | Identical inputs give identical planning |
| L-Sequence | `doc/formal/backend_ir.tex:727-734` | `passes/cq-sequence-lowering/src/lib.rs:30-128,134-262` | Checked Structures program. | Lowers modes/operators to one-part Sequence, final Count Store, no tiles/grade; independent checker validates construction. | exact, with documented membership-projection specialization | Default lowering preserves Physical retention |
| EXT-SeqStore | `doc/formal/backend_ir.tex:759` | `passes/cq-sequence-plan/src/rewrites/mod.rs:93-117` | Selected definition is Flatten. | `store_level` checks selected flatten definitions and preserves support. General `store`/`publish` APIs extend beyond this displayed rule. | exact for `store_level` | Stored flatten preserves support and offsets |
| EXT-SeqTile | `doc/formal/backend_ir.tex:761` | `passes/cq-sequence-plan/src/rewrites/mod.rs:187-220` | Positive tile; selected definition is not an unbounded unknown-length Store. | Rejects that Store shape, then uses structural checking to reject nonpositive geometry. | exact | Tiling preserves the covered domain |
| EXT-SeqGrade | `doc/formal/backend_ir.tex:767` | `passes/cq-sequence-plan/src/rewrites/mod.rs:325-381` | Named machine and placement assignments. | Checks names/duplicates/machine consistency; changes grading only and defers machine validation. | exact | Placement advice does not change construction |
| EXT-SeqLookup | `doc/formal/backend_ir.tex` (`EXT-SeqLookup`) | `passes/cq-sequence-plan/src/rewrites/lookup_first.rs`; `languages/cq-lang-sequence/src/lookup_semantics.rs` | Equal support, unchanged Rust payloads/bindings and recorded placement. | Support equivalence does not grant downstream planning authority. | **unverified** | Refinements preserve the recorded plan. |
| PL-Evidence | `doc/formal/backend_ir.tex:855-858` | `passes/cq-sequence-plan/src/rewrites/evidence.rs:12-30`; `languages/cq-lang-sequence/src/contract.rs:1365-1380` | Well-formed evidence; asserted keys true; planner invocation. | Records evidence separately; rejects structural rewrites inventing new key premises. Key truth remains author responsibility. | exact, conditional on asserted keys | Estimates cannot become semantic proof premises |
| BE-Group | `doc/formal/backend_ir.tex:1114-1118` | `languages/cq-lang-sequence/src/contract.rs:966-1049` | Complete distinct valuation store; each group enumerated exactly once. | Checks Physical Group provenance, uniqueness and valid flat/key-disjoint/partial shape. | exact, structural | Aggregates consume complete distinct valuations |
| BE-GroupKey | `doc/formal/backend_ir.tex:1130-1132` | `languages/cq-lang-sequence/src/contract.rs:422-469` | Asserted unique physical keys; valuation determines atom keys. | Transfers keys through rename and existing Physical inherited-key analysis. | exact, conditional on asserted keys | Removing canon cannot introduce duplicate valuations |

## Findings
Historical findings 1--4 are retained below with their prior resolutions.
Finding 5 concerns machine topology. Contract 7 replaces Finding 6's scalar
name policy with explicit representation and host-Rust capability facts.
Updated rows marked `unverified` have no new verification-run claim.


### 1. Capacity admissibility is re-derived from syntax, not the checked length facts [D] [severity high]

- **Evidence — governing doc:** `doc/formal/backend_ir.tex:298-299`:
  > “`\mathsf{store}\,t` at a separated level (host stores need no bound)”  
  > “`|t|` is in its type before the write (a scan), or a capacity with an overflow flag”

  Inventory: `doc/formal/rules.tsv:242` identifies this as BE-Length.

  **Code:** Structural checking joins actual arm lengths at `languages/cq-lang-sequence/src/contract.rs:1148-1150`:
  ```rust
  if previous.len != next.len {
      previous.len = Len::Unknown(None);
  }
  ```

  Grading instead uses term inequality at `languages/cq-lang-sequence/src/grading.rs:751-755`:
  ```rust
  arms.iter().any(|(_, arm)| self.unknown_length(arm))
      || arms.windows(2).any(|pair| pair[0].1 != pair[1].1)
  ```

  The same function’s unmatched constructors return `false` (`grading.rs:755`); `Map` is absent from its recursive cases (`grading.rs:733-755`). This result controls rejection of an unbounded separated Store at `grading.rs:413-423`.
- **Violation:** The language already computes symbolic lengths, but grading runs a second, less compositional size analysis. Syntactically different equal-length arms can require a spurious capacity. Conversely, a length-preserving Map can carry an unknown length structurally (`contract.rs:663-774`) while grading stops at Map. **[INFERENCE]** Wrapping an unknown-size choice in such a Map can therefore bypass BE-Length. No executable reproducer was run.
- **Belongs in:** Per-node checked size/allocation facts shared by structural checking, grading and placement; distinguish branch-local fixed-cover allocation explicitly rather than recognizing it syntactically.

### 2. Costing reconstructs Group’s execution strategy despite an explicit checked mode [D] [severity med]

- **Evidence — governing doc:** `doc/formal/backend_ir.tex:220-224`:
  > “The checker records each Group’s proven partial mode in `Analysis.partial_groups`…”  
  > “Grading and target interpretations consume these checked modes, rather than rediscovering routing provenance through nested Lookup, Flatten or Choose terms.”

  **Code:** The checker records the mode at `languages/cq-lang-sequence/src/contract.rs:1046-1049`; grading consumes it at `languages/cq-lang-sequence/src/grading.rs:535-537`.

  Costing independently classifies it at `passes/cq-sequence-plan/src/cost.rs:814-819`:
  ```rust
  let contiguous = !fact.layers.is_empty()
      && fact.layers.iter().all(|layer| !matches!(layer, Layer::KeyPart { .. }));
  let partial = !fact.layers.is_empty() && (global || contiguous);
  ```
- **Violation:** Group has several execution interpretations—ordinary grouping, independent keyed parts, and partial aggregation plus host merge. Cost maintains a separate reconstructed layer stack instead of reading the checker’s recorded interpretation. Its `Choose` handler retains the first arm’s `layers` while merging other facts (`cost.rs:932-958`), making this especially sensitive to mixed-shape alternatives. **[INFERENCE]** Mode or merge-cost disagreements can select a materially worse plan without violating support checks.
- **Belongs in:** The existing `Analysis.partial_groups`, consumed with constructor ownership/preorder; checked per-node execution facts for any additional cost geometry.

### 3. Catalog selection silently prefers the existing provider over catalog order [B/F] [severity med]

- **Evidence — governing doc:** `passes/cq-sequence-plan/src/rewrites/catalog.rs:6-7`:
  > “Replace built provider types with the first catalog token satisfying all of their read obligations. Entry order and catalog order are significant.”

  Formal PL-Select says `k = first { k ∈ K | … }` at `doc/formal/backend_ir.tex:458-464`; inventory anchor is `doc/formal/rules.tsv:257`.

  **Code:** `passes/cq-sequence-plan/src/rewrites/catalog.rs:207-209`:
  ```rust
  .min_by_key(|token| match &entry.structure {
      Structure::Built { keyed, .. } => {
          !crate::catalog::matches_type(&token.ty, &keyed.ty)
  ```
  Analogous preference applies to Selection, KeySet and JoinFilter types at `catalog.rs:211-214`.
- **Violation:** A later eligible token matching the existing representation wins over an earlier eligible token. Thus authored provider spelling supplies an undocumented preference in addition to catalog order. The selected representation is recorded afterward, but the selection policy contradicts both the rule and API documentation.
- **Belongs in:** Explicit catalog-selection advice/policy, or the documented first-admissible-token implementation.

### 4. Adaptive planning recognizes generated names instead of recorded optional-filter provenance [C] [severity med]

- **Evidence — governing doc:** `doc/formal/backend_ir.tex:865-869`:
  > “The RPT+ selective-filter heuristic … uses static selectivity evidence when present; otherwise a `choose` reads an already-published count.”

  **Doc: silent on generated-name eligibility**—checked `doc/formal/backend_ir.tex` and `doc/SEQUENCE-PLAN.md`.

  **Code:** `passes/cq-sequence-plan/src/rewrites/adaptive_candidates.rs:94-96`:
  ```rust
  if !symbol_name(&filter.name).starts_with("__lookup_lookup")
      || ModeKind::from(&filter.mode) != ModeKind::Pipe
      || !has_filter(source, &symbol_name(&keyed.right))
  ```

  Eligibility also depends on fresh default lowering at `adaptive_candidates.rs:170-172`:
  ```rust
  let mut lowered = cq_sequence_lowering::compile(&source.structures)?;
  lowered.sequence.output = source.sequence.output.clone();
  Ok(lowered.sequence == source.sequence)
  ```
- **Violation:** Generated spelling is functioning as a semantic/provenance tag. **[INFERENCE]** Alpha-renaming the same checked semijoin construction, or making an otherwise compatible authored scheduling change, can remove adaptive candidates. The public replay transformation has actual support/publication checks; automatic discovery adds an unrelated naming convention and whole-program syntactic gate.
- **Belongs in:** Recorded optional-filter/covering-join provenance from the semantic planning layer, carried into Sequence and checked by adaptive preparation.

### 5. Shared placement assumes a first-child machine spine [E/F] [severity med]

- **Evidence — governing doc:** `doc/formal/backend_ir.tex:665-667`:
  > “Host rows remain at host; publication and stores consumed by lookup or canon use the deepest admissible level, while intervening terms inherit producer placement.”

  **Code:** `passes/cq-sequence-plan/src/placement.rs:192-195`:
  ```rust
  let producer = if publication.len() > 1 {
      machine.root.children[0]
          .children
          .first()
  ```
  Publication is itself selected from `machine.root.children.first()` at `placement.rs:182-190`; alternatives are restricted at `placement.rs:345-346`:
  ```rust
  let selected = [&preferred, &publication, &producer, &host]
      .into_iter()
  ```
- **Violation:** The machine is represented and checked as a tree, but planning substitutes a fixed host→first-child→first-grandchild template. It does not enumerate other branches or deeper admissible levels. **[INFERENCE]** A catalog-supported sibling/deeper execution level can be ignored, producing host fallback or rejection despite a legal placement.
- **Belongs in:** Machine-driven placement candidate enumeration, with the chosen execution/publication levels recorded as existing Grade/Publish advice.

### 6. Contract 7: representation facts replace type-spelling policy

Sequence records `representation { identity [...]; encoded [...]; }` after
evidence. Machine levels record `host_rust`; CPU tasks support host Rust,
whereas device levels require identity representations for every read column.
Unknown and codec-encoded reads cannot justify device Rust. Closed operands
are evaluated at host and encoded for relational `where` comparisons.
There is no `String`, integer, decimal or optional-type classification.
This obligation supersedes the prior spelling-based policy; verification is
left to integration.

### 7. Provider partition advice rejects routing projections that the language and docs support [F] [severity med]

- **Evidence — governing doc:** `doc/formal/backend_ir.tex:242-245`:
  > “The routed provider retains the routing projection `h` and applies it in the written routing-column order to every lookup key. This includes proper subsets and reordered routing tuples; the planner’s default candidate uses the whole canonical lookup key.”

  Formal BE-KeyPartProvider has `L → K` and `K = h ∘ L`, not `K = L`, at `backend_ir.tex:187-190`; inventory is `doc/formal/rules.tsv:227`.

  **Code:** Public provider rewrite, `passes/cq-sequence-plan/src/rewrites/key_partition.rs:822-825`:
  ```rust
  if keyed.level_names().first() != Some(&keys) {
      return Err(syn::Error::new_spanned(
          &advice.name,
          "BE-KeyPartProvider: routing keys must equal the complete outer provider key",
  ```
  Language checking only requires routing-key containment at `languages/cq-lang-sequence/src/contract.rs:1178-1193`. The authored test `languages/cq-lang-sequence/tests/sequence_language.rs:2056-2078` explicitly supplies `[b]` and `[b,a]` routing for an `[a,b]` provider and calls `contract::check`.
- **Violation:** “The default planner chooses the complete key” has become a restriction in the explicit rewrite itself. The language can express/check these representations, but the public advice path cannot construct them.
- **Belongs in:** Explicit routing projection in provider representation advice, checked against lookup keys; whole-key routing should remain only the automatic planner default.

### 8. The roadmap still specifies obsolete capacity and Group restrictions [F] [severity low]

- **Evidence — capacity doc:** `doc/SEQUENCE-PLAN.md:180-181`:
  > “Checks without a machine conservatively require capacities for all unknown-length stores.”

  **Code:** `languages/cq-lang-sequence/src/contract.rs:1116-1119`:
  ```rust
  let capacity = capacity.or(carried);
  fact.len = Len::Unknown(capacity);
  *fact.shape.last_mut().expect("row level") = fact.len.clone();
  ```
  Structural checking records an absent capacity rather than rejecting it. The authored test at `languages/cq-lang-sequence/tests/sequence_language.rs:978-989` explicitly allows the unknown-length host Store and rejects it only after separated grading. This also conflicts with the roadmap’s own “host (and ungraded) stores need none” at `SEQUENCE-PLAN.md:163`.

  **Evidence — Group doc:** `doc/SEQUENCE-PLAN.md:165`:
  > “Grouping executes at host, never per tile or at a separated pool/task.”

  **Code:** `languages/cq-lang-sequence/src/grading.rs:537-540`:
  ```rust
  let partial = self.partial_groups[&self.name(owner)][slot];
  if level != 0
      && !partial
      && (!self.levels[level].children.is_empty() || !self.key_partitioned(inner))
  ```
  Partial groups bypass that host-only rejection. Authored tests at `languages/cq-lang-sequence/tests/sequence_language.rs:2082-2128` grade partial Groups at `pool`.
- **Violation:** Two important admissibility contracts in the roadmap no longer describe the checker. The formal document already describes partial Group modes and host-growable stores, so readers encounter incompatible specifications of the same IR.
- **Belongs in:** One authoritative Sequence language/grading specification, with the roadmap updated to describe current contracts rather than superseded restrictions.

The dominant problem is not that Tile/Store interpretation is wholly absent: `Analysis.tilings`, publication kinds and dispatch producer levels already record much of it.
The remaining leaks are duplicated semantic analyses, syntax/name-based eligibility, and target/topology policy embedded in shared planning.
The highest-risk correction is to make allocation admissibility consume checked size facts; the largest maintainability gains come from consuming recorded Group modes and explicit capability/provenance data.