//! File acquisition belongs to this explicit front end, never a compiler printer.
use cq_cost::export::{self, Configuration, Target};
use std::collections::BTreeMap;
use cq_cost::artifacts::publish;
use std::path::Path;

const USAGE: &str = "cq-cost export --language cq|datalog --source INPUT --compiler-hash sha256:HEX --out plans.json\n  [--target cpu|cpu-par|gpu]                            (default cpu)\n  [--plan-options 'order(...)']                        (CPU CQ compilation only)\n  [--cq NORMAL.cq --vo CHECKED.vo]                      (consumed backend-neutral CQ)\n  [--round CHECKED.round --cq HOLE0.cq --vo HOLE0.vo ...] (consumed target Datalog)\n\nEach consumed valuation requires its separately recorded normalized CQ, in the\nsame hole order. Without checked records, compile the common CPU route once\nusing the production parsers/passes. Non-CPU targets require consumed records.\nCQ VO is backend-neutral; Datalog targets must agree with recorded device,\nbatch and parallel execution facts. No dataset is opened and no cost/advice is\ngenerated. Evidence is the source's separate declared_evidence attachment, not\nplan evidence. The compiler hash is the caller's build identity; retain\ninput_hash and full Identity for later check_against. A self-hash is not\nauthentication.";

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else { return Err(USAGE.into()); };
    if command == "--help" || command == "-h" {
        println!("{USAGE}\n\ncq-cost check --plans plans.json\ncq-cost advise --plans plans.json --evidence evidence.json --target cpu|cpu-par|gpu --out costs.json --advice orders.dl");
        #[cfg(feature = "planner-calibration")]
        println!("cq-cost review --plans plans.json --evidence facts.json --costs costs.json --report calibration.report.json --out review.json [--vo-out plans-review.json]\nReassess saved observations only; no workload execution or fan-out inference.");
        return Ok(());
    }
    if command == "check" {
        if args.next().as_deref() != Some("--plans") {
            return Err("usage: cq-cost check --plans plans.json".into());
        }
        let path = args.next().ok_or("missing --plans path")?;
        if args.next().is_some() {
            return Err("unexpected check option".into());
        }
        let record = export::parse_json(&std::fs::read_to_string(path)?)?;
        println!("{}", record.input_hash);
        return Ok(());
    }
    if command == "advise" { return advise(args); }
    #[cfg(feature = "planner-calibration")]
    if command == "review" { return review(args); }
    if command != "export" { return Err(format!("expected export, check or advise\n{USAGE}").into()); }
    let mut flags = BTreeMap::new();
    let mut vos = Vec::new();
    let mut cqs = Vec::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--language" | "--target" | "--source" | "--compiler-hash" | "--out" | "--plan-options" | "--round" | "--cq" | "--vo") {
            return Err(format!("unknown export option {flag}").into());
        }
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        if flag == "--vo" { vos.push(value); }
        else if flag == "--cq" { cqs.push(value); }
        else if flags.insert(flag.clone(), value).is_some() { return Err(format!("duplicate {flag}").into()); }
    }
    let required = |flag: &str| -> Result<&str> {
        flags.get(flag).map(String::as_str).ok_or_else(|| format!("missing {flag}\n{USAGE}").into())
    };
    let language = required("--language")?;
    let target = match flags.get("--target").map(String::as_str).unwrap_or("cpu") {
        "cpu" => Target::Cpu,
        "cpu-par" => Target::CpuPar,
        "gpu" => Target::Gpu,
        _ => return Err("--target must be cpu, cpu-par or gpu".into()),
    };
    let source = std::fs::read_to_string(required("--source")?)?;
    let compiler_hash = required("--compiler-hash")?.to_owned();
    let out = required("--out")?;
    let options_text = flags.get("--plan-options").map(String::as_str).unwrap_or("");
    if cqs.len() != vos.len() { return Err("each --vo requires one separately recorded --cq".into()); }
    let plans = cqs.iter().zip(&vos).map(|(cq, vo)| -> Result<_> {
        let normal = syn::parse_str(&std::fs::read_to_string(cq)?)?;
        let valuation = syn::parse::Parser::parse_str(
            |input: syn::parse::ParseStream| cq_lang_vo::Program::parse(input, &normal),
            &std::fs::read_to_string(vo)?,
        )?;
        Ok((normal, valuation))
    }).collect::<Result<Vec<_>>>()?;
    let consumed = !plans.is_empty() || flags.contains_key("--round");
    if consumed && flags.contains_key("--plan-options") {
        return Err("consumed records cannot also request a new plan".into());
    }
    let configuration = Configuration {
        compiler_hash, target,
        invocation: if consumed { format!("cq-cost export {language}: consumed checked records") }
            else { format!("cq-cost export {language}: production CPU default route; plan({options_text})") },
    };
    let record = match language {
        "cq" => {
            if flags.contains_key("--round") { return Err("--round is Datalog-only".into()); }
            if consumed {
                if plans.len() != 1 { return Err("CQ export requires exactly one --vo".into()); }
                export::export_cq(&source, &plans[0].0, &plans[0].1, configuration)?
            } else {
                let options = syn::parse_str(options_text)?;
                export::compile_cq(&source, &options, configuration)?
            }
        }
        "datalog" => {
            if flags.contains_key("--plan-options") { return Err("Datalog advice must be supplied in consumed checked records".into()); }
            if consumed {
                let path = required("--round")?;
                let round = syn::parse_str(&std::fs::read_to_string(path)?)?;
                export::export_datalog(&source, &round, &plans, configuration)?
            } else {
                export::compile_datalog(&source, configuration)?
            }
        }
        _ => return Err("--language must be cq or datalog".into()),
    };
    // Validate completely in memory before opening the output. Rename publishes
    // either the complete document or nothing, even after an interrupted write.
    let json = record.canonical_json()?;
    publish(Path::new(out), json.as_bytes())?;
    println!("{}", record.input_hash);
    Ok(())
}

fn advise(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut flags = BTreeMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--plans" | "--evidence" | "--target" | "--out" | "--advice" | "--expected-input-hash") {
            return Err(format!("unknown advise option {flag}").into());
        }
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        if flags.insert(flag.clone(), value).is_some() { return Err(format!("duplicate {flag}").into()); }
    }
    let required = |flag: &str| -> Result<&str> {
        flags.get(flag).map(String::as_str).ok_or_else(|| format!("missing {flag}").into())
    };
    // analyze establishes the complete compiler-record contract once.
    let input: export::PlannerInput = serde_json::from_str(&std::fs::read_to_string(required("--plans")?)?)?;
    if flags.get("--expected-input-hash").is_some_and(|hash| hash != &input.input_hash) {
        return Err("planner input differs from --expected-input-hash".into());
    }
    let evidence = cq_evidence::parse_bytes(&std::fs::read(required("--evidence")?)?)?;
    let output = Path::new(required("--out")?);
    let advice = Path::new(required("--advice")?);
    let identity = |path: &Path| -> Result<std::path::PathBuf> {
        if path.exists() { return Ok(path.canonicalize()?); }
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Ok(parent.canonicalize()?.join(path.file_name().ok_or("output must name a file")?))
    };
    let outputs = [identity(output)?, identity(advice)?];
    if outputs[0] == outputs[1] { return Err("report and advice need distinct output paths".into()); }
    for flag in ["--plans", "--evidence"] {
        if outputs.contains(&Path::new(required(flag)?).canonicalize()?) {
            return Err("advice output would overwrite an input".into());
        }
    }
    let report = cq_cost::cost::analyze(&input, &evidence, required("--target")?)?;
    let json = serde_json::to_vec_pretty(&report)?;
    publish(output, &json)?;
    publish(advice, report.advice.as_bytes())?;
    let changed = report.holes.iter().filter(|hole| hole.decision == cq_cost::cost::Decision::Changed).count();
    let unknown = report.holes.iter().filter(|hole| hole.decision == cq_cost::cost::Decision::Unknown).count();
    let crossing = report.holes.iter().filter(|hole| hole.decision == cq_cost::cost::Decision::Crossing).count();
    println!("dataset={} holes={} changed={changed} unknown={unknown} crossing={crossing}", report.dataset_hash, report.holes.len());
    Ok(())
}

#[cfg(feature = "planner-calibration")]
fn review(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut flags = BTreeMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--plans" | "--evidence" | "--costs" | "--report" | "--out" | "--vo-out") {
            return Err(format!("unknown review option {flag}").into());
        }
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        if flags.insert(flag.clone(), value).is_some() { return Err(format!("duplicate {flag}").into()); }
    }
    let required = |flag: &str| -> Result<&str> {
        flags.get(flag).map(String::as_str).ok_or_else(|| format!("missing {flag}").into())
    };
    let plans = export::parse_json(&std::fs::read_to_string(required("--plans")?)?)?;
    let evidence = cq_evidence::parse_bytes(&std::fs::read(required("--evidence")?)?)?;
    let cost_bytes = std::fs::read(required("--costs")?)?;
    let report_bytes = std::fs::read(required("--report")?)?;
    let costs: cq_cost::cost::AdviceReport = serde_json::from_slice(&cost_bytes)?;
    let report: cq_cost::calibration::CalibrationReport = serde_json::from_slice(&report_bytes)?;
    let reviews = cq_cost::calibration::assess_bounds(&report, &costs, &plans, &evidence)?;
    let valuations = flags.get("--vo-out").map(|_| review_valuations(&plans, &costs)).transpose()?;
    let record = serde_json::json!({
        "schema": "mini-linq.bound-review.v1", "input_hash": plans.input_hash,
        "dataset_hash": evidence.dataset.hash, "run": report.run,
        "source_cost_hash": export::sha256(&cost_bytes), "source_report_hash": export::sha256(&report_bytes),
        "bound_reviews": reviews,
        "provenance": "exact arithmetic over cited recorded bounds and same-scope operator observations; not a fan-out certificate or timing"
    });
    cq_cost::artifacts::publish_new(Path::new(required("--out")?), &serde_json::to_vec_pretty(&record)?)?;
    if let Some(valuations) = valuations {
        cq_cost::artifacts::publish_new(Path::new(required("--vo-out")?), &serde_json::to_vec_pretty(&valuations)?)?;
    }
    println!("reviewed={} source=recorded-calibration no_new_execution=true calibrated_fanout_recost=unknown", reviews.len());
    Ok(())
}

#[cfg(feature = "planner-calibration")]
fn review_valuations(input: &export::PlannerInput, costs: &cq_cost::cost::AdviceReport) -> Result<Vec<serde_json::Value>> {
    use quote::ToTokens;
    let directives: cq_cost::cost::advice::AdviceList = syn::parse_str(&costs.advice)?;
    let planned = cq_cost::cost::advice::check_datalog_advice(input, costs, &directives.0)?;
    let mut result = Vec::new();
    for choice in costs.holes.iter().filter(|choice| choice.decision == cq_cost::cost::Decision::Changed) {
        let export::Origin::Rule { variant, .. } = choice.origin else { continue; };
        let rule = planned.context.strata.iter().flat_map(|s| s.seeds.iter().chain(&s.steps))
            .find(|rule| rule.variant == variant).ok_or("review lost advised variant")?;
        let hole = &planned.holes[rule.hole];
        let pipeline = hole.invocation_pipeline()?;
        let at = hole.plan.ok_or("review hole lacks planning invocation")?;
        let options = syn::parse2(pipeline.passes[at].arguments.clone())?;
        let (normal, proposed) = cq_planning::prefix_valuation(&hole.cq, &options)?;
        cq_lang_vo::contract::check(&normal, &proposed)?;
        let old = input.holes.iter().find(|hole| hole.id == choice.hole).ok_or("review lost incumbent")?;
        let old_cq = syn::parse_str(&old.normalized_cq)?;
        let incumbent = syn::parse::Parser::parse_str(
            |input: syn::parse::ParseStream| cq_lang_vo::Program::parse(input, &old_cq), &old.checked_vo,
        )?;
        // Numeric attachments remain in the separate CQ/input evidence records;
        // valuation-only displays contain no attachment and preserve every step.
        result.push(serde_json::json!({
            "hole": choice.hole, "origin": choice.origin, "input_hash": input.input_hash,
            "numeric_attachment_omitted": true, "execution": "incumbent only; proposed VO is checked, not executed",
            "incumbent_vo": incumbent.to_token_stream().to_string(),
            "proposed_vo": proposed.to_token_stream().to_string()
        }));
    }
    Ok(result)
}


fn main() {
    if let Err(error) = run() {
        eprintln!("cq-cost: {error}");
        std::process::exit(1);
    }
}
