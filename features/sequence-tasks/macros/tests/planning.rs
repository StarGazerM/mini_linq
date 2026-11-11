use cq_lang_sequence::structures;
use quote::ToTokens;

// Authored fixtures retain each language layer independently of Sequence IR.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fixture {
    physical: structures::physical::Program,
    structures: structures::Program,
    program: cq_lang_sequence::Program,
}

impl syn::parse::Parse for Fixture {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let physical = input.parse()?;
        let structures = structures::Program::parse(input, &physical)?;
        let program = input.parse()?;
        Ok(Self { physical, structures, program })
    }
}

impl From<(structures::physical::Program, structures::Program, cq_lang_sequence::Program)> for Fixture {
    fn from((physical, structures, program): (structures::physical::Program, structures::Program, cq_lang_sequence::Program)) -> Self {
        Self { physical, structures, program }
    }
}

impl ToTokens for Fixture {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.physical.to_tokens(tokens);
        self.structures.to_tokens(tokens);
        self.program.to_tokens(tokens);
    }
}

use quote::quote;

macro_rules! capture {
    (pipeline {} $(planning_tables { $($tables:tt)* })?; realization {
        physical { $($physical:tt)* }
        structures { $($structures:tt)* }
        sequence { $($sequence:tt)* }
    }) => { stringify!($($physical)* $($structures)* $($sequence)*) };
    (pipeline {} $(planning_tables { $($tables:tt)* })?; $($program:tt)*) => { stringify!($($program)*) };
}

macro_rules! source {
    ($($entry:ident)::+; [$($prefix:tt)*]; $($body:tt)*) => {
        $($entry)::+ ! {
            $($prefix)*
            physical {
                relation R(a:i32);
                r = let canon rename R {a -> a};
                return r as answer(a);
            }
            structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; } r host scan = @host_scan as ::core::iter::Map<_,_> []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; }
            sequence { $($body)* }
        }
    };
}

#[test]
fn facade_target_plan_equals_the_shared_planner_without_authored_decomposition() {
    let source: Fixture = syn::parse_str(source!(capture; [pipeline {};];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let actual: Fixture = syn::parse_str(source!(mini_linq::passes::plan;
        [arguments { cq_sequence_tasks_macros::plan }; pipeline { capture };];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let machine = cq_sequence_tasks::machine();
    let expected =
        cq_sequence_plan::decompose(&source.physical, &source.structures, &source.program, &machine, &cq_sequence_tasks::catalog()).unwrap();
    assert_eq!(actual.physical, expected.physical);
    assert_eq!(actual.structures, expected.structures);
    assert_eq!(actual.program, expected.program);
    cq_lang_sequence::check_grading(&actual.physical, &actual.structures, &actual.program, &machine).unwrap();
}

#[test]
fn authored_grade_output_and_tile_precede_facade_planning() {
    let actual: Fixture = syn::parse_str(source!(cq_sequence_rewrites::grade;
        [arguments { tasks, input at host, result at pool };
         pipeline {
             cq_sequence_rewrites::output(reserve),
             mini_linq::passes::plan(cq_sequence_tasks_macros::plan),
             capture
         };];
        input = store rows r;
        result = store tile 7 items 3 input;
    ))
    .unwrap();
    let expected: Fixture = syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = let canon rename R {a -> a};
            return r as answer(a);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; } r host scan = @host_scan as ::core::iter::Map<_,_> []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; }
        sequence {
            input = store rows r;
            result = store tile 7 items 3 input;
            output reserve;
        }
        graded tasks { input at host; result at pool; }
    })
    .unwrap();
    assert_eq!(actual, expected);
    let replay = cq_sequence_plan::decompose(&expected.physical, &expected.structures, &expected.program,
        &cq_sequence_tasks::machine(),
        &cq_sequence_tasks::catalog(),
    )
    .unwrap();
    assert_eq!(replay.physical, expected.physical);
    assert_eq!(replay.structures, expected.structures);
    assert_eq!(replay.program, expected.program);
    assert!(replay.trace.is_empty());
}

#[test]
fn sequential_facade_planning_uses_the_exported_catalog() {
    let source: Fixture = syn::parse_str(source!(capture; [pipeline {};];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let actual: Fixture = syn::parse_str(source!(mini_linq::passes::plan;
        [arguments { mini_linq::passes::plan_sequential }; pipeline { capture };];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let expected = cq_sequence_plan::decompose(&source.physical, &source.structures, &source.program,
        &cq_sequence_pull::machine(),
        &cq_sequence_pull::catalog(),
    )
    .unwrap();
    assert_eq!(actual.physical, expected.physical);
    assert_eq!(actual.structures, expected.structures);
    assert_eq!(actual.program, expected.program);
}

#[test]
fn union_occurrences_have_legal_task_boundaries() {
    let mut source: Fixture = syn::parse_str(mini_linq::compile! {
        pipeline { print = capture; }
        struct Q;
        relation r(x:i32);
        answer(x) :- { r(x) } | { r(x) };
    }).unwrap();
    source.program.graded = None;
    let planned = cq_sequence_plan::decompose(&source.physical, &source.structures, &source.program,
        &cq_sequence_tasks::machine(),
        &cq_sequence_tasks::catalog(),
    ).unwrap_or_else(|error| panic!("{error}\n{}", quote!(#source)));
    let facts =
        cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &cq_sequence_tasks::machine()).unwrap();
    assert!(facts.dispatches.len() >= 2);
    assert_ne!(facts.dispatches[0].outer_store, facts.dispatches[1].outer_store);
    let rendered = cq_sequence_tasks::translate(&planned.physical, &planned.structures, &planned.program, &facts).unwrap();
    let rendered = quote!(#rendered).to_string();
    assert!(rendered.contains("fold_tiles"), "{rendered}");
    assert!(rendered.contains("Iterator :: chain"), "{rendered}");
    assert!(rendered.contains("par_sort"), "{rendered}");
}

#[test]
fn task_union_answers_preserve_set_semantics() {
    for r in [vec![], vec![(3,), (1,), (3,), (-2,)]] {
        let mut actual: Vec<_> = mini_linq::compile! {
            pipeline {
                decompose = [cq_sequence_tasks_macros::plan];
                grade = cq_sequence_tasks_macros::tasks;
                print = cq_sequence_tasks_macros::translate_tasks;
            }
            struct Q;
            relation r(x:i32);
            answer(x) :- { r(x) } | { r(x) };
        }.collect();
        actual.sort();
        let expected: std::collections::BTreeSet<_> = r.iter().map(|(x,)| (x,)).collect();
        let expected: Vec<_> = expected.into_iter().collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn task_union_with_distinct_arms_preserves_all_answers() {
    let r = [(3,), (1,), (3,)];
    let s = [(2,), (3,), (-1,)];
    let mut actual: Vec<_> = mini_linq::compile! {
        pipeline {
            decompose = [cq_sequence_tasks_macros::plan];
            grade = cq_sequence_tasks_macros::tasks;
            print = cq_sequence_tasks_macros::translate_tasks;
        }
        struct Q;
        relation r(x:i32);
        relation s(x:i32);
        answer(x) :- { r(x) } | { s(x) };
    }.collect();
    actual.sort();
    assert_eq!(actual, vec![(&-1,), (&1,), (&2,), (&3,)]);
}
