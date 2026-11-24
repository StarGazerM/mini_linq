use crate::Data;
use cq_load::{StringCode, StringDictionary};
use cq_storage::domain::exact;
use std::collections::BTreeSet;

/// An owned, non-Copy label; the empty label is distinct from every node.
fn owned(x: i32) -> Vec<i64> {
    if x == 0 {
        Vec::new()
    } else {
        vec![i64::from(x), -1]
    }
}

pub fn check(data: &Data) {
    let edge: Vec<_> = data
        .edge
        .iter()
        .map(|&(x, y)| (owned(x), owned(y)))
        .collect();
    let seed: Vec<_> = data.seed.iter().map(|&(x,)| (owned(x),)).collect();
    let oracle = ascent::ascent_run! {
        relation edge(Vec<i64>, Vec<i64>) = edge.clone();
        relation seed(Vec<i64>) = seed.clone();
        relation answer(Vec<i64>);
        answer(x.clone()) <-- seed(x);
        answer(y.clone()) <-- answer(x), edge(x,y);
    };
    let mini: BTreeSet<_> = cq_datalog::datalog! {
            struct Owned;
            relation edge(x:Vec<i64>,y:Vec<i64>) = (edge);
            relation seed(x:Vec<i64>) = (seed);
            relation answer(x:Vec<i64>);
            output answer;
            answer((*x).clone()) :- seed(x);
            answer((*y).clone()) :- answer(x), edge(x,y);
    }
    .collect();
    assert_eq!(mini, oracle.answer.into_iter().collect());
    let naive: BTreeSet<_> = crate::naive("reach", data)
        .unwrap()
        .into_iter()
        .map(|row| (owned(row[0]),))
        .collect();
    assert_eq!(mini, naive);
    check_constants(data);
    check_canonical_inputs(data);
    check_delta_runs(data);
}

/// Codes of the string constants written in `check_constants`, for the Ascent
/// mirror (Ascent has no code domains: its constants are host codes).
struct Literals {
    void: StringCode,
    string_array: StringCode,
    public: StringCode,
    abstract_: StringCode,
    static_: StringCode,
    missing: StringCode,
    absent: StringCode,
    empty: StringCode,
    java_string: StringCode,
}

// Differential-only cases extend the scenario/random matrices without changing
// the integer CSV corpus or pretending a heterogeneous tuple is one relation.
// String columns are dictionary codes: one domain per compared column family
// (method names, which join across relations; signature types; modifier
// kinds). Labels include Unicode, punctuation, and the empty string;
// "missing" and "absent" are absent from their domains.
fn check_constants(data: &Data) {
    let label = |x: i32| {
        if x == 0 {
            String::new()
        } else {
            format!("method,{x}:λ")
        }
    };
    let labels: Vec<_> = data
        .nodes
        .iter()
        .chain(&data.extra)
        .chain(&data.seed)
        .map(|&(x,)| x)
        .chain(data.edge.iter().flat_map(|&(x, y)| [x, y]))
        .map(label)
        .collect();
    let names = StringDictionary::new(labels.iter().map(String::as_str));
    let types = StringDictionary::new(["void", "int", "String[]", ""]);
    let kinds = StringDictionary::new(["public", "abstract", "static"]);
    let texts = StringDictionary::new(["java.lang.String"]);
    let name = |x: i32| exact(&names, &label(x));
    let c = Literals {
        void: exact(&types, "void"),
        string_array: exact(&types, "String[]"),
        public: exact(&kinds, "public"),
        abstract_: exact(&kinds, "abstract"),
        static_: exact(&kinds, "static"),
        missing: exact(&kinds, "missing"),
        absent: exact(&names, "absent"),
        empty: exact(&names, ""),
        java_string: exact(&texts, "java.lang.String"),
    };
    let methods: Vec<_> = data
        .nodes
        .iter()
        .map(|&(x,)| {
            (
                name(x),
                exact(&types, if x % 2 == 0 { "void" } else { "int" }),
                exact(&types, if x % 3 == 0 { "String[]" } else { "" }),
            )
        })
        .collect();
    let edges: Vec<_> = data.edge.iter().map(|&(x, y)| (name(x), name(y))).collect();
    let mut modifiers: Vec<_> = data
        .nodes
        .iter()
        .filter(|&&(x,)| x % 2 == 0)
        .map(|&(x,)| (c.public, name(x), 1_i32))
        .collect();
    modifiers.extend(data.extra.iter().map(|&(x,)| (c.abstract_, name(x), 7)));
    modifiers.extend(data.seed.iter().map(|&(x,)| (c.static_, name(x), 2)));
    let oracle = ascent::ascent_run! {
        relation method(StringCode,StringCode,StringCode) = methods.clone();
        relation edge(StringCode,StringCode) = edges.clone();
        relation modifier(StringCode,StringCode,i32) = modifiers.clone();
        relation concrete(StringCode);
        relation reach(StringCode);
        relation descriptor(StringCode,Vec<StringCode>);
        relation main_method(StringCode);
        relation no_static(StringCode);
        relation absent(StringCode);
        relation present(StringCode);
        relation literal(StringCode,StringCode,i32,bool);
        concrete(*type_) <-- method(type_,_return_type,_parameters), !modifier(c.abstract_,type_,_);
        reach(*type_) <-- modifier(c.public,type_,1), !modifier(c.abstract_,type_,_);
        reach(*super_) <-- reach(type_), edge(type_,super_), !modifier(c.abstract_,super_,_);
        descriptor(*type_,vec![*ret,*params]) <-- concrete(type_), method(type_,ret,params);
        main_method(*type_) <-- method(type_,c.void,c.string_array);
        no_static(*type_) <-- method(type_,_return_type,_parameters), !modifier(c.static_,type_,_);
        absent(*type_) <-- method(type_,_return_type,_parameters), !modifier(c.missing,_,_), !modifier(c.abstract_,c.absent,7);
        present(*type_) <-- method(type_,_return_type,_parameters), !modifier(c.static_,c.empty,2);
        literal(*type_,c.java_string,-7,true) <-- concrete(type_);
    };
    let java_string = c.java_string;
    let (
        descriptor,
        reach,
        concrete,
        main_method,
        no_static,
        absent,
        present,
        literal,
        method,
        modifier,
    ) = cq_datalog::datalog! {
        struct Constants;
        relation Method(name:cq_load::StringCode in (names),result:cq_load::StringCode in (types),params:cq_load::StringCode in (types)) = (methods);
        relation Edge(caller:cq_load::StringCode in (names),callee:cq_load::StringCode in (names)) = (edges);
        relation Modifier(kind:cq_load::StringCode in (kinds),name:cq_load::StringCode in (names),flag:i32) = (modifiers);
        relation Concrete(name:cq_load::StringCode in (names));
        relation Reach(name:cq_load::StringCode in (names));
        relation Descriptor(name:cq_load::StringCode in (names),signature:Vec<cq_load::StringCode>);
        relation MainMethod(name:cq_load::StringCode in (names));
        relation NoStatic(name:cq_load::StringCode in (names));
        relation Absent(name:cq_load::StringCode in (names));
        relation Present(name:cq_load::StringCode in (names));
        relation Literal(name:cq_load::StringCode in (names),text:cq_load::StringCode,flag:i32,enabled:bool);
        output Descriptor, Reach, Concrete, MainMethod, NoStatic, Absent, Present, Literal, Method, Modifier;
        Concrete(*type_) :- Method(type_,_,_), !Modifier("abstract",type_,_);
        Reach(*type_) :- Modifier("public",type_,1), !Modifier("abstract",type_,_);
        Reach(*super_) :- Reach(type_), Edge(type_,super_), !Modifier("abstract",super_,_);
        Descriptor(*type_,vec![*ret,*params]) :- Concrete(type_), Method(type_,ret,params);
        MainMethod(*type_) :- Method(type_,"void","String[]");
        NoStatic(*type_) :- Method(type_,_,_), !Modifier(String::from("static"),type_,_);
        Absent(*type_) :- Method(type_,_,_), !Modifier("missing",_,_), !Modifier("abstract","absent",7);
        Present(*type_) :- Method(type_,_,_), !Modifier("static","",2);
        Literal(*type_,{ java_string },-7,true) :- Concrete(type_);
    };
    assert_eq!(
        descriptor.collect::<BTreeSet<_>>(),
        oracle.descriptor.into_iter().collect()
    );
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        oracle.reach.into_iter().collect()
    );
    assert_eq!(
        concrete.collect::<BTreeSet<_>>(),
        oracle.concrete.into_iter().collect()
    );
    assert_eq!(
        main_method.collect::<BTreeSet<_>>(),
        oracle.main_method.into_iter().collect()
    );
    assert_eq!(
        no_static.collect::<BTreeSet<_>>(),
        oracle.no_static.into_iter().collect()
    );
    assert_eq!(
        absent.collect::<BTreeSet<_>>(),
        oracle.absent.into_iter().collect()
    );
    assert_eq!(
        present.collect::<BTreeSet<_>>(),
        oracle.present.into_iter().collect()
    );
    assert_eq!(
        literal.collect::<BTreeSet<_>>(),
        oracle.literal.into_iter().collect()
    );
    assert_set(method, oracle.method);
    assert_set(modifier, oracle.modifier);
}

fn assert_set<T: Ord + std::fmt::Debug>(
    actual: impl Iterator<Item = T>,
    expected: impl IntoIterator<Item = T>,
) {
    let actual: Vec<_> = actual.collect();
    let expected: BTreeSet<_> = expected.into_iter().collect();
    assert_eq!(actual.len(), expected.len(), "duplicate output tuples");
    assert_eq!(actual.into_iter().collect::<BTreeSet<_>>(), expected);
}

// Every scenario (including empty and random graphs) exercises duplicate grouped
// inputs, several EDB layouts, projected negation, recursion, and EDB outputs
// over owned non-Copy columns.
fn check_canonical_inputs(data: &Data) {
    let label = |x: i32| vec![i64::from(x), -2];
    let edge: Vec<_> = data
        .edge
        .iter()
        .flat_map(|&(x, y)| std::iter::repeat_n((label(x), label(y)), 3))
        .collect();
    let nodes: Vec<_> = data
        .nodes
        .iter()
        .flat_map(|&(x,)| std::iter::repeat_n((label(x),), 2))
        .collect();
    let mut grouped = std::collections::BTreeMap::<_, Vec<_>>::new();
    for (x, y) in &edge {
        grouped.entry((x.clone(),)).or_default().push((y.clone(),));
    }
    let oracle = ascent::ascent_run! {
        relation edge(Vec<i64>,Vec<i64>) = edge;
        relation node(Vec<i64>) = nodes.clone();
        relation paths(Vec<i64>,Vec<i64>);
        relation shared(Vec<i64>,Vec<i64>);
        relation missing(Vec<i64>);
        relation reach(Vec<i64>,Vec<i64>);
        paths(x.clone(),z.clone()) <-- edge(x,y), edge(y,z);
        shared(x.clone(),z.clone()) <-- edge(x,y), edge(z,y);
        missing(x.clone()) <-- node(x), !edge(x,_);
        reach(x.clone(),y.clone()) <-- edge(x,y);
        reach(x.clone(),z.clone()) <-- reach(x,y), edge(y,z);
    };
    let (facts, domain, paths, shared, missing, reach) = cq_datalog::datalog! {
        struct CanonicalInputs;
        relation E(x:Vec<i64>,y:Vec<i64>) = (&grouped);
        relation D(x:Vec<i64>) = (&nodes);
        relation Paths(x:Vec<i64>,z:Vec<i64>);
        relation Shared(x:Vec<i64>,z:Vec<i64>);
        relation Missing(x:Vec<i64>);
        relation Reach(x:Vec<i64>,y:Vec<i64>);
        output E, D, Paths, Shared, Missing, Reach;
        Paths((*x).clone(),(*z).clone()) :- E(x,y), E(y,z);
        Shared((*x).clone(),(*z).clone()) :- E(x,y), E(z,y);
        Missing((*x).clone()) :- D(x), !E(x,_);
        Reach((*x).clone(),(*y).clone()) :- E(x,y);
        Reach((*x).clone(),(*z).clone()) :- Reach(x,y), E(y,z);
    };
    assert_set(facts, oracle.edge);
    assert_set(domain, oracle.node);
    assert_set(paths, oracle.paths);
    assert_set(shared, oracle.shared);
    assert_set(missing, oracle.missing);
    assert_set(reach, oracle.reach);
    let single = cq_datalog::datalog! {
        struct Facts;
        relation E(x:Vec<i64>,y:Vec<i64>) = (&grouped);
        output E;
    };
    assert_set(
        single,
        grouped
            .into_iter()
            .flat_map(|((x,), values)| values.into_iter().map(move |(y,)| (x.clone(), y))),
    );
}

// The same per-round non-Copy-keyed run serves nonlinear and linear holes,
// alongside reversed keys and a later zero-key delta in a mutual SCC.
fn check_delta_runs(data: &Data) {
    let label = |x: i32| vec![i64::from(x), -3];
    let edges: Vec<_> = data
        .edge
        .iter()
        .flat_map(|&(x, y)| [0_i32, 1, 1].map(|divisor| (label(x), label(y), divisor)))
        .collect();
    let flags = vec![(7_i32,), (7,)];
    let oracle = ascent::ascent_run! {
        relation edge(Vec<i64>,Vec<i64>,i32) = edges.clone();
        relation flag_input(i32) = flags.clone();
        relation reach(Vec<i64>,Vec<i64>);
        relation flag(i32);
        reach(x.clone(),y.clone()) <-- edge(x,y,_divisor);
        reach(x.clone(),z.clone()) <-- reach(x,y), reach(y,z);
        reach(x.clone(),z.clone()) <-- edge(x,y,d), if *d != 0, if 8 / *d > 0, reach(y,z);
        reach(x.clone(),z.clone()) <-- edge(y,z,d), if *d != 0, if 8 / *d > 0, reach(x,y);
        reach(x.clone(),y.clone()) <-- edge(x,y,_divisor), reach(x,y);
        reach(x.clone(),y.clone()) <-- flag(f), if *f == 7, reach(x,y);
        flag(*f) <-- flag_input(f), reach(_x,_y);
    };
    let (reach, flags) = cq_datalog::datalog! {
        struct DeltaRuns;
        relation Edge(x:Vec<i64>,y:Vec<i64>,divisor:i32) = (&edges);
        relation FlagInput(flag:i32) = (&flags);
        relation Reach(x:Vec<i64>,y:Vec<i64>);
        relation Flag(flag:i32);
        output Reach, Flag;
        Reach((*x).clone(),(*y).clone()) :- Edge(x,y,_);
        Reach((*x).clone(),(*z).clone()) :- Reach(x,y), Reach(y,z);
        Reach((*x).clone(),(*z).clone()) :-
            Edge(x,y,d), where d != 0, if 8 / *d > 0, Reach(y,z);
        Reach((*x).clone(),(*z).clone()) :-
            Edge(y,z,d), where d != 0, if 8 / *d > 0, Reach(x,y);
        Reach((*x).clone(),(*y).clone()) :- Edge(x,y,_), Reach(x,y);
        Reach((*x).clone(),(*y).clone()) :- Flag(f), where f == 7, Reach(x,y);
        Flag(*f) :- FlagInput(f), Reach(x,y);
    };
    assert_set(reach, oracle.reach);
    assert_set(flags, oracle.flag);
}
