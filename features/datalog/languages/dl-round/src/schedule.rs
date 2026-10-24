//! Recorded evaluation schedule of a Round program.
//!
//! A schedule records, per component, each rule hole's positive read set and
//! an ordered list of waves. Holes of one round read only the committed
//! snapshot, so the schedule changes no value (DL-WaveCommute): it records which
//! holes a printer may evaluate concurrently and after which wave a head's
//! admission may begin. Read sets are derived from the Storage `Read` records
//! and checked for equality; they are not a second source of truth.
//!
//! Only `synchronous` progress is admitted. The text form follows `record!`:
//! `progress { synchronous } strata [ { reads [ { phase { seed } index { 0 }
//! reads [ { relation { 1 } version { delta } } ] } ] waves [ { phase { step }
//! holes [ 0 1 ] } ] } ]`.
use crate::{Program, RoundContext, RuleHole, Stratum, storage};
use cq_datalog_support::ensure;
use cq_syntax::symbol_name;
use std::collections::BTreeSet;
use storage::record;
use syn::Ident;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub progress: Ident,
    pub strata: Vec<StratumSchedule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StratumSchedule {
    pub reads: Vec<HoleReads>,
    pub waves: Vec<Wave>,
}

/// The positive relational inputs of one rule hole, in relation order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoleReads {
    pub phase: Ident,
    pub index: usize,
    pub reads: Vec<Read>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub relation: usize,
    pub version: Ident,
}

/// Positions into the named phase's rule-hole list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave {
    pub phase: Ident,
    pub holes: Vec<usize>,
}

record!(Schedule {
    progress: Ident,
    strata: Vec<StratumSchedule>
});
record!(StratumSchedule {
    reads: Vec<HoleReads>,
    waves: Vec<Wave>
});
record!(HoleReads {
    phase: Ident,
    index: usize,
    reads: Vec<Read>
});
record!(Read {
    relation: usize,
    version: Ident
});
record!(Wave {
    phase: Ident,
    holes: Vec<usize>
});

pub const SEED: &str = "seed";
pub const STEP: &str = "step";
pub const SYNCHRONOUS: &str = "synchronous";

fn ident(name: &str) -> Ident {
    Ident::new(name, proc_macro2::Span::call_site())
}

impl Schedule {
    /// One wave per phase containing every hole: Fork's default.
    pub fn whole(storage: &storage::Program, analysis: &storage::Analysis, context: &RoundContext) -> syn::Result<Self> {
        Self::with_waves(storage, analysis, context, |stratum| {
            let mut waves = Vec::new();
            if !stratum.seeds.is_empty() {
                waves.push(Wave {
                    phase: ident(SEED),
                    holes: (0..stratum.seeds.len()).collect(),
                });
            }
            if !stratum.steps.is_empty() {
                waves.push(Wave {
                    phase: ident(STEP),
                    holes: (0..stratum.steps.len()).collect(),
                });
            }
            waves
        })
    }

    /// One hole per wave, in phase list order.
    pub fn per_hole(storage: &storage::Program, analysis: &storage::Analysis, context: &RoundContext) -> syn::Result<Self> {
        Self::with_waves(storage, analysis, context, |stratum| {
            (0..stratum.seeds.len())
                .map(|index| Wave { phase: ident(SEED), holes: vec![index] })
                .chain((0..stratum.steps.len()).map(|index| Wave { phase: ident(STEP), holes: vec![index] }))
                .collect()
        })
    }

    /// Holes grouped by head relation, waves in head order within each phase.
    pub fn by_head(storage: &storage::Program, analysis: &storage::Analysis, context: &RoundContext) -> syn::Result<Self> {
        Self::with_waves(storage, analysis, context, |stratum| {
            let group = |phase: &str, holes: &[RuleHole]| {
                let heads: BTreeSet<usize> = holes.iter().map(|hole| hole.head).collect();
                heads
                    .into_iter()
                    .map(|head| Wave {
                        phase: ident(phase),
                        holes: holes
                            .iter()
                            .enumerate()
                            .filter(|(_, hole)| hole.head == head)
                            .map(|(index, _)| index)
                            .collect(),
                    })
                    .collect::<Vec<_>>()
            };
            let mut waves = group(SEED, &stratum.seeds);
            waves.extend(group(STEP, &stratum.steps));
            waves
        })
    }

    /// Derive read sets and attach the supplied wave partition per component.
    pub fn with_waves(
        storage: &storage::Program,
        analysis: &storage::Analysis,
        context: &RoundContext,
        waves: impl Fn(&Stratum) -> Vec<Wave>,
    ) -> syn::Result<Self> {
        let strata = context
            .strata
            .iter()
            .map(|stratum| {
                Ok(StratumSchedule {
                    reads: derive_reads(storage, analysis, stratum)?,
                    waves: waves(stratum),
                })
            })
            .collect::<syn::Result<_>>()?;
        Ok(Self {
            progress: ident(SYNCHRONOUS),
            strata,
        })
    }

    /// Replace the waves of one component, keeping its derived read sets.
    pub fn with_stratum_waves(mut self, stratum: usize, waves: Vec<Wave>) -> Self {
        if let Some(entry) = self.strata.get_mut(stratum) {
            entry.waves = waves;
        }
        self
    }
}

/// Derive the positive read set of every rule hole of one component from the
/// Storage read records: a direct read names its delta relation, a delta
/// layout read its layout's relation at delta, a full layout read its layout's
/// relation at full. Pairs are deduplicated and ordered.
pub fn derive_reads(
    storage: &storage::Program,
    analysis: &storage::Analysis,
    stratum: &Stratum,
) -> syn::Result<Vec<HoleReads>> {
    let phase = |name: &str, holes: &[RuleHole]| {
        holes
            .iter()
            .enumerate()
            .map(|(index, hole)| {
                let mut pairs = BTreeSet::new();
                for read in storage.reads.iter().filter(|read| read.variant == hole.variant) {
                    let kind = read.kind.to_string();
                    let (relation, version) = match kind.as_str() {
                        "direct" => (read.index, "delta"),
                        "delta" | "full" => {
                            let layouts = if kind == "delta" {
                                &storage.delta_layouts
                            } else {
                                &storage.layouts
                            };
                            let layout = layouts.get(read.index).ok_or_else(|| {
                                syn::Error::new_spanned(&read.kind, "read names an absent layout")
                            })?;
                            let relation = *analysis
                                .positions
                                .get(&symbol_name(&layout.relation))
                                .ok_or_else(|| {
                                    syn::Error::new_spanned(&layout.relation, "layout relation is not declared")
                                })?;
                            (relation, if kind == "delta" { "delta" } else { "full" })
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(&read.kind, "unknown read kind"));
                        }
                    };
                    pairs.insert((relation, version));
                }
                Ok(HoleReads {
                    phase: ident(name),
                    index,
                    reads: pairs
                        .into_iter()
                        .map(|(relation, version)| Read {
                            relation,
                            version: ident(version),
                        })
                        .collect(),
                })
            })
            .collect::<syn::Result<Vec<_>>>()
    };
    let mut reads = phase(SEED, &stratum.seeds)?;
    reads.extend(phase(STEP, &stratum.steps)?);
    Ok(reads)
}

/// Formal rule WF-DL-Schedule: derived read sets, partitioning phase-ordered waves, synchronous progress.
pub fn check(program: &Program, analysis: &storage::Analysis) -> syn::Result<()> {
    let schedule = &program.schedule;
    ensure(
        schedule.progress == SYNCHRONOUS,
        &schedule.progress,
        "the only admitted schedule progress is `synchronous`",
    )?;
    ensure(
        schedule.strata.len() == program.context.strata.len(),
        schedule,
        "schedule must cover every component exactly once",
    )?;
    for (entry, stratum) in schedule.strata.iter().zip(&program.context.strata) {
        let derived = derive_reads(&program.storage, analysis, stratum)?;
        ensure(
            entry.reads == derived,
            entry,
            "recorded read sets differ from the sets derived from Storage reads",
        )?;
        check_waves(&entry.waves, stratum)?;
    }
    Ok(())
}

/// Waves partition each phase's holes exactly once; seed waves precede step waves.
pub fn check_waves(waves: &[Wave], stratum: &Stratum) -> syn::Result<()> {
    let mut seen_step = false;
    let mut seeds = BTreeSet::new();
    let mut steps = BTreeSet::new();
    for wave in waves {
        ensure(!wave.holes.is_empty(), wave, "a wave must name at least one hole")?;
        let (set, limit) = match wave.phase.to_string().as_str() {
            SEED => {
                ensure(!seen_step, wave, "seed waves must precede step waves")?;
                (&mut seeds, stratum.seeds.len())
            }
            STEP => {
                seen_step = true;
                (&mut steps, stratum.steps.len())
            }
            _ => return Err(syn::Error::new_spanned(&wave.phase, "wave phase must be seed or step")),
        };
        for &hole in &wave.holes {
            ensure(hole < limit, wave, "wave names a hole outside its phase")?;
            ensure(set.insert(hole), wave, "wave repeats a hole already scheduled")?;
        }
    }
    ensure(
        seeds.len() == stratum.seeds.len() && steps.len() == stratum.steps.len(),
        stratum,
        "waves must schedule every seed and step hole exactly once",
    )
}

/// Head relations whose holes are complete once wave `wave` has finished:
/// every hole of that head in the phase lies in waves `0..=wave`.
pub fn heads_completed_after(waves: &[Wave], stratum: &Stratum, wave: usize) -> Vec<usize> {
    let phase = waves[wave].phase.to_string();
    let holes: &[RuleHole] = if phase == SEED { &stratum.seeds } else { &stratum.steps };
    let scheduled: BTreeSet<usize> = waves
        .iter()
        .take(wave + 1)
        .filter(|w| w.phase == phase)
        .flat_map(|w| w.holes.iter().copied())
        .collect();
    let before: BTreeSet<usize> = waves
        .iter()
        .take(wave)
        .filter(|w| w.phase == phase)
        .flat_map(|w| w.holes.iter().copied())
        .collect();
    let heads: BTreeSet<usize> = waves[wave].holes.iter().map(|&h| holes[h].head).collect();
    heads
        .into_iter()
        .filter(|&head| {
            holes
                .iter()
                .enumerate()
                .filter(|(_, hole)| hole.head == head)
                .all(|(index, _)| scheduled.contains(&index))
                && !holes
                    .iter()
                    .enumerate()
                    .filter(|(_, hole)| hole.head == head)
                    .all(|(index, _)| before.contains(&index))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hole(head: usize) -> RuleHole {
        RuleHole {
            variant: 0,
            head,
            hole: 0,
            scan: None,
            insertion: ident("insert"),
        }
    }
    fn stratum(seed_heads: &[usize], step_heads: &[usize]) -> Stratum {
        Stratum {
            members: vec![0, 1],
            keys: Vec::new(),
            seeds: seed_heads.iter().map(|&h| hole(h)).collect(),
            steps: step_heads.iter().map(|&h| hole(h)).collect(),
            phases: vec![ident("evaluate"), ident("admit"), ident("publish")],
            publication: Vec::new(),
            termination: vec![0, 1],
            termination_test: ident("sum_delta_lengths"),
            observer: ident("before_step"),
            key_scope: ident("component"),
            loop_form: ident("while_nonempty"),
        }
    }
    fn wave(phase: &str, holes: &[usize]) -> Wave {
        Wave {
            phase: ident(phase),
            holes: holes.to_vec(),
        }
    }

    #[test]
    fn waves_partition_each_phase_in_order() {
        let s = stratum(&[0], &[0, 1, 0]);
        check_waves(&[wave(SEED, &[0]), wave(STEP, &[0, 1, 2])], &s).unwrap();
        check_waves(&[wave(SEED, &[0]), wave(STEP, &[2]), wave(STEP, &[0, 1])], &s).unwrap();
        let rejected = |waves: &[Wave]| check_waves(waves, &s).unwrap_err().to_string();
        assert!(rejected(&[wave(STEP, &[0, 1, 2]), wave(SEED, &[0])]).contains("precede"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[0, 1])]).contains("exactly once"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[0, 1, 2, 1])]).contains("repeats"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[0, 1, 3])]).contains("outside"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[]), wave(STEP, &[0, 1, 2])]).contains("at least one"));
        assert!(rejected(&[wave(SEED, &[0]), wave(ident("admit").to_string().as_str(), &[0, 1, 2])]).contains("seed or step"));
    }

    #[test]
    fn heads_complete_after_their_last_wave_only() {
        let s = stratum(&[], &[0, 1, 0, 2]);
        let waves = [wave(STEP, &[0]), wave(STEP, &[1, 2]), wave(STEP, &[3])];
        assert_eq!(heads_completed_after(&waves, &s, 0), Vec::<usize>::new());
        assert_eq!(heads_completed_after(&waves, &s, 1), vec![0, 1]);
        assert_eq!(heads_completed_after(&waves, &s, 2), vec![2]);
        let one = [wave(STEP, &[0, 1, 2, 3])];
        assert_eq!(heads_completed_after(&one, &s, 0), vec![0, 1, 2]);
    }

    #[test]
    fn schedule_text_roundtrips() {
        let schedule = Schedule {
            progress: ident(SYNCHRONOUS),
            strata: vec![StratumSchedule {
                reads: vec![HoleReads {
                    phase: ident(STEP),
                    index: 0,
                    reads: vec![
                        Read { relation: 1, version: ident("delta") },
                        Read { relation: 2, version: ident("full") },
                    ],
                }],
                waves: vec![wave(STEP, &[0])],
            }],
        };
        let text = quote::ToTokens::to_token_stream(&schedule);
        let parsed: Schedule = syn::parse2(text).unwrap();
        assert_eq!(parsed, schedule);
    }
}
