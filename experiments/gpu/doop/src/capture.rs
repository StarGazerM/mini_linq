//! Capture one subdirectory per hole/round; the manifest commits a completed
//! observation only after the generated kernel has returned its raw batch.
use gpu_datalog_runtime::{Result, capture::RowSource};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, env, fs, path::PathBuf};

struct Capture {
    root: PathBuf,
    min: usize,
    max: usize,
    label: Option<String>,
    holes: Option<Vec<usize>>,
    rounds: Option<Vec<usize>>,
    // (scan rows, iteration), smallest first. Ties keep the earlier round.
    kept: HashMap<usize, Vec<(usize, usize)>>,
}

impl Capture {
    fn from_env() -> Result<Option<Self>> {
        let Some(root) = env::var_os("JOIN_CAPTURE_DIR") else { return Ok(None) };
        let number = |key, default| -> Result<usize> {
            match env::var(key) {
                Ok(value) => value.parse().map_err(|e| format!("{key}: {e}").into()),
                Err(env::VarError::NotPresent) => Ok(default),
                Err(e) => Err(e.into()),
            }
        };
        let selection = |key| -> Result<Option<Vec<usize>>> {
            match env::var(key) {
                Ok(value) => value.split(',').map(|v| v.trim().parse()
                    .map_err(|e| format!("{key}: {e}").into())).collect::<Result<Vec<_>>>().map(Some),
                Err(env::VarError::NotPresent) => Ok(None),
                Err(e) => Err(e.into()),
            }
        };
        Ok(Some(Self {
            root: root.into(),
            min: number("JOIN_CAPTURE_MIN_ROWS", 1)?,
            max: number("JOIN_CAPTURE_MAX", 2)?,
            label: env::var("JOIN_CAPTURE_LABEL").ok(),
            holes: selection("JOIN_CAPTURE_HOLES")?,
            rounds: selection("JOIN_CAPTURE_ROUNDS")?,
            kept: HashMap::new(),
        }))
    }

    fn begin(&mut self, label: &str, hole: usize, iteration: usize,
             reads: &[&str], sources: &[&dyn RowSource]) -> Result<Option<(PathBuf, Value)>> {
        let scan = sources[0].len();
        if self.max == 0 || scan < self.min || self.label.as_deref().is_some_and(|l| l != label)
            || self.holes.as_ref().is_some_and(|h| !h.contains(&hole))
            || self.rounds.as_ref().is_some_and(|r| !r.contains(&iteration)) {
            return Ok(None);
        }
        let kept = self.kept.entry(hole).or_default();
        if kept.len() == self.max && kept[0].0 >= scan {
            return Ok(None);
        }
        let parent = self.root.join(format!("{label}__h{hole}"));
        if kept.is_empty() {
            fs::create_dir_all(&self.root)?;
            // Refuse to overwrite a previous run or mix two runs' snapshots.
            fs::create_dir(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let dir = parent.join(iteration.to_string());
        fs::create_dir(&dir)?;
        let mut inputs = Vec::with_capacity(sources.len());
        for (i, source) in sources.iter().enumerate() {
            source.write_u32_le(&mut fs::File::create(dir.join(format!("{i}.u32")))?)?;
            inputs.push(json!({"rows": source.len(), "runs": source.runs(), "columns": source.columns()}));
        }
        if kept.len() == self.max {
            let (_, old) = kept.remove(0);
            fs::remove_dir_all(parent.join(old.to_string()))?;
        }
        // On equal scan length the latest iteration is the first eviction.
        let pos = kept.partition_point(|&(rows, _)| rows < scan);
        kept.insert(pos, (scan, iteration));
        let manifest = json!({"label": label, "hole": hole, "iteration": iteration, "reads": reads, "inputs": inputs});
        Ok(Some((dir, manifest)))
    }
}

thread_local! {
    static CAPTURE: RefCell<Option<Option<Capture>>> = const { RefCell::new(None) };
}

pub(crate) fn observe(label: &str, hole: usize, iteration: usize, reads: &[&str], sources: &[&dyn RowSource])
    -> Result<impl FnOnce(usize) -> Result<()> + use<>>
{
    let pending = CAPTURE.with_borrow_mut(|slot| -> Result<_> {
        if slot.is_none() { *slot = Some(Capture::from_env()?); }
        match slot.as_mut().and_then(Option::as_mut) {
            Some(capture) => capture.begin(label, hole, iteration, reads, sources),
            None => Ok(None),
        }
    })?;
    Ok(move |candidates| {
        if let Some((dir, mut manifest)) = pending {
            manifest["candidates"] = json!(candidates);
            let tmp = dir.join("manifest.tmp");
            fs::write(&tmp, serde_json::to_vec_pretty(&manifest)?)?;
            fs::rename(tmp, dir.join("manifest.json"))?;
        }
        Ok(())
    })
}
