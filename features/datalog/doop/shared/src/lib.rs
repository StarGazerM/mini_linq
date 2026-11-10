//! Shared DOOP facts, full-tuple results, engine generators and benchmark harness.

#![recursion_limit = "1024"]

pub mod ascent_engine;
pub mod harness;
pub mod interner;
mod loader;
mod manifest;
pub mod mini_engine;

pub use interner::IStr;
use std::{io, path::Path};

/// The 32 raw input relations and dataset-precomputed method descriptors.
#[derive(Default)]
pub struct Facts {
    pub directsuperclass: Vec<(IStr, IStr)>,
    pub directsuperinterface: Vec<(IStr, IStr)>,
    pub mainclass: Vec<(IStr,)>,
    pub formalparam: Vec<(i32, IStr, IStr)>,
    pub componenttype: Vec<(IStr, IStr)>,
    pub assignreturnvalue: Vec<(IStr, IStr)>,
    pub actualparam: Vec<(i32, IStr, IStr)>,
    pub method_modifier: Vec<(IStr, IStr)>,
    pub var_type: Vec<(IStr, IStr)>,
    pub _classtype: Vec<(IStr,)>,
    pub _arraytype: Vec<(IStr,)>,
    pub _interfacetype: Vec<(IStr,)>,
    pub _var_declaringmethod: Vec<(IStr, IStr)>,
    pub _applicationclass: Vec<(IStr,)>,
    pub _thisvar: Vec<(IStr, IStr)>,
    pub _normalheap: Vec<(IStr, IStr)>,
    pub _stringconstant: Vec<(IStr,)>,
    pub _assignheapallocation: Vec<(IStr, i32, IStr, IStr, IStr, i32)>,
    pub _assignlocal: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _assigncast: Vec<(IStr, i32, IStr, IStr, IStr, IStr)>,
    pub _field: Vec<(IStr, IStr, IStr, IStr)>,
    pub _staticmethodinvocation: Vec<(IStr, i32, IStr, IStr)>,
    pub _specialmethodinvocation: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _virtualmethodinvocation: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _method: Vec<(IStr, IStr, IStr, IStr, IStr, IStr, i32)>,
    pub method_descriptor: Vec<(IStr, IStr)>,
    pub _storeinstancefield: Vec<(IStr, i32, IStr, IStr, IStr, IStr)>,
    pub _loadinstancefield: Vec<(IStr, i32, IStr, IStr, IStr, IStr)>,
    pub _storestaticfield: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _loadstaticfield: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _storearrayindex: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _loadarrayindex: Vec<(IStr, i32, IStr, IStr, IStr)>,
    pub _return: Vec<(IStr, i32, IStr, IStr)>,
}

impl Facts {
    pub fn load(directory: &Path) -> io::Result<Self> {
        let manifest = std::env::var_os("DOOP_SET_MANIFEST").map(std::path::PathBuf::from);
        Self::load_with_manifest(directory, manifest.as_deref())
    }

    /// Load with an explicit certificate path, or discover doop-set-manifest.json.
    /// Invalid or absent certificates retain full-row deduplication.
    pub fn load_with_manifest(directory: &Path, path: Option<&Path>) -> io::Result<Self> {
        let manifest = manifest::Manifest::load(directory, path);
        macro_rules! load {
            ($name:literal) => { loader::load(&directory.join($name), &manifest) };
        }
        let methods = load!("Method.facts")?;
        let method_descriptor = loader::method_descriptors(&methods);
        Ok(Self {
            directsuperclass: load!("DirectSuperclass.facts")?,
            directsuperinterface: load!("DirectSuperinterface.facts")?,
            mainclass: load!("MainClass.facts")?,
            formalparam: load!("FormalParam.facts")?,
            componenttype: load!("ComponentType.facts")?,
            assignreturnvalue: load!("AssignReturnValue.facts")?,
            actualparam: load!("ActualParam.facts")?,
            method_modifier: load!("Method-Modifier.facts")?,
            var_type: load!("Var-Type.facts")?,
            _classtype: load!("ClassType.facts")?,
            _arraytype: load!("ArrayType.facts")?,
            _interfacetype: load!("InterfaceType.facts")?,
            _var_declaringmethod: load!("Var-DeclaringMethod.facts")?,
            _applicationclass: load!("ApplicationClass.facts")?,
            _thisvar: load!("ThisVar.facts")?,
            _normalheap: load!("NormalHeap.facts")?,
            _stringconstant: load!("StringConstant.facts")?,
            _assignheapallocation: load!("AssignHeapAllocation.facts")?,
            _assignlocal: load!("AssignLocal.facts")?,
            _assigncast: load!("AssignCast.facts")?,
            _field: load!("Field.facts")?,
            _staticmethodinvocation: load!("StaticMethodInvocation.facts")?,
            _specialmethodinvocation: load!("SpecialMethodInvocation.facts")?,
            _virtualmethodinvocation: load!("VirtualMethodInvocation.facts")?,
            _method: methods,
            method_descriptor,
            _storeinstancefield: load!("StoreInstanceField.facts")?,
            _loadinstancefield: load!("LoadInstanceField.facts")?,
            _storestaticfield: load!("StoreStaticField.facts")?,
            _loadstaticfield: load!("LoadStaticField.facts")?,
            _storearrayindex: load!("StoreArrayIndex.facts")?,
            _loadarrayindex: load!("LoadArrayIndex.facts")?,
            _return: load!("Return.facts")?,
        })
    }
}

/// All 26 published outputs, retaining their typed full tuples.
pub struct Results {
    pub mainclass: Vec<(IStr,)>,
    pub method_simplename: Vec<(IStr, IStr)>,
    pub method_declaringtype: Vec<(IStr, IStr)>,
    pub method_modifier: Vec<(IStr, IStr)>,
    pub method_paramtypes: Vec<(IStr, IStr)>,
    pub method_returntype: Vec<(IStr, IStr)>,
    pub method_descriptor: Vec<(IStr, IStr)>,
    pub methodlookup: Vec<(IStr, IStr, IStr, IStr)>,
    pub methodimplemented: Vec<(IStr, IStr, IStr, IStr)>,
    pub directsubclass: Vec<(IStr, IStr)>,
    pub subclass: Vec<(IStr, IStr)>,
    pub superclass: Vec<(IStr, IStr)>,
    pub superinterface: Vec<(IStr, IStr)>,
    pub subtypeof: Vec<(IStr, IStr)>,
    pub supertypeof: Vec<(IStr, IStr)>,
    pub subtypeofdifferent: Vec<(IStr, IStr)>,
    pub mainmethoddeclaration: Vec<(IStr,)>,
    pub classinitializer: Vec<(IStr, IStr)>,
    pub initializedclass: Vec<(IStr,)>,
    pub assign: Vec<(IStr, IStr)>,
    pub varpointsto: Vec<(IStr, IStr)>,
    pub instancefieldpointsto: Vec<(IStr, IStr, IStr)>,
    pub staticfieldpointsto: Vec<(IStr, IStr)>,
    pub callgraphedge: Vec<(IStr, IStr)>,
    pub arrayindexpointsto: Vec<(IStr, IStr)>,
    pub reachable: Vec<(IStr,)>,
}

impl Results {
    /// Timing-only semantic witness. Canonicalize typed rows before counting;
    /// resolve dictionary strings only for the actual rows selected by policy.
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
                        vec![$(interner::res(row.$column).to_string()),+]
                    },
                    policy,
                ));
            }};
        }
        relation!("MainClass", mainclass, 0);
        relation!("Method_SimpleName", method_simplename, 0, 1);
        relation!("Method_DeclaringType", method_declaringtype, 0, 1);
        relation!("Method_Modifier", method_modifier, 0, 1);
        relation!("Method_ParamTypes", method_paramtypes, 0, 1);
        relation!("Method_ReturnType", method_returntype, 0, 1);
        relation!("Method_Descriptor", method_descriptor, 0, 1);
        relation!("MethodLookup", methodlookup, 0, 1, 2, 3);
        relation!("MethodImplemented", methodimplemented, 0, 1, 2, 3);
        relation!("DirectSubclass", directsubclass, 0, 1);
        relation!("Subclass", subclass, 0, 1);
        relation!("Superclass", superclass, 0, 1);
        relation!("Superinterface", superinterface, 0, 1);
        relation!("SubtypeOf", subtypeof, 0, 1);
        relation!("SupertypeOf", supertypeof, 0, 1);
        relation!("SubtypeOfDifferent", subtypeofdifferent, 0, 1);
        relation!("MainMethodDeclaration", mainmethoddeclaration, 0);
        relation!("ClassInitializer", classinitializer, 0, 1);
        relation!("InitializedClass", initializedclass, 0);
        relation!("Assign", assign, 0, 1);
        relation!("VarPointsTo", varpointsto, 0, 1);
        relation!("InstanceFieldPointsTo", instancefieldpointsto, 0, 1, 2);
        relation!("StaticFieldPointsTo", staticfieldpointsto, 0, 1);
        relation!("CallGraphEdge", callgraphedge, 0, 1);
        relation!("ArrayIndexPointsTo", arrayindexpointsto, 0, 1);
        relation!("Reachable", reachable, 0);
        witness
    }

    pub fn print_sizes(&self) {
        println!("MainClass	{}", self.mainclass.len());
        println!("Method_SimpleName	{}", self.method_simplename.len());
        println!("Method_DeclaringType	{}", self.method_declaringtype.len());
        println!("Method_Modifier	{}", self.method_modifier.len());
        println!("Method_ParamTypes	{}", self.method_paramtypes.len());
        println!("Method_ReturnType	{}", self.method_returntype.len());
        println!("Method_Descriptor	{}", self.method_descriptor.len());
        println!("MethodLookup	{}", self.methodlookup.len());
        println!("MethodImplemented	{}", self.methodimplemented.len());
        println!("DirectSubclass	{}", self.directsubclass.len());
        println!("Subclass	{}", self.subclass.len());
        println!("Superclass	{}", self.superclass.len());
        println!("Superinterface	{}", self.superinterface.len());
        println!("SubtypeOf	{}", self.subtypeof.len());
        println!("SupertypeOf	{}", self.supertypeof.len());
        println!("SubtypeOfDifferent	{}", self.subtypeofdifferent.len());
        println!("MainMethodDeclaration	{}", self.mainmethoddeclaration.len());
        println!("ClassInitializer	{}", self.classinitializer.len());
        println!("InitializedClass	{}", self.initializedclass.len());
        println!("Assign	{}", self.assign.len());
        println!("VarPointsTo	{}", self.varpointsto.len());
        println!("InstanceFieldPointsTo	{}", self.instancefieldpointsto.len());
        println!("StaticFieldPointsTo	{}", self.staticfieldpointsto.len());
        println!("CallGraphEdge	{}", self.callgraphedge.len());
        println!("ArrayIndexPointsTo	{}", self.arrayindexpointsto.len());
        println!("Reachable	{}", self.reachable.len());
    }

    /// Compare sets, not cardinalities or hashes. Sorting is outside run timing.
    pub fn compare(&mut self, other: &mut Self) -> Result<(), String> {
        self.mainclass.sort_unstable();
        self.mainclass.dedup();
        other.mainclass.sort_unstable();
        other.mainclass.dedup();
        if self.mainclass != other.mainclass {
            return Err(format!(
                "MainClass: tuple sets differ (MiniLinq {}, Ascent {})",
                self.mainclass.len(),
                other.mainclass.len()
            ));
        }
        self.method_simplename.sort_unstable();
        self.method_simplename.dedup();
        other.method_simplename.sort_unstable();
        other.method_simplename.dedup();
        if self.method_simplename != other.method_simplename {
            return Err(format!(
                "Method_SimpleName: tuple sets differ (MiniLinq {}, Ascent {})",
                self.method_simplename.len(),
                other.method_simplename.len()
            ));
        }
        self.method_declaringtype.sort_unstable();
        self.method_declaringtype.dedup();
        other.method_declaringtype.sort_unstable();
        other.method_declaringtype.dedup();
        if self.method_declaringtype != other.method_declaringtype {
            return Err(format!(
                "Method_DeclaringType: tuple sets differ (MiniLinq {}, Ascent {})",
                self.method_declaringtype.len(),
                other.method_declaringtype.len()
            ));
        }
        self.method_modifier.sort_unstable();
        self.method_modifier.dedup();
        other.method_modifier.sort_unstable();
        other.method_modifier.dedup();
        if self.method_modifier != other.method_modifier {
            return Err(format!(
                "Method_Modifier: tuple sets differ (MiniLinq {}, Ascent {})",
                self.method_modifier.len(),
                other.method_modifier.len()
            ));
        }
        self.method_paramtypes.sort_unstable();
        self.method_paramtypes.dedup();
        other.method_paramtypes.sort_unstable();
        other.method_paramtypes.dedup();
        if self.method_paramtypes != other.method_paramtypes {
            return Err(format!(
                "Method_ParamTypes: tuple sets differ (MiniLinq {}, Ascent {})",
                self.method_paramtypes.len(),
                other.method_paramtypes.len()
            ));
        }
        self.method_returntype.sort_unstable();
        self.method_returntype.dedup();
        other.method_returntype.sort_unstable();
        other.method_returntype.dedup();
        if self.method_returntype != other.method_returntype {
            return Err(format!(
                "Method_ReturnType: tuple sets differ (MiniLinq {}, Ascent {})",
                self.method_returntype.len(),
                other.method_returntype.len()
            ));
        }
        self.method_descriptor.sort_unstable();
        self.method_descriptor.dedup();
        other.method_descriptor.sort_unstable();
        other.method_descriptor.dedup();
        if self.method_descriptor != other.method_descriptor {
            return Err(format!(
                "Method_Descriptor: tuple sets differ (MiniLinq {}, Ascent {})",
                self.method_descriptor.len(),
                other.method_descriptor.len()
            ));
        }
        self.methodlookup.sort_unstable();
        self.methodlookup.dedup();
        other.methodlookup.sort_unstable();
        other.methodlookup.dedup();
        if self.methodlookup != other.methodlookup {
            return Err(format!(
                "MethodLookup: tuple sets differ (MiniLinq {}, Ascent {})",
                self.methodlookup.len(),
                other.methodlookup.len()
            ));
        }
        self.methodimplemented.sort_unstable();
        self.methodimplemented.dedup();
        other.methodimplemented.sort_unstable();
        other.methodimplemented.dedup();
        if self.methodimplemented != other.methodimplemented {
            return Err(format!(
                "MethodImplemented: tuple sets differ (MiniLinq {}, Ascent {})",
                self.methodimplemented.len(),
                other.methodimplemented.len()
            ));
        }
        self.directsubclass.sort_unstable();
        self.directsubclass.dedup();
        other.directsubclass.sort_unstable();
        other.directsubclass.dedup();
        if self.directsubclass != other.directsubclass {
            return Err(format!(
                "DirectSubclass: tuple sets differ (MiniLinq {}, Ascent {})",
                self.directsubclass.len(),
                other.directsubclass.len()
            ));
        }
        self.subclass.sort_unstable();
        self.subclass.dedup();
        other.subclass.sort_unstable();
        other.subclass.dedup();
        if self.subclass != other.subclass {
            return Err(format!(
                "Subclass: tuple sets differ (MiniLinq {}, Ascent {})",
                self.subclass.len(),
                other.subclass.len()
            ));
        }
        self.superclass.sort_unstable();
        self.superclass.dedup();
        other.superclass.sort_unstable();
        other.superclass.dedup();
        if self.superclass != other.superclass {
            return Err(format!(
                "Superclass: tuple sets differ (MiniLinq {}, Ascent {})",
                self.superclass.len(),
                other.superclass.len()
            ));
        }
        self.superinterface.sort_unstable();
        self.superinterface.dedup();
        other.superinterface.sort_unstable();
        other.superinterface.dedup();
        if self.superinterface != other.superinterface {
            return Err(format!(
                "Superinterface: tuple sets differ (MiniLinq {}, Ascent {})",
                self.superinterface.len(),
                other.superinterface.len()
            ));
        }
        self.subtypeof.sort_unstable();
        self.subtypeof.dedup();
        other.subtypeof.sort_unstable();
        other.subtypeof.dedup();
        if self.subtypeof != other.subtypeof {
            return Err(format!(
                "SubtypeOf: tuple sets differ (MiniLinq {}, Ascent {})",
                self.subtypeof.len(),
                other.subtypeof.len()
            ));
        }
        self.supertypeof.sort_unstable();
        self.supertypeof.dedup();
        other.supertypeof.sort_unstable();
        other.supertypeof.dedup();
        if self.supertypeof != other.supertypeof {
            return Err(format!(
                "SupertypeOf: tuple sets differ (MiniLinq {}, Ascent {})",
                self.supertypeof.len(),
                other.supertypeof.len()
            ));
        }
        self.subtypeofdifferent.sort_unstable();
        self.subtypeofdifferent.dedup();
        other.subtypeofdifferent.sort_unstable();
        other.subtypeofdifferent.dedup();
        if self.subtypeofdifferent != other.subtypeofdifferent {
            return Err(format!(
                "SubtypeOfDifferent: tuple sets differ (MiniLinq {}, Ascent {})",
                self.subtypeofdifferent.len(),
                other.subtypeofdifferent.len()
            ));
        }
        self.mainmethoddeclaration.sort_unstable();
        self.mainmethoddeclaration.dedup();
        other.mainmethoddeclaration.sort_unstable();
        other.mainmethoddeclaration.dedup();
        if self.mainmethoddeclaration != other.mainmethoddeclaration {
            return Err(format!(
                "MainMethodDeclaration: tuple sets differ (MiniLinq {}, Ascent {})",
                self.mainmethoddeclaration.len(),
                other.mainmethoddeclaration.len()
            ));
        }
        self.classinitializer.sort_unstable();
        self.classinitializer.dedup();
        other.classinitializer.sort_unstable();
        other.classinitializer.dedup();
        if self.classinitializer != other.classinitializer {
            return Err(format!(
                "ClassInitializer: tuple sets differ (MiniLinq {}, Ascent {})",
                self.classinitializer.len(),
                other.classinitializer.len()
            ));
        }
        self.initializedclass.sort_unstable();
        self.initializedclass.dedup();
        other.initializedclass.sort_unstable();
        other.initializedclass.dedup();
        if self.initializedclass != other.initializedclass {
            return Err(format!(
                "InitializedClass: tuple sets differ (MiniLinq {}, Ascent {})",
                self.initializedclass.len(),
                other.initializedclass.len()
            ));
        }
        self.assign.sort_unstable();
        self.assign.dedup();
        other.assign.sort_unstable();
        other.assign.dedup();
        if self.assign != other.assign {
            return Err(format!(
                "Assign: tuple sets differ (MiniLinq {}, Ascent {})",
                self.assign.len(),
                other.assign.len()
            ));
        }
        self.varpointsto.sort_unstable();
        self.varpointsto.dedup();
        other.varpointsto.sort_unstable();
        other.varpointsto.dedup();
        if self.varpointsto != other.varpointsto {
            return Err(format!(
                "VarPointsTo: tuple sets differ (MiniLinq {}, Ascent {})",
                self.varpointsto.len(),
                other.varpointsto.len()
            ));
        }
        self.instancefieldpointsto.sort_unstable();
        self.instancefieldpointsto.dedup();
        other.instancefieldpointsto.sort_unstable();
        other.instancefieldpointsto.dedup();
        if self.instancefieldpointsto != other.instancefieldpointsto {
            return Err(format!(
                "InstanceFieldPointsTo: tuple sets differ (MiniLinq {}, Ascent {})",
                self.instancefieldpointsto.len(),
                other.instancefieldpointsto.len()
            ));
        }
        self.staticfieldpointsto.sort_unstable();
        self.staticfieldpointsto.dedup();
        other.staticfieldpointsto.sort_unstable();
        other.staticfieldpointsto.dedup();
        if self.staticfieldpointsto != other.staticfieldpointsto {
            return Err(format!(
                "StaticFieldPointsTo: tuple sets differ (MiniLinq {}, Ascent {})",
                self.staticfieldpointsto.len(),
                other.staticfieldpointsto.len()
            ));
        }
        self.callgraphedge.sort_unstable();
        self.callgraphedge.dedup();
        other.callgraphedge.sort_unstable();
        other.callgraphedge.dedup();
        if self.callgraphedge != other.callgraphedge {
            return Err(format!(
                "CallGraphEdge: tuple sets differ (MiniLinq {}, Ascent {})",
                self.callgraphedge.len(),
                other.callgraphedge.len()
            ));
        }
        self.arrayindexpointsto.sort_unstable();
        self.arrayindexpointsto.dedup();
        other.arrayindexpointsto.sort_unstable();
        other.arrayindexpointsto.dedup();
        if self.arrayindexpointsto != other.arrayindexpointsto {
            return Err(format!(
                "ArrayIndexPointsTo: tuple sets differ (MiniLinq {}, Ascent {})",
                self.arrayindexpointsto.len(),
                other.arrayindexpointsto.len()
            ));
        }
        self.reachable.sort_unstable();
        self.reachable.dedup();
        other.reachable.sort_unstable();
        other.reachable.dedup();
        if self.reachable != other.reachable {
            return Err(format!(
                "Reachable: tuple sets differ (MiniLinq {}, Ascent {})",
                self.reachable.len(),
                other.reachable.len()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod witness_tests {
    use super::*;
    use bench_check::{Mode, Policy};

    fn empty_results() -> Results {
        Results {
            mainclass: Vec::new(),
            method_simplename: Vec::new(),
            method_declaringtype: Vec::new(),
            method_modifier: Vec::new(),
            method_paramtypes: Vec::new(),
            method_returntype: Vec::new(),
            method_descriptor: Vec::new(),
            methodlookup: Vec::new(),
            methodimplemented: Vec::new(),
            directsubclass: Vec::new(),
            subclass: Vec::new(),
            superclass: Vec::new(),
            superinterface: Vec::new(),
            subtypeof: Vec::new(),
            supertypeof: Vec::new(),
            subtypeofdifferent: Vec::new(),
            mainmethoddeclaration: Vec::new(),
            classinitializer: Vec::new(),
            initializedclass: Vec::new(),
            assign: Vec::new(),
            varpointsto: Vec::new(),
            instancefieldpointsto: Vec::new(),
            staticfieldpointsto: Vec::new(),
            callgraphedge: Vec::new(),
            arrayindexpointsto: Vec::new(),
            reachable: Vec::new(),
        }
    }

    #[test]
    fn full_witness_uses_semantic_strings_and_logical_counts() {
        let mut result = empty_results();
        let heap = interner::sym("semantic-heap");
        let variable = interner::sym("semantic-variable");
        result.varpointsto = vec![(heap, variable), (heap, variable)];
        let witness = result.witness(Policy::full());
        assert_eq!(witness.len(), 26);
        assert_eq!(witness["VarPointsTo"].count, 1);
        assert_eq!(witness["VarPointsTo"].rows,
                   vec![vec!["semantic-heap".to_string(), "semantic-variable".to_string()]]);
        assert_eq!(witness["Reachable"].count, 0);
        assert!(witness["Reachable"].rows.is_empty());
    }

    #[test]
    fn sampled_witness_contains_distinct_actual_semantic_rows() {
        let mut result = empty_results();
        result.reachable = (0..1501)
            .map(|index| (interner::sym(&format!("reachable-method-{index}")),))
            .collect();
        result.reachable.push(result.reachable[0]);
        let policy = Policy { mode: Mode::Sampled, seed: 17, sample_size: 1000 };
        let witness = result.witness(policy);
        let relation = &witness["Reachable"];
        assert_eq!(relation.count, 1501);
        assert_eq!(relation.rows.len(), 1000);
        assert_eq!(relation.rows.iter().collect::<std::collections::BTreeSet<_>>().len(), 1000);
        assert!(relation.rows.iter().all(|row| row.len() == 1
            && row[0].strip_prefix("reachable-method-")
                .and_then(|suffix| suffix.parse::<usize>().ok())
                .is_some_and(|index| index < 1501)));
        assert_eq!(relation.rows, result.witness(policy)["Reachable"].rows);
    }
}
