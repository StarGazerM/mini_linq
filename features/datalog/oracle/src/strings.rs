use crate::Data;
use std::collections::BTreeSet;

pub fn check(data: &Data) {
    // Include Unicode, punctuation, and empty strings; no integer coercion.
    let label = |x: i32| {
        if x == 0 {
            String::new()
        } else {
            format!("node,{x}:λ")
        }
    };
    let edge: Vec<_> = data
        .edge
        .iter()
        .map(|&(x, y)| (label(x), label(y)))
        .collect();
    let seed: Vec<_> = data.seed.iter().map(|&(x,)| (label(x),)).collect();
    let oracle = ascent::ascent_run! {
        relation edge(String, String) = edge.clone();
        relation seed(String) = seed.clone();
        relation answer(String);
        answer(x.clone()) <-- seed(x);
        answer(y.clone()) <-- answer(x), edge(x,y);
    };
    let mini: BTreeSet<_> = cq_datalog::datalog! {
            struct Owned;
            relation edge(x:String,y:String) = (edge);
            relation seed(x:String) = (seed);
            relation answer(x:String);
            output answer;
            answer((*x).clone()) :- seed(x);
            answer((*y).clone()) :- answer(x), edge(x,y);
    }
    .collect();
    assert_eq!(mini, oracle.answer.into_iter().collect());
    let naive: BTreeSet<_> = crate::naive("reach", data)
        .unwrap()
        .into_iter()
        .map(|row| (label(row[0]),))
        .collect();
    assert_eq!(mini, naive);
    check_constants(data);
}

// Differential-only cases extend the scenario/random matrices without changing
// the integer CSV corpus or pretending a heterogeneous tuple is one relation.
fn check_constants(data: &Data) {
    let label = |x: i32| {
        if x == 0 {
            String::new()
        } else {
            format!("method,{x}:λ")
        }
    };
    let methods: Vec<_> = data
        .nodes
        .iter()
        .map(|&(x,)| {
            (
                label(x),
                if x % 2 == 0 {
                    String::from("void")
                } else {
                    String::from("int")
                },
                if x % 3 == 0 {
                    String::from("String[]")
                } else {
                    String::new()
                },
            )
        })
        .collect();
    let edges: Vec<_> = data
        .edge
        .iter()
        .map(|&(x, y)| (label(x), label(y)))
        .collect();
    let mut modifiers: Vec<_> = data
        .nodes
        .iter()
        .filter(|&&(x,)| x % 2 == 0)
        .map(|&(x,)| (String::from("public"), label(x), 1_i32))
        .collect();
    modifiers.extend(
        data.extra
            .iter()
            .map(|&(x,)| (String::from("abstract"), label(x), 7)),
    );
    modifiers.extend(
        data.seed
            .iter()
            .map(|&(x,)| (String::from("static"), label(x), 2)),
    );
    let oracle = ascent::ascent_run! {
        relation method(String,String,String) = methods.clone();
        relation edge(String,String) = edges.clone();
        relation modifier(String,String,i32) = modifiers.clone();
        relation concrete(String);
        relation reach(String);
        relation descriptor(String,String);
        relation main_method(String);
        relation no_static(String);
        relation absent(String);
        relation present(String);
        relation literal(String,String,i32,bool);
        concrete(type_.clone()) <-- method(type_,_return_type,_parameters), !modifier("abstract".to_owned(),type_,_);
        reach(type_.clone()) <-- modifier("public".to_owned(),type_,1), !modifier("abstract".to_owned(),type_,_);
        reach(super_.clone()) <-- reach(type_), edge(type_,super_), !modifier("abstract".to_owned(),super_,_);
        descriptor(type_.clone(),format!("{}({})",ret,params)) <-- concrete(type_), method(type_,ret,params);
        main_method(type_.clone()) <-- method(type_,"void".to_owned(),"String[]".to_owned());
        no_static(type_.clone()) <-- method(type_,_return_type,_parameters), !modifier("static".to_owned(),type_,_);
        absent(type_.clone()) <-- method(type_,_return_type,_parameters), !modifier("missing".to_owned(),_,_), !modifier("abstract".to_owned(),"absent".to_owned(),7);
        present(type_.clone()) <-- method(type_,_return_type,_parameters), !modifier("static".to_owned(),"".to_owned(),2);
        literal(type_.clone(),"java.lang.String".to_owned(),-7,true) <-- concrete(type_);
    };
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
        relation Method(name:String,result:String,params:String) = (methods);
        relation Edge(caller:String,callee:String) = (edges);
        relation Modifier(kind:String,name:String,flag:i32) = (modifiers);
        relation Concrete(name:String);
        relation Reach(name:String);
        relation Descriptor(name:String,text:String);
        relation MainMethod(name:String);
        relation NoStatic(name:String);
        relation Absent(name:String);
        relation Present(name:String);
        relation Literal(name:String,text:String,flag:i32,enabled:bool);
        output Descriptor, Reach, Concrete, MainMethod, NoStatic, Absent, Present, Literal, Method, Modifier;
        Concrete((*type_).clone()) :- Method(type_,_,_), !Modifier("abstract",type_,_);
        Reach((*type_).clone()) :- Modifier("public",type_,1), !Modifier("abstract",type_,_);
        Reach((*super_).clone()) :- Reach(type_), Edge(type_,super_), !Modifier("abstract",super_,_);
        Descriptor((*type_).clone(),format!("{}({})",ret,params)) :- Concrete(type_), Method(type_,ret,params);
        MainMethod((*type_).clone()) :- Method(type_,"void","String[]");
        NoStatic((*type_).clone()) :- Method(type_,_,_), !Modifier(String::from("static"),type_,_);
        Absent((*type_).clone()) :- Method(type_,_,_), !Modifier("missing",_,_), !Modifier("abstract","absent",7);
        Present((*type_).clone()) :- Method(type_,_,_), !Modifier("static","",2);
        Literal((*type_).clone(),"java.lang.String".to_owned(),-7,true) :- Concrete(type_);
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
    assert_eq!(
        method.collect::<BTreeSet<_>>(),
        oracle.method.into_iter().collect()
    );
    assert_eq!(
        modifier.collect::<BTreeSet<_>>(),
        oracle.modifier.into_iter().collect()
    );
}
