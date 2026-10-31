//! Explicit Round rewrite `lanes(streams = K)`: record DL-WaveLanes.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, Program, schedule::WaveLanes};

/// Record `K` streams and, for every wave with more than one hole, its lanes:
/// holes in recorded wave order go round-robin to `min(K, holes)` lanes, so a
/// hole keeps its lane (and its arena's capacity hint) in every round. A hole
/// whose query reads the environment around `datalog!` cannot leave the
/// caller's thread and stays on lane 0. No size evidence orders holes.
pub fn compile_with(source: &Program, arguments: proc_macro2::TokenStream) -> syn::Result<Program> {
    struct Streams(usize);
    impl syn::parse::Parse for Streams {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let name: syn::Ident = input.parse()?;
            if name != "streams" {
                return Err(syn::Error::new_spanned(name, "expected streams = K"));
            }
            input.parse::<syn::Token![=]>()?;
            Ok(Self(input.parse::<syn::LitInt>()?.base10_parse()?))
        }
    }
    let Streams(streams) = syn::parse2(arguments)?;
    ensure(streams >= 1, &source.schedule, "lanes need at least one stream")?;
    round::contract::check(source)?;
    let mut target = source.clone();
    target.schedule.streams = streams;
    for (entry, stratum) in target.schedule.strata.iter_mut().zip(&source.context.strata) {
        entry.lanes = entry
            .waves
            .iter()
            .enumerate()
            .filter(|(_, wave)| streams > 1 && wave.holes.len() > 1)
            .map(|(index, wave)| {
                let rules = if wave.phase == round::schedule::SEED { &stratum.seeds } else { &stratum.steps };
                let count = streams.min(wave.holes.len());
                let mut lanes = vec![Vec::new(); count];
                let mut next = 0usize;
                for &hole in &wave.holes {
                    let rule = &rules[hole];
                    if crate::print::reads_environment(&source.holes[rule.hole].cq) {
                        lanes[0].push(hole);
                    } else {
                        lanes[next % count].push(hole);
                        next += 1;
                    }
                }
                lanes.retain(|lane| !lane.is_empty());
                WaveLanes { wave: index, lanes }
            })
            .filter(|entry| entry.lanes.len() > 1)
            .collect();
    }
    round::check_rewrite(source, &target)?;
    check(&target)?;
    Ok(target)
}

/// P-DL-Gpu lane admissibility: a hole on a worker lane (index above 0) runs
/// on another thread, so its query must not read the caller's environment.
pub fn check(program: &Program) -> syn::Result<()> {
    for (entry, stratum) in program.schedule.strata.iter().zip(&program.context.strata) {
        for lanes in &entry.lanes {
            let wave = &entry.waves[lanes.wave];
            let rules = if wave.phase == round::schedule::SEED { &stratum.seeds } else { &stratum.steps };
            for &hole in lanes.lanes.iter().skip(1).flatten() {
                ensure(
                    !crate::print::reads_environment(&program.holes[rules[hole].hole].cq),
                    lanes,
                    "a hole that reads the environment around datalog! must stay on lane 0",
                )?;
            }
        }
    }
    Ok(())
}
