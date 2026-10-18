// Translated from flowlog-rs/flowlog-bench programs/oracle/ascent/doop/src/main.rs.
// All 135 rules and body atom order match the upstream DOOP default.dl.
#![allow(unused_variables)]
use crate::{
    Facts, Results,
    interner::{IStr, res, sym},
};

pub fn run(facts: Facts) -> Results {
    let (
        mainclass,
        method_simplename,
        method_declaringtype,
        method_modifier,
        method_paramtypes,
        method_returntype,
        method_descriptor,
        methodlookup,
        methodimplemented,
        directsubclass,
        subclass,
        superclass,
        superinterface,
        subtypeof,
        supertypeof,
        subtypeofdifferent,
        mainmethoddeclaration,
        classinitializer,
        initializedclass,
        assign,
        varpointsto,
        instancefieldpointsto,
        staticfieldpointsto,
        callgraphedge,
        arrayindexpointsto,
        reachable,
    ) = cq_datalog::datalog! {
        struct Doop;
        relation directsuperclass(c0: IStr, c1: IStr) = (facts.directsuperclass);
        relation directsuperinterface(c0: IStr, c1: IStr) = (facts.directsuperinterface);
        relation mainclass(c0: IStr) = (facts.mainclass);
        relation formalparam(c0: i32, c1: IStr, c2: IStr) = (facts.formalparam);
        relation componenttype(c0: IStr, c1: IStr) = (facts.componenttype);
        relation assignreturnvalue(c0: IStr, c1: IStr) = (facts.assignreturnvalue);
        relation actualparam(c0: i32, c1: IStr, c2: IStr) = (facts.actualparam);
        relation method_modifier(c0: IStr, c1: IStr) = (facts.method_modifier);
        relation var_type(c0: IStr, c1: IStr) = (facts.var_type);
        relation heapallocation_type(c0: IStr, c1: IStr);
        relation istype(c0: IStr);
        relation isreferencetype(c0: IStr);
        relation isarraytype(c0: IStr);
        relation isclasstype(c0: IStr);
        relation isinterfacetype(c0: IStr);
        relation applicationclass(c0: IStr);
        relation field_declaringtype(c0: IStr, c1: IStr);
        relation method_declaringtype(c0: IStr, c1: IStr);
        relation method_returntype(c0: IStr, c1: IStr);
        relation method_simplename(c0: IStr, c1: IStr);
        relation method_paramtypes(c0: IStr, c1: IStr);
        relation thisvar(c0: IStr, c1: IStr);
        relation var_declaringmethod(c0: IStr, c1: IStr);
        relation instruction_method(c0: IStr, c1: IStr);
        relation isvirtualmethodinvocation_insn(c0: IStr);
        relation isstaticmethodinvocation_insn(c0: IStr);
        relation fieldinstruction_signature(c0: IStr, c1: IStr);
        relation loadinstancefield_base(c0: IStr, c1: IStr);
        relation loadinstancefield_to(c0: IStr, c1: IStr);
        relation storeinstancefield_from(c0: IStr, c1: IStr);
        relation storeinstancefield_base(c0: IStr, c1: IStr);
        relation loadstaticfield_to(c0: IStr, c1: IStr);
        relation storestaticfield_from(c0: IStr, c1: IStr);
        relation loadarrayindex_base(c0: IStr, c1: IStr);
        relation loadarrayindex_to(c0: IStr, c1: IStr);
        relation storearrayindex_from(c0: IStr, c1: IStr);
        relation storearrayindex_base(c0: IStr, c1: IStr);
        relation assigninstruction_to(c0: IStr, c1: IStr);
        relation assigncast_from(c0: IStr, c1: IStr);
        relation assigncast_type(c0: IStr, c1: IStr);
        relation assignlocal_from(c0: IStr, c1: IStr);
        relation assignheapallocation_heap(c0: IStr, c1: IStr);
        relation returnnonvoid_var(c0: IStr, c1: IStr);
        relation methodinvocation_method(c0: IStr, c1: IStr);
        relation virtualmethodinvocation_base(c0: IStr, c1: IStr);
        relation virtualmethodinvocation_simplename(c0: IStr, c1: IStr);
        relation virtualmethodinvocation_descriptor(c0: IStr, c1: IStr);
        relation specialmethodinvocation_base(c0: IStr, c1: IStr);
        relation methodinvocation_base(c0: IStr, c1: IStr);
        relation loadinstancefield(c0: IStr, c1: IStr, c2: IStr, c3: IStr);
        relation storeinstancefield(c0: IStr, c1: IStr, c2: IStr, c3: IStr);
        relation loadstaticfield(c0: IStr, c1: IStr, c2: IStr);
        relation storestaticfield(c0: IStr, c1: IStr, c2: IStr);
        relation loadarrayindex(c0: IStr, c1: IStr, c2: IStr);
        relation storearrayindex(c0: IStr, c1: IStr, c2: IStr);
        relation assigncast(c0: IStr, c1: IStr, c2: IStr, c3: IStr);
        relation assignlocal(c0: IStr, c1: IStr, c2: IStr);
        relation assignheapallocation(c0: IStr, c1: IStr, c2: IStr);
        relation returnvar(c0: IStr, c1: IStr);
        relation staticmethodinvocation(c0: IStr, c1: IStr, c2: IStr);
        relation _classtype(c0: IStr) = (facts._classtype);
        relation _arraytype(c0: IStr) = (facts._arraytype);
        relation _interfacetype(c0: IStr) = (facts._interfacetype);
        relation _var_declaringmethod(c0: IStr, c1: IStr) = (facts._var_declaringmethod);
        relation _applicationclass(c0: IStr) = (facts._applicationclass);
        relation _thisvar(c0: IStr, c1: IStr) = (facts._thisvar);
        relation _normalheap(c0: IStr, c1: IStr) = (facts._normalheap);
        relation _stringconstant(c0: IStr) = (facts._stringconstant);
        relation _assignheapallocation(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr, c5: i32) = (facts._assignheapallocation);
        relation _assignlocal(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._assignlocal);
        relation _assigncast(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr, c5: IStr) = (facts._assigncast);
        relation _field(c0: IStr, c1: IStr, c2: IStr, c3: IStr) = (facts._field);
        relation _staticmethodinvocation(c0: IStr, c1: i32, c2: IStr, c3: IStr) = (facts._staticmethodinvocation);
        relation _specialmethodinvocation(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._specialmethodinvocation);
        relation _virtualmethodinvocation(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._virtualmethodinvocation);
        relation _method(c0: IStr, c1: IStr, c2: IStr, c3: IStr, c4: IStr, c5: IStr, c6: i32) = (facts._method);
        relation method_descriptor(c0: IStr, c1: IStr);
        relation _storeinstancefield(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr, c5: IStr) = (facts._storeinstancefield);
        relation _loadinstancefield(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr, c5: IStr) = (facts._loadinstancefield);
        relation _storestaticfield(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._storestaticfield);
        relation _loadstaticfield(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._loadstaticfield);
        relation _storearrayindex(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._storearrayindex);
        relation _loadarrayindex(c0: IStr, c1: i32, c2: IStr, c3: IStr, c4: IStr) = (facts._loadarrayindex);
        relation _return(c0: IStr, c1: i32, c2: IStr, c3: IStr) = (facts._return);
        relation methodlookup(c0: IStr, c1: IStr, c2: IStr, c3: IStr);
        relation methodimplemented(c0: IStr, c1: IStr, c2: IStr, c3: IStr);
        relation directsubclass(c0: IStr, c1: IStr);
        relation subclass(c0: IStr, c1: IStr);
        relation superclass(c0: IStr, c1: IStr);
        relation superinterface(c0: IStr, c1: IStr);
        relation subtypeof(c0: IStr, c1: IStr);
        relation supertypeof(c0: IStr, c1: IStr);
        relation subtypeofdifferent(c0: IStr, c1: IStr);
        relation mainmethoddeclaration(c0: IStr);
        relation classinitializer(c0: IStr, c1: IStr);
        relation initializedclass(c0: IStr);
        relation assign(c0: IStr, c1: IStr);
        relation varpointsto(c0: IStr, c1: IStr);
        relation instancefieldpointsto(c0: IStr, c1: IStr, c2: IStr);
        relation staticfieldpointsto(c0: IStr, c1: IStr);
        relation callgraphedge(c0: IStr, c1: IStr);
        relation arrayindexpointsto(c0: IStr, c1: IStr);
        relation reachable(c0: IStr);
        output mainclass, method_simplename, method_declaringtype, method_modifier, method_paramtypes, method_returntype, method_descriptor, methodlookup, methodimplemented, directsubclass, subclass, superclass, superinterface, subtypeof, supertypeof, subtypeofdifferent, mainmethoddeclaration, classinitializer, initializedclass, assign, varpointsto, instancefieldpointsto, staticfieldpointsto, callgraphedge, arrayindexpointsto, reachable;
    istype(*class) :- _classtype(class);
    isreferencetype(*class) :- _classtype(class);
    isclasstype(*class) :- _classtype(class);

    istype(*arraytype) :- _arraytype(arraytype);
    isreferencetype(*arraytype) :- _arraytype(arraytype);
    isarraytype(*arraytype) :- _arraytype(arraytype);

    istype(*interface) :- _interfacetype(interface);
    isreferencetype(*interface) :- _interfacetype(interface);
    isinterfacetype(*interface) :- _interfacetype(interface);

    var_declaringmethod(*var, *method) :- _var_declaringmethod(var, method);

    istype(*type_) :- _applicationclass(type_);
    isreferencetype(*type_) :- _applicationclass(type_);
    applicationclass(*type_) :- _applicationclass(type_);

    thisvar(*method, *var) :- _thisvar(method, var);

    istype(*type_) :- _normalheap(_, type_);
    heapallocation_type(*id, *type_) :- _normalheap(id, type_);
    heapallocation_type(*id, sym("java.lang.String")) :- _stringconstant(id);

    instruction_method(*instruction, *method) :-
            _assignheapallocation(instruction, index, heap, to, method, linenumber);
    assigninstruction_to(*instruction, *to) :-
            _assignheapallocation(instruction, index, heap, to, method, linenumber);
    assignheapallocation_heap(*instruction, *heap) :-
            _assignheapallocation(instruction, index, heap, to, method, linenumber);

    instruction_method(*instruction, *method) :-
            _assignlocal(instruction, index, from, to, method);
    assignlocal_from(*instruction, *from) :-
            _assignlocal(instruction, index, from, to, method);
    assigninstruction_to(*instruction, *to) :-
            _assignlocal(instruction, index, from, to, method);

    instruction_method(*instruction, *method) :-
            _assigncast(instruction, index, from, to, type_, method);
    assigncast_type(*instruction, *type_) :-
            _assigncast(instruction, index, from, to, type_, method);
    assigncast_from(*instruction, *from) :-
            _assigncast(instruction, index, from, to, type_, method);
    assigninstruction_to(*instruction, *to) :-
            _assigncast(instruction, index, from, to, type_, method);

    field_declaringtype(*signature, *declaringtype) :- _field(signature, declaringtype, _, _);

    methodinvocation_base(*invocation, *base) :- virtualmethodinvocation_base(invocation, base);
    methodinvocation_base(*invocation, *base) :- specialmethodinvocation_base(invocation, base);

    instruction_method(*instruction, *method) :-
            _staticmethodinvocation(instruction, index, signature, method);
    isstaticmethodinvocation_insn(*instruction) :-
            _staticmethodinvocation(instruction, index, signature, method);
    methodinvocation_method(*instruction, *signature) :-
            _staticmethodinvocation(instruction, index, signature, method);

    instruction_method(*instruction, *method) :-
            _specialmethodinvocation(instruction, index, signature, base, method);
    specialmethodinvocation_base(*instruction, *base) :-
            _specialmethodinvocation(instruction, index, signature, base, method);
    methodinvocation_method(*instruction, *signature) :-
            _specialmethodinvocation(instruction, index, signature, base, method);

    instruction_method(*instruction, *method) :-
            _virtualmethodinvocation(instruction, index, signature, base, method);
    isvirtualmethodinvocation_insn(*instruction) :-
            _virtualmethodinvocation(instruction, index, signature, base, method);
    virtualmethodinvocation_base(*instruction, *base) :-
            _virtualmethodinvocation(instruction, index, signature, base, method);
    methodinvocation_method(*instruction, *signature) :-
            _virtualmethodinvocation(instruction, index, signature, base, method);

    method_simplename(*method, *simplename) :-
            _method(method, simplename, params, declaringtype, returntype, jvmdescriptor, arity);
    method_paramtypes(*method, *params) :-
            _method(method, simplename, params, declaringtype, returntype, jvmdescriptor, arity);
    method_declaringtype(*method, *declaringtype) :-
            _method(method, simplename, params, declaringtype, returntype, jvmdescriptor, arity);
    method_returntype(*method, *returntype) :-
            _method(method, simplename, params, declaringtype, returntype, jvmdescriptor, arity);

        // Method_Descriptor computed via cat
    method_descriptor(*method, sym(&format!("{}({})", res(*returntype), res(*params)))) :-
            method_returntype(method, returntype),
            method_paramtypes(method, params);

    instruction_method(*instruction, *method) :-
            _storeinstancefield(instruction, index, from, base, signature, method);
    fieldinstruction_signature(*instruction, *signature) :-
            _storeinstancefield(instruction, index, from, base, signature, method);
    storeinstancefield_base(*instruction, *base) :-
            _storeinstancefield(instruction, index, from, base, signature, method);
    storeinstancefield_from(*instruction, *from) :-
            _storeinstancefield(instruction, index, from, base, signature, method);

    instruction_method(*instruction, *method) :-
            _loadinstancefield(instruction, index, to, base, signature, method);
    fieldinstruction_signature(*instruction, *signature) :-
            _loadinstancefield(instruction, index, to, base, signature, method);
    loadinstancefield_base(*instruction, *base) :-
            _loadinstancefield(instruction, index, to, base, signature, method);
    loadinstancefield_to(*instruction, *to) :-
            _loadinstancefield(instruction, index, to, base, signature, method);

    instruction_method(*instruction, *method) :-
            _storestaticfield(instruction, index, from, signature, method);
    fieldinstruction_signature(*instruction, *signature) :-
            _storestaticfield(instruction, index, from, signature, method);
    storestaticfield_from(*instruction, *from) :-
            _storestaticfield(instruction, index, from, signature, method);

    instruction_method(*instruction, *method) :-
            _loadstaticfield(instruction, index, to, signature, method);
    fieldinstruction_signature(*instruction, *signature) :-
            _loadstaticfield(instruction, index, to, signature, method);
    loadstaticfield_to(*instruction, *to) :-
            _loadstaticfield(instruction, index, to, signature, method);

    instruction_method(*instruction, *method) :-
            _storearrayindex(instruction, index, from, base, method);
    storearrayindex_base(*instruction, *base) :-
            _storearrayindex(instruction, index, from, base, method);
    storearrayindex_from(*instruction, *from) :-
            _storearrayindex(instruction, index, from, base, method);

    instruction_method(*instruction, *method) :-
            _loadarrayindex(instruction, index, to, base, method);
    loadarrayindex_base(*instruction, *base) :-
            _loadarrayindex(instruction, index, to, base, method);
    loadarrayindex_to(*instruction, *to) :-
            _loadarrayindex(instruction, index, to, base, method);

    instruction_method(*instruction, *method) :-
            _return(instruction, index, var, method);
    returnnonvoid_var(*instruction, *var) :-
            _return(instruction, index, var, method);

        // fat schema population
    loadinstancefield(*base, *sig, *to, *inmethod) :-
            instruction_method(insn, inmethod),
            loadinstancefield_base(insn, base),
            fieldinstruction_signature(insn, sig),
            loadinstancefield_to(insn, to);
    storeinstancefield(*from, *base, *sig, *inmethod) :-
            instruction_method(insn, inmethod),
            storeinstancefield_from(insn, from),
            storeinstancefield_base(insn, base),
            fieldinstruction_signature(insn, sig);
    loadstaticfield(*sig, *to, *inmethod) :-
            instruction_method(insn, inmethod),
            fieldinstruction_signature(insn, sig),
            loadstaticfield_to(insn, to);
    storestaticfield(*from, *sig, *inmethod) :-
            instruction_method(insn, inmethod),
            storestaticfield_from(insn, from),
            fieldinstruction_signature(insn, sig);
    loadarrayindex(*base, *to, *inmethod) :-
            instruction_method(insn, inmethod),
            loadarrayindex_base(insn, base),
            loadarrayindex_to(insn, to);
    storearrayindex(*from, *base, *inmethod) :-
            instruction_method(insn, inmethod),
            storearrayindex_from(insn, from),
            storearrayindex_base(insn, base);
    assigncast(*type_, *from, *to, *inmethod) :-
            instruction_method(insn, inmethod),
            assigncast_from(insn, from),
            assigninstruction_to(insn, to),
            assigncast_type(insn, type_);
    assignlocal(*from, *to, *inmethod) :-
            assigninstruction_to(insn, to),
            instruction_method(insn, inmethod),
            assignlocal_from(insn, from);
    assignheapallocation(*heap, *to, *inmethod) :-
            instruction_method(insn, inmethod),
            assignheapallocation_heap(insn, heap),
            assigninstruction_to(insn, to);
    returnvar(*var, *method) :-
            instruction_method(insn, method),
            returnnonvoid_var(insn, var);
    staticmethodinvocation(*invocation, *signature, *inmethod) :-
            isstaticmethodinvocation_insn(invocation),
            instruction_method(invocation, inmethod),
            methodinvocation_method(invocation, signature);
    virtualmethodinvocation_simplename(*invocation, *simplename) :-
            isvirtualmethodinvocation_insn(invocation),
            methodinvocation_method(invocation, signature),
            method_simplename(signature, simplename),
            method_descriptor(signature, descriptor);
    virtualmethodinvocation_descriptor(*invocation, *descriptor) :-
            isvirtualmethodinvocation_insn(invocation),
            methodinvocation_method(invocation, signature),
            method_simplename(signature, simplename),
            method_descriptor(signature, descriptor);

        // Basic (type-based) analysis
    methodlookup(*simplename, *descriptor, *type_, *method) :-
            methodimplemented(simplename, descriptor, type_, method);
    methodlookup(*simplename, *descriptor, *type_, *method) :-
            directsuperclass(type_, supertype),
            methodlookup(simplename, descriptor, supertype, method),
            !methodimplemented(simplename, descriptor, type_, _);
    methodlookup(*simplename, *descriptor, *type_, *method) :-
            directsuperinterface(type_, supertype),
            methodlookup(simplename, descriptor, supertype, method),
            !methodimplemented(simplename, descriptor, type_, _);
    methodimplemented(*simplename, *descriptor, *type_, *method) :-
            method_simplename(method, simplename),
            method_descriptor(method, descriptor),
            method_declaringtype(method, type_),
            !method_modifier(sym("abstract"), method);
    mainmethoddeclaration(*method) :-
            mainclass(type_),
            method_declaringtype(method, type_),
            if *method != sym("<java.util.prefs.Base64: void main(java.lang.String[])>"),
            if *method != sym("<sun.java2d.loops.GraphicsPrimitiveMgr: void main(java.lang.String[])>"),
            if *method != sym("<sun.security.provider.PolicyParser: void main(java.lang.String[])>"),
            method_simplename(method, sym("main")),
            method_descriptor(method, sym("void(java.lang.String[])")),
            method_modifier(sym("public"), method),
            method_modifier(sym("static"), method);

    directsubclass(*a, *c) :- directsuperclass(a, c);
    subclass(*c, *a) :- directsubclass(a, c);
    subclass(*c, *a) :-
            subclass(b, a),
            directsubclass(b, c);
    superclass(*c, *a) :- subclass(a, c);
    superinterface(*k, *c) :- directsuperinterface(c, k);
    superinterface(*k, *c) :-
            directsuperinterface(c, j),
            superinterface(k, j);
    superinterface(*k, *c) :-
            directsuperclass(c, super_),
            superinterface(k, super_);

    subtypeof(*s, *s) :- isclasstype(s);
    subtypeof(*t, *t) :- istype(t);
    subtypeof(*s, *t) :- subclass(t, s);
    subtypeof(*s, *s) :- isinterfacetype(s);
    subtypeof(*s, *t) :-
            isclasstype(s),
            superinterface(t, s);
    subtypeof(*s, *t) :-
            isinterfacetype(s),
            istype(t),
            if *t == sym("java.lang.Object");
    subtypeof(*s, *t) :-
            isarraytype(s),
            istype(t),
            if *t == sym("java.lang.Object");
    subtypeof(*s, *t) :-
            isinterfacetype(s),
            superinterface(t, s);
    subtypeof(*s, *t) :-
            subtypeof(sc, tc),
            componenttype(s, sc),
            componenttype(t, tc),
            isreferencetype(sc),
            isreferencetype(tc);
    subtypeof(*s, *t) :-
            isarraytype(s),
            isinterfacetype(t),
            istype(t),
            if *t == sym("java.lang.Cloneable");
    subtypeof(*s, *t) :-
            isarraytype(s),
            isinterfacetype(t),
            istype(t),
            if *t == sym("java.io.Serializable");

    supertypeof(*s, *t) :- subtypeof(t, s);
    subtypeofdifferent(*s, *t) :-
            subtypeof(s, t),
            if s != t;

        // class initialization
    classinitializer(*type_, *method) :-
            methodimplemented(sym("<clinit>"), sym("void()"), type_, method);
    initializedclass(*superclass_) :-
            initializedclass(class),
            directsuperclass(class, superclass_);
    initializedclass(*superinterface_) :-
            initializedclass(classorinterface),
            directsuperinterface(classorinterface, superinterface_);
    initializedclass(*class) :-
            mainmethoddeclaration(method),
            method_declaringtype(method, class);
    initializedclass(*class) :-
            reachable(inmethod),
            assignheapallocation(heap, _, inmethod),
            heapallocation_type(heap, class);
    initializedclass(*class) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            isstaticmethodinvocation_insn(invocation),
            methodinvocation_method(invocation, signature),
            method_declaringtype(signature, class);
    initializedclass(*classorinterface) :-
            reachable(inmethod),
            storestaticfield(_, signature, inmethod),
            field_declaringtype(signature, classorinterface);
    initializedclass(*classorinterface) :-
            reachable(inmethod),
            loadstaticfield(signature, _, inmethod),
            field_declaringtype(signature, classorinterface);
    reachable(*clinit) :-
            initializedclass(class),
            classinitializer(class, clinit);

        // Main (value-based) analysis
    assign(*actual, *formal) :-
            callgraphedge(invocation, method),
            formalparam(index, method, formal),
            actualparam(index, invocation, actual);
    assign(*ret, *local) :-
            callgraphedge(invocation, method),
            returnvar(ret, method),
            assignreturnvalue(invocation, local);
    varpointsto(*heap, *var) :-
            assignheapallocation(heap, var, inmethod),
            reachable(inmethod);
    varpointsto(*heap, *to) :-
            assign(from, to),
            varpointsto(heap, from);
    varpointsto(*heap, *to) :-
            reachable(inmethod),
            assignlocal(from, to, inmethod),
            varpointsto(heap, from);
    varpointsto(*heap, *to) :-
            reachable(inmethod),
            assigncast(type_, from, to, inmethod),
            supertypeof(type_, heaptype),
            heapallocation_type(heap, heaptype),
            varpointsto(heap, from);
    arrayindexpointsto(*baseheap, *heap) :-
            reachable(inmethod),
            storearrayindex(from, base, inmethod),
            varpointsto(baseheap, base),
            varpointsto(heap, from),
            heapallocation_type(heap, heaptype),
            heapallocation_type(baseheap, baseheaptype),
            componenttype(baseheaptype, componenttype_),
            supertypeof(componenttype_, heaptype);
    varpointsto(*heap, *to) :-
            reachable(inmethod),
            loadarrayindex(base, to, inmethod),
            varpointsto(baseheap, base),
            arrayindexpointsto(baseheap, heap),
            var_type(to, type_),
            heapallocation_type(baseheap, baseheaptype),
            componenttype(baseheaptype, basecomponenttype),
            supertypeof(type_, basecomponenttype);
    varpointsto(*heap, *to) :-
            reachable(inmethod),
            loadinstancefield(base, signature, to, inmethod),
            varpointsto(baseheap, base),
            instancefieldpointsto(heap, signature, baseheap);
    varpointsto(*heap, *to) :-
            reachable(inmethod),
            loadstaticfield(fld, to, inmethod),
            staticfieldpointsto(heap, fld);
    varpointsto(*heap, *this) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            virtualmethodinvocation_base(invocation, base),
            varpointsto(heap, base),
            heapallocation_type(heap, heaptype),
            virtualmethodinvocation_simplename(invocation, simplename),
            virtualmethodinvocation_descriptor(invocation, descriptor),
            methodlookup(simplename, descriptor, heaptype, tomethod),
            thisvar(tomethod, this);
    instancefieldpointsto(*heap, *fld, *baseheap) :-
            reachable(inmethod),
            storeinstancefield(from, base, fld, inmethod),
            varpointsto(heap, from),
            varpointsto(baseheap, base);
    staticfieldpointsto(*heap, *fld) :-
            reachable(inmethod),
            storestaticfield(from, fld, inmethod),
            varpointsto(heap, from);

        // Reachable(toMethod), CallGraphEdge(invocation, toMethod)
    reachable(*tomethod) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            virtualmethodinvocation_base(invocation, base),
            varpointsto(heap, base),
            heapallocation_type(heap, heaptype),
            virtualmethodinvocation_simplename(invocation, simplename),
            virtualmethodinvocation_descriptor(invocation, descriptor),
            methodlookup(simplename, descriptor, heaptype, tomethod);
    callgraphedge(*invocation, *tomethod) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            virtualmethodinvocation_base(invocation, base),
            varpointsto(heap, base),
            heapallocation_type(heap, heaptype),
            virtualmethodinvocation_simplename(invocation, simplename),
            virtualmethodinvocation_descriptor(invocation, descriptor),
            methodlookup(simplename, descriptor, heaptype, tomethod);

    reachable(*tomethod) :-
            reachable(inmethod),
            staticmethodinvocation(invocation, tomethod, inmethod);
    callgraphedge(*invocation, *tomethod) :-
            reachable(inmethod),
            staticmethodinvocation(invocation, tomethod, inmethod);

    reachable(*tomethod) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            specialmethodinvocation_base(invocation, base),
            varpointsto(heap, base),
            methodinvocation_method(invocation, tomethod),
            thisvar(tomethod, this);
    callgraphedge(*invocation, *tomethod) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            specialmethodinvocation_base(invocation, base),
            varpointsto(heap, base),
            methodinvocation_method(invocation, tomethod),
            thisvar(tomethod, this);
    varpointsto(*heap, *this) :-
            reachable(inmethod),
            instruction_method(invocation, inmethod),
            specialmethodinvocation_base(invocation, base),
            varpointsto(heap, base),
            methodinvocation_method(invocation, tomethod),
            thisvar(tomethod, this);

    reachable(*method) :- mainmethoddeclaration(method);
        };
    Results {
        mainclass: mainclass.collect(),
        method_simplename: method_simplename.collect(),
        method_declaringtype: method_declaringtype.collect(),
        method_modifier: method_modifier.collect(),
        method_paramtypes: method_paramtypes.collect(),
        method_returntype: method_returntype.collect(),
        method_descriptor: method_descriptor.collect(),
        methodlookup: methodlookup.collect(),
        methodimplemented: methodimplemented.collect(),
        directsubclass: directsubclass.collect(),
        subclass: subclass.collect(),
        superclass: superclass.collect(),
        superinterface: superinterface.collect(),
        subtypeof: subtypeof.collect(),
        supertypeof: supertypeof.collect(),
        subtypeofdifferent: subtypeofdifferent.collect(),
        mainmethoddeclaration: mainmethoddeclaration.collect(),
        classinitializer: classinitializer.collect(),
        initializedclass: initializedclass.collect(),
        assign: assign.collect(),
        varpointsto: varpointsto.collect(),
        instancefieldpointsto: instancefieldpointsto.collect(),
        staticfieldpointsto: staticfieldpointsto.collect(),
        callgraphedge: callgraphedge.collect(),
        arrayindexpointsto: arrayindexpointsto.collect(),
        reachable: reachable.collect(),
    }
}
