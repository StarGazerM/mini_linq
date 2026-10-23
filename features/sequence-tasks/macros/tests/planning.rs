use cq_lang_sequence::Program;
use quote::quote;

macro_rules! capture {
    (pipeline {}; $($program:tt)*) => { stringify!($($program)*) };
}

macro_rules! source {
    ($($entry:ident)::+; [$($prefix:tt)*]; $($body:tt)*) => {
        $($entry)::+ ! {
            $($prefix)*
            physical {
                relation R(a:i32);
                r = pipe canon rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { r host scan = ::core::iter::Map<_,_> []; r canon = ::cq_storage::HashSet<_>; }
            sequence { $($body)* }
        }
    };
}

#[test]
fn facade_target_plan_equals_the_shared_planner_without_authored_decomposition() {
    let source: Program = syn::parse_str(source!(capture; [pipeline {};];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let actual: Program = syn::parse_str(source!(mini_linq::passes::plan;
        [arguments { cq_sequence_tasks_macros::plan }; pipeline { capture };];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let machine = cq_sequence_tasks::machine();
    let expected =
        cq_sequence_plan::plan(&source, &machine, &cq_sequence_tasks::catalog()).unwrap();
    assert_eq!(actual, expected.program);
    cq_lang_sequence::check_grading(&actual, &machine).unwrap();
}

#[test]
fn authored_grade_output_and_tile_precede_facade_planning() {
    let actual: Program = syn::parse_str(source!(cq_sequence_rewrites::grade;
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
    let expected: Program = syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
        structures { r host scan = ::core::iter::Map<_,_> []; r canon = ::cq_storage::HashSet<_>; }
        sequence {
            input = store rows r;
            result = store tile 7 items 3 input;
            output reserve;
        }
        graded tasks { input at host; result at pool; }
    })
    .unwrap();
    assert_eq!(actual, expected);
    let replay = cq_sequence_plan::plan(
        &expected,
        &cq_sequence_tasks::machine(),
        &cq_sequence_tasks::catalog(),
    )
    .unwrap();
    assert_eq!(replay.program, expected);
    assert!(replay.trace.is_empty());
}

#[test]
fn sequential_facade_planning_uses_the_exported_catalog() {
    let source: Program = syn::parse_str(source!(capture; [pipeline {};];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let actual: Program = syn::parse_str(source!(mini_linq::passes::plan;
        [arguments { mini_linq::passes::plan_sequential }; pipeline { capture };];
        input = rows r;
        result = input;
    ))
    .unwrap();
    let expected = cq_sequence_plan::plan(
        &source,
        &cq_sequence_pull::machine(),
        &cq_sequence_pull::catalog(),
    )
    .unwrap();
    assert_eq!(actual, expected.program);
}
