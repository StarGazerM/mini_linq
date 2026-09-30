use crate::Result;
use cq_workloads::{adversarial, costs, extended_costs};

#[derive(Debug, PartialEq, Eq)]
struct Options {
    scale: usize,
    seed: u64,
}
impl Options {
    fn parse(arguments: Vec<String>) -> Result<Self> {
        let mut scale = None;
        let mut seed = None;
        let mut args = arguments.into_iter();
        while let Some(option) = args.next() {
            match option.as_str() {
                "--scale" if scale.is_none() => {
                    scale = Some(
                        args.next()
                            .ok_or("--scale requires an integer")?
                            .parse::<usize>()?,
                    );
                }
                "--seed" if seed.is_none() => {
                    seed = Some(
                        args.next()
                            .ok_or("--seed requires an integer")?
                            .parse::<u64>()?,
                    );
                }
                _ => return Err(format!("unknown or repeated bench option `{option}`").into()),
            }
        }
        let scale = scale.unwrap_or(128);
        // The largest scale uses four times N and host payloads use 3*key.
        if !(8..=i32::MAX as usize / 12).contains(&scale) {
            return Err("bench scale must be at least 8 and 12*scale must fit i32".into());
        }
        Ok(Self {
            scale,
            seed: seed.unwrap_or(adversarial::SEED),
        })
    }
}

pub(super) fn run(arguments: Vec<String>) -> Result<()> {
    let options = Options::parse(arguments)?;
    let start = std::time::Instant::now();
    let mut loaded_rows = 0;
    let mut loaded_cases = 0;
    for case in cq_workloads::catalog::all() {
        for &scenario in cq_workloads::execution::scenarios(case.name) {
            let config = crate::generation_config(case, scenario, cq_workloads::Scale::Tiny);
            let data = case.load(crate::data_directory(case, scenario, config.scale))?;
            data.validate_provenance(case.suite, case.name, config)?;
            loaded_rows += data.relations.iter().map(|r| r.rows.len()).sum::<usize>();
            loaded_cases += 1;
            std::hint::black_box(data);
        }
    }
    println!(
        "corpus CSV: {loaded_cases} cases, {loaded_rows} raw rows, {:.3} ms (generated tuple loading and manifest validation)",
        start.elapsed().as_secs_f64() * 1_000.0,
    );
    let observations: Vec<_> = [options.scale, options.scale * 2, options.scale * 4]
        .into_iter()
        .flat_map(|scale| costs::run(scale, options.seed))
        .collect();
    println!(
        "seed={} (setup and oracle excluded; query initialization included)",
        options.seed
    );
    print!("{}", costs::table(&observations));
    let extended: Vec<_> = [options.scale, options.scale * 2, options.scale * 4]
        .into_iter()
        .flat_map(extended_costs::run)
        .collect();
    print!("{}", extended_costs::table(&extended));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }
    #[test]
    fn bench_scales_and_seed_are_checked_before_execution() {
        assert_eq!(
            Options::parse(args(&["--scale", "24", "--seed", "7"])).unwrap(),
            Options { scale: 24, seed: 7 }
        );
        for values in [
            vec!["--scale"],
            vec!["--scale", "0"],
            vec!["--scale", "18446744073709551615"],
            vec!["--scale", "24", "--scale", "48"],
            vec!["--time-limit", "1"],
            vec!["--seed", "-1"],
        ] {
            assert!(Options::parse(args(&values)).is_err());
        }
    }
}
