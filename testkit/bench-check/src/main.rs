use bench_check::{Mode, Policy, Random, References, RelationWitness, Row, Witness, check_witness, stable_seed};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

type Result<T, E = String> = std::result::Result<T, E>;
type SampleKey = (String, String, u64);

struct Server {
    references: BTreeMap<String, References>,
    checked: BTreeMap<(String, String), u64>,
    plans: BTreeMap<SampleKey, Policy>,
    seed: u64,
    sample_size: usize,
}

impl Default for Server {
    fn default() -> Self {
        Self { references: BTreeMap::new(), checked: BTreeMap::new(), plans: BTreeMap::new(), seed: 20261103, sample_size: 1000 }
    }
}

fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value.get(name).and_then(Value::as_str).ok_or_else(|| format!("missing string {name}"))
}
fn ordinal(value: &Value) -> Result<u64> {
    value.get("ordinal").and_then(Value::as_u64).ok_or_else(|| "missing unsigned ordinal".into())
}
fn files(value: &Value) -> Result<BTreeMap<String, String>> {
    serde_json::from_value(value.get("files").cloned().ok_or("missing files")?).map_err(|e| e.to_string())
}
fn read_json(path: &Path) -> Result<Value> {
    let input = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_reader(std::io::BufReader::new(input)).map_err(|e| format!("{}: {e}", path.display()))
}
fn read_witness(value: &Value) -> Result<Witness> {
    let value = if let Some(path) = value.get("path").and_then(Value::as_str) {
        read_json(Path::new(path))?
    } else { value.get("witness").cloned().ok_or("missing witness or path")? };
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// TSV exporters supply canonical distinct relations. Count records exactly;
/// parse only reservoir-selected rows on later samples. No Python tuple loop.
fn tsv_witness(path: &Path, relation: &str, policy: Policy) -> Result<RelationWitness> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut input = std::io::BufReader::new(file);
    let mut line = String::new();
    let mut count = 0usize;
    let mut rows = Vec::new();
    let mut random = Random::new(stable_seed(policy.seed, relation));
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 { break; }
        let row = line.trim_end_matches(['\r', '\n']);
        if row.is_empty() { continue; }
        count = count.checked_add(1).ok_or("relation count overflow")?;
        let selected = if policy.mode == Mode::Full || rows.len() < policy.sample_size {
            rows.len()
        } else { random.below(count) };
        if policy.mode == Mode::Full || selected < policy.sample_size {
            let row: Row = row.split('\t').map(str::to_owned).collect();
            if selected == rows.len() { rows.push(row); } else { rows[selected] = row; }
        }
    }
    Ok(RelationWitness { count, rows })
}

fn file_witnesses(value: &Value, policy: Policy) -> Result<Witness> {
    let format = value.get("format").and_then(Value::as_str).unwrap_or("tsv");
    files(value)?.into_iter().map(|(name, path)| {
        let witness = match format {
            "tsv" => tsv_witness(Path::new(&path), &name, policy)?,
            "json" => {
                let mut value = read_json(Path::new(&path))?;
                if value.is_object() { value = value.get_mut("rows").ok_or("JSON reference lacks rows")?.take(); }
                let rows: Vec<Row> = serde_json::from_value(value).map_err(|e| e.to_string())?;
                let count = rows.len();
                if policy.mode == Mode::Full { RelationWitness { count, rows } }
                else { bench_check::make_witness(&name, count, |i| rows[i].clone(), policy) }
            }
            _ => return Err(format!("unsupported tuple file format {format}")),
        };
        Ok((name, witness))
    }).collect()
}

fn index(witness: Witness) -> Result<References> {
    witness.into_iter().map(|(name, relation)| {
        if relation.count != relation.rows.len() {
            return Err(format!("{name}: reference witness must contain every tuple"));
        }
        let expected: BTreeSet<_> = relation.rows.into_iter().collect();
        if expected.len() != relation.count {
            return Err(format!("{name}: reference export has duplicate tuples"));
        }
        Ok((name, expected))
    }).collect()
}

impl Server {
    fn request(&mut self, request: Value) -> Result<Value> {
        let started = Instant::now();
        match text(&request, "op")? {
            "configure" => {
                if !self.references.is_empty() || !self.plans.is_empty() { return Err("configure must precede reference registration".into()); }
                self.seed = request.get("seed").and_then(Value::as_u64).ok_or("missing seed")?;
                self.sample_size = request.get("sample_size").and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok()).filter(|&n| n > 0).ok_or("sample_size must be positive")?;
                Ok(json!({"seed":self.seed,"sample_size":self.sample_size}))
            }
            "register" | "register_files" | "register_witness" => {
                let id = text(&request, "reference_id")?.to_owned();
                if self.references.contains_key(&id) { return Ok(json!({"cached":true,"reference_id":id})); }
                let witness = match text(&request, "op")? {
                    "register_files" => file_witnesses(&request, Policy::full())?,
                    "register_witness" => read_witness(&request)?,
                    _ => {
                        let rows: BTreeMap<String, Vec<Row>> = serde_json::from_value(request.get("relations").cloned().ok_or("missing relations")?).map_err(|e| e.to_string())?;
                        rows.into_iter().map(|(name, rows)| (name, RelationWitness { count: rows.len(), rows })).collect()
                    }
                };
                let reference = index(witness)?;
                let counts: BTreeMap<_, _> = reference.iter().map(|(name, rows)| (name.clone(), rows.len())).collect();
                self.references.insert(id.clone(), reference);
                Ok(json!({"cached":false,"reference_id":id,"counts":counts,"reference_seconds":started.elapsed().as_secs_f64()}))
            }
            "plan" => {
                let key = text(&request, "key")?.to_owned();
                let reference = text(&request, "reference_id")?.to_owned();
                if !self.references.contains_key(&reference) { return Err("reference is not registered".into()); }
                let sample = ordinal(&request)?;
                let identity = (key.clone(), reference.clone());
                let policy = Policy {
                    mode: if self.checked.contains_key(&identity) { Mode::Sampled } else { Mode::Full },
                    seed: match request.get("seed") {
                        Some(seed) => seed.as_u64().ok_or("seed must be an unsigned integer")?,
                        None => stable_seed(stable_seed(stable_seed(self.seed, &key), &reference), &sample.to_string()),
                    },
                    sample_size: self.sample_size,
                };
                let sample_key = (key, reference, sample);
                if self.plans.contains_key(&sample_key) {
                    return Err("sample already has a validation plan".into());
                }
                self.plans.insert(sample_key, policy);
                Ok(json!({"mode":policy.mode,"seed":policy.seed,"sample_size":policy.sample_size,"sample":sample}))
            }
            "check" | "check_files" => {
                let key = text(&request, "key")?.to_owned();
                let reference_id = text(&request, "reference_id")?.to_owned();
                let sample = ordinal(&request)?;
                let sample_key = (key.clone(), reference_id.clone(), sample);
                let policy = *self.plans.get(&sample_key).ok_or("sample has no validation plan")?;
                let witness = if text(&request, "op")? == "check_files" { file_witnesses(&request, policy)? } else { read_witness(&request)? };
                let reference = self.references.get(&reference_id).ok_or("reference is not registered")?;
                let evidence = check_witness(reference, &witness, policy)?;
                let first_full_sample = *self.checked.entry((key.clone(), reference_id.clone())).or_insert(sample);
                self.plans.remove(&sample_key);
                Ok(json!({"key":key,"reference_id":reference_id,"first_full_sample":first_full_sample,"mode":policy.mode,"seed":policy.seed,"sample":sample,"relations":evidence,"validation_seconds":started.elapsed().as_secs_f64()}))
            }
            op => Err(format!("unknown operation {op}")),
        }
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut output = std::io::BufWriter::new(stdout.lock());
    let mut server = Server::default();
    for line in stdin.lock().lines() {
        let response = match serde_json::from_str::<Value>(&line?) {
            Ok(request) => match server.request(request) {
                Ok(result) => json!({"ok":true,"result":result}),
                Err(error) => json!({"ok":false,"error":error}),
            },
            Err(error) => json!({"ok":false,"error":error.to_string()}),
        };
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        let mut server = Server::default();
        server.request(json!({"op":"register","reference_id":"data","relations":{"r":[["a"],["b"]]}})).unwrap();
        server
    }
    fn plan(server: &mut Server, key: &str, n: u64) -> Value {
        server.request(json!({"op":"plan","key":key,"reference_id":"data","ordinal":n})).unwrap()
    }
    fn check(server: &mut Server, key: &str, n: u64, rows: Value) -> Result<Value> {
        server.request(json!({"op":"check","key":key,"reference_id":"data","ordinal":n,"witness":{"r":{"count":2,"rows":rows}}}))
    }

    #[test]
    fn first_success_per_binary_config_is_full_and_failures_do_not_advance() {
        let mut server = server();
        assert_eq!(plan(&mut server,"a",0)["mode"],"full");
        assert!(check(&mut server,"a",0,json!([["a"],["x"]])).is_err());
        assert_eq!(plan(&mut server,"a",1)["mode"],"full");
        check(&mut server,"a",1,json!([["a"],["b"]])).unwrap();
        assert_eq!(plan(&mut server,"a",2)["mode"],"sampled");
        assert_eq!(plan(&mut server,"b",0)["mode"],"full");
        assert!(check(&mut server,"b",1,json!([["a"],["b"]])).is_err());
    }

    #[test]
    fn registering_existing_reference_does_not_reopen_source() {
        let mut server = server();
        let result = server.request(json!({"op":"register_files","reference_id":"data","files":{"r":"/not/a/file"}})).unwrap();
        assert_eq!(result["cached"],true);
        assert_eq!(server.references.len(),1);
    }

    #[test]
    fn relation_count_errors_and_missing_names_fail() {
        let mut server = server();
        plan(&mut server,"a",0);
        assert!(server.request(json!({"op":"check","key":"a","reference_id":"data","ordinal":0,"witness":{}})).is_err());
        assert!(server.request(json!({"op":"check","key":"a","reference_id":"data","ordinal":0,"witness":{"r":{"count":1,"rows":[["a"]]}}})).is_err());
    }
}
