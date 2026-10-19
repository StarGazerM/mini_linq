//! DOOP context-insensitive points-to analysis, sequential and parallel engines.
//!
//! Both engines preserve all 103 relations, 135 rules and body atom order from
//! `flowlog-rs/flowlog-bench/programs/oracle/flowlog/doop/default.dl`.
//! Both `ascent!` and `ascent_par!` expand the same upstream translation.
//! Strings share one process-wide u32 interner; loading is identical for both.
//!
//! From `features/datalog`, build with `cargo build --release -p datalog-doop`.
//! Fetch with `python3 doop/fetch.py cassandra`, then invoke the built binary:
//! `doop /tmp/doop-data/cassandra --threads 5 --check`.
//! `--check` compares all 26 full tuple sets across all four engines against
//! sequential Ascent. Uninstrumented output order is unspecified.
//! `--engine mini|mini-par|ascent|ascent-par` isolates timing and peak RSS.
//! `python3 doop/sweep.py --binary /tmp/target-Par/release/doop --baseline /path/to/baseline/doop`
//! checks all six applications at 1,2,4,5,8,15 threads before isolated timings.
//! The sweep retains `/usr/bin/time -l` stderr, including phase-timing output
//! when built with `--features phase-timing`, outside the external facts cache.
//! Compute timing includes derived indexes and materializing the 26 outputs;
//! load timing includes parsing and interning, not engine initialization.
//! No counters are enabled. Do not compile concurrently with measured runs.

#![recursion_limit = "1024"]

pub mod ascent_engine;
mod interner;
mod loader;
pub mod mini_engine;

use interner::IStr;
use std::{io, path::Path};

/// The 32 raw input relations, loaded once per engine using the same parser.
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
        Ok(Self {
            directsuperclass: loader::load(&directory.join("DirectSuperclass.facts"))?,
            directsuperinterface: loader::load(&directory.join("DirectSuperinterface.facts"))?,
            mainclass: loader::load(&directory.join("MainClass.facts"))?,
            formalparam: loader::load(&directory.join("FormalParam.facts"))?,
            componenttype: loader::load(&directory.join("ComponentType.facts"))?,
            assignreturnvalue: loader::load(&directory.join("AssignReturnValue.facts"))?,
            actualparam: loader::load(&directory.join("ActualParam.facts"))?,
            method_modifier: loader::load(&directory.join("Method-Modifier.facts"))?,
            var_type: loader::load(&directory.join("Var-Type.facts"))?,
            _classtype: loader::load(&directory.join("ClassType.facts"))?,
            _arraytype: loader::load(&directory.join("ArrayType.facts"))?,
            _interfacetype: loader::load(&directory.join("InterfaceType.facts"))?,
            _var_declaringmethod: loader::load(&directory.join("Var-DeclaringMethod.facts"))?,
            _applicationclass: loader::load(&directory.join("ApplicationClass.facts"))?,
            _thisvar: loader::load(&directory.join("ThisVar.facts"))?,
            _normalheap: loader::load(&directory.join("NormalHeap.facts"))?,
            _stringconstant: loader::load(&directory.join("StringConstant.facts"))?,
            _assignheapallocation: loader::load(&directory.join("AssignHeapAllocation.facts"))?,
            _assignlocal: loader::load(&directory.join("AssignLocal.facts"))?,
            _assigncast: loader::load(&directory.join("AssignCast.facts"))?,
            _field: loader::load(&directory.join("Field.facts"))?,
            _staticmethodinvocation: loader::load(&directory.join("StaticMethodInvocation.facts"))?,
            _specialmethodinvocation: loader::load(
                &directory.join("SpecialMethodInvocation.facts"),
            )?,
            _virtualmethodinvocation: loader::load(
                &directory.join("VirtualMethodInvocation.facts"),
            )?,
            _method: loader::load(&directory.join("Method.facts"))?,
            _storeinstancefield: loader::load(&directory.join("StoreInstanceField.facts"))?,
            _loadinstancefield: loader::load(&directory.join("LoadInstanceField.facts"))?,
            _storestaticfield: loader::load(&directory.join("StoreStaticField.facts"))?,
            _loadstaticfield: loader::load(&directory.join("LoadStaticField.facts"))?,
            _storearrayindex: loader::load(&directory.join("StoreArrayIndex.facts"))?,
            _loadarrayindex: loader::load(&directory.join("LoadArrayIndex.facts"))?,
            _return: loader::load(&directory.join("Return.facts"))?,
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
mod tests {
    use super::*;
    use crate::interner::sym;

    #[test]
    fn all_outputs_match_on_small_program() {
        // The unoptimized 103-relation expansions exceed libtest's 2 MiB
        // worker stack. Release benchmark execution uses the normal main stack.
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(compare_small_program)
            .unwrap()
            .join()
            .unwrap();
    }

    fn compare_small_program() {
        fn facts() -> Facts {
            let class = sym("Example");
            let object = sym("java.lang.Object");
            let method = sym("<Example: void main(java.lang.String[])>");
            let abstract_method = sym("<Example: void abstractMethod()>");
            Facts {
                mainclass: vec![(class,)],
                _classtype: vec![(class,), (object,)],
                directsuperclass: vec![(class, object)],
                method_modifier: vec![
                    (sym("public"), method),
                    (sym("static"), method),
                    (sym("abstract"), abstract_method),
                ],
                _method: vec![
                    (
                        method,
                        sym("main"),
                        sym("java.lang.String[]"),
                        class,
                        sym("void"),
                        sym("([Ljava/lang/String;)V"),
                        1,
                    ),
                    (
                        abstract_method,
                        sym("abstractMethod"),
                        sym(""),
                        class,
                        sym("void"),
                        sym("()V"),
                        0,
                    ),
                ],
                _assignheapallocation: vec![(
                    sym("allocation"),
                    0,
                    sym("heap"),
                    sym("local"),
                    method,
                    1,
                )],
                _normalheap: vec![(sym("heap"), class)],
                ..Facts::default()
            }
        }
        let mut mini = mini_engine::run(facts());
        let mut ascent = ascent_engine::run(facts());
        mini.compare(&mut ascent).unwrap();
        assert_eq!(mini.varpointsto, vec![(sym("heap"), sym("local"))]);
        assert_eq!(mini.mainmethoddeclaration.len(), 1);
        assert_eq!(mini.methodimplemented.len(), 1);
        assert!(mini.method_descriptor.contains(&(
            sym("<Example: void main(java.lang.String[])>"),
            sym("void(java.lang.String[])"),
        )));
    }
}
