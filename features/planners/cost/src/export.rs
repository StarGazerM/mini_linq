//! Versioned, deterministic input for an external planner.
//!
//! Canonical compiler-language text is authoritative, including all opaque
//! expressions, premises, reductions and invocation queues. JSON projections
//! are checked against that text, not against a replay of the scheduler.
use cq_lang_cq as cq;
use cq_lang_datalog_round as round;
use cq_lang_vo as vo;
use cq_syntax::symbol_name;
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const SCHEMA: &str = "mini-linq.planner-input.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportError(pub String);
impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for ExportError {}
impl From<syn::Error> for ExportError {
    fn from(error: syn::Error) -> Self { Self(error.to_string()) }
}
impl From<serde_json::Error> for ExportError {
    fn from(error: serde_json::Error) -> Self { Self(error.to_string()) }
}
pub type Result<T, E = ExportError> = std::result::Result<T, E>;

fn require(condition: bool, message: &str) -> Result<()> {
    if condition { Ok(()) } else { Err(ExportError(message.into())) }
}
fn text(value: &(impl ToTokens + ?Sized)) -> String { value.to_token_stream().to_string() }
pub fn sha256(bytes: impl AsRef<[u8]>) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes.as_ref()))
}
fn check_hash(value: &str) -> Result<()> {
    require(value.len() == 71 && value.starts_with("sha256:")
        && value.as_bytes()[7..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)),
        "expected sha256:<64 lowercase hex digits>")
}

/// Caller-supplied compiler build identity and the actual invocation description.
/// The compiler digest is an identity, not a claim that export rebuilt a compiler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub compiler_hash: String,
    pub target: Target,
    pub invocation: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target { Cpu, CpuPar, Gpu }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DatasetIdentity {
    /// No evidence supplied: unknown, not the identity of an empty dataset.
    Unbound {},
    Bound { dataset_hash: String, schema_hash: String, evidence_hash: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub source_hash: String,
    /// Canonical semantic source with only declared_evidence erased; numeric
    /// run evidence may cite this digest without creating a self-reference.
    pub program_hash: String,
    pub compiler_hash: String,
    pub configuration_hash: String,
    /// Binds every consumed plan and candidate projection, not merely its source.
    pub checked_records_hash: String,
    pub dataset: DatasetIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "language", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckedIr {
    Cq {},
    /// Complete Round includes Source, Strata, Incremental and Storage records.
    Datalog { round: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerInput {
    pub schema: String,
    pub identity: Identity,
    pub configuration: Configuration,
    /// Exact UTF-8 input bytes, not a pathname or generated Rust translation.
    pub source: String,
    pub checked_ir: CheckedIr,
    /// All holes, including negative-key construction holes, in compiler order.
    pub holes: Vec<Hole>,
    pub layouts: Vec<Layout>,
    /// Digest of this entire record with this field replaced by the empty string.
    pub input_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Checked independently with cq_syntax::evidence, never legacy PlanOptions.
    pub canonical_json: String,
    /// The complete declared_evidence attachment, including explicit aliases.
    pub attachment_ir: String,
    pub bindings: Vec<EvidenceBinding>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBinding {
    pub input_relation: String,
    pub dataset_relation: String,
    pub view: EvidenceView,
    pub columns: Vec<EvidenceColumn>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceView { Full, Delta, Subset }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceColumn {
    pub input_column: String,
    pub dataset_column: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Origin {
    Cq {},
    Rule {
        stratum: usize,
        seed: bool,
        variant: usize,
        source_rule: usize,
        source_label: Option<String>,
        /// Source's single-head rule index plus head relation is the split-head
        /// identity. Export does not invent a pre-elaboration multihead label.
        head_relation: usize,
        original_delta: Option<usize>,
        driver: String,
        permutation: Vec<usize>,
        authored: bool,
        admission: String,
        fanout: Vec<String>,
    },
    NegativeKey { stratum: usize, key: usize, relation: usize },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hole {
    pub id: usize,
    pub origin: Origin,
    pub source_cq: String,
    pub normalized_cq: String,
    pub checked_vo: String,
    pub vo_hash: String,
    pub captures_ir: String,
    pub ordering_ir: Option<String>,
    pub evidence: Option<Evidence>,
    pub candidate: Scope,
    /// Source-written proof reference, NOT another recorded executable plan.
    /// Contains every predicate even when the incumbent uses Restrict instead.
    /// Its coverage/binding analysis deliberately imposes no VO-Order schedule.
    pub candidate_reference: Scope,
    pub occurrences: Vec<Occurrence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    /// CQ scope path alternates enclosing body-clause and branch ordinals.
    pub source_path: Vec<usize>,
    pub normalized_path: Vec<usize>,
    pub logical_relation: String,
    pub input_relation: String,
    /// Input-column ordinal -> logical source-column ordinal.
    pub columns: Vec<usize>,
    pub version: Version,
    pub demand: Option<Demand>,
    /// VO step paths alternate enclosing step and branch ordinals.
    pub vo_steps: Vec<Vec<usize>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Version { Full, Delta }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    pub kind: String,
    pub index: usize,
    pub columns: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub version: Version,
    pub index: usize,
    pub logical_relation: String,
    pub input_relation: String,
    pub columns: Vec<usize>,
    pub key: usize,
    pub provider_ir: String,
    pub hasher_ir: String,
    pub maintenance: String,
    pub protocol_ir: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Complete normalized candidate body, not just the introducing atoms.
    pub clauses: Vec<Clause>,
    /// All positive occurrences in this scope; nested scopes are separate.
    pub positive_clauses: Vec<usize>,
    pub steps: Vec<Step>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClauseKind { Atom, Block, Equality, Constraint, OpaqueGuard, OpaqueLet, OpaqueFor, Negation }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clause {
    pub index: usize,
    pub kind: ClauseKind,
    pub ir: String,
    pub opaque: Option<OpaqueBindings>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueBindings {
    /// Supplied lexical analysis, not a Rust-expression classifier in this tool.
    pub observed_free_names: Vec<String>,
    pub conservative_environment: bool,
    pub bindings: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub index: usize,
    pub ir: String,
    pub required: Vec<String>,
    pub provides: Vec<String>,
    pub bound_before: Vec<String>,
    pub provided_before: Vec<String>,
    pub introduces: Vec<String>,
    pub atom: bool,
    pub preprocessing: bool,
    pub occurrence: Option<Vec<usize>>,
    /// Block kind/placement are semantic scope boundaries, not guessed barriers.
    pub block_kind: Option<String>,
    pub block_placement: Option<String>,
    pub domain_requires: Vec<String>,
    pub branches: Vec<Scope>,
}

fn evidence(record: Option<&cq_syntax::declared_evidence::DeclaredEvidence>) -> Result<Option<Evidence>> {
    record.map(|record| Ok(Evidence {
        canonical_json: record.document.canonical_json().map_err(|e| ExportError(e.to_string()))?,
        attachment_ir: text(record),
        bindings: record.bindings.iter().map(|binding| EvidenceBinding {
            input_relation: symbol_name(&binding.relation),
            dataset_relation: binding.source.clone(),
            view: match binding.view {
                cq_syntax::declared_evidence::EvidenceView::Full => EvidenceView::Full,
                cq_syntax::declared_evidence::EvidenceView::Delta => EvidenceView::Delta,
                cq_syntax::declared_evidence::EvidenceView::Subset => EvidenceView::Subset,
            },
            columns: binding.columns.iter().map(|column| EvidenceColumn {
                input_column: symbol_name(&column.column),
                dataset_column: column.source.clone(),
            }).collect(),
        }).collect(),
    })).transpose()
}
fn dataset(record: Option<&cq_syntax::declared_evidence::DeclaredEvidence>) -> Result<DatasetIdentity> {
    match record {
        None => Ok(DatasetIdentity::Unbound {}),
        Some(record) => {
            let canonical = record.document.canonical_json().map_err(|e| ExportError(e.to_string()))?;
            Ok(DatasetIdentity::Bound {
                dataset_hash: record.document.dataset.hash.clone(),
                schema_hash: record.document.dataset.schema_hash.clone(),
                evidence_hash: sha256(canonical),
            })
        }
    }
}
fn identity(source: &str, program: &impl ToTokens, config: &Configuration, dataset: DatasetIdentity) -> Result<Identity> {
    check_hash(&config.compiler_hash)?;
    require(!config.invocation.trim().is_empty(), "configuration invocation must not be blank")?;
    Ok(Identity {
        source_hash: sha256(source), program_hash: sha256(text(program)),
        compiler_hash: config.compiler_hash.clone(),
        configuration_hash: sha256(serde_json::to_vec(config)?), dataset,
        checked_records_hash: String::new(),
    })
}

fn scope(body: &cq::CommaList<cq::BodyItem>, steps: &[vo::Step], analysis: &vo::Analysis) -> Result<Scope> {
    let clauses = body.iter().enumerate().map(|(index, item)| {
        let kind = match item {
            cq::BodyItem::Positive { .. } => ClauseKind::Atom,
            cq::BodyItem::Block(_) => ClauseKind::Block,
            cq::BodyItem::Equality(_) => ClauseKind::Equality,
            cq::BodyItem::Constraint(_) => ClauseKind::Constraint,
            cq::BodyItem::Filter(_) => ClauseKind::OpaqueGuard,
            cq::BodyItem::Let(_) => ClauseKind::OpaqueLet,
            cq::BodyItem::For(_) => ClauseKind::OpaqueFor,
            cq::BodyItem::Negation(_) => ClauseKind::Negation,
            _ => return Err(ExportError("unsupported clause at checked normal-form boundary".into())),
        };
        Ok(Clause { index, kind, ir: text(item), opaque: item.rust_step().map(|rust| OpaqueBindings {
            observed_free_names: rust.free_variables.names.into_iter().collect(),
            conservative_environment: rust.free_variables.opaque,
            bindings: rust.bindings.iter().map(symbol_name).collect(),
        }) })
    }).collect::<Result<Vec<_>>>()?;
    let mut records = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let facts = &analysis.order[index];
        let mut record = Step {
            index, ir: text(step), required: facts.required.iter().cloned().collect(),
            provides: facts.provides.iter().cloned().collect(),
            bound_before: analysis.bound_before[index].clone(),
            provided_before: analysis.provided_before[index].iter().cloned().collect(),
            introduces: analysis.introduces[index].clone(), atom: facts.atom,
            preprocessing: facts.preprocessing, occurrence: None,
            block_kind: None, block_placement: None, domain_requires: Vec::new(), branches: Vec::new(),
        };
        match step {
            vo::Step::Atom(atom, _) => record.occurrence = Some(occurrence_path(&atom.clause)),
            vo::Step::Block(block, _) => {
                let clause = block.source.clause();
                record.occurrence = Some(occurrence_path(&block.source.clause));
                record.block_kind = Some(match block.kind { cq::blocks::Kind::Union => "union", cq::blocks::Kind::Fallback => "fallback" }.into());
                record.block_placement = Some(text(&block.placement));
                record.domain_requires = analysis.domain_requires.get(&clause).into_iter().flatten().cloned().collect();
                let cq::BodyItem::Block(source) = &body[clause] else {
                    return Err(ExportError("checked block has no candidate scope".into()));
                };
                for ((source, branch), facts) in source.branches.iter().zip(&block.branches).zip(&analysis.branches[&clause]) {
                    record.branches.push(scope(&source.body, &branch.steps, facts)?);
                }
            }
            _ => {}
        }
        records.push(record);
    }
    Ok(Scope {
        positive_clauses: body.iter().enumerate().filter_map(|(i, item)| matches!(item, cq::BodyItem::Positive { .. } | cq::BodyItem::Block(_)).then_some(i)).collect(),
        clauses, steps: records,
    })
}

fn occurrence_path(path: &cq_syntax::Occurrence) -> Vec<usize> {
    path.0.iter().map(|part| part.parse().expect("checked occurrence ordinal")).collect()
}
fn step_paths(steps: &[vo::Step], wanted: &[usize], prefix: &[usize], result: &mut Vec<Vec<usize>>) {
    for (index, step) in steps.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(index);
        match step {
            vo::Step::Atom(atom, _) if occurrence_path(&atom.clause) == wanted => result.push(path),
            vo::Step::Block(block, _) => {
                if occurrence_path(&block.source.clause) == wanted { result.push(path.clone()); }
                for (branch, body) in block.branches.iter().enumerate() {
                    let mut nested = path.clone(); nested.push(branch);
                    step_paths(&body.steps, wanted, &nested, result);
                }
            }
            _ => {}
        }
    }
}
fn map_occurrences(source: &cq::CommaList<cq::BodyItem>, normal: &cq::CommaList<cq::BodyItem>, source_prefix: &[usize], normal_prefix: &[usize], steps: &[vo::Step], records: &mut Vec<Occurrence>) -> Result<()> {
    let positives = |body: &cq::CommaList<cq::BodyItem>| body.iter().enumerate().filter_map(|(i, item)| matches!(item, cq::BodyItem::Positive { .. } | cq::BodyItem::Block(_)).then_some(i)).collect::<Vec<_>>();
    let original = positives(source);
    let normalized = positives(normal);
    require(original.len() == normalized.len(), "normalization changed candidate occurrence coverage")?;
    for (a, b) in original.into_iter().zip(normalized) {
        let mut source_path = source_prefix.to_vec(); source_path.push(a);
        let mut normalized_path = normal_prefix.to_vec(); normalized_path.push(b);
        match (&source[a], &normal[b]) {
            (cq::BodyItem::Positive { atom: old }, cq::BodyItem::Positive { atom }) => {
                let mut vo_steps = Vec::new(); step_paths(steps, &normalized_path, &[], &mut vo_steps);
                require(!vo_steps.is_empty(), "candidate occurrence has no checked VO read")?;
                records.push(Occurrence { source_path, normalized_path, logical_relation: symbol_name(&old.relation), input_relation: symbol_name(&atom.relation), columns: (0..old.args.len()).collect(), version: Version::Full, demand: None, vo_steps });
            }
            (cq::BodyItem::Block(old), cq::BodyItem::Block(new)) => {
                require(old.branches.len() == new.branches.len(), "normalization changed branch coverage")?;
                for (branch, (old, new)) in old.branches.iter().zip(&new.branches).enumerate() {
                    let mut sp = source_path.clone(); sp.push(branch);
                    let mut np = normalized_path.clone(); np.push(branch);
                    map_occurrences(&old.body, &new.body, &sp, &np, steps, records)?;
                }
            }
            _ => return Err(ExportError("normalization changed occurrence kind".into())),
        }
    }
    Ok(())
}

/// Mechanical source-to-proof-reference translation. There is exactly one step
/// per source clause, in source order; no readiness drainage or ranking occurs.
/// The production reference checker, not this adapter, supplies binding facts.
fn reference_steps(
    body: &cq::CommaList<cq::BodyItem>,
    context: &BTreeSet<String>,
    parent: Option<(&cq_syntax::Occurrence, usize)>,
) -> Result<Vec<vo::Step>> {
    let mut written = context.clone();
    let mut result = Vec::with_capacity(body.len());
    for (index, item) in body.iter().enumerate() {
        let path = match parent {
            Some((path, branch)) => path.child(branch, index),
            None => cq_syntax::Occurrence::new(index),
        };
        let step = match item {
            cq::BodyItem::Positive { atom } => {
                written.extend(atom.variables().map(symbol_name));
                vo::Step::Atom(vo::Subatom {
                    relation: atom.relation.clone(), at: Default::default(),
                    clause: path, paren: Default::default(),
                    columns: atom.variables().cloned().map(vo::Column::Variable).collect(),
                    check: None,
                }, Default::default())
            }
            cq::BodyItem::Block(block) => {
                let facts = block.analyze(&written);
                let context = facts.free.keys().cloned().collect();
                let mut branches = Vec::with_capacity(block.branches.len());
                for (branch, source) in block.branches.iter().enumerate() {
                    branches.push(vo::blocks::Branch {
                        brace: source.brace,
                        steps: reference_steps(&source.body, &context, Some((&path, branch)))?,
                    });
                }
                written.extend(facts.interface.keys().cloned());
                vo::Step::Block(vo::blocks::Block {
                    source: vo::Subatom {
                        relation: syn::Ident::new("block", proc_macro2::Span::call_site()),
                        at: Default::default(), clause: path, paren: Default::default(),
                        columns: facts.free.values().chain(facts.interface.values())
                            .cloned().map(vo::Column::Variable).collect(),
                        check: None,
                    },
                    placement: if facts.rust { vo::Placement::Correlated } else { vo::Placement::Hoisted },
                    kind: block.kind, branches,
                }, Default::default())
            }
            cq::BodyItem::Equality(value) => vo::Step::Equality(value.clone(), Default::default()),
            cq::BodyItem::Constraint(value) => vo::Step::Constraint(value.clone(), Default::default()),
            cq::BodyItem::Negation(value) => vo::Step::Negation(value.clone(), Default::default()),
            cq::BodyItem::Filter(value) => vo::Step::Guard(value.clone(), Default::default()),
            cq::BodyItem::Let(value) => vo::Step::Let(value.clone(), Default::default()),
            cq::BodyItem::For(value) => vo::Step::For(value.clone(), Default::default()),
            _ => return Err(ExportError("non-normal clause in candidate reference".into())),
        };
        if let Some(rust) = item.rust_step() {
            written.extend(rust.bindings.iter().map(symbol_name));
        }
        result.push(step);
    }
    Ok(result)
}

fn hole(id: usize, source: &cq::Program, plan: &vo::Program, origin: Origin) -> Result<Hole> {
    cq::contract::check(source)?;
    let analysis = vo::contract::check(plan)?;
    // Normalization is an identity translation, not a second ordering pass.
    let normal = cq::normal::normalize(source)?;
    require(normal == plan.cq, "checked VO does not embed this complete normalized candidate")?;
    let reference = reference_steps(&plan.cq.query.body, &BTreeSet::new(), None)?;
    let reference_analysis = vo::order::analyze_reference(&plan.cq, &reference)?;
    let mut occurrences = Vec::new();
    map_occurrences(&source.query.body, &normal.query.body, &[], &[], &plan.valuation.steps, &mut occurrences)?;
    let checked_vo = text(plan);
    Ok(Hole {
        id, origin, source_cq: text(source), normalized_cq: text(&plan.cq),
        vo_hash: sha256(&checked_vo), checked_vo, captures_ir: text(&plan.cq.query.captures),
        ordering_ir: plan.valuation.ordering.as_ref().map(text), evidence: evidence(plan.cq.declared_evidence.as_ref())?,
        candidate: scope(&plan.cq.query.body, &plan.valuation.steps, &analysis)?, occurrences,
        candidate_reference: scope(&plan.cq.query.body, &reference, &reference_analysis)?,
    })
}

fn seal(mut record: PlannerInput) -> Result<PlannerInput> {
    record.identity.checked_records_hash = sha256(serde_json::to_vec(&(
        &record.checked_ir, &record.holes, &record.layouts,
    ))?);
    record.input_hash.clear();
    record.input_hash = sha256(serde_json::to_vec(&record)?);
    Ok(record)
}
/// Project a consumed checked VO. Does not select an order or a provider.
pub fn export_cq(source: &str, plan: &vo::Program, configuration: Configuration) -> Result<PlannerInput> {
    let mut program: cq::Program = syn::parse_str(source)?;
    // Numeric attachments may themselves cite program_hash. Exclude only this
    // nonsemantic attachment, retaining legacy keys and authored ordering.
    let declared = program.declared_evidence.take();
    let identity = identity(source, &program, &configuration, dataset(declared.as_ref())?)?;
    program.declared_evidence = declared;
    seal(PlannerInput {
        schema: SCHEMA.into(), identity, configuration, source: source.into(), checked_ir: CheckedIr::Cq {},
        holes: vec![hole(0, &program, plan, Origin::Cq {})?], layouts: Vec::new(), input_hash: String::new(),
    })
}

/// Project the complete checked Round and the VOs actually consumed for every
/// hole. Missing key/seed/steady holes and unrelated VOs are errors.
pub fn export_datalog(source: &str, program: &round::Program, plans: &[vo::Program], configuration: Configuration) -> Result<PlannerInput> {
    use round::storage::text::Text;
    round::contract::check(program)?;
    match configuration.target {
        Target::Cpu => require(
            program.storage.device.is_none() && program.execution.is_empty()
                && program.holes.iter().all(|hole| hole.interface != round::HoleInterface::Batch),
            "CPU target conflicts with recorded device, batch or parallel execution",
        )?,
        Target::CpuPar => {
            let partitioned: BTreeSet<_> = program.execution.iter().map(|entry| entry.hole).collect();
            let scans_covered = program.context.strata.iter()
                .flat_map(|stratum| stratum.seeds.iter().chain(&stratum.steps))
                .filter(|rule| rule.scan.is_some())
                .all(|rule| partitioned.contains(&rule.hole));
            require(
                program.storage.device.is_none() && scans_covered
                    && program.execution.iter().all(|entry| entry.strategy == "rayon_sharded")
                    && program.holes.iter().all(|hole| hole.interface != round::HoleInterface::Batch),
                "CPU-par target requires rayon_sharded certificates for every eligible scan and no device/batch holes",
            )?;
        }
        Target::Gpu => require(
            program.storage.device.is_some() && program.execution.is_empty()
                && program.holes.iter().all(|hole| hole.interface == round::HoleInterface::Batch),
            "GPU target requires recorded device storage and batch holes without CPU execution",
        )?,
    }
    require(plans.len() == program.holes.len(), "export requires exactly one checked VO per Round hole")?;
    let mut parsed: round::incremental::source::Program = syn::parse_str(source)?;
    let original = &program.storage.incremental.strata.source;
    require(&parsed == original, "Round embeds a different source program")?;
    let declared = parsed.declared_evidence.take();
    let identity = identity(source, &parsed, &configuration, dataset(declared.as_ref())?)?;
    let mut origins = vec![None; plans.len()];
    for (stratum, component) in program.context.strata.iter().enumerate() {
        for (seed, rules) in [(true, &component.seeds), (false, &component.steps)] {
            for rule in rules {
                let variant = &program.storage.incremental.variants[rule.variant];
                require(origins[rule.hole].is_none(), "a Round hole has multiple owners")?;
                origins[rule.hole] = Some(Origin::Rule {
                    stratum, seed, variant: rule.variant, source_rule: variant.rule,
                    source_label: original.labels[variant.rule].as_ref().map(symbol_name),
                    head_relation: rule.head, original_delta: variant.delta,
                    driver: text(&program.storage.incremental.driver), permutation: variant.order.clone(),
                    authored: variant.authored, admission: rule.known.text().to_string(),
                    fanout: rule.fanout.iter().map(|f| f.text().to_string()).collect(),
                });
            }
        }
        for (key, owner) in component.keys.iter().enumerate() {
            require(origins[owner.hole].is_none(), "a key hole has multiple owners")?;
            origins[owner.hole] = Some(Origin::NegativeKey { stratum, key, relation: owner.relation });
        }
    }
    let mut holes = Vec::with_capacity(plans.len());
    for (id, ((input, plan), origin)) in program.holes.iter().zip(plans).zip(origins).enumerate() {
        let origin = origin.ok_or_else(|| ExportError("unowned Round hole".into()))?;
        let mut record = hole(id, &input.cq, plan, origin)?;
        if let Origin::Rule { variant, source_rule, original_delta, .. } = &record.origin {
            let variant_index = *variant;
            let variant = &program.storage.incremental.variants[variant_index];
            for occurrence in &mut record.occurrences {
                // Datalog's source contract has flat rule bodies. No source
                // occurrence identity is inferred from a generated alias name.
                require(occurrence.source_path.len() == 1, "nested Datalog occurrence lacks source correspondence")?;
                let position = occurrence.source_path[0];
                let original_position = variant.order[position];
                let cq::BodyItem::Positive { atom } = &original.rules[*source_rule].body[original_position] else {
                    return Err(ExportError("variant occurrence has no positive source clause".into()));
                };
                let read = program.storage.reads.iter().find(|r| r.variant == variant_index && r.position == position)
                    .ok_or_else(|| ExportError("positive occurrence lacks checked Storage demand".into()))?;
                occurrence.source_path[0] = original_position;
                occurrence.logical_relation = symbol_name(&atom.relation);
                occurrence.columns = read.columns.clone();
                occurrence.version = if *original_delta == Some(original_position) { Version::Delta } else { Version::Full };
                occurrence.demand = Some(Demand { kind: symbol_name(&read.kind), index: read.index, columns: read.columns.clone() });
            }
        } else if let Origin::NegativeKey { relation, .. } = record.origin {
            for occurrence in &mut record.occurrences {
                occurrence.logical_relation = symbol_name(&original.declarations[relation].name);
                // Key construction reads the canonical complete relation layout.
                occurrence.columns = program.context.layouts[program.context.relations[relation].providers.canonical].columns.clone();
            }
        }
        holes.push(record);
    }
    let mut layouts = Vec::new();
    for (version, entries) in [(Version::Full, &program.context.layouts), (Version::Delta, &program.context.delta_layouts)] {
        for (index, layout) in entries.iter().enumerate() {
            layouts.push(Layout {
                version, index, logical_relation: symbol_name(&original.declarations[layout.relation].name),
                input_relation: symbol_name(&layout.name), columns: layout.columns.clone(), key: layout.key,
                provider_ir: layout.provider.text().to_string(), hasher_ir: layout.hasher.text().to_string(),
                maintenance: symbol_name(&layout.maintenance), protocol_ir: layout.protocol.text().to_string(),
            });
        }
    }
    seal(PlannerInput { schema: SCHEMA.into(), identity, configuration, source: source.into(),
        checked_ir: CheckedIr::Datalog { round: text(program) }, holes, layouts, input_hash: String::new() })
}

impl PlannerInput {
    /// Recheck full IR and every projection, without rerunning any producer.
    pub fn check(&self) -> Result<()> {
        require(self.schema == SCHEMA, "unsupported planner-input version")?;
        let plans = self.holes.iter().map(|hole| syn::parse_str::<vo::Program>(&hole.checked_vo)).collect::<syn::Result<Vec<_>>>()?;
        let expected = match &self.checked_ir {
            CheckedIr::Cq {} => {
                require(plans.len() == 1, "CQ requires exactly one complete checked VO")?;
                export_cq(&self.source, &plans[0], self.configuration.clone())?
            }
            CheckedIr::Datalog { round } => {
                let round: round::Program = syn::parse_str(round)?;
                export_datalog(&self.source, &round, &plans, self.configuration.clone())?
            }
        };
        require(self == &expected, "planner input identity, candidate, occurrence or checked-record projection differs")
    }
    /// Binding a later result to a compilation requires an out-of-band expected
    /// identity; a self-consistent digest is not an authenticity certificate.
    pub fn check_against(&self, expected: &Identity) -> Result<()> {
        self.check()?;
        require(&self.identity == expected, "planner input belongs to a different source/compiler/configuration/dataset")
    }
    pub fn canonical_json(&self) -> Result<String> {
        self.check()?;
        Ok(serde_json::to_string(self)?)
    }
}
pub fn parse_json(input: &str) -> Result<PlannerInput> {
    let record: PlannerInput = serde_json::from_str(input)?;
    record.check()?;
    Ok(record)
}

/// A front-end convenience, explicitly separate from record-only export.
pub fn compile_cq(source: &str, options: &cq_planning::PlanOptions, configuration: Configuration) -> Result<PlannerInput> {
    require(configuration.target == Target::Cpu, "compile_cq constructs only the CPU route; export a consumed backend-neutral VO for another target")?;
    let parsed: cq::Program = syn::parse_str(source)?;
    let plan = cq_planning::prefix_valuation(&parsed, options)?;
    export_cq(source, &plan, configuration)
}
/// Compile the common CPU DL-Source route exactly once. Custom pass queues
/// should supply their actual Round and VOs to `export_datalog` instead.
pub fn compile_datalog(source: &str, configuration: Configuration) -> Result<PlannerInput> {
    require(configuration.target == Target::Cpu, "compile_datalog constructs only the CPU route; supply consumed target Round/VO records")?;
    let parsed: round::incremental::source::Program = syn::parse_str(source)?;
    let strata = cq_datalog_stratify::compile(&parsed)?;
    let incremental = cq_datalog_incremental::compile(&strata)?;
    let storage = cq_datalog_storage::compile(&incremental)?;
    let round = cq_datalog_fork::compile(&storage)?;
    let mut plans = Vec::with_capacity(round.holes.len());
    for hole in &round.holes {
        let pipeline = hole.invocation_pipeline()?;
        let plan = hole.plan.ok_or_else(|| ExportError("Round hole has no planning invocation".into()))?;
        let options: cq_planning::PlanOptions = syn::parse2(pipeline.passes[plan].arguments.clone())?;
        plans.push(cq_planning::prefix_valuation(&hole.cq, &options)?);
    }
    export_datalog(source, &round, &plans, configuration)
}
