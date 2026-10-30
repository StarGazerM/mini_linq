use cq_sequence_conformance::{Target, assert_conformant, assert_conformant_with_inputs, plan};

fn program() -> cq_lang_sequence::Program {
    syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            consumer canonical;
            r = pipe rename R@0 {a -> a,b -> b};
            return r as answer(b,a);
        }
        structures {
            r host scan = ::core::iter::Map<_,_> [];
        }
        sequence {
            input = store rows r;
            result = store input;
        }
    }
}

#[test]
fn identical_programs_compare_after_the_answer_map_with_duplicate_inputs() {
    let program = program();
    let targets = [Target::Sequential, Target::Tasks];
    assert_conformant(&program, &targets);
    assert_conformant_with_inputs(
        &program,
        &targets,
        &[("R", &[&[1, 20], &[1, 20], &[2, 10], &[3, 20]])],
    );
}

#[test]
fn planning_uses_target_catalogs_and_preserves_authored_decisions() {
    let original = program();
    for target in [Target::Sequential, Target::Tasks] {
        let (planned, _) = plan(&original, target).unwrap();
        assert_eq!(planned.structures, original.structures);
        assert_eq!(planned.sequence.definitions, original.sequence.definitions);
        assert_eq!(plan(&planned, target).unwrap().0, planned);
        let (machine, catalog) = match target {
            Target::Sequential => (cq_sequence_pull::machine(), cq_sequence_pull::catalog()),
            Target::Tasks => (cq_sequence_tasks::machine(), cq_sequence_tasks::catalog()),
        };
        assert_eq!(
            planned,
            cq_sequence_plan::decompose(&original, &machine, &catalog)
                .unwrap()
                .program,
        );
    }
}

#[test]
fn caller_runtime_links_exact_storage_and_rayon_artifacts() {
    use cq_testkit::rustc::{Runtime, linked_dependency};

    let runtime = Runtime::with_dependencies(vec![
        linked_dependency::<cq_storage::SortedRun<i32, i32>>(
            "cq_storage",
            "cq_storage::SortedRun<i32, i32>",
        ),
        linked_dependency::<rayon::ThreadPool>("rayon", "rayon::ThreadPool"),
    ]);
    let setup = quote::quote! {
        use rayon::prelude::*;
        let R: Vec<(i32, i32)> = vec![(1, 20), (1, 20), (2, 10)]
            .into_par_iter()
            .collect();
    };
    cq_sequence_conformance::assert_conformant_with_runtime(
        &program(),
        &[Target::Sequential, Target::Tasks],
        &setup,
        &runtime,
        &cq_lang_sequence::Catalog::default(),
    );
}
