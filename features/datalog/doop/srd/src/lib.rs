//! Exact integer prepared-input interface for SRDatalog's DOOP query: the
//! field-flow formulation by default, or SRDatalog's default formulation with
//! the `default-query` feature (the program its default benchmark builds).
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

#[path = "../../shared/src/manifest.rs"]
mod manifest;
use manifest::{CertifiedReader, Manifest};

#[cfg(not(feature = "default-query"))]
mod generated;
#[cfg(feature = "default-query")]
#[path = "generated_default.rs"]
mod generated;
pub use generated::{SrdConstants, SrdFacts, SrdResults};

// The fixture asserts the 43 field-flow outputs.
#[cfg(all(feature = "test-fixture", feature = "default-query"))]
compile_error!("test-fixture covers only the field-flow query");
#[cfg(feature = "test-fixture")]
pub mod fixture;

impl SrdResults {
    /// Prepared integers are the shared SRD semantic domain, not process-local
    /// interner IDs. Deduplicate before exact logical counts and actual sampling.
    pub fn witness(&mut self, policy: bench_check::Policy) -> bench_check::Witness {
        let mut witness = bench_check::Witness::new();
        macro_rules! relation {
            ($name:literal, $field:ident, $($column:tt),+) => {{
                self.$field.sort_unstable();
                self.$field.dedup();
                witness.insert($name.into(), bench_check::make_witness(
                    $name,
                    self.$field.len(),
                    |index| {
                        let row = &self.$field[index];
                        vec![$(row.$column.to_string()),+]
                    },
                    policy,
                ));
            }};
        }
        relation!("isType", r_istype, 0);
        relation!("isReferenceType", r_isreferencetype, 0);
        relation!("isClassType_IDB", r_isclasstype_idb, 0);
        relation!("isArrayType_IDB", r_isarraytype_idb, 0);
        relation!("isInterfaceType_IDB", r_isinterfacetype_idb, 0);
        relation!("DirectSubclass", r_directsubclass, 0, 1);
        relation!("Superinterface", r_superinterface, 0, 1);
        relation!("MethodImplemented", r_methodimplemented, 0, 1, 2, 3);
        relation!("MainMethodDeclaration", r_mainmethoddeclaration, 0);
        relation!("MethodInvocation_Base", r_methodinvocation_base, 0, 1);
        relation!("Subclass", r_subclass, 0, 1);
        relation!("Superclass", r_superclass, 0, 1);
        relation!("SubtypeOf", r_subtypeof, 0, 1);
        relation!("SupertypeOf", r_supertypeof, 0, 1);
        relation!("SubtypeOfDifferent", r_subtypeofdifferent, 0, 1);
        relation!("MethodLookup", r_methodlookup, 0, 1, 2, 3);
        relation!("ClassInitializer", r_classinitializer, 0, 1);
        relation!("CastTo", r_castto, 0, 1, 2, 3);
        relation!("IsCastableToString", r_iscastabletostring, 0);
        relation!("IsStringHeap", r_isstringheap, 0);
        relation!("HeapHelper", r_heaphelper, 0, 1, 2, 3, 4);
        relation!("HeapHelperNoThis", r_heaphelpernothis, 0, 1, 2, 3);
        relation!("VirtualMethodInvocation", r_virtualmethodinvocation, 0, 1, 2, 3);
        relation!("NonStringHeapAllocSuperType", r_nonstringheapallocsupertype, 0, 1);
        relation!("IsStringArrayHeap", r_isstringarrayheap, 0);
        relation!("EligibleStringArrayHeap", r_eligiblestringarrayheap, 0);
        relation!("IsObjectArrayHeap", r_isobjectarrayheap, 0);
        relation!("EligibleObjectArrayHeap", r_eligibleobjectarrayheap, 0);
        relation!("ArrayTypeCompat", r_arraytypecompat, 0, 1);
        relation!("InitializedClass", r_initializedclass, 0);
        relation!("Reachable", r_reachable, 0);
        relation!("CallGraphEdge", r_callgraphedge, 0, 1);
        relation!("Assign", r_assign, 0, 1);
        relation!("VarPointsTo", r_varpointsto, 0, 1);
        relation!("ReachableLoadInstanceField", r_reachableloadinstancefield, 0, 1, 2);
        relation!("InstanceFieldPointsTo", r_instancefieldpointsto, 0, 1, 2);
        relation!("StaticFieldPointsTo", r_staticfieldpointsto, 0, 1);
        relation!("ArrayIndexPointsTo", r_arrayindexpointsto, 0, 1);
        relation!("ReachableInstruction", r_reachableinstruction, 0);
        relation!("ReachableSortedIndex", r_reachablesortedindex, 0, 1);
        #[cfg(not(feature = "default-query"))]
        {
            relation!("ReachableStoreInstanceField", r_reachablestoreinstancefield, 0, 1, 2);
            relation!("StoreObjFrom", r_storeobjfrom, 0, 1, 2);
            relation!("FieldFlow", r_fieldflow, 0, 1);
        }
        witness
    }
}

/// Only complete integer tuples implement this private decoder conversion.
trait IntegerRow<const N: usize>: Ord {
    fn from_row(row: [i32; N]) -> Self;
}

macro_rules! integer_row {
    ($n:literal; $($index:tt => $name:ident),+) => {
        impl IntegerRow<$n> for ($(integer_row!(@type $name),)+) {
            fn from_row(row: [i32; $n]) -> Self { ($(row[$index],)+) }
        }
    };
    (@type $name:ident) => { i32 };
}
integer_row!(1; 0 => a);
integer_row!(2; 0 => a, 1 => b);
integer_row!(3; 0 => a, 1 => b, 2 => c);
integer_row!(4; 0 => a, 1 => b, 2 => c, 3 => d);

fn load_rows<const N: usize, T: IntegerRow<N>>(path: &Path, manifest: &Manifest) -> Result<Vec<T>> {
    let mut rows = Vec::new();
    let projection: [usize; N] = std::array::from_fn(|i| i);
    let hash = manifest.expected_hash(path, &["i32"; N], &projection);
    let mut reader = BufReader::new(CertifiedReader::new(File::open(path)?, hash));
    let mut line = String::new();
    let mut line_no = 0;
    while {
        line.clear();
        reader.read_line(&mut line)? != 0
    } {
        line_no += 1;
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let mut row = [0; N];
        let mut columns = line.split('\t');
        for value in &mut row {
            let column = columns.next().ok_or_else(|| {
                format!("{}:{line_no}: expected {N} columns", path.display())
            })?;
            *value = column.parse().map_err(|error| {
                format!(
                    "{}:{}: invalid i32 {column:?}: {error}",
                    path.display(),
                    line_no
                )
            })?;
        }
        if columns.next().is_some() {
            return Err(format!("{}:{line_no}: expected {N} columns", path.display()).into());
        }
        rows.push(T::from_row(row));
    }
    if !reader.into_inner().verified() {
        rows.sort_unstable();
        rows.dedup();
    }
    Ok(rows)
}

fn export_rows<const N: usize>(path: &Path, rows: impl Iterator<Item = [i32; N]>) -> Result<()> {
    let mut output = BufWriter::new(File::create(path)?);
    for row in rows {
        for (i, value) in row.iter().enumerate() {
            if i != 0 {
                write!(output, "\t")?;
            }
            write!(output, "{value}")?;
        }
        writeln!(output)?;
    }
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod witness_tests {
    use super::*;
    use bench_check::{Mode, Policy};

    #[test]
    fn prepared_input_loading_preserves_full_row_support() {
        let path = std::env::temp_dir().join(format!("srd-set-input-{}.csv", std::process::id()));
        std::fs::write(&path, "2\t7\n1\t9\n2\t7\n1\t8\n").unwrap();
        let rows = load_rows::<2, (i32, i32)>(&path, &Manifest::default()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(rows, [(1, 8), (1, 9), (2, 7)]);
    }

    #[test]
    fn prepared_manifest_override_and_stale_hash_fallback() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("srd-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let schema: serde_json::Value = serde_json::from_str(include_str!("generated.schema.json")).unwrap();
        for input in schema["inputs"].as_object().unwrap().values() {
            std::fs::write(root.join(input["file"].as_str().unwrap()), b"").unwrap();
        }
        let meta: std::collections::BTreeMap<_, _> = schema["metadata_keys"].as_array().unwrap()
            .iter().map(|key| (key.as_str().unwrap(), 0)).collect();
        std::fs::write(root.join("meta.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        let bytes = b"2\t7\r\n\r\n1\t9\r\n";
        std::fs::write(root.join("DirectSuperclass.csv"), bytes).unwrap();
        let manifest_path = root.join("override.json");
        let manifest = serde_json::json!({
            "version": 1, "relations": {"DirectSuperclass.csv": {
                "types": ["i32", "i32"],
                "verified_keys": [{"columns": [0], "set": "verified",
                    "sha256": format!("{:x}", Sha256::digest(bytes))}]
            }}
        });
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(SrdFacts::load_with_manifest(&root, None).unwrap().r_directsuperclass,
            [(1, 9), (2, 7)]);
        assert_eq!(SrdFacts::load_with_manifest(&root, Some(&manifest_path)).unwrap().r_directsuperclass,
            [(2, 7), (1, 9)]);
        std::fs::write(root.join("DirectSuperclass.csv"), b"2\t7\n1\t9\n+2\t7\n").unwrap();
        assert_eq!(SrdFacts::load_with_manifest(&root, Some(&manifest_path)).unwrap().r_directsuperclass,
            [(1, 9), (2, 7)]);
        std::fs::write(&manifest_path, b"{}").unwrap();
        assert_eq!(SrdFacts::load_with_manifest(&root, Some(&manifest_path)).unwrap().r_directsuperclass,
            [(1, 9), (2, 7)]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn witnesses_preserve_schema_and_exact_logical_integer_rows() {
        let mut result = SrdResults::default();
        result.r_varpointsto = vec![(-7, 11), (-7, 11)];
        let witness = result.witness(Policy::full());
        assert_eq!(witness.len(), if cfg!(feature = "default-query") { 40 } else { 43 });
        assert_eq!(witness["VarPointsTo"].count, 1);
        assert_eq!(witness["VarPointsTo"].rows, vec![vec!["-7".to_string(), "11".to_string()]]);
        assert_eq!(witness["Reachable"].count, 0);
        assert!(witness["Reachable"].rows.is_empty());
        assert_eq!(witness.contains_key("FieldFlow"), !cfg!(feature = "default-query"));
    }

    #[test]
    fn sampled_integer_rows_are_actual_and_reproducible() {
        let mut result = SrdResults::default();
        result.r_reachable = (0..1501).map(|value| (value,)).collect();
        result.r_reachable.push((1,));
        let policy = Policy { mode: Mode::Sampled, seed: 29, sample_size: 1000 };
        let witness = result.witness(policy);
        let relation = &witness["Reachable"];
        assert_eq!(relation.count, 1501);
        assert_eq!(relation.rows.len(), 1000);
        assert_eq!(relation.rows.iter().collect::<std::collections::BTreeSet<_>>().len(), 1000);
        assert!(relation.rows.iter().all(|row| row.len() == 1
            && row[0].parse::<i32>().is_ok_and(|value| (0..1501).contains(&value))));
        assert_eq!(relation.rows, result.witness(policy)["Reachable"].rows);
    }

    #[cfg(feature = "planner-calibration")]
    #[test]
    fn calibration_observes_the_same_raw_metadata_bytes_once() {
        use cq_storage::counters::calibration::{Observer, with_observer};
        use std::sync::{Arc, Mutex};
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        #[derive(Default)]
        struct Metadata(Mutex<Vec<(String, Vec<u8>)>>);
        impl Observer for Metadata {
            fn metadata(&self, path: &str, bytes: &[u8]) {
                self.0.lock().unwrap().push((path.into(), bytes.into()));
            }
        }
        let root = Directory(std::env::temp_dir().join(format!("srd-calibration-metadata-{}", std::process::id())));
        std::fs::create_dir(&root.0).unwrap();
        let schema: serde_json::Value = serde_json::from_str(include_str!("generated.schema.json")).unwrap();
        for input in schema["inputs"].as_object().unwrap().values() {
            std::fs::write(root.0.join(input["file"].as_str().unwrap()), b"").unwrap();
        }
        let constants = schema["metadata_keys"].as_array().unwrap().iter().enumerate()
            .map(|(index, key)| (key.as_str().unwrap().to_owned(), index as i32))
            .collect::<std::collections::BTreeMap<_, _>>();
        let bytes = serde_json::to_vec_pretty(&constants).unwrap();
        std::fs::write(root.0.join("meta.json"), &bytes).unwrap();
        let observed = Arc::new(Metadata::default());
        let facts = with_observer(observed.clone(), || SrdFacts::load(&root.0)).unwrap();
        assert_eq!(facts.constants.c_public, constants["public"]);
        assert_eq!(*observed.0.lock().unwrap(), vec![("meta.json".into(), bytes)]);
    }
}
