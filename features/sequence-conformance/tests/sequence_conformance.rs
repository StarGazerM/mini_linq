use cq_sequence_conformance::{Target, assert_conformant, assert_conformant_with_inputs, plan};

fn program() -> (
    cq_lang_structures::physical::Program,
    cq_lang_structures::Program,
    cq_lang_sequence::Program,
) {
    let physical: cq_lang_structures::physical::Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            consumer canonical;
            r = pipe rename R {a -> a,b -> b};
            return r as answer(b,a);
        }
    };
    let structures = syn::parse::Parser::parse2(
        |input: syn::parse::ParseStream<'_>| cq_lang_structures::Program::parse(input, &physical),
        quote::quote! {
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; }
            r host scan = @host_scan as ::core::iter::Map<_,_> [];
        }
        },
    ).unwrap();
    let program = syn::parse_quote! {
        sequence {
            input = store rows r;
            result = store input;
        }
    };
    (physical, structures, program)
}

#[test]
fn identical_programs_compare_after_the_answer_map_with_duplicate_inputs() {
    let (physical, structures, program) = program();
    let targets = [Target::Sequential, Target::Tasks];
    assert_conformant(&physical, &structures, &program, &targets);
    assert_conformant_with_inputs(
        &physical,
        &structures,
        &program,
        &targets,
        &[("R", &[&[1, 20], &[1, 20], &[2, 10], &[3, 20]])],
    );
}

#[test]
fn planning_uses_target_catalogs_and_preserves_authored_decisions() {
    let (physical, structures, original) = program();
    for target in [Target::Sequential, Target::Tasks] {
        let (planned_physical, planned_structures, planned, _) = plan(&physical, &structures, &original, target).unwrap();
        assert_eq!(planned_physical, physical);
        assert_eq!(planned_structures, structures);
        assert_eq!(planned.sequence.definitions, original.sequence.definitions);
        assert_eq!(plan(&planned_physical, &planned_structures, &planned, target).unwrap().2, planned);
        let (machine, catalog) = match target {
            Target::Sequential => (cq_sequence_pull::machine(), cq_sequence_pull::catalog()),
            Target::Tasks => (cq_sequence_tasks::machine(), cq_sequence_tasks::catalog()),
        };
        assert_eq!(
            planned,
            cq_sequence_plan::decompose(&physical, &structures, &original, &machine, &catalog)
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
    let (physical, structures, program) = program();
    cq_sequence_conformance::assert_conformant_with_runtime(
        &physical,
        &structures,
        &program,
        &[Target::Sequential, Target::Tasks],
        &setup,
        &runtime,
        &cq_lang_sequence::Catalog::default(),
    );
}
