use cq_datalog_print::{Program, compile, evaluate, render_with, round};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use round::schedule::Schedule;

fn program() -> Program {
    let source = syn::parse_quote! {
        struct Waves;
        relation E(x:i32) = (input);
        relation A(x:i32);
        relation B(x:i32);
        output A, B;
        A(*x) :- E(x);
        B(*x) :- E(x);
        A(*x) :- B(x);
        B(*x) :- A(x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}

fn schedules(program: &Program) -> [Schedule; 3] {
    let analysis = round::storage::contract::check(&program.storage).unwrap();
    [Schedule::whole, Schedule::per_hole, Schedule::by_head]
        .map(|make| make(&program.storage, &analysis, &program.context).unwrap())
}

#[test]
fn rendered_invocations_follow_wave_and_within_wave_order() {
    let mut program = program();
    assert_eq!(program.context.strata.len(), 1);
    assert_eq!(program.context.strata[0].seeds.len(), 2);
    assert_eq!(program.context.strata[0].steps.len(), 2);
    let holes: Vec<_> = (0..program.holes.len())
        .map(|index| {
            let name = format_ident!("scheduled_hole_{index}");
            quote!(#name())
        })
        .collect();
    for mut schedule in schedules(&program) {
        // Reverse both the wave order within a phase and each wave's holes.
        // This catches printers that silently retain the original context order.
        for index in 0..schedule.strata.len() {
            let mut waves = schedule.strata[index].waves.clone();
            for wave in &mut waves {
                wave.holes.reverse();
            }
            let seeds = waves.partition_point(|wave| wave.phase == "seed");
            waves[..seeds].reverse();
            waves[seeds..].reverse();
            schedule = schedule.with_stratum_waves(index, waves, &program.context);
        }
        program.schedule = schedule;
        for timed in [false, true] {
            program.policy.timing = syn::parse_str(if timed { "phases" } else { "none" }).unwrap();
            let rendered = render_with(&program, &holes, |rules, context, holes, _| {
                Ok(evaluate(rules, context, holes, &program.holes, timed))
            })
            .unwrap()
            .to_token_stream()
            .to_string();
            let stratum = &program.context.strata[0];
            let expected: Vec<_> = program.schedule.strata[0]
                .waves
                .iter()
                .flat_map(|wave| {
                    let rules = if wave.phase == "seed" {
                        &stratum.seeds
                    } else {
                        &stratum.steps
                    };
                    wave.holes
                        .iter()
                        .map(|&index| rules[index].hole)
                        .collect::<Vec<_>>()
                })
                .collect();
            let positions: Vec<_> = expected
                .iter()
                .map(|index| {
                    let name = format!("scheduled_hole_{index} ()");
                    assert_eq!(rendered.matches(&name).count(), 1);
                    rendered.find(&name).unwrap()
                })
                .collect();
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}

#[test]
fn renderer_rejects_nonsynchronous_progress() {
    let mut program = program();
    program.schedule.progress = syn::parse_quote!(asynchronous);
    assert!(
        compile(&program)
            .unwrap_err()
            .to_string()
            .contains("synchronous")
    );
}

#[test]
fn printed_schedules_match_round_reference_answers() {
    let mut program = program();
    let mut checks = Vec::new();
    for schedule in schedules(&program) {
        program.schedule = schedule;
        for timed in [false, true] {
            program.policy.timing = syn::parse_str(if timed { "phases" } else { "none" }).unwrap();
            let printed = compile(&program).unwrap();
            checks.push(quote! {
                let (a, b) = #printed;
                let actual = (
                    a.collect::<::std::collections::HashSet<_>>(),
                    b.collect::<::std::collections::HashSet<_>>(),
                );
                assert_eq!(actual, ::cq_datalog_test_passes::eval_round! {
                    pipeline {};
                    #program
                });
            });
        }
    }
    run_native(quote! {
        fn main() {
            let input = [(1i32,), (2,), (2,), (3,)];
            #(#checks)*
        }
    });
}

fn run_native(source: TokenStream) {
    struct TemporaryDirectory(std::path::PathBuf);
    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let executable = std::env::current_exe().unwrap();
    let dependencies = executable.parent().unwrap();
    let directory = TemporaryDirectory(
        std::env::temp_dir().join(format!("cq-datalog-print-schedules-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("main.rs");
    let binary = directory.0.join("run");
    std::fs::write(&path, source.to_string()).unwrap();
    let mut command =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
    command
        .arg("--edition=2024")
        .arg(&path)
        .arg("-L")
        .arg(format!("dependency={}", dependencies.display()))
        .arg("-o")
        .arg(&binary);
    for name in [
        "cq_datalog",
        "mini_linq",
        "cq_storage",
        "cq_datalog_testkit",
        "cq_datalog_test_passes",
    ] {
        let prefix = format!("lib{name}-");
        let extension = if name == "cq_datalog_test_passes" {
            "so"
        } else {
            "rlib"
        };
        let library = std::fs::read_dir(dependencies)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(&prefix)
                    && path.extension().is_some_and(|ext| ext == extension)
            })
            .max_by_key(|path| path.metadata().unwrap().modified().unwrap())
            .unwrap();
        command
            .arg("--extern")
            .arg(format!("{name}={}", library.display()));
    }
    let compiled = command.output().unwrap();
    assert!(
        compiled.status.success(),
        "native compilation failed:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = std::process::Command::new(&binary).output().unwrap();
    assert!(
        executed.status.success(),
        "native execution failed:\n{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}
