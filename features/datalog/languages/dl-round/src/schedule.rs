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
    /// Per-hole realization advice; holes without an entry use printer defaults.
    pub policy: Vec<HolePolicy>,
    /// Derived: the wave after which each member's publication may begin.
    pub publication: Vec<PublishAfter>,
}

/// Realization advice for one hole: chunk grain and the driver length below
/// which the hole's own chunking is inlined. Never changes a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HolePolicy {
    pub phase: Ident,
    pub index: usize,
    pub grain: Option<usize>,
    pub inline_below: Option<usize>,
}

/// In `phase`, publication of `relation` may start once wave `wave` (a global
/// index into `waves`) has joined: every hole writing or reading the relation
/// in that phase lies in a wave at or before it (DL-PublishAfterReaders).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishAfter {
    pub phase: Ident,
    pub relation: usize,
    pub wave: usize,
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
    waves: Vec<Wave>,
    policy: Vec<HolePolicy>,
    publication: Vec<PublishAfter>
});
record!(HolePolicy {
    phase: Ident,
    index: usize,
    grain: Option<usize>,
    inline_below: Option<usize>
});
record!(PublishAfter {
    phase: Ident,
    relation: usize,
    wave: usize
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
    pub fn whole(
        storage: &storage::Program,
        analysis: &storage::Analysis,
        context: &RoundContext,
    ) -> syn::Result<Self> {
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
    pub fn per_hole(
        storage: &storage::Program,
        analysis: &storage::Analysis,
        context: &RoundContext,
    ) -> syn::Result<Self> {
        Self::with_waves(storage, analysis, context, |stratum| {
            (0..stratum.seeds.len())
                .map(|index| Wave {
                    phase: ident(SEED),
                    holes: vec![index],
                })
                .chain((0..stratum.steps.len()).map(|index| Wave {
                    phase: ident(STEP),
                    holes: vec![index],
                }))
                .collect()
        })
    }

    /// Holes grouped by head relation, waves in head order within each phase.
    pub fn by_head(
        storage: &storage::Program,
        analysis: &storage::Analysis,
        context: &RoundContext,
    ) -> syn::Result<Self> {
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
                let reads = derive_reads(storage, analysis, stratum)?;
                let waves = waves(stratum);
                let publication = derive_publication(&reads, &waves, stratum);
                Ok(StratumSchedule {
                    reads,
                    waves,
                    policy: Vec::new(),
                    publication,
                })
            })
            .collect::<syn::Result<_>>()?;
        Ok(Self {
            progress: ident(SYNCHRONOUS),
            strata,
        })
    }

    /// Replace the waves of one component, keeping its derived read sets and
    /// re-deriving its publication placement.
    pub fn with_stratum_waves(
        mut self,
        stratum: usize,
        waves: Vec<Wave>,
        context: &RoundContext,
    ) -> Self {
        if let (Some(entry), Some(members)) =
            (self.strata.get_mut(stratum), context.strata.get(stratum))
        {
            entry.publication = derive_publication(&entry.reads, &waves, members);
            entry.waves = waves;
        }
        self
    }

    /// Set or replace one hole's policy entry in one component.
    pub fn with_policy(mut self, stratum: usize, policy: HolePolicy) -> Self {
        if let Some(entry) = self.strata.get_mut(stratum) {
            entry
                .policy
                .retain(|p| !(p.phase == policy.phase && p.index == policy.index));
            entry.policy.push(policy);
            entry.policy.sort_by_key(|p| (p.phase != SEED, p.index));
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
                for read in storage
                    .reads
                    .iter()
                    .filter(|read| read.variant == hole.variant)
                {
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
                                    syn::Error::new_spanned(
                                        &layout.relation,
                                        "layout relation is not declared",
                                    )
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

/// Formal rule DL-PublishAfterReaders: a member's publication in a phase may start only after the last wave holding a hole that writes or reads it.
///
/// For every phase with at least one wave and every component member, the
/// recorded wave is the maximum global index of a wave of that phase containing
/// a hole whose head is the member or whose read set names the member (either
/// version); a member nobody touches in that phase gets the phase's last wave.
pub fn derive_publication(
    reads: &[HoleReads],
    waves: &[Wave],
    stratum: &Stratum,
) -> Vec<PublishAfter> {
    let mut out = Vec::new();
    for phase in [SEED, STEP] {
        let indexed: Vec<(usize, &Wave)> = waves
            .iter()
            .enumerate()
            .filter(|(_, wave)| wave.phase == phase)
            .collect();
        let Some(&(last, _)) = indexed.last() else {
            continue;
        };
        let holes: &[RuleHole] = if phase == SEED {
            &stratum.seeds
        } else {
            &stratum.steps
        };
        for &member in &stratum.members {
            let touches = |hole: usize| {
                holes.get(hole).is_some_and(|h| h.head == member)
                    || reads.iter().any(|r| {
                        r.phase == phase
                            && r.index == hole
                            && r.reads.iter().any(|read| read.relation == member)
                    })
            };
            let wave = indexed
                .iter()
                .filter(|(_, wave)| wave.holes.iter().any(|&hole| touches(hole)))
                .map(|(index, _)| *index)
                .max()
                .unwrap_or(last);
            out.push(PublishAfter {
                phase: ident(phase),
                relation: member,
                wave,
            });
        }
    }
    out
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
        let mut seen = BTreeSet::new();
        for policy in &entry.policy {
            let limit = match policy.phase.to_string().as_str() {
                SEED => stratum.seeds.len(),
                STEP => stratum.steps.len(),
                _ => {
                    return Err(syn::Error::new_spanned(
                        &policy.phase,
                        "policy phase must be seed or step",
                    ));
                }
            };
            ensure(
                policy.index < limit,
                policy,
                "policy names a hole outside its phase",
            )?;
            ensure(
                seen.insert((policy.phase.to_string(), policy.index)),
                policy,
                "policy repeats a hole",
            )?;
            ensure(
                policy.grain.is_none_or(|grain| grain > 0),
                policy,
                "policy grain must be positive",
            )?;
        }
        ensure(
            entry.publication == derive_publication(&entry.reads, &entry.waves, stratum),
            entry,
            "recorded publication placement differs from the last writer or reader wave of each member",
        )?;
    }
    Ok(())
}

/// Waves partition each phase's holes exactly once; seed waves precede step waves.
pub fn check_waves(waves: &[Wave], stratum: &Stratum) -> syn::Result<()> {
    let mut seen_step = false;
    let mut seeds = BTreeSet::new();
    let mut steps = BTreeSet::new();
    for wave in waves {
        ensure(
            !wave.holes.is_empty(),
            wave,
            "a wave must name at least one hole",
        )?;
        let (set, limit) = match wave.phase.to_string().as_str() {
            SEED => {
                ensure(!seen_step, wave, "seed waves must precede step waves")?;
                (&mut seeds, stratum.seeds.len())
            }
            STEP => {
                seen_step = true;
                (&mut steps, stratum.steps.len())
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    &wave.phase,
                    "wave phase must be seed or step",
                ));
            }
        };
        for &hole in &wave.holes {
            ensure(hole < limit, wave, "wave names a hole outside its phase")?;
            ensure(
                set.insert(hole),
                wave,
                "wave repeats a hole already scheduled",
            )?;
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
    let holes: &[RuleHole] = if phase == SEED {
        &stratum.seeds
    } else {
        &stratum.steps
    };
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
        check_waves(
            &[wave(SEED, &[0]), wave(STEP, &[2]), wave(STEP, &[0, 1])],
            &s,
        )
        .unwrap();
        let rejected = |waves: &[Wave]| check_waves(waves, &s).unwrap_err().to_string();
        assert!(rejected(&[wave(STEP, &[0, 1, 2]), wave(SEED, &[0])]).contains("precede"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[0, 1])]).contains("exactly once"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[0, 1, 2, 1])]).contains("repeats"));
        assert!(rejected(&[wave(SEED, &[0]), wave(STEP, &[0, 1, 3])]).contains("outside"));
        assert!(
            rejected(&[wave(SEED, &[0]), wave(STEP, &[]), wave(STEP, &[0, 1, 2])])
                .contains("at least one")
        );
        assert!(
            rejected(&[
                wave(SEED, &[0]),
                wave(ident("admit").to_string().as_str(), &[0, 1, 2])
            ])
            .contains("seed or step")
        );
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
                        Read {
                            relation: 1,
                            version: ident("delta"),
                        },
                        Read {
                            relation: 2,
                            version: ident("full"),
                        },
                    ],
                }],
                waves: vec![wave(STEP, &[0])],
                policy: vec![HolePolicy {
                    phase: ident(STEP),
                    index: 0,
                    grain: Some(128),
                    inline_below: None,
                }],
                publication: vec![PublishAfter {
                    phase: ident(STEP),
                    relation: 1,
                    wave: 0,
                }],
            }],
        };
        let text = quote::ToTokens::to_token_stream(&schedule);
        let parsed: Schedule = syn::parse2(text).unwrap();
        assert_eq!(parsed, schedule);
    }

    #[test]
    fn publication_follows_last_writer_or_reader_wave() {
        // steps: hole0 writes A(0) reads B; hole1 writes B(1) reads A; hole2 writes A(0) reads nothing of A/B.
        let s = stratum(&[], &[0, 1, 0]);
        let reads = vec![
            HoleReads {
                phase: ident(STEP),
                index: 0,
                reads: vec![Read {
                    relation: 1,
                    version: ident("delta"),
                }],
            },
            HoleReads {
                phase: ident(STEP),
                index: 1,
                reads: vec![Read {
                    relation: 0,
                    version: ident("full"),
                }],
            },
            HoleReads {
                phase: ident(STEP),
                index: 2,
                reads: vec![Read {
                    relation: 5,
                    version: ident("full"),
                }],
            },
        ];
        let waves = [wave(STEP, &[1]), wave(STEP, &[0]), wave(STEP, &[2])];
        let derived = derive_publication(&reads, &waves, &s);
        // A(0): written by holes 0 (wave 1) and 2 (wave 2), read by hole 1 (wave 0) -> wave 2.
        // B(1): written by hole 1 (wave 0), read by hole 0 (wave 1) -> wave 1.
        assert_eq!(
            derived,
            vec![
                PublishAfter {
                    phase: ident(STEP),
                    relation: 0,
                    wave: 2
                },
                PublishAfter {
                    phase: ident(STEP),
                    relation: 1,
                    wave: 1
                },
            ]
        );
        // No seed waves: no seed entries.
        assert!(derived.iter().all(|p| p.phase == STEP));
    }
}
