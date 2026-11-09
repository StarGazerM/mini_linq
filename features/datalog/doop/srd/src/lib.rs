//! Exact integer prepared-input interface for SRDatalog's DOOP query: the
//! field-flow formulation by default, or SRDatalog's default formulation with
//! the `default-query` feature (the program its default benchmark builds).
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

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

fn load_rows<const N: usize, T: Ord>(path: &Path, make: impl Fn([i32; N]) -> T) -> Result<Vec<T>> {
    let mut rows = Vec::new();
    for (line_no, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        let mut row = [0; N];
        let mut columns = line.split('\t');
        for value in &mut row {
            let column = columns.next().ok_or_else(|| {
                format!("{}:{}: expected {N} columns", path.display(), line_no + 1)
            })?;
            *value = column.parse().map_err(|error| {
                format!(
                    "{}:{}: invalid i32 {column:?}: {error}",
                    path.display(),
                    line_no + 1
                )
            })?;
        }
        if columns.next().is_some() {
            return Err(format!("{}:{}: expected {N} columns", path.display(), line_no + 1).into());
        }
        rows.push(make(row));
    }
    rows.sort_unstable();
    rows.dedup();
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
        let rows = load_rows(&path, |[left, right]| (left, right)).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(rows, [(1, 8), (1, 9), (2, 7)]);
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
