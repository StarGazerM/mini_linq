//! GPU publication storage and equality-key recipes selected before printing.
// DESIGN gpu-full-row-set
use crate::{Analysis, Monoid, Program, Term};
use cq_lang_structures::physical::symbol_name;
use std::collections::{BTreeMap, BTreeSet};

pub use super::bindings::ColumnBounds;

/// Bounds are a column's recorded provenance domain, or a scalar fold's value range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldBounds {
    /// Copied from `Bindings::bounds`; every packing of this field renders it.
    Column(ColumnBounds),
    Inclusive {
        min: i64,
        max: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyField {
    pub column: String,
    pub bounds: FieldBounds,
}

/// Minimum-base, range-bit-width fields, concatenated in the recorded order.
/// Empty domains use base/width zero. Fields and u64 lanes are low-first; even
/// a zero-bit key occupies one lane. No canonicalization provider is implied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyCodec {
    FrameOfReference { fields: Vec<KeyField> },
}

impl KeyCodec {
    pub fn fields(&self) -> &[KeyField] {
        match self {
            Self::FrameOfReference { fields } => fields,
        }
    }
}

/// Equality granularity of published packed keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Granularity {
    /// Candidate keys publish as produced, duplicates included.
    Rows,
    /// Keys are canonicalized by this recorded sort provider before publication.
    Canonical { provider: syn::Type },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedKeys {
    pub codec: KeyCodec,
    pub granularity: Granularity,
}

/// Slots use the shared carrier order, not just the live result heading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatRows {
    pub columns: Vec<String>,
    /// The empty carrier retains one sentinel slot.
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicationStorage {
    FlatRows(FlatRows),
    PackedKeys(PackedKeys),
}

impl PublicationStorage {
    pub fn codec(&self) -> Option<&KeyCodec> {
        match self {
            Self::FlatRows(_) => None,
            Self::PackedKeys(packed) => Some(&packed.codec),
        }
    }

    /// The recorded canonicalization provider; None publishes candidate rows.
    pub fn canonical_provider(&self) -> Option<&syn::Type> {
        match self {
            Self::PackedKeys(PackedKeys {
                granularity: Granularity::Canonical { provider },
                ..
            }) => Some(provider),
            _ => None,
        }
    }
}

/// One caller-owned arena field: a head column at its recorded device carrier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaColumn {
    pub column: String,
    /// Copied from `Bindings::carriers`; its `Scalar::COLUMN` fixes the width
    /// and signedness of the field.
    pub carrier: syn::Type,
}

/// The caller-owned arena encoding of a raw-candidate device terminal
/// (`gpu_datalog_runtime::Packing`): fields in recorded order, high-first;
/// signed fields sign-flipped; keys of at most two u64 lanes store their low
/// lane first, wider keys their high lane first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaRows {
    pub columns: Vec<ArenaColumn>,
}

/// Ordinary complete-key/unit-item set construction at a publication boundary.
/// The FOR codec owns field order and widths; these facts own construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetBuild {
    pub ratio: usize,
    pub radix_bits: u32,
    pub merge_tile_rows: u32,
    pub block_threads: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationRecord {
    pub storage: PublicationStorage,
    pub set_build: Option<SetBuild>,
    /// Host unions concatenate into this carrier before any recorded packing.
    pub union_staging: Option<FlatRows>,
    /// Present exactly for a raw-candidate terminal dispatch publication: the
    /// encoding a caller-owned arena translation publishes instead of `storage`.
    pub arena: Option<ArenaRows>,
}

/// Recorded Sequence facts shared by selection and checking. No storage
/// policy lives here.
struct Facts<'a> {
    analysis: &'a Analysis,
    definitions: BTreeMap<String, &'a Term>,
    /// The outermost store of each device dispatch.
    dispatches: BTreeSet<String>,
    /// Stores materialized at a boundary: dispatch publications and host stores.
    materialized: BTreeSet<String>,
    terminal: String,
    output: Vec<String>,
    carrier: Vec<String>,
}

impl<'a> Facts<'a> {
    fn new(program: &'a Program, analysis: &'a Analysis) -> syn::Result<Self> {
        let definitions: BTreeMap<_, _> = program
            .sequence
            .definitions
            .iter()
            .map(|definition| (symbol_name(&definition.name), &definition.term))
            .collect();
        let dispatches: BTreeSet<_> = analysis
            .dispatches
            .iter()
            .filter_map(|dispatch| {
                dispatch
                    .inner_stores
                    .iter()
                    .rev()
                    .find(|name| matches!(definitions.get(*name), Some(Term::Store { .. })))
                    .cloned()
            })
            .collect();
        let mut materialized = dispatches.clone();
        materialized.extend(definitions.iter().filter_map(|(name, term)| {
            (matches!(term, Term::Store { .. })
                && analysis
                    .levels
                    .get(name)
                    .is_some_and(|level| level.len() == 1))
            .then_some(name.clone())
        }));
        let terminal = analysis
            .output
            .as_ref()
            .map(|(name, _)| name.clone())
            .or_else(|| {
                program
                    .sequence
                    .definitions
                    .last()
                    .map(|d| symbol_name(&d.name))
            })
            .ok_or_else(|| error("missing terminal publication"))?;
        let output = program
            .structures
            .physical
            .program
            .head
            .args
            .iter()
            .map(|arg| {
                cq_lang_structures::syntax::rust::variable(arg)
                    .map(symbol_name)
                    .ok_or_else(|| error("output requires recorded code columns"))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let carrier = analysis
            .structures
            .physical
            .headings
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(Self {
            analysis,
            definitions,
            dispatches,
            materialized,
            terminal,
            output,
            carrier,
        })
    }

    fn term(&self, name: &str) -> syn::Result<&'a Term> {
        self.definitions
            .get(name)
            .copied()
            .ok_or_else(|| error("unknown publication"))
    }

    fn flat(&self) -> FlatRows {
        FlatRows {
            columns: self.carrier.clone(),
            width: self.carrier.len().max(1),
        }
    }

    /// GPU-Arena: the terminal dispatch publication of raw candidate rows (no
    /// group, scalar fold or canonical granularity) may publish into an arena.
    fn raw_terminal(&self, name: &str, term: &'a Term, storage: &PublicationStorage) -> bool {
        name == self.terminal
            && self.dispatches.contains(name)
            && !matches!(root(self, term), Term::Group(..))
            && root_fold(self, term).is_none()
            && storage.canonical_provider().is_none()
    }

    /// The provider of the canonical rows record of this publication's result.
    fn sort_provider(&self, publication: &str) -> Option<&'a syn::Type> {
        let analysis = self.analysis;
        analysis
            .obligations
            .iter()
            .find_map(|obligation| match obligation {
                crate::Obligation::Canon {
                    result,
                    canonical: Some(canonical),
                    ..
                } if result == publication => {
                    canonical.rows_provider().map(|provider| &provider.ty)
                }
                _ => None,
            })
            .or_else(|| {
                analysis
                    .results
                    .get(publication)
                    .and_then(|result| analysis.structures.canonical.get(result))
                    .and_then(|canonical| canonical.rows_provider())
                    .map(|provider| &provider.ty)
            })
    }
}

/// Storage selection policy. Used only by `derive`.
struct Context<'a> {
    facts: Facts<'a>,
}

impl<'a> Context<'a> {
    fn group(&self, term: &Term) -> bool {
        match term {
            Term::Group(..) => true,
            Term::Store { inner, .. } | Term::Canon(inner) => self.group(inner),
            Term::Ref(name) => self.group(self.facts.definitions[&symbol_name(name)]),
            _ => false,
        }
    }

    fn fold<'b>(&'b self, term: &'b Term) -> Option<&'b Monoid> {
        match term {
            Term::Fold(monoid, _) => Some(monoid),
            Term::Canon(inner) | Term::Flatten(inner) | Term::Store { inner, .. } => {
                self.fold(inner)
            }
            Term::Ref(name) => self.fold(self.facts.definitions[&symbol_name(name)]),
            _ => None,
        }
    }

    fn union(&self, term: &Term) -> Option<bool> {
        match term {
            Term::Union(_) => Some(false),
            Term::Store { inner, .. } => self.union(inner),
            Term::Canon(inner) => self.union(inner).map(|_| true),
            Term::Ref(name) => self.union(self.facts.definitions[&symbol_name(name)]),
            _ => None,
        }
    }

    /// A dispatch publication canonicalizes when every union/choice arm ends
    /// in Canon followed only by Flatten/Store, inside this dispatch.
    fn dispatch_canon(&self, term: &Term) -> bool {
        match term {
            Term::Canon(_) => true,
            Term::Store { inner, .. } | Term::Flatten(inner) => self.dispatch_canon(inner),
            Term::Ref(name) => {
                let name = symbol_name(name);
                !self.facts.materialized.contains(&name)
                    && self.dispatch_canon(self.facts.definitions[&name])
            }
            Term::Union(arms) => {
                !arms.is_empty() && arms.iter().all(|arm| self.dispatch_canon(arm))
            }
            Term::Choose(arms) => {
                !arms.is_empty() && arms.iter().all(|(_, arm)| self.dispatch_canon(arm))
            }
            _ => false,
        }
    }

    /// A host output transfer canonicalizes when its alias chain names a Canon.
    fn alias_canon(&self, term: &Term) -> bool {
        match term {
            Term::Canon(_) => true,
            Term::Store { inner, .. } => self.alias_canon(inner),
            Term::Ref(name) => self
                .facts
                .definitions
                .get(&symbol_name(name))
                .is_some_and(|term| self.alias_canon(term)),
            _ => false,
        }
    }

    fn host_union(&self, name: &str, term: &Term) -> Option<bool> {
        (matches!(term, Term::Store { .. })
            && !self.facts.dispatches.contains(name)
            && self
                .facts
                .analysis
                .levels
                .get(name)
                .is_some_and(|level| level.len() == 1))
        .then(|| self.union(term))
        .flatten()
    }

    fn canonical(&self, name: &str, term: &Term) -> bool {
        if self.facts.dispatches.contains(name) {
            self.dispatch_canon(term)
        } else if let Some(canonical) = self.host_union(name, term) {
            canonical
        } else {
            self.alias_canon(term)
        }
    }

    fn packed(&self, name: &str, term: &Term) -> bool {
        !self.group(term)
            && (name == self.facts.terminal
                || (self.facts.dispatches.contains(name) && self.dispatch_canon(term))
                || self.host_union(name, term) == Some(true))
    }

    fn columns(&self, name: &str, term: &Term) -> syn::Result<&[String]> {
        let facts = &self.facts;
        if name == facts.terminal {
            Ok(&facts.output)
        } else if self.fold(term).is_some() {
            if facts.output.len() != 1 {
                return Err(error("scalar fold publication requires one output column"));
            }
            Ok(&facts.output)
        } else {
            facts
                .analysis
                .results
                .get(name)
                .and_then(|result| facts.analysis.structures.physical.headings.get(result))
                .map(Vec::as_slice)
                .ok_or_else(|| error("publication lacks a checked row heading"))
        }
    }

    fn bounds(&self, column: &str, monoid: Option<&Monoid>) -> syn::Result<FieldBounds> {
        Ok(match monoid {
            None => FieldBounds::Column(
                self.facts
                    .analysis
                    .gpu_bindings
                    .bounds
                    .get(column)
                    .cloned()
                    .ok_or_else(|| {
                        error(&format!(
                            "packed column {column} lacks a recorded bounds source"
                        ))
                    })?,
            ),
            Some(Monoid::Count) => FieldBounds::Inclusive {
                min: 0,
                max: i64::MAX,
            },
            Some(Monoid::Or) => FieldBounds::Inclusive { min: 0, max: 1 },
            Some(Monoid::Sum(_)) => FieldBounds::Inclusive {
                min: i64::MIN,
                max: i64::MAX,
            },
        })
    }
}

/// Select the target storage once, during GPU grading and after GPU bindings,
/// including host-only terminal publications which have no device dispatch.
/// Formal rule GPU-PublicationStorage: upstream records own carrier layout,
/// ordered FOR recipes with their bounds sources, and equality granularity.
pub fn derive(
    program: &Program,
    analysis: &Analysis,
) -> syn::Result<BTreeMap<String, PublicationRecord>> {
    let context = Context {
        facts: Facts::new(program, analysis)?,
    };
    let facts = &context.facts;
    let mut records = BTreeMap::new();
    for definition in &program.sequence.definitions {
        let name = symbol_name(&definition.name);
        let term = &definition.term;
        if !matches!(term, Term::Store { .. }) && name != facts.terminal {
            continue;
        }
        let storage = if context.packed(&name, term) {
            let fold = context.fold(term);
            let fields = context
                .columns(&name, term)?
                .iter()
                .map(|column| {
                    Ok(KeyField {
                        column: column.clone(),
                        bounds: context.bounds(column, fold)?,
                    })
                })
                .collect::<syn::Result<_>>()?;
            let granularity = if context.canonical(&name, term) {
                Granularity::Canonical {
                    provider: facts.sort_provider(&name).cloned().ok_or_else(|| {
                        error("canonical publication requires its recorded sort provider")
                    })?,
                }
            } else {
                Granularity::Rows
            };
            PublicationStorage::PackedKeys(PackedKeys {
                codec: KeyCodec::FrameOfReference { fields },
                granularity,
            })
        } else {
            PublicationStorage::FlatRows(facts.flat())
        };
        let arena = facts
            .raw_terminal(&name, term, &storage)
            .then(|| -> syn::Result<ArenaRows> {
                let columns = facts
                    .output
                    .iter()
                    .map(|column| {
                        let carrier = analysis.gpu_bindings.carriers.get(column).cloned();
                        Ok(ArenaColumn {
                            column: column.clone(),
                            carrier: carrier.ok_or_else(|| {
                                error("arena field requires its recorded device carrier")
                            })?,
                        })
                    })
                    .collect::<syn::Result<_>>()?;
                Ok(ArenaRows { columns })
            })
            .transpose()?;
        let set_build = storage.canonical_provider().map(|_| SetBuild {
            ratio: 4,
            radix_bits: 0,
            merge_tile_rows: 1024,
            block_threads: 256,
        });
        records.insert(
            name.clone(),
            PublicationRecord {
                storage,
                set_build,
                union_staging: context.host_union(&name, term).map(|_| facts.flat()),
                arena,
            },
        );
    }
    Ok(records)
}

/// Canon on root paths through set-preserving wrappers (Store, Flatten, Canon,
/// Refs, union/choice arms): `(some path, every path)`. A Ref to a materialized
/// publication ends its path unless `crossing`.
fn root_canon(facts: &Facts, term: &Term, crossing: bool) -> (bool, bool) {
    fn arms<'t>(
        facts: &Facts,
        arms: impl Iterator<Item = &'t Term>,
        crossing: bool,
    ) -> (bool, bool) {
        arms.map(|arm| root_canon(facts, arm, crossing))
            .fold(None, |paths: Option<(bool, bool)>, (some, every)| {
                Some(paths.map_or((some, every), |(any, all)| (any || some, all && every)))
            })
            .unwrap_or((false, false))
    }
    match term {
        Term::Canon(_) => (true, true),
        Term::Store { inner, .. } | Term::Flatten(inner) => root_canon(facts, inner, crossing),
        Term::Ref(name) => {
            let name = symbol_name(name);
            match facts.definitions.get(&name) {
                Some(term) if crossing || !facts.materialized.contains(&name) => {
                    root_canon(facts, term, crossing)
                }
                _ => (false, false),
            }
        }
        Term::Union(terms) => arms(facts, terms.iter(), crossing),
        Term::Choose(terms) => arms(facts, terms.iter().map(|(_, arm)| arm), crossing),
        _ => (false, false),
    }
}

/// The node beneath Store/Canon wrappers and Refs, where a publication's
/// collective or union is realized.
fn root<'a>(facts: &Facts<'a>, term: &'a Term) -> &'a Term {
    match term {
        Term::Store { inner, .. } | Term::Canon(inner) => root(facts, inner),
        Term::Ref(name) => facts
            .definitions
            .get(&symbol_name(name))
            .copied()
            .map_or(term, |term| root(facts, term)),
        _ => term,
    }
}

/// Scalar fold beneath Store/Canon/Flatten wrappers and Refs.
fn root_fold<'a>(facts: &Facts<'a>, term: &'a Term) -> Option<&'a Monoid> {
    match term {
        Term::Fold(monoid, _) => Some(monoid),
        Term::Store { inner, .. } | Term::Canon(inner) | Term::Flatten(inner) => {
            root_fold(facts, inner)
        }
        Term::Ref(name) => facts
            .definitions
            .get(&symbol_name(name))
            .copied()
            .and_then(|term| root_fold(facts, term)),
        _ => None,
    }
}

/// Validate supplied storage, codec, and granularity records against stated
/// invariants over checked Sequence provenance and GPU bindings. The checker
/// never invokes storage selection; where several records are sound it
/// accepts each of them.
pub fn check(
    program: &Program,
    analysis: &Analysis,
    records: &BTreeMap<String, PublicationRecord>,
) -> syn::Result<()> {
    let facts = Facts::new(program, analysis)?;
    let publications: BTreeSet<_> = facts
        .definitions
        .iter()
        .filter(|(name, term)| matches!(term, Term::Store { .. }) || **name == facts.terminal)
        .map(|(name, _)| name)
        .collect();
    if !records.keys().eq(publications.iter().copied()) {
        return Err(error(
            "publication record coverage differs from Sequence stores",
        ));
    }
    let flat = facts.flat();
    for (name, record) in records {
        let term = facts.term(name)?;
        let terminal = name == &facts.terminal;
        let grouped = matches!(root(&facts, term), Term::Group(..));
        // Union staging exists exactly for host-level stores realized as a union.
        let host_union = matches!(term, Term::Store { .. })
            && !facts.dispatches.contains(name)
            && facts.materialized.contains(name)
            && matches!(root(&facts, term), Term::Union(_));
        match &record.union_staging {
            Some(staging) if host_union && staging == &flat => {}
            None if !host_union => {}
            _ => return Err(error("union staging contradicts publication provenance")),
        }
        // GPU-Arena: exactly a raw-candidate terminal dispatch records the
        // arena encoding: the output heading in order at recorded carriers.
        let raw_terminal = facts.raw_terminal(name, term, &record.storage);
        match &record.arena {
            None if !raw_terminal => {}
            Some(arena)
                if raw_terminal
                    && arena
                        .columns
                        .iter()
                        .map(|field| &field.column)
                        .eq(facts.output.iter())
                    && arena.columns.iter().all(|field| {
                        analysis.gpu_bindings.carriers.get(&field.column) == Some(&field.carrier)
                    }) => {}
            _ => {
                return Err(error(
                    "arena encoding contradicts the raw-candidate terminal",
                ));
            }
        }
        // A device-realized boundary owns any Canon on its own root paths.
        let device = terminal || host_union || facts.dispatches.contains(name);
        let (local_canon, _) = root_canon(&facts, term, false);
        if device && !grouped && local_canon && record.storage.canonical_provider().is_none() {
            return Err(error("publication drops its Canon boundary"));
        }
        // Formal rule GPU-RadixSet: check the recorded construction contract.
        // The consumed FOR codec, checked below, owns width provenance.
        match (&record.set_build, record.storage.canonical_provider()) {
            (None, None) => {}
            (Some(build), Some(_)) => {
                if build.ratio < 2
                    || build.radix_bits != 0
                    || build.merge_tile_rows == 0
                    || build.block_threads != 256
                {
                    return Err(error("set construction contradicts its runtime contract"));
                }
            }
            _ => {
                return Err(error(
                    "set construction coverage differs from canonical references",
                ));
            }
        }
        match &record.storage {
            PublicationStorage::FlatRows(rows) => {
                if rows != &flat {
                    return Err(error("flat storage contradicts carrier facts"));
                }
                if terminal && !grouped {
                    return Err(error(
                        "terminal output adapter requires recorded packed keys",
                    ));
                }
            }
            PublicationStorage::PackedKeys(PackedKeys { codec, granularity }) => {
                if grouped || !device {
                    return Err(error("packed storage has no device publication boundary"));
                }
                match granularity {
                    // Consumers of nonterminal packed keys read canonical owners.
                    Granularity::Rows if !terminal => {
                        return Err(error(
                            "nonterminal packed keys require a canonical publication",
                        ));
                    }
                    Granularity::Rows => {}
                    Granularity::Canonical { provider } => {
                        if root_canon(&facts, term, true) != (true, true) {
                            return Err(error(
                                "canonical granularity lacks a Canon on every root path",
                            ));
                        }
                        if facts.sort_provider(name) != Some(provider) {
                            return Err(error(
                                "canonical provider differs from the recorded sort strategy",
                            ));
                        }
                    }
                }
                check_codec(&facts, name, term, codec)?;
            }
        }
        if let Some(dispatch) = analysis.dispatches.iter().find(|dispatch| {
            dispatch
                .inner_stores
                .iter()
                .rev()
                .find(|name| matches!(facts.definitions.get(*name), Some(Term::Store { .. })))
                .is_some_and(|publication| publication == name)
        }) {
            if dispatch.gpu.as_ref().is_some_and(|execution| {
                matches!(
                    execution.publication,
                    super::Publication::DeferredRange { .. }
                )
            }) && record.storage.codec().is_some()
            {
                return Err(error("deferred ranges cannot publish packed row keys"));
            }
        }
    }
    Ok(())
}

/// Fields enumerate the observable heading once, in observable order: the
/// program output for terminal and scalar-fold publications, otherwise the
/// checked result heading. Column bounds equal the bound column record; fold
/// bounds contain every value of the fold.
fn check_codec<'a>(
    facts: &Facts<'a>,
    name: &str,
    term: &'a Term,
    codec: &KeyCodec,
) -> syn::Result<()> {
    let fold = root_fold(facts, term);
    let heading = if name == facts.terminal || fold.is_some() {
        Some(facts.output.as_slice())
    } else {
        facts
            .analysis
            .results
            .get(name)
            .and_then(|result| facts.analysis.structures.physical.headings.get(result))
            .map(Vec::as_slice)
    };
    let fields = codec.fields();
    if heading.is_none_or(|heading| !fields.iter().map(|field| &field.column).eq(heading.iter())) {
        return Err(error(
            "codec field order contradicts the publication heading",
        ));
    }
    for field in fields {
        let sound = match (&field.bounds, fold) {
            (FieldBounds::Column(bounds), None) => {
                facts.analysis.gpu_bindings.bounds.get(&field.column) == Some(bounds)
            }
            (FieldBounds::Inclusive { min, max }, Some(monoid)) => {
                let (low, high) = match monoid {
                    Monoid::Count => (0, i64::MAX),
                    Monoid::Or => (0, 1),
                    Monoid::Sum(_) => (i64::MIN, i64::MAX),
                };
                *min <= low && high <= *max
            }
            _ => false,
        };
        if !sound {
            return Err(error(
                "codec bounds contradict the column record or fold range",
            ));
        }
    }
    Ok(())
}

fn error(message: &str) -> syn::Error {
    syn::Error::new(
        proc_macro2::Span::call_site(),
        format!("GPU-PublicationStorage: {message}"),
    )
}
