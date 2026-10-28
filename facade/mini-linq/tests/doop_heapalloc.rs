//! SRDatalog DOOP `HeapAllocHelper` through the core routes.
//!
//! The body is acyclic (a path `Alloc@0 — Super — Component — Alloc@1`) with
//! one selection written last against captured constants. A correct plan is
//! output-bounded: no join of two operands sharing no column while a
//! connected atom is ready, the selection applied where `baseheaptype` is
//! bound, and every join-tree edge reduced. See doc/PLANNING-REDESIGN.md.
use cq_lang_cq as cq;
use cq_lang_ra as ra;
use std::collections::BTreeSet;

const SOURCE: &str = "
    struct Helper;
    relation Alloc(h: i32, t: i32);
    relation Component(a: i32, c: i32);
    relation Super(c: i32, t: i32);
    answer(heap, baseheap) :-
        Alloc(heap, heaptype), Alloc(baseheap, baseheaptype),
        Component(baseheaptype, comptype), Super(comptype, heaptype),
        if *baseheaptype != constants.c_object_array && *baseheaptype != constants.c_string_array;
";

#[derive(Debug, Default, PartialEq, Eq)]
struct Shape {
    /// `natural_join` definitions whose operands share no column.
    cross_joins: usize,
    /// Relational selections (recognized predicates) and opaque guards.
    selects: usize,
    guards: usize,
    semijoins: usize,
    /// Each selection's input heading, to check it sits where its variable is bound.
    select_inputs: Vec<BTreeSet<String>>,
}

fn shape(program: &ra::Program) -> Shape {
    let analysis = ra::contract::check(program).unwrap();
    let heading = |name: &syn::Ident| analysis[&name.to_string()].heading.clone();
    let mut shape = Shape::default();
    for definition in &program.plan.definitions {
        match &definition.operator {
            ra::Operator::NaturalJoin(join) => {
                if heading(&join.left).is_disjoint(&heading(&join.right)) {
                    shape.cross_joins += 1;
                }
            }
            ra::Operator::Semijoin(_) => shape.semijoins += 1,
            ra::Operator::Select(select) => {
                shape.selects += 1;
                shape.select_inputs.push(heading(&select.input));
            }
            ra::Operator::Guard(_) => shape.guards += 1,
            _ => {}
        }
    }
    shape
}

/// The facade `plan` composition: normalize, order, reduce, elaborate.
fn default_plan(options: cq_order::Options, semijoins: cq_reduce::Reductions) -> ra::Program {
    let source: cq::Program = syn::parse_str(SOURCE).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let ordered = cq_order::compile(&normal, &options).unwrap();
    cq_order::check(&normal, &ordered, &options).unwrap();
    let reduced = cq_reduce::compile(&ordered, &cq_reduce::Options { semijoins }).unwrap();
    cq_reduce::check(&ordered, &reduced, &cq_reduce::Options { semijoins }).unwrap();
    let plan = cq_elaborate::compile(&reduced).unwrap();
    cq_elaborate::check(&reduced, &plan).unwrap();
    plan
}

fn assert_output_bounded(plan: &ra::Program, route: &str) {
    let shape = shape(plan);
    assert_eq!(shape.cross_joins, 0, "{route}: cross product in {shape:?}");
    assert_eq!(
        shape.guards, 0,
        "{route}: captured-constant predicate stayed opaque in {shape:?}"
    );
    assert!(
        shape.selects >= 1,
        "{route}: no relational selection in {shape:?}"
    );
    // The selection reads baseheaptype only: it must run on a result that has not
    // expanded over heap (Alloc@0's private column) yet.
    assert!(
        shape
            .select_inputs
            .iter()
            .all(|heading| !heading.contains("heap")),
        "{route}: selection applied after expanding heap: {shape:?}"
    );
    // Acyclic path rooted at the stream: each inner clause is reduced by its
    // child subtree (Component by filtered Alloc@1, Super by Component); the
    // root scan itself is input-bounded and needs no reduction.
    assert!(
        shape.semijoins >= 2,
        "{route}: acyclic body not fully reduced: {shape:?}"
    );
}

#[test]
fn default_plan_is_output_bounded() {
    let options = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    };
    let plan = default_plan(options, cq_reduce::Reductions::Masks);
    assert_output_bounded(&plan, "plan");
}

#[test]
fn explicit_written_order_is_the_reference_and_keeps_the_cross_product() {
    // The exact written reference deliberately keeps the authored join order; it
    // must still recognize the captured-constant selection.
    let plan = default_plan(
        cq_order::Options {
            heuristic: cq_order::Heuristic::Written,
            split: cq_order::Split::Coarse,
        },
        cq_reduce::Reductions::None,
    );
    let shape = shape(&plan);
    assert_eq!(shape.cross_joins, 1, "{shape:?}");
    assert_eq!(shape.guards, 0, "{shape:?}");
}

#[test]
fn facade_plan_macro_emits_the_same_output_bounded_relational_plan() {
    let text: &str = ::mini_linq::compile! { pipeline { ix = ::core::stringify; }
        struct Helper;
        relation Alloc(h: i32, t: i32);
        relation Component(a: i32, c: i32);
        relation Super(c: i32, t: i32);
        answer(heap, baseheap) :-
            Alloc(heap, heaptype), Alloc(baseheap, baseheaptype),
            Component(baseheaptype, comptype), Super(comptype, heaptype),
            if *baseheaptype != constants.c_object_array && *baseheaptype != constants.c_string_array;
    };
    let relational = &text[text.find("relational").unwrap()..];
    let plan: ra::Program = syn::parse_str(relational).unwrap();
    assert_output_bounded(&plan, "compile!/ix");
}

/// Execution on an instance whose cross product is 250x the output.
mod instance {
    pub struct Constants {
        pub c_object_array: i32,
        pub c_string_array: i32,
    }
    pub const OBJECT_ARRAY: i32 = 1;
    pub const STRING_ARRAY: i32 = 2;
    pub const INT_ARRAY: i32 = 3;
    pub const ELEMENT: [i32; 3] = [11, 12, 13];

    pub fn alloc() -> Vec<(i32, i32)> {
        let mut rows = Vec::new();
        let mut heap = 0;
        let mut add = |ty: i32, count: i32| {
            for _ in 0..count {
                rows.push((heap, ty));
                heap += 1;
            }
        };
        add(OBJECT_ARRAY, 100);
        add(STRING_ARRAY, 100);
        add(INT_ARRAY, 20);
        for element in ELEMENT {
            add(element, 100);
        }
        rows
    }
    pub fn component() -> Vec<(i32, i32)> {
        vec![
            (OBJECT_ARRAY, ELEMENT[0]),
            (STRING_ARRAY, ELEMENT[1]),
            (INT_ARRAY, ELEMENT[2]),
        ]
    }
    pub fn supertype() -> Vec<(i32, i32)> {
        ELEMENT
            .iter()
            .map(|&e| (e, e))
            .chain([(ELEMENT[2], OBJECT_ARRAY)])
            .collect()
    }

    #[allow(non_snake_case)]
    pub fn run<'a>(
        Alloc: &'a [(i32, i32)],
        Component: &'a [(i32, i32)],
        Super: &'a [(i32, i32)],
        constants: &Constants,
    ) -> Vec<(i32, i32)> {
        ::mini_linq::compile! { pipeline {}
            struct Helper;
            relation Alloc(h: i32, t: i32);
            relation Component(a: i32, c: i32);
            relation Super(c: i32, t: i32);
            answer(heap, baseheap) :-
                Alloc(heap, heaptype), Alloc(baseheap, baseheaptype),
                Component(baseheaptype, comptype), Super(comptype, heaptype),
                if *baseheaptype != constants.c_object_array && *baseheaptype != constants.c_string_array;
        }
        .map(|(heap, baseheap)| (*heap, *baseheap))
        .collect()
    }
}

#[test]
fn execution_work_is_bounded_by_input_plus_output() {
    use instance::*;
    let (alloc, component, supertype) = (alloc(), component(), supertype());
    let constants = Constants {
        c_object_array: OBJECT_ARRAY,
        c_string_array: STRING_ARRAY,
    };
    let expected: BTreeSet<(i32, i32)> = alloc
        .iter()
        .flat_map(|&(heap, heaptype)| {
            let (component, supertype) = (&component, &supertype);
            alloc.iter().filter_map(move |&(baseheap, baseheaptype)| {
                (baseheaptype != OBJECT_ARRAY
                    && baseheaptype != STRING_ARRAY
                    && component
                        .iter()
                        .any(|&(a, c)| a == baseheaptype && supertype.contains(&(c, heaptype))))
                .then_some((heap, baseheap))
            })
        })
        .collect();
    cq_storage::counters::reset();
    let actual: BTreeSet<_> = run(&alloc, &component, &supertype, &constants)
        .into_iter()
        .collect();
    let counts = cq_storage::counters::snapshot();
    assert_eq!(actual, expected);
    let input = (alloc.len() + component.len() + supertype.len()) as u64;
    let output = expected.len() as u64;
    let work = counts.rows_tested
        + counts.rows_enumerated
        + counts.cursor_steps
        + counts.lookups
        + counts.distinct_insertions;
    let bound = 8 * (input + output);
    assert!(
        work <= bound,
        "work {work} exceeds 8 x (input {input} + output {output}) = {bound}; counters {counts:?}"
    );
}
