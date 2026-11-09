use std::collections::{BTreeMap, BTreeSet};

use cq_cost::cost::{Candidate, IndexVersion};
use cq_cost::cost::enumerate::enumerate;
use cq_cost::export::{self, ClauseKind, Configuration, Origin, PlannerInput, Target, Version};
use cq_syntax::symbol_name;

fn configuration() -> Configuration {
    Configuration { compiler_hash: export::sha256("enumeration test compiler"), target: Target::Cpu,
        invocation: "checked source-prefix enumeration fixture".into() }
}
fn cq(source: &str) -> PlannerInput {
    export::compile_cq(source, &Default::default(), configuration()).unwrap()
}
fn candidates(input: &PlannerInput) -> Vec<Candidate> {
    enumerate(input, &input.holes[0]).unwrap()
}
fn orders(candidates: &[Candidate]) -> BTreeSet<Vec<usize>> {
    candidates.iter().map(|candidate| candidate.order.clone()).collect()
}
fn one_current(candidates: &[Candidate]) -> &Candidate {
    let current: Vec<_> = candidates.iter().filter(|candidate| candidate.current).collect();
    assert_eq!(current.len(), 1, "exactly the checked incumbent must be marked");
    current[0]
}

// An independent oracle: generate EVERY permutation first, then reject I2/I3
// violations. It deliberately does not implement the enumerator's choice DFS.
fn permutations(count: usize) -> Vec<Vec<usize>> {
    fn permute(values: &mut [usize], position: usize, result: &mut Vec<Vec<usize>>) {
        if position == values.len() { result.push(values.to_vec()); return; }
        for next in position..values.len() {
            values.swap(position, next);
            permute(values, position + 1, result);
            values.swap(position, next);
        }
    }
    let mut result = Vec::new();
    permute(&mut (0..count).collect::<Vec<_>>(), 0, &mut result);
    result
}
fn oracle(atoms: &[Vec<&str>]) -> BTreeSet<Vec<usize>> {
    permutations(atoms.len()).into_iter().filter(|order| {
        let mut bound = BTreeSet::new();
        for (position, &next) in order.iter().enumerate() {
            let pending = &order[position..];
            let filter = pending.iter().copied().filter(|&atom| atoms[atom].iter().all(|name| bound.contains(name)))
                .min();
            if let Some(filter) = filter {
                if next != filter { return false; }
            } else if !atoms[next].iter().any(|name| bound.contains(name))
                && pending.iter().any(|&atom| atoms[atom].iter().any(|name| bound.contains(name)))
            { return false; }
            bound.extend(atoms[next].iter().copied());
        }
        true
    }).collect()
}
fn atom_query(atoms: &[Vec<&str>]) -> String {
    let declarations = atoms.iter().enumerate().map(|(index, variables)| {
        format!("relation R{index}({}) = (r{index});", (0..variables.len())
            .map(|column| format!("c{column}:i32")).collect::<Vec<_>>().join(","))
    }).collect::<Vec<_>>().join("\n");
    let body = atoms.iter().enumerate().map(|(index, variables)| format!("R{index}({})", variables.join(",")))
        .collect::<Vec<_>>().join(",");
    let first = atoms[0][0];
    format!("struct Query; {declarations} Query({first}) :- {body};")
}

#[test]
fn complete_small_candidate_sets_equal_independent_permutations() {
    for atoms in [
        vec![vec!["x"], vec!["y"], vec!["z"]],
        vec![vec!["x", "y"], vec!["y", "z"], vec!["z", "w"]],
        vec![vec!["x", "y"], vec!["x"], vec!["y"], vec!["z"]],
        vec![vec!["x", "y"], vec!["y", "z"], vec!["z", "x"]],
        vec![vec!["a", "b"], vec!["c", "d"], vec!["b", "c"], vec!["a", "d"]],
    ] {
        let input = cq(&atom_query(&atoms));
        let actual = candidates(&input);
        assert_eq!(orders(&actual), oracle(&atoms), "{atoms:?}");
        assert_eq!(actual.len(), orders(&actual).len(), "no factorial filter duplicates");
        one_current(&actual);
        for candidate in actual {
            assert!(candidate.unbounded.is_empty());
            // Final legality belongs to production, not the enumeration oracle.
            let normal: cq_lang_cq::Program = syn::parse_str(&input.holes[0].normalized_cq).unwrap();
            let selectors = candidate.order.iter().map(|source| {
                let read = candidate.steps.iter().filter_map(|step| step.read.as_ref())
                    .find(|read| read.source_occurrence == *source).unwrap();
                format!("{}@{}", read.input_relation, read.normalized_clause)
            }).collect::<Vec<_>>().join(",");
            let options = syn::parse_str(&format!("order({selectors})")).unwrap();
            let (normal, realized) = cq_planning::prefix_valuation(&normal, &options).unwrap();
            cq_lang_vo::contract::check(&normal, &realized).unwrap();
        }
    }
}

#[test]
fn field_flow_like_triangle_has_all_six_orders_and_new_key_demands() {
    let input = cq(&atom_query(&[vec!["a", "b"], vec!["b", "c"], vec!["a", "c"]]));
    let actual = candidates(&input);
    assert_eq!(orders(&actual), permutations(3).into_iter().collect());
    assert_eq!(actual.len(), 6);
    let reverse = actual.iter().find(|candidate| candidate.order == [2, 1, 0]).unwrap();
    assert!(reverse.demands.iter().any(|demand| demand.logical_relation == "R1" && demand.key == [1]));
    assert!(reverse.demands.iter().any(|demand| demand.logical_relation == "R0" && demand.key == [0, 1]));
}

#[test]
fn eight_independent_atoms_are_not_silently_truncated() {
    let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let atoms: Vec<_> = names.iter().map(|name| vec![*name]).collect();
    let input = cq(&atom_query(&atoms));
    let actual = candidates(&input);
    assert_eq!(actual.len(), 40_320);
    assert_eq!(orders(&actual), permutations(8).into_iter().collect());
    one_current(&actual);
}

#[test]
fn every_ready_filter_drains_before_a_ready_introducing_let() {
    let input = cq(r#"
        struct Q;
        relation R(x:i32,y:i32) = (r);
        relation S(x:i32) = (s);
        relation T(y:i32) = (t);
        Q(z) :- R(x,y), S(x), T(y), let z:i32 = *x + 1;
    "#);
    let actual = candidates(&input);
    let root = actual.iter().find(|candidate| candidate.order.first() == Some(&0)).unwrap();
    let binding = root.steps.iter().position(|step| step.kind == ClauseKind::OpaqueLet).unwrap();
    assert_eq!(root.order, [0, 1, 2]);
    assert!(root.steps.iter().enumerate().filter(|(_, step)| step.read.is_some()).all(|(index, _)| index < binding));
    assert_eq!(root.steps[1].bound_before, ["x", "y"]);
    assert!(!root.steps[1].introduces);
    assert!(!root.steps[2].introduces);
    for candidate in &actual {
        let mut pending: BTreeSet<_> = (0..input.holes[0].candidate_reference.steps.len()).collect();
        let mut bound = BTreeSet::new();
        for step in &candidate.steps {
            assert!(step.required.iter().all(|name| bound.contains(name)), "I1");
            if step.introduces {
                for clause in pending.iter().copied().filter(|clause| *clause != step.clause) {
                    let facts = &input.holes[0].candidate_reference.steps[clause];
                    assert!(!facts.required.iter().all(|name| bound.contains(name))
                        || facts.provides.iter().any(|name| !bound.contains(name)), "I2 before let");
                }
            }
            pending.remove(&step.clause);
            bound.extend(step.provides.iter().cloned());
        }
    }
}

#[test]
fn introducing_let_keeps_structural_priority_and_atom_readiness() {
    let input = cq(r#"
        struct Q;
        relation R(x:i32) = (r);
        relation S(k:i32,y:i32) = (s);
        relation T(y:i32) = (t);
        Q(y) :- let k:i32 = 4, R(x), S(k,y), T(y);
    "#);
    let actual = candidates(&input);
    // k is also an atom binder, so normalization cannot turn its let into a capture.
    for candidate in &actual {
        assert_eq!(candidate.steps[0].kind, ClauseKind::OpaqueLet);
        let read = candidate.steps.iter().find(|step| step.read.is_some()).unwrap();
        assert_eq!(read.read.as_ref().unwrap().logical_relation, "S", "I3 sees the let-provided key");
        assert_eq!(read.bound_before, ["k"]);
        assert!(read.required.contains(&"k".to_owned()));
    }
}

#[test]
fn opaque_barrier_limits_advice_to_the_first_source_positive_prefix() {
    let input = cq(r#"
        struct Q;
        relation R(x:i32,y:i32) = (r);
        relation S(y:i32,z:i32) = (s);
        relation T(z:i32,w:i32) = (t);
        relation U(w:i32,q:i32) = (u);
        Q(q) :- R(x,y), S(y,z), if opaque!(x,z), T(z,w), U(w,q);
    "#);
    let actual = candidates(&input);
    assert_eq!(orders(&actual), BTreeSet::from([vec![0, 1], vec![1, 0]]));
    for candidate in actual {
        let guard = candidate.steps.iter().position(|step| step.kind == ClauseKind::OpaqueGuard).unwrap();
        let later = candidate.steps.iter().position(|step| step.read.as_ref().is_some_and(|read| read.logical_relation == "T")).unwrap();
        assert!(guard < later);
        assert!(candidate.steps[guard].required.contains(&"x".to_owned()));
        assert!(candidate.steps[guard].required.contains(&"z".to_owned()));
    }
}

fn delta_first(source: &str) -> PlannerInput {
    let original = syn::parse_str(source).unwrap();
    let strata = cq_datalog_stratify::compile(&original).unwrap();
    let incremental = cq_datalog_incremental::compile_with_driver(&strata, cq_datalog_incremental::DriverPolicy::DeltaFirst).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let program = cq_datalog_fork::compile(&storage).unwrap();
    let plans = program.holes.iter().map(|hole| {
        let pipeline = hole.invocation_pipeline().unwrap();
        let options = syn::parse2(pipeline.passes[hole.plan.unwrap()].arguments.clone()).unwrap();
        cq_planning::prefix_valuation(&hole.cq, &options).unwrap()
    }).collect::<Vec<_>>();
    export::export_datalog(source, &program, &plans, configuration()).unwrap()
}
const CLOSURE: &str = r#"
    struct Closure;
    relation Edge(x:i32,y:i32) = (edges);
    relation Reach(x:i32,y:i32);
    output Reach;
    Seed: Reach(*x,*y) :- Edge(x,y);
    Transitive: Reach(*x,*z) :- Reach(x,y), Reach(y,z);
"#;

#[test]
fn repeated_relations_keep_original_delta_and_source_permutation() {
    let input = delta_first(CLOSURE);
    let mut seen = BTreeSet::new();
    for hole in &input.holes {
        let Origin::Rule { original_delta: Some(delta), permutation, .. } = &hole.origin else { continue; };
        seen.insert(*delta);
        assert_eq!(permutation[0], *delta);
        let actual = enumerate(&input, hole).unwrap();
        assert_eq!(orders(&actual), BTreeSet::from([vec![0, 1], vec![1, 0]]));
        let current = one_current(&actual);
        assert_eq!(current.order[0], *delta);
        for candidate in actual {
            for step in &candidate.steps {
                let Some(read) = &step.read else { continue; };
                assert_eq!(read.logical_relation, "Reach");
                assert_eq!(read.version == Version::Delta, read.source_occurrence == *delta);
                let occurrence = hole.occurrences.iter().find(|record| record.normalized_path == [step.clause]).unwrap();
                assert_eq!(read.source_occurrence, occurrence.source_path[0]);
                assert_eq!(read.columns.iter().map(|column| column.source_column).collect::<Vec<_>>(), occurrence.columns);
                assert_eq!(read.columns.len(), 2);
            }
            if candidate.order[0] == *delta {
                assert!(!candidate.demands.iter().any(|demand| demand.version == IndexVersion::Delta));
            } else {
                assert!(candidate.demands.iter().any(|demand| demand.version == IndexVersion::Full && demand.key.is_empty()));
                assert!(candidate.demands.iter().any(|demand| demand.version == IndexVersion::Delta && !demand.key.is_empty()));
            }
        }
    }
    assert_eq!(seen, BTreeSet::from([0, 1]));
}

#[test]
fn reordered_layout_columns_demand_original_logical_keys() {
    let input = delta_first(CLOSURE);
    let hole = input.holes.iter().find(|hole| matches!(hole.origin, Origin::Rule { original_delta: Some(1), .. })).unwrap();
    let actual = enumerate(&input, hole).unwrap();
    let candidate = actual.iter().find(|candidate| candidate.order == [1, 0]).unwrap();
    let read = candidate.steps.iter().filter_map(|step| step.read.as_ref()).find(|read| read.source_occurrence == 0).unwrap();
    let columns: BTreeMap<_, _> = read.columns.iter().map(|column| (column.source_column, column.variable.as_deref())).collect();
    assert_eq!(columns[&0], Some("x"));
    assert_eq!(columns[&1], Some("y"));
    assert!(candidate.demands.iter().any(|demand| demand.version == IndexVersion::Full && demand.key == [1]));
    let normal: cq_lang_cq::Program = syn::parse_str(&hole.normalized_cq).unwrap();
    let declaration = normal.inputs().find(|declaration| symbol_name(&declaration.name) == read.input_relation).unwrap();
    assert_eq!(read.columns.iter().map(|column| column.input_column.clone()).collect::<Vec<_>>(),
        declaration.columns.iter().map(|column| symbol_name(&column.name)).collect::<Vec<_>>());
}

#[test]
fn auxiliary_negative_key_holes_are_exportable_without_a_rule_label() {
    let input = export::compile_datalog(r#"
        struct Negative;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation Result(x:i32);
        output Result;
        Keep: Result(*x) :- Edge(x,y), !Blocked(x);
    "#, configuration()).unwrap();
    let hole = input.holes.iter().find(|hole| matches!(hole.origin, Origin::NegativeKey { .. })).unwrap();
    let actual = enumerate(&input, hole).unwrap();
    assert_eq!(actual.len(), 1);
    assert_eq!(one_current(&actual).order, [0]);
    assert_eq!(actual[0].steps.iter().filter(|step| step.read.is_some()).count(), 1);
}

#[test]
fn actual_partial_read_incumbent_is_not_replaced_by_a_coarse_skeleton() {
    let source = r#"
        struct Q;
        relation Seed(x:i32) = (seed);
        relation R(x:i32,y:i32) = (r);
        relation S(x:i32,z:i32) = (s);
        Q(x,y,z) :- Seed(x), R(x,y), S(x,z);
    "#;
    let options = syn::parse_str("order(Seed,R@1(x),S@2(x),R@1,S@2)").unwrap();
    let input = export::compile_cq(source, &options, configuration()).unwrap();
    let actual = candidates(&input);
    let current = one_current(&actual);
    assert!(current.unbounded.iter().any(|reason| reason.contains("partial-read")));
    assert_eq!(current.steps.iter().filter_map(|step| step.read.as_ref()).map(|read| read.source_occurrence).collect::<Vec<_>>(), [0, 1, 2, 1, 2]);
    assert_eq!(current.steps[1].read.as_ref().unwrap().columns.len(), 1);
    assert_eq!(current.steps[3].read.as_ref().unwrap().columns.len(), 2);
    assert_eq!(current.steps[1].bound_before, ["x"]);
    assert!(!current.steps[1].introduces);
    assert!(current.steps[3].introduces);
    assert!(actual.iter().filter(|candidate| !candidate.current).all(|candidate| candidate.steps.len() == 3));
}

#[test]
fn nested_scopes_preserve_interfaces_and_report_unknown_work() {
    let input = cq(r#"
        struct Branches;
        relation R(x:i32,y:i32) = (r);
        relation S(y:i32,z:i32) = (s);
        relation T(y:i32,z:i32) = (t);
        Branches(x,z) :- R(x,y), { S(y,z) } | { T(y,z) };
    "#);
    let actual = candidates(&input);
    one_current(&actual);
    for candidate in actual {
        assert_eq!(candidate.order, [0]);
        assert!(candidate.unbounded.iter().any(|reason| reason.contains("nested scope")));
        let block = candidate.steps.iter().find(|step| step.kind == ClauseKind::Block).unwrap();
        assert!(block.provides.contains(&"z".to_owned()));
        assert!(block.read.is_none(), "a block is not a made-up relation scan");
        assert_eq!(candidate.steps.iter().filter(|step| step.read.is_some()).count(), 1);
    }
    assert_eq!(input.holes[0].candidate_reference.steps.iter().find(|step| !step.branches.is_empty()).unwrap().branches.len(), 2);
}

#[test]
fn constants_are_fixed_columns_and_host_captures_are_not_stream_steps() {
    let input = cq(r#"
        struct Q;
        relation R(a:i32,b:i32) = (r);
        Q(x,one) :- R(x,7), let one:i32 = 1, if host();
    "#);
    let actual = candidates(&input);
    assert_eq!(actual.len(), 1);
    let candidate = one_current(&actual);
    assert_eq!(candidate.steps.len(), 1);
    let read = candidate.steps[0].read.as_ref().unwrap();
    assert_eq!(read.columns[0].variable.as_deref(), Some("x"));
    assert_eq!(read.columns[1].variable, None);
    assert_eq!(candidate.steps[0].bound_before, Vec::<String>::new());
    assert!(candidate.demands.iter().any(|demand| demand.key == [1]));
    assert!(input.holes[0].captures_ir.contains("host"));
}

#[test]
fn closed_relational_operands_are_fixed_without_classifying_rust_guards() {
    let input = cq(r#"
        struct Q;
        relation R(x:i32,y:i32) = (r);
        Q(x) :- R(x,host()), if *x > 5;
    "#);
    let actual = candidates(&input);
    let candidate = one_current(&actual);
    let read = candidate.steps.iter().find_map(|step| step.read.as_ref()).unwrap();
    assert_eq!(read.columns[1].variable, None);
    assert_eq!(read.columns[0].variable.as_deref(), Some("x"));
    assert!(candidate.steps.iter().any(|step| step.kind == ClauseKind::Constraint));
    assert!(candidate.steps.iter().any(|step| step.kind == ClauseKind::OpaqueGuard));
    assert!(candidate.demands.iter().any(|demand| demand.key == [1]));
}

#[test]
fn incomplete_or_foreign_candidate_problems_are_rejected() {
    let input = cq(&atom_query(&[vec!["x", "y"], vec!["y", "z"]]));
    let mut missing = input.clone();
    missing.holes[0].candidate_reference.steps.pop();
    assert!(enumerate(&missing, &missing.holes[0]).is_err());
    let mut foreign = input.holes[0].clone();
    foreign.id += 1;
    assert!(enumerate(&input, &foreign).is_err());
}

#[test]
fn split_head_records_keep_their_own_source_and_head_identity() {
    // Source's checked boundary is already single-head: these are the two
    // elaborated heads, not an invented multihead identity inferred from labels.
    let input = export::compile_datalog(r#"
        struct Split;
        relation R(x:i32,y:i32) = (r);
        relation S(y:i32,z:i32) = (s);
        relation Left(x:i32,z:i32);
        relation Right(x:i32,z:i32);
        output Left, Right;
        LeftHead: Left(*x,*z) :- R(x,y), S(y,z);
        RightHead: Right(*x,*z) :- R(x,y), S(y,z);
    "#, configuration()).unwrap();
    let mut heads = BTreeSet::new();
    for hole in &input.holes {
        let Origin::Rule { source_rule, head_relation, source_label, .. } = &hole.origin else { continue; };
        heads.insert((*source_rule, *head_relation, source_label.clone()));
        let actual = enumerate(&input, hole).unwrap();
        assert_eq!(orders(&actual), BTreeSet::from([vec![0, 1], vec![1, 0]]));
        one_current(&actual);
    }
    assert_eq!(heads.len(), 2);
}

#[test]
fn nested_constant_constraints_do_not_fix_an_outer_read_column() {
    let input = cq(r#"
        struct Scoped;
        relation R(x:i32,y:i32) = (r);
        relation S(y:i32,z:i32) = (s);
        relation T(y:i32,z:i32) = (t);
        Scoped(x,z) :- R(x,y), { S(y,z), where y == 7 } | { T(y,z), where y == 7 };
    "#);
    for candidate in candidates(&input) {
        let outer = candidate.steps.iter().find_map(|step| step.read.as_ref()).unwrap();
        assert_eq!(outer.columns[1].variable.as_deref(), Some("y"));
        assert!(candidate.unbounded.iter().any(|reason| reason.contains("nested scope")));
    }
}

#[test]
fn disjunctive_constants_do_not_invent_a_single_fixed_key() {
    let input = cq(r#"
        struct Q;
        relation R(x:i32,y:i32) = (r);
        Q(x) :- R(x,y), where (y == 1) || (y == 2);
    "#);
    let actual = candidates(&input);
    let read = one_current(&actual).steps.iter().find_map(|step| step.read.as_ref()).unwrap();
    assert_eq!(read.columns[1].variable.as_deref(), Some("y"));
    assert!(actual[0].demands.is_empty());
}

#[test]
fn current_marker_follows_checked_advice_not_written_source_order() {
    let source = atom_query(&[vec!["x", "y"], vec!["y", "z"], vec!["x", "z"]]);
    let options = syn::parse_str("order(R2,R1,R0)").unwrap();
    let input = export::compile_cq(&source, &options, configuration()).unwrap();
    let actual = candidates(&input);
    assert_eq!(actual.len(), 6);
    assert_eq!(one_current(&actual).order, [2, 1, 0]);
}

#[test]
fn normalized_insertions_do_not_renumber_source_advice_occurrences() {
    let input = cq(r#"
        struct Q;
        relation R(x:i32,k:i32) = (r);
        relation S(x:i32,z:i32) = (s);
        Q(z) :- R(x,host()), S(x,z);
    "#);
    let hole = &input.holes[0];
    assert_ne!(hole.occurrences[1].source_path, hole.occurrences[1].normalized_path);
    let actual = candidates(&input);
    assert_eq!(orders(&actual), BTreeSet::from([vec![0, 1], vec![1, 0]]));
    for candidate in actual {
        for read in candidate.steps.iter().filter_map(|step| step.read.as_ref()) {
            let occurrence = hole.occurrences.iter().find(|record| record.source_path == [read.source_occurrence]).unwrap();
            assert_eq!(read.normalized_clause, occurrence.normalized_path[0]);
        }
    }
}
