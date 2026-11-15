//! Estimated facts of derived relations: this dataset's structural bound
//! scaled by what completed runs of the same program showed on example
//! datasets (PLANNING-REDESIGN, static costing). An estimate is a prediction,
//! never a certificate, and every fact it produces is cited as `estimate:`.
use std::collections::{BTreeMap, BTreeSet};

use cq_evidence::{self as ev, DatasetEvidence};
use cq_lang_datalog_round as round;
use cq_syntax::symbol_name;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};

use super::{bounds, inferred::{Inferred, InputFacts}};
use crate::export::{self, CheckedIr, ExportError, PlannerInput};

pub const OBSERVED_SCHEMA: &str = "cq.observed-relations.v1";
pub const PROFILE_SCHEMA: &str = "cq.inferred-profile.v1";
/// Citation prefix of a predicted fact.
pub const ESTIMATE: &str = "estimate:";
/// Citation prefix of a fact certified from the rules and input facts alone.
pub const STRUCTURAL: &str = "structural:";

fn error(message: impl Into<String>) -> ExportError { ExportError(message.into()) }

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedColumn {
    #[serde(default)] pub distinct: Option<u64>,
    #[serde(default)] pub max_degree: Option<u64>,
}
/// What a completed run produced for one derived relation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedRelation {
    pub rows: u64,
    /// Executed steady rounds of the relation's component.
    #[serde(default)] pub rounds: Option<u64>,
    /// By declared column name.
    #[serde(default)] pub columns: BTreeMap<String, ObservedColumn>,
}
/// The stated outcome of one completed run. The tool cannot replay it; it
/// rejects a value above the certified bound of the same dataset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observed {
    pub schema: String,
    pub program_hash: String,
    pub dataset_hash: String,
    /// By declared relation name.
    pub relations: BTreeMap<String, ObservedRelation>,
}

/// An observed value and the structural bound of the same dataset, in decimal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pair { pub observed: u64, pub bound: String }
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfiledColumn {
    #[serde(default)] pub distinct: Option<Pair>,
    #[serde(default)] pub max_degree: Option<Pair>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfiledRelation {
    pub rows: Pair,
    #[serde(default)] pub rounds: Option<u64>,
    #[serde(default)] pub columns: BTreeMap<String, ProfiledColumn>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub dataset_hash: String,
    pub relations: BTreeMap<String, ProfiledRelation>,
}
/// Per example dataset, observed values beside their structural bounds, for
/// one program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: String,
    pub program_hash: String,
    pub examples: Vec<Example>,
}
impl Profile {
    pub fn new(program_hash: impl Into<String>) -> Self {
        Self { schema: PROFILE_SCHEMA.into(), program_hash: program_hash.into(), examples: Vec::new() }
    }
    /// Add an example, replacing an earlier one of the same dataset.
    pub fn add(&mut self, example: Example) {
        self.examples.retain(|held| held.dataset_hash != example.dataset_hash);
        self.examples.push(example);
        self.examples.sort_by(|left, right| left.dataset_hash.cmp(&right.dataset_hash));
    }
}

/// The structural bounds of one derived relation: rows, and per column the
/// distinct values and the largest group, each when one exists.
struct Structural { rows: BigInt, columns: Vec<(Option<BigInt>, Option<BigInt>)> }

struct Analysis { inferred: Inferred, facts: InputFacts, bound: BTreeSet<String> }
impl Analysis {
    fn new(input: &PlannerInput, evidence: &DatasetEvidence) -> export::Result<Self> {
        let CheckedIr::Datalog { round: recorded } = &input.checked_ir else { return Err(error("derived-relation estimates need a recorded Datalog Round")); };
        let program: round::Program = syn::parse_str(recorded)?;
        let source = &program.storage.incremental.strata.source;
        let bound = source.declared_evidence.iter().flat_map(|attachment| &attachment.bindings)
            .map(|binding| symbol_name(&binding.relation)).collect();
        Ok(Self { inferred: Inferred::new(source)?, facts: InputFacts::bound(source, evidence)?, bound })
    }
    fn structural(&mut self, relation: usize) -> Option<Structural> {
        let arity = self.inferred.column_names(relation).len();
        let rows = self.inferred.support(&self.facts, relation, &(0..arity).collect()).ok()?.value;
        let columns = (0..arity).map(|column| {
            let key = BTreeSet::from([column]);
            let capped = |value: BigInt| value.min(rows.clone());
            (self.inferred.support(&self.facts, relation, &key).ok().map(|bound| capped(bound.value)),
                self.inferred.degree(&self.facts, relation, &key).ok().map(|bound| capped(bound.value)))
        }).collect();
        Some(Structural { rows, columns })
    }
}

/// One example for a profile: the observation of a completed run beside the
/// structural bounds of the same dataset.
pub fn example(input: &PlannerInput, evidence: &DatasetEvidence, observed: &Observed) -> export::Result<Example> {
    input.check()?;
    bounds::validate_dataset(input, evidence)?;
    if observed.schema != OBSERVED_SCHEMA { return Err(error("unsupported observation schema")); }
    if observed.program_hash != input.identity.program_hash { return Err(error("observation is of another program")); }
    if observed.dataset_hash != evidence.dataset.hash { return Err(error("observation is of another dataset")); }
    let mut analysis = Analysis::new(input, evidence)?;
    let mut relations = BTreeMap::new();
    for (name, seen) in &observed.relations {
        let relation = analysis.inferred.index(name).filter(|&relation| analysis.inferred.derived(relation))
            .ok_or_else(|| error(format!("observed relation {name} is not a derived relation of the program")))?;
        // A relation without a structural bound has nothing to scale.
        let Some(structural) = analysis.structural(relation) else { continue; };
        let pair = |observed: u64, bound: &BigInt, what: &str| -> export::Result<Pair> {
            if BigInt::from(observed) > *bound {
                return Err(error(format!("observed {name} {what} {observed} exceeds its certified bound {bound}: another dataset or program")));
            }
            Ok(Pair { observed, bound: bound.to_string() })
        };
        let mut columns = BTreeMap::new();
        for (column, seen) in &seen.columns {
            let at = analysis.inferred.column_names(relation).iter().position(|held| held == column)
                .ok_or_else(|| error(format!("observed column {name}.{column} is not declared")))?;
            let (distinct, degree) = &structural.columns[at];
            columns.insert(column.clone(), ProfiledColumn {
                distinct: seen.distinct.zip(distinct.as_ref()).map(|(seen, bound)| pair(seen, bound, &format!("{column} distinct"))).transpose()?,
                max_degree: seen.max_degree.zip(degree.as_ref()).map(|(seen, bound)| pair(seen, bound, &format!("{column} max degree"))).transpose()?,
            });
        }
        relations.insert(name.clone(), ProfiledRelation { rows: pair(seen.rows, &structural.rows, "rows")?, rounds: seen.rounds, columns });
    }
    Ok(Example { dataset_hash: observed.dataset_hash.clone(), relations })
}

fn median(mut values: Vec<BigRational>) -> Option<BigRational> {
    values.sort();
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        odd if odd % 2 == 1 => Some(values[middle].clone()),
        _ => Some((&values[middle - 1] + &values[middle]) / BigRational::from_integer(2.into())),
    }
}
/// The bound times the median observed/bound ratio: rounded up, at least one
/// when the bound is, never above the bound.
fn scaled(bound: &BigInt, pairs: Vec<&Pair>) -> export::Result<Option<u64>> {
    let mut ratios = Vec::new();
    for pair in pairs {
        let seen: BigInt = pair.bound.parse().map_err(|_| error("profile bound is not a decimal integer"))?;
        if seen < BigInt::from(pair.observed) { return Err(error("profile records an observation above its bound")); }
        if !seen.is_zero() { ratios.push(BigRational::new(pair.observed.into(), seen)); }
    }
    let Some(ratio) = median(ratios) else { return Ok(None); };
    let estimate = (BigRational::from_integer(bound.clone()) * ratio).ceil().to_integer();
    Ok(estimate.max(BigInt::one().min(bound.clone())).min(bound.clone()).to_u64())
}

/// Evidence extended with a synthesized relation for every derived relation
/// the evidence does not bind.
pub struct Synthesized {
    pub evidence: DatasetEvidence,
    /// Declared relation name to its synthesized evidence relation.
    pub relations: BTreeMap<String, String>,
    /// Every synthesized fact, by citation id.
    pub facts: BTreeMap<String, u64>,
}

/// Synthesize facts of unbound derived relations: the estimate where the
/// profile has a ratio, else the structural bound, else nothing.
pub fn synthesize(input: &PlannerInput, evidence: &DatasetEvidence, profile: &Profile) -> export::Result<Synthesized> {
    if profile.schema != PROFILE_SCHEMA { return Err(error("unsupported profile schema")); }
    if profile.program_hash != input.identity.program_hash { return Err(error("profile is of another program")); }
    let mut analysis = Analysis::new(input, evidence)?;
    let derived: Vec<usize> = (0..analysis.inferred.relations())
        .filter(|&relation| analysis.inferred.derived(relation) && !analysis.bound.contains(analysis.inferred.name(relation))).collect();
    let structural: BTreeMap<usize, Structural> = derived.iter().filter_map(|&relation| Some((relation, analysis.structural(relation)?))).collect();
    let mut facts = BTreeMap::new();
    let dataset = evidence.dataset.hash.clone();
    let mut count = |name: &str, what: &str, unit: ev::Unit, estimate: Option<u64>, bound: Option<u64>| -> Option<ev::CountFact> {
        let (prefix, value, source, claim, method) = match (estimate, bound) {
            (Some(value), _) => (ESTIMATE, value, ev::Source::Declared, ev::Claim::Equal,
                "prediction: structural bound of this dataset times the median observed/bound ratio of the profile's examples; not a certificate"),
            (None, Some(value)) => (STRUCTURAL, value, ev::Source::Bound, ev::Claim::Upper,
                "certified from the rules and exact input facts: factored size-bound adornments"),
            (None, None) => return None,
        };
        let id = format!("{prefix}{name}:{what}");
        facts.insert(id.clone(), value);
        Some(ev::Fact { id: id.clone(), source, claim, value: Some(value), lower: None, upper: None, dataset_hash: dataset.clone(), unit,
            provenance: ev::Provenance { producer: "cq-cost".into(), method: method.into(), locator: id, run: None, artifact_hash: None } })
    };
    let observed = |name: &str| -> Vec<&ProfiledRelation> { profile.examples.iter().filter_map(|example| example.relations.get(name)).collect() };
    // Rounds are shared by a component: the largest carried-over median, else
    // one more than the rows its members can hold.
    let mut rounds = BTreeMap::<usize, (String, Option<u64>, Option<u64>)>::new();
    for &relation in derived.iter().filter(|&&relation| analysis.inferred.recursive(relation)) {
        let component = analysis.inferred.component(relation);
        let first = *component.first().expect("a component holds its relation");
        if rounds.contains_key(&first) { continue; }
        let carried = component.iter().filter_map(|&member| {
            let seen: Vec<BigRational> = observed(analysis.inferred.name(member)).into_iter().filter_map(|held| held.rounds)
                .map(|rounds| BigRational::from_integer(rounds.into())).collect();
            median(seen).and_then(|rounds| rounds.ceil().to_integer().to_u64())
        }).max();
        let capacity = component.iter().map(|member| structural.get(member).map(|bound| bound.rows.clone()))
            .sum::<Option<BigInt>>().and_then(|rows| (rows + BigInt::one()).to_u64());
        rounds.insert(first, (analysis.inferred.name(first).to_string(), carried, capacity));
    }
    let mut extended = evidence.clone();
    let mut relations = BTreeMap::new();
    for (&relation, bound) in &structural {
        let name = analysis.inferred.name(relation).to_string();
        let seen: Vec<&ProfiledRelation> = observed(&name);
        let rows = scaled(&bound.rows, seen.iter().map(|held| &held.rows).collect())?;
        let Some(rows_fact) = count(&name, "rows", ev::Unit::Rows, rows, bound.rows.to_u64()) else { continue; };
        let held = rows_fact.value.expect("synthesized value");
        let mut columns = BTreeMap::new();
        for (at, (distinct, degree)) in bound.columns.iter().enumerate() {
            let declared = &analysis.inferred.column_names(relation)[at];
            let profiled = |pick: fn(&ProfiledColumn) -> Option<&Pair>| -> Vec<&Pair> {
                seen.iter().filter_map(|held| held.columns.get(declared)).filter_map(pick).collect()
            };
            // No column holds more distinct values or a larger group than the relation has rows.
            let fact = |what: &str, unit, structural: &Option<BigInt>, pairs: Vec<&Pair>, count: &mut dyn FnMut(&str, &str, ev::Unit, Option<u64>, Option<u64>) -> Option<ev::CountFact>| -> export::Result<Option<ev::CountFact>> {
                let Some(structural) = structural else { return Ok(None); };
                let estimate = scaled(structural, pairs)?.map(|value| value.min(held));
                Ok(count(&name, &format!("c{at}:{what}"), unit, estimate, structural.to_u64().map(|value| value.min(held))))
            };
            columns.insert(format!("c{at}"), ev::ColumnEvidence {
                domain: ev::ScalarDomain::Unordered { limitation: "synthesized derived column: no scalar order is known".into() },
                distinct: fact("distinct", ev::Unit::Distinct, distinct, profiled(|column| column.distinct.as_ref()), &mut count)?,
                min: None, max: None,
                max_degree: fact("max_degree", ev::Unit::Degree, degree, profiled(|column| column.max_degree.as_ref()), &mut count)?,
                heavy: None, ranges: None,
            });
        }
        let recursive = if analysis.inferred.recursive(relation) {
            let first = *analysis.inferred.component(relation).first().expect("a component holds its relation");
            let (stratum, carried, capacity) = &rounds[&first];
            let Some(rounds) = count(stratum, "rounds", ev::Unit::Rounds, *carried, *capacity) else { continue; };
            Some(ev::RecursiveEvidence {
                stratum: format!("inferred:{stratum}"), final_rows: rows_fact.clone(), rounds,
                round_convention: ev::ROUND_CONVENTION.into(),
                run: ev::RunIdentity { program_hash: input.identity.program_hash.clone(), configuration_hash: input.identity.configuration_hash.clone(),
                    admission: ev::Admission::InsertOnlySet, run_id: "cq-cost synthesized; no run".into(), completed: false },
                seed_rows: None, unconsumed_final_delta_rows: None,
            })
        } else { None };
        let held = format!("inferred:{name}");
        if extended.relations.contains_key(&held) { return Err(error(format!("evidence already names a relation {held}"))); }
        if recursive.is_some() {
            let id = format!("{STRUCTURAL}{name}:delta_subset");
            extended.premises.push(ev::Premise::DeltaSubset { id: id.clone(), relations: vec![held.clone()], source: ev::Source::Declared, dataset_hash: dataset.clone(),
                provenance: ev::Provenance { producer: "cq-cost".into(), method: "set fixpoint: every delta is a subset of the final relation, and deltas are disjoint".into(), locator: id, run: None, artifact_hash: None } });
        }
        extended.relations.insert(held.clone(), ev::RelationEvidence {
            semantics: ev::RelationSemantics::Set,
            scope: if recursive.is_some() { ev::RelationScope::RecursiveFinal } else { ev::RelationScope::Frozen },
            columns, rows: recursive.is_none().then_some(rows_fact), key_sets: Vec::new(), keys: Vec::new(), recursive,
        });
        relations.insert(name, held);
    }
    Ok(Synthesized { evidence: extended, relations, facts })
}
