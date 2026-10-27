//! Hand-derived exactness fixture: recursive field flow, multihead calls,
//! parameter/return propagation, duplicates, negation, and unreachable stores.
use crate::{SrdConstants, SrdFacts, SrdResults};

pub fn facts() -> SrdFacts {
    SrdFacts {
        constants: SrdConstants {
            c_abstract: -100,
            c_class_init_method: -101,
            c_clinit: -102,
            c_clinit_descriptor: -103,
            c_desiredassertionstatus_method: -104,
            c_java_io_serializable: -105,
            c_java_lang_class_type: -106,
            c_java_lang_cloneable: -107,
            c_java_lang_object: -108,
            c_java_lang_object_array: -109,
            c_java_lang_string_array: -110,
            c_java_lang_string_type: -111,
            c_main: 101,
            c_main_descriptor: 102,
            c_public: 103,
            c_register_natives_method: -115,
            c_static: 104,
        },
        r_classtype: vec![(1,)],
        r_mainclass: vec![(1,)],
        r_method_declaringtype: vec![(10, 1), (12, 1)],
        r_method_simplename: vec![(10, 101), (12, 105)],
        r_method_descriptor: vec![(10, 102), (12, 102)],
        r_method_modifier: vec![(103, 10), (104, 10), (-100, 12)],
        r_assignheapallocation: vec![(20, 30, 10), (21, 31, 10), (21, 31, 10)],
        r_storeinstancefield: vec![(31, 30, 40, 10), (99, 30, 40, 999)],
        r_loadinstancefield: vec![(30, 40, 32, 10)],
        r_assignlocal: vec![(32, 33, 10), (33, 31, 10)],
        r_staticmethodinvocation: vec![(50, 11, 10)],
        r_formalparam: vec![(0, 11, 34)],
        r_actualparam: vec![(0, 50, 33)],
        r_returnvar: vec![(34, 11)],
        r_assignreturnvalue: vec![(50, 35)],
        ..SrdFacts::default()
    }
}

pub fn expected() -> SrdResults {
    SrdResults {
        r_istype: vec![(1,)],
        r_isreferencetype: vec![(1,)],
        r_isclasstype_idb: vec![(1,)],
        r_methodimplemented: vec![(101, 102, 1, 10)],
        r_mainmethoddeclaration: vec![(10,)],
        r_subtypeof: vec![(1, 1)],
        r_supertypeof: vec![(1, 1)],
        r_methodlookup: vec![(101, 102, 1, 10)],
        r_initializedclass: vec![(1,)],
        r_reachable: vec![(10,), (11,)],
        r_callgraphedge: vec![(50, 11)],
        r_assign: vec![(33, 34), (34, 35)],
        r_varpointsto: vec![(20, 30), (21, 31), (21, 32), (21, 33), (21, 34), (21, 35)],
        r_reachableloadinstancefield: vec![(30, 40, 32)],
        r_reachablestoreinstancefield: vec![(30, 40, 31)],
        r_storeobjfrom: vec![(20, 40, 31)],
        r_fieldflow: vec![(31, 32)],
        r_instancefieldpointsto: vec![(21, 40, 20)],
        ..SrdResults::default()
    }
}

pub fn assert_results(mut actual: SrdResults) {
    let mut expected = expected();
    macro_rules! check {
        ($($field:ident),* $(,)?) => { $(
            actual.$field.sort_unstable();
            expected.$field.sort_unstable();
            assert_eq!(actual.$field, expected.$field, stringify!($field));
        )* };
    }
    check!(
        r_arrayindexpointsto,
        r_arraytypecompat,
        r_assign,
        r_callgraphedge,
        r_castto,
        r_classinitializer,
        r_directsubclass,
        r_eligibleobjectarrayheap,
        r_eligiblestringarrayheap,
        r_fieldflow,
        r_heaphelper,
        r_heaphelpernothis,
        r_initializedclass,
        r_instancefieldpointsto,
        r_iscastabletostring,
        r_isobjectarrayheap,
        r_isstringarrayheap,
        r_isstringheap,
        r_mainmethoddeclaration,
        r_methodimplemented,
        r_methodinvocation_base,
        r_methodlookup,
        r_nonstringheapallocsupertype,
        r_reachable,
        r_reachableinstruction,
        r_reachableloadinstancefield,
        r_reachablesortedindex,
        r_reachablestoreinstancefield,
        r_staticfieldpointsto,
        r_storeobjfrom,
        r_subclass,
        r_subtypeof,
        r_subtypeofdifferent,
        r_superclass,
        r_superinterface,
        r_supertypeof,
        r_varpointsto,
        r_virtualmethodinvocation,
        r_isarraytype_idb,
        r_isclasstype_idb,
        r_isinterfacetype_idb,
        r_isreferencetype,
        r_istype,
    );
}
