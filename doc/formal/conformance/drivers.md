# Rule conformance: drivers

Scope: cq-stage pipeline protocol, facade pass registry, GPU TPC-H build drivers, xtask explain, planners.

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

Read-only audit; no builds or tests run. The table covers pipeline/protocol rules, planner compositions, and planning contracts used by the drivers. Operator-level rewrite laws delegated through the registry are outside this infrastructure audit.

Implementation inventory: planner compositions and pass protocol are recorded in `doc/formal/rules.tsv`. Plan-Prefix and R-Advice now anchor the shared `facade/cq-planning` core API, re-exported by `features/planners/plan`; the former Written/Coarse inventory drift has been corrected.

| Rule | TeX anchor | Code anchor opened | Premises as written | What implementation checks/does; **INVARIANT** | Status |
|---|---|---|---|---|---|
| PL-Select | `doc/formal/backend_ir.tex:458-464` | `passes/cq-sequence-plan/src/rewrites/catalog.rs:186-224` | `s'=retainUnsupportedHostReads(s,K)`; entry `e`; `k=first` catalog token satisfying capabilities, nonempty levels, accepted elements | Filters eligible tokens, but `min_by_key` prefers a token matching the existing representation over an earlier eligible token. **INVARIANT:** eligible provider chosen according to declared catalog priority | **weaker** — eligibility is enforced, first-token priority is not |
| ST-Entry | `doc/formal/composition.tex` (`ST-Entry`) | `facade/mini-linq-macros/src/lib.rs` (`expand_compile`, `defaults`); `facade/cq-stage/src/lib.rs` (`Configuration::resolve`) | `π=resolve(configuration,defaults)`; `π≠[]` | Default `plan_sequential` belongs to configuration decomposition; resolution suppresses it when planning or representation advice or explicit Sequence phases are authored. Entry starts the resolved queue without path matching or insertion. **INVARIANT:** executed queue equals the resolved configured queue | **exact** |
| DR-Target | `doc/formal/composition.tex` (`DR-Target`) | `experiments/gpu/tpch/build_stages.rs` (`compile`); `experiments/gpu/tpch/build.rs` (`compile_stage`) | Target is an explicit pipeline input; return its compilation result | Compiles once and propagates rejection without retry or clearing advice. **INVARIANT:** failures cannot select another target | **exact** |
| DR-Publication | `doc/formal/composition.tex` (`DR-Publication`) | `experiments/gpu/tpch/build.rs` (`Publication`, `publication`) | Explicit placement and recorded semantic nullable columns; nullable/host-target device publication rejected | Aggregate kinds and incoming publication facts determine validity; generated carriers consume the recorded placement. **INVARIANT:** no type-spelling residency decision | **exact** |
| ST-Endpoint | `doc/formal/composition.tex:23-25` | `facade/cq-stage/src/lib.rs` (`ST-Endpoint`) | Explicit terminal input language; Structures excludes authored Sequence phases | Typed `PrintInput` controls default phase inclusion; actual `pass<L,M>` contracts check payload compatibility. **INVARIANT:** renaming a macro cannot change its language endpoint | **exact** |
| DR-Recipe | `doc/formal/composition.tex` (`DR-Recipe`) | `experiments/gpu/tpch/src/advice.rs` (`Recipe`, `Schedule`, `catalog_recipe`); `experiments/gpu/tpch/build.rs` (`compile_stage`) | Catalog resolution produces explicit recipe input; replay uses checked advice | Compiler accepts no workload/profile label; selectors and geometry are serializable facts; completed replay clears expanded recipe. **INVARIANT:** changing a report/query label cannot change compilation | **exact** |
| DR-Registry | `doc/formal/composition.tex` (`DR-Registry`) | `features/rewrites/passes/src/registry.rs`; `features/rewrites/src/lib.rs`; `experiments/gpu/tpch/src/advice.rs` | Shared Invocation parser, registered endpoint identity, opaque arguments | One registry generates macro adapters and programmatic dispatch; unknown namespaces are rejected. **INVARIANT:** no GPU-only grammar or rewrite allowlist | **exact** |
| ST-Next | `doc/formal/composition.tex:10-13` | `facade/cq-stage/src/lib.rs:395-466` | `parse_L(T)=s`; `WF_L(s)`; `f(s,args)=t`; `WF_M(t)`; `M≠Rust`; `π=k(args')::π'` | Parses input; checks source/target; runs correspondence callback; quotes target into next invocation. **INVARIANT:** checked complete payload crosses each continuation | **exact** |
| ST-Context | `doc/formal/composition.tex:15-16` | `facade/cq-stage/src/lib.rs:270-298,340-345` | `pass_f(args,π,quote(s))→T'`, lifted to enclosing context `C[...]` | Produces the next macro invocation in place, without a separate Rust execution context. **INVARIANT:** expansion remains in enclosing lexical Rust context | **exact** |
| ST-Sequence | `doc/formal/composition.tex:18-21` | `facade/mini-linq-macros/src/lib.rs:400-417,512-534` | `q=sequence(s)`; `check(s,q)`; `checkGrading(q,host)`; `e=translatePull(q)`; `π=[]` | Lowering correspondence check, unchanged Sequential grading, checked terminal translation; cq-stage enforces terminal queue shape. **INVARIANT:** checked host Sequence interpretation terminates the queue | **exact** |
| ST-Observe | `doc/formal/composition.tex` (`ST-Observe`) | `facade/cq-stage/src/lib.rs` (`Boundary`, `resolve_observed`, `run`); `facade/mini-linq-macros/src/observation.rs` | Checked execution of the selected resolved queue prefix | Resolver records cutoffs while constructing the ordinary queue; observer receives the checked payload, including terminal Rust after translation. Default VO uses the shared planner prefix; authored planners are not classified by spelling. **INVARIANT:** diagnostics preserve preceding passes, arguments and checks | **exact** |
| DR-Explain | `doc/formal/composition.tex` (`DR-Explain`) | `features/workloads/xtask/src/explain.rs` (`explain_case`, `observer_source`) | Every selected boundary through the requested stopping point is observed with `core::stringify` | Generates public facade consumer invocations for all twelve stages; removes driver-owned planning, formatting and lowering. Earlier artifacts survive a later failure. **INVARIANT:** explain observes compilation rather than reconstructing it | **exact** |
| CTX-Entry | `doc/formal/context.tex:80-82` | `facade/cq-stage/src/lib.rs:402-445` | `Γ_d(s)≠∅` and envelope absent or its facts differ | Requires exact envelope equality for factful entry; also checks authorization and fact preservation/introduction. **INVARIANT:** restricted facts retain explicit provenance | **exact**, including surrounding prose’s empty-envelope/addition constraints |
| Plan-Prefix | `doc/formal/passes.tex` (`Plan-Prefix`) | `facade/cq-planning/src/lib.rs` (`PlanOptions`, `prefix_valuation`, `compile_prefix`); `facade/mini-linq-macros/src/lib.rs` (`plan`) | Shared options grammar; normalized CQ; explicit order or Prefix(Connected)/Coarse; None under `no_reduce`, otherwise Masks; checked advice and elaboration | One reusable parser/composition; facade delegates rather than reimplementing it. VO is exposed before checked RA elaboration. **INVARIANT:** the same options denote the same checked composition for every caller | **exact** |
| R-Advice | `doc/formal/passes.tex` (`R-Advice`) | `facade/cq-planning/src/lib.rs` (`advise_prefix`) | Skip names an existing reduction; reduce/skip operands are distinct top-level atoms sharing variables | Resolves positive clauses, checks distinct/shared operands, rejects nonexistent skip, inserts added reductions before first stream step. **INVARIANT:** advice changes only admissible recorded reductions | **exact** |
| Plan-Generic | `doc/formal/passes.tex:719-720` | `features/planners/plan/src/lib.rs:86-104,206-228,381-391` | `n=N(q)`; `v=O_Written,Fine(n)`; `a=E(v)` | WrittenFine selects Written/Fine; normalization/order/reduction/elaboration calls are checked. **INVARIANT:** fine schedule is explicit VO | **exact** |
| Plan-Graph | `doc/formal/passes.tex:722-725` | `features/planners/plan/src/lib.rs:107-132,206-228,381-391` | Written/Coarse; Hypergraph for blocks, otherwise Tree if acyclic, otherwise None; `a=S(E(w))` | Implements those reduction cases; Hypergraph then uses checked `simplify_reductions`. **INVARIANT:** graph reductions and subsequent absorption preserve checked source | **exact** |
| Plan-Transfer | `doc/formal/passes.tex:727-731` | `features/planners/plan/src/lib.rs:206-228,233-286,297-375,381-391` | Connected atom runs over Written/Coarse; Masks; filtered forward root edges and backward parent-first edges; checked simplification | Connects atom runs, builds described reductions, declines extra transfer in opaque scopes as qualified by `passes.tex:769-773`. **INVARIANT:** transfer preserves predicates/Rust boundaries and records semijoins | **exact**, reading the rule with its stated prose qualifications |
| CK-Transfer | `doc/formal/passes.tex:733-734` | `features/planners/plan/src/lib.rs:396-434` | `erase(t)=v`; `t=transfer(R_None,Masks(v))` | Erases reductions recursively, checks ordering, reconstructs expected transfer schedule, requires equality. **INVARIANT:** authored transfer reductions equal the declared schedule | **exact** |
| Plan-Le | `doc/formal/passes.tex` (`Plan-Le`) | `features/planners/plan/src/le.rs` | Explicit outer stream, ear-ordered bound-key lookups, expansion by column demand or scope exit. | Rust contributes free-variable dependencies, not temporal predecessor boundaries. **INVARIANT:** recorded lookup/expand phases preserve binding ownership | **unverified** |
| CK-Le | `doc/formal/passes.tex` (`CK-Le`) | `passes/cq-order/src/le.rs`; `features/planners/plan/src/lib.rs` | CQ erasure identity; one lookup before full expansion per selected occurrence; automatic: written ownership/outer; advised: outer = first read; WF-VO | One shared checker: automatic LE selects all inner occurrences recursively, hand advice selects its explicit lookup occurrences; caller separately checks reductions. **INVARIANT:** lookup/expand schedule preserves occurrences; automatic plans also written ownership and outer stream | **exact** |
| O-StepAdvice | `doc/formal/passes.tex` (`O-StepAdvice`) | `passes/cq-order/src/lib.rs` (`compile_steps`); `facade/cq-planning/src/lib.rs` | Exact authored subatom sequence plus ready dependencies and unnamed occurrences; WF-VO and CK-Le for lookups | Full reads, bound-key lookups and one-variable partial reads use existing VO; rejects foreign/duplicate columns, incomplete rows, after-expand reads and invalid ownership/outer. **INVARIANT:** advice chooses a checked schedule without new IR or changed default planning | **exact** |
| PL-Closure | `doc/formal/backend_ir.tex:446-449` | `passes/cq-sequence-plan/src/lib.rs:573-576,610-651` | `plan(t,M,K)=(t',[r₁,…,rₙ])`; conclusion requires replay equality, each checked rewrite, completed grading | Applies checked advice, records each step, and checks final machine grading/catalog obligations. **INVARIANT:** successful planner result has checked replay and grading | **exact** for shared planner; driver decisions outside that trace are separate findings |
| PL-Precedence | `doc/formal/backend_ir.tex:451-452` | `passes/cq-sequence-plan/src/lib.rs`; `experiments/gpu/tpch/build_stages.rs` (`compile`) | `plan(t,M,K)=(t',ρ)` preserves authored-choice restriction | Shared planner retains authored choices; driver now propagates rejection instead of clearing advice and changing target. **INVARIANT:** authored choices survive completion | **exact** |
| PL-Det | `doc/formal/backend_ir.tex:454-456` | `passes/cq-sequence-plan/src/lib.rs:273-291,353-397` | Fixed `(t,M,K)`, fixed candidate/catalog order | Fixed geometry enumeration, ordered alternatives, strict-improvement replacement. **INVARIANT:** repeatable selection/tie resolution | **exact** for inspected enumeration |
| PL-Cost | `doc/formal/backend_ir.tex:466-471` | `passes/cq-sequence-plan/src/cost.rs:160-184`; `passes/cq-sequence-plan/src/lib.rs:380-387` | Ordered admissible candidates preserving authored choices; lexicographic `(W,I,S,B,stores,dispatches)` | Compares those fields; replaces best only on strict improvement; ties/unresolved comparisons retain prior choice. **INVARIANT:** deterministic cost ordering of admitted alternatives | **exact** for inspected comparison |
| PL-Evidence | `doc/formal/backend_ir.tex:855-858` | `passes/cq-sequence-plan/src/rewrites/evidence.rs:17-31`; `experiments/gpu/tpch/build.rs:318-374` | Well-formed evidence; `E_key` holds; planner result under evidence | Checked evidence insertion; driver constructs manifest count facts and declared-key assertions. Key truth remains the explicitly stated premise, not a runtime proof. **INVARIANT:** estimates/assertions remain explicit metadata | **exact** as a conditional contract |

## Findings

### 1. Any device-compilation error can silently select a different target and discard advice [B, E, F] [severity high]

**Resolved by DR-Target:** `Pipeline.target` is explicit (`device` by default,
`host` when authored). Each stage compiles once; all failures propagate unchanged.
The evidence below records the historical defect, not the current implementation.

- **Evidence:**
  - **Doc:** `experiments/gpu/README.md:15-17`: “Translator rejection is not a license to substitute another query or plan.”
  - **Doc:** `doc/formal/backend_ir.tex:478-480`: “PL-Precedence preserves authored grades, tiles, stores, choices, output protocols and codec/canon tokens: planning fills missing decisions rather than optimizing them away.”
  - **Code:** `experiments/gpu/tpch/build_stages.rs:80-82`:
    ```rust
    Err(device_rejection) => {
        if override_pipeline.is_some() {
            return Err(device_rejection);
    ```
  - **Code:** `experiments/gpu/tpch/build_stages.rs:87-90`:
    ```rust
    let mut host_pipeline = pipeline.clone();
    host_pipeline.structures.clear();
    host_pipeline.sequence.clear();
    ```
    The retry sets `complete = false` and calls `compile_stage(..., true)` at `:90-98`.
- **Violation:** The catch covers **all** `compile_stage` failures, not only a typed “unsupported device placement” result: parsing, advice, lowering, checking, planning, and translation all return the same `Rejected` type. Without an override file, the driver clears representation/schedule advice and retries on Sequential. Recording the rejection in JSON afterward does not make the replacement an input decision checked by the pipeline. **[INFERENCE]** This can conceal GPU compiler regressions as successful host-stage execution and create severe performance cliffs.
- **Belongs in:** A checked multi-stage target/publication plan, with an explicit admissible host alternative and typed rejection classification.

### 2. Cross-stage residency is decided by an `Option` spelling test after checked planning [A, B, C] [severity high]

**Resolved by DR-Publication:** `Pipeline.publication` explicitly selects the
carrier (`host` by default). The pre-generation publication record carries
nullable columns derived from aggregate kinds and incoming publication facts.
Unsupported nullable device publication is rejected, never inferred from `Option`
spelling. Reports and downstream stage bindings consume that same record.
The evidence below is historical.

- **Evidence:**
  - **Doc:** `doc/formal/backend_ir.tex:370-373`: “Nullable publications require a representation that retains validity, or explicit host publication.”
  - **Doc:** `doc/DESIGN.md:172-173`: “Pass arguments record choices in target IR; subsequent passes receive complete programs.”
  - **Code:** `experiments/gpu/tpch/build.rs:663-666`:
    ```rust
    let nullable = answer_types
        .iter()
        .any(|ty| ty.to_token_stream().to_string().starts_with("Option"));
    ```
    Immediately afterward, `resident = !host && !nullable` selects `DeviceColumns` versus `Vec` at `:666-671`.
  - **Code:** `experiments/gpu/tpch/build.rs:685-689`:
    ```rust
    let publish = if nullable {
        quote!({
            let started = ::std::time::Instant::now();
    ```
    The generated arm downloads using `output.to_host()?` at `:689`.
- **Violation:** A build-script type-name heuristic decides an actual transfer and the next stage’s carrier after Sequence planning/grading. The driver later records `"publication": "device" | "host"` in JSON (`:773-774`), but that is not a checked input IR decision. **[INFERENCE]** Equivalent spellings such as qualified `Option` paths or aliases can take different routes; a name merely beginning with `Option` also satisfies the prefix test.
- **Belongs in:** A typed inter-stage publication record carrying residency, validity representation, schema, bounds, and transfer protocol.

### 3. Default planning: shared composition cutover

`cq_planning::PlanOptions` and `compile_prefix` now own the production
composition, re-exported by `mini_linq::planning` and `cq_planners_plan`,
with `prefix_valuation` exposing its checked VO boundary. The
facade parses and executes this API; GPU `advice::valuation` now parses the same
options and calls `prefix_valuation` directly instead of selecting a separate
`Planner` default. Duplicate `order`/`no_reduce` and trailing options commas
are rejected by that single parser. `no_reduce` retains connected-prefix
ordering. The rule and inventory now identify this shared implementation.

Historical audit evidence motivating the route migrations:
  - **Doc:** `doc/formal/passes.tex:114-117`: “The production facade uses Prefix(Connected)/Coarse: the written root, then at each step a ready atom sharing a bound equality class before a disconnected one…”
  - **Code, facade:** `facade/mini-linq-macros/src/lib.rs:276-279`:
    ```rust
    let ordering = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    ```
  - **Code, GPU default advice:** `experiments/gpu/tpch/src/advice.rs:314-317`:
    ```rust
    let ordered = if plan.order.is_empty() {
        let options = cq_order::Options::default();
        let ordered = cq_order::compile(source, &options)?;
    ```
  - **Code, actual default:** `passes/cq-order/src/lib.rs:49-50`:
    ```rust
    #[default]
    Written,
    ```
  - **Code, explain:** `features/workloads/xtask/src/explain.rs:385-386`:
    ```rust
    fn compile_cq(source: &cq::Program) -> syn::Result<ra::Program> {
        cq_planners_plan::compile(source, cq_planners_plan::Planner::WrittenCoarse)
    ```
  - **Additional doc drift:** `doc/formal/passes.tex:838-839` says `plan(no_reduce)` disables semijoins “while retaining written order”; the facade code above still selects Prefix(Connected). `doc/formal/rules.tsv:94` likewise describes Written/Coarse despite the TeX rule at `passes.tex:708-711` specifying Prefix(Connected).
- **Violation:** The same empty/default planning intent does not identify one composition. The GPU advice route independently reimplements planning and defaults to Written; explain independently chooses WrittenCoarse without the facade’s default Masks composition. **[INFERENCE]** Diagnostic plans and route-to-route timings can therefore describe materially different joins even before target representation choices.
- **Belongs in:** One shared, printable planner invocation/default composition consumed by facade, tooling, and build-time drivers.

### 4. Catalog selection does not follow its formal “first eligible token” rule [B, F] [severity med]

- **Evidence:**
  - **Doc:** `doc/formal/backend_ir.tex:458-464` defines:
    ```text
    k = first { k ∈ K | needs(e) ⊆ caps(k), levels(k) ≠ ∅, … }
    ```
  - **Doc:** `passes/cq-sequence-plan/src/rewrites/catalog.rs:6-7`: “Replace built provider types with the first catalog token satisfying all of their read obligations. Entry order and catalog order are significant.”
  - **Code:** `passes/cq-sequence-plan/src/rewrites/catalog.rs:207-209`:
    ```rust
    .min_by_key(|token| match &entry.structure {
        Structure::Built { keyed, .. } => {
            !crate::catalog::matches_type(&token.ty, &keyed.ty)
    ```
- **Violation:** After eligibility filtering, selection prefers a token matching the existing representation, even if a different eligible token occurs earlier in the ordered catalog. The result remains capability-checked, but source representation introduces an additional undocumented priority. **[INFERENCE]** Two otherwise equivalent routes with different preselected Structures types can resist the same target catalog differently.
- **Belongs in:** An explicit catalog-selection policy, reflected consistently in PL-Select and the recorded selection invocation.

### 5. cq-stage identifies printer input languages by macro-name spelling [C, E] [severity med]

Resolved by ST-Endpoint: `print_input = structures` explicitly selects a
Structures terminal; Sequence defaults otherwise remain intact. The historical
evidence below describes the removed path-spelling dispatch. The execution
regression aliases a Structures consumer as `translate_pull` and compares its
answers with the default pull route.

- **Evidence:**
  - **Doc:** `doc/formal/composition.tex:46-47`: “Custom Sequence terminal paths require explicit sequence and grade phases; terminal compatibility is checked by the pass protocol.”
  - **Doc:** `doc/DESIGN.md:166-170`: “Languages implement `Parse`, `ToTokens`, and `cq_stage::Language`. The protocol checks source and target contracts and queue shape…”
  - **Code:** `facade/cq-stage/src/lib.rs:536-542`:
    ```rust
    let structures_terminal = !explicit_sequence
        && self.print.as_ref().is_some_and(|print| {
            print
    ```
    Its path test is `.segments.last().is_none_or(|p| p.ident != "translate_pull")`.
  - **Code:** `facade/cq-stage/src/lib.rs:559-564`:
    ```rust
    if sequence_target
        && (sequence.is_none()
            || grade.is_none()
    ```
    The same condition includes `terminal.is_some_and(|p| p.ident == "push")`; `:572` separately special-cases `"translate_pull"`.
- **Violation:** Before the typed pass protocol can reject incompatible IR, `Configuration::resolve` itself interprets terminal behavior from the last identifier. The namespace is ignored: an unrelated macro named `push` is categorically rejected for Sequence, while any macro named `translate_pull` is treated as the built-in route. Aliasing a macro changes configuration behavior without changing its endpoint types.
- **Belongs in:** Explicit terminal-input-language/phase metadata, leaving actual payload compatibility to the checked pass boundary.

### 6. The TPC-H driver is a profile-name compiler rather than a generic pipeline executor [C, E] [severity med]

**Resolved by DR-Recipe:** workload registration now produces serializable
`Recipe`/`Schedule` facts. `compile_stage` and schedule expansion receive no
query/profile string. Retention/membership selectors, planner mode, expansion
selectors and geometry are input facts and expand through checked rewrites.
Replay reports retain the original recipe and contain a completed pipeline with
that recipe cleared, avoiding double expansion. The evidence below is historical.

- **Evidence:**
  - **Doc:** `doc/formal/composition.tex:50-52`: “Passes receive no configuration beyond their arguments: every choice is in the program text.”
  - **Doc:** `doc/formal/backend_ir.tex:873-877`: “Serialized author advice and generated advice use the same checked operations. The facade’s Sequence-input `plan = path` phase may replace the rule-based implementation with an ordinary custom pass…”
  - **Code:** `experiments/gpu/tpch/build.rs:387-390`:
    ```rust
    } else if profile == "h05" {
        advice::h05_schedule(&authored.program)
    } else if hand_profile {
    ```
    The next line dispatches `advice::hand_schedule(&authored.program, profile)`.
  - **Code:** `experiments/gpu/tpch/src/advice.rs:886-888`:
    ```rust
    let expanded: &[&str] = match query {
        "h09" | "h08" | "d27" | "d19" | "d85" => &["Lineitem"],
        "h03" | "h07" | "h07_block" => &["Orders"],
    ```
  - Additional profile-dependent Physical/Structures insertion is at `build.rs:165-223`; segment sizes are selected in `advice.rs:909-913`.
- **Violation:** There is an existing public custom-pass mechanism, but a second driver-owned compiler selects recipe implementations and producer roles from query/profile strings. **Important qualification:** these helpers do apply checked `Advice` and append traces; the finding is **not** that their final tiling is unrecorded. The bypass is the untyped profile-to-recipe selection and separate execution machinery, which the input `advice::Pipeline` cannot express as an ordinary pass invocation.
- **Belongs in:** Explicit configured planner/recipe invocations producing the existing checked advice trace.

### 7. GPU advice duplicates both the grammar and the pass registry, with observable parser divergence [C, D, E] [severity med]

**Resolved by DR-Registry and Plan-Prefix:** GPU advice now parses
`cq_stage::Invocation` and `cq_planning::PlanOptions`, and calls shared
`prefix_valuation`. Physical/Structures execution and public proc-macro adapters
are generated from one registry, accepting registered bare or `cq_rewrites`
paths and rejecting arbitrary same-suffix namespaces. The evidence below is
historical; no GPU execution is claimed by this structural conformance update.

- **Evidence:**
  - **Doc:** `experiments/gpu/README.md:602-603`: “`plan` uses the CPU `order(...)`, `no_reduce`, `reduce(T with S)` and `skip(T with S)` conventions.”
  - **Doc:** `doc/DESIGN.md:138-140`: “`Path(tokens)` supplies optional opaque arguments to a pass.”
  - **Code, facade parser:** `facade/mini-linq-macros/src/lib.rs:199-201`:
    ```rust
    if option == "order" && options.order.is_none() {
        let content;
        syn::parenthesized!(content in input);
    ```
    It separately rejects repeated `no_reduce` at `:208` and trailing comma at `:235-236`.
  - **Code, GPU parser:** `experiments/gpu/tpch/src/advice.rs:273-275`:
    ```rust
    if key == "order" {
        plan.order = arguments
            .parse_terminated(cq_order::OrderAtom::parse, Token![,])?
    ```
  - **Code, second registry:** `experiments/gpu/tpch/src/advice.rs:407-409`:
    ```rust
    current = match name.to_string().as_str() {
        "retain_streams" => cq_rewrite_passes::retain_streams(&current, arguments)?,
        "hoist" => cq_rewrite_passes::hoist(&current, arguments)?,
    ```
    Structures has another string allowlist at `:428-435`.
- **Violation:** Repeated GPU `order(...)` overwrites the earlier order, repeated `no_reduce` is accepted, and trailing commas are accepted, whereas the facade rejects those forms. Separately, the GPU route parses a bare `Ident(arguments)` rather than `cq_stage::Invocation`’s arbitrary path and opaque argument packet. Adding a public rewrite does not make it available to this driver without updating its bespoke allowlists. Existing rewrite implementations do contain checks; this is not evidence that correspondence is universally skipped.
- **Belongs in:** A shared planning-options type and one invocation/registry protocol, reused by macro and programmatic entry points.

### 8. Explain observes the resolved facade pipeline [resolved: ST-Observe, DR-Explain]

- **Implementation:** `features/workloads/xtask/src/explain.rs` generates `mini_linq::compile!` observations with `stage = ::core::stringify`, rather than invoking its own fixed planner/lowerer/printer chain.
- **Coverage:** CQ, VO, RA, IX, Physical, Structures, representation, staging, Sequence, decomposition, grading and final Rust are selectable. Default VO is the shared `prefix_valuation` prefix, not WrittenCoarse planning; custom planner internals are not guessed from macro names.
- **Protocol:** `facade/cq-stage/src/lib.rs` resolves the ordinary queue and records boundary cutoffs in that same construction. The receiver runs after ordinary source/target/correspondence/context checks. Terminal Rust is captured after translation, not by stringifying an unexpanded printer invocation.
- **Evidence boundary:** Source inspection and regression tests cover queue prefixes, argument preservation and payload round trips. Capturing Rust does not type-check or execute the query. No new test execution is claimed here.

Remaining driver findings above are independent of this diagnostic cutover.
