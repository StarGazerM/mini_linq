//! Reproduce the paper's worked example using the actual typed pass APIs.
//! All generated artifacts go beneath doc/formal/build/example by default.
//! Also executes both public macro routes and compiles/runs both final Rust files.
use cq_ir::{cq, iterator_pipeline, relational_plan, rust_access_plan};
use cq_plan::Rule as JoinRule;
use quote::quote;
use std::collections::BTreeSet;
use syn::visit_mut::VisitMut;

mini_linq::mini_linq! {
    pub struct LazyOverview;
    relation R(a:i32,b:i32,c:i32,d:i32);
    relation S(a:i32,b:i32,c:i32);
    relation T(a:i32,b:i32);
    relation answer(a:i32,b:i32);
    answer(x,y) :- R(x,k,tag,unused), S(k,tag,z), T(z,y), if y > 0;
}
mini_linq::planned_query! {
    pub struct MaterializedOverview;
    relation R(a:i32,b:i32,c:i32,d:i32);
    relation S(a:i32,b:i32,c:i32);
    relation T(a:i32,b:i32);
    relation answer(a:i32,b:i32);
    answer(x,y) :- R(x,k,tag,unused), S(k,tag,z), T(z,y), if y > 0;
}
struct ExposePipelines;
impl VisitMut for ExposePipelines {
    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        let next = match mac.path.segments.last().unwrap().ident.to_string().as_str() {
            "pull" => syn::parse_quote!(::mini_linq::iterator_pipeline),
            "collect_access" => syn::parse_quote!(::mini_linq::collect_pipeline),
            _ => return,
        };
        let access: rust_access_plan::Plan = syn::parse2(mac.tokens.clone()).unwrap();
        rust_access_plan::contract::check(&access).unwrap();
        let target = cq_pull::compile(&access).unwrap();
        iterator_pipeline::contract::check(&target).unwrap();
        mac.path = next;
        mac.tokens = quote!(#target);
    }
}

fn lower_pipeline(mac: &syn::Macro) -> Option<syn::Expr> {
    let lower = match mac.path.segments.last().unwrap().ident.to_string().as_str() {
        "iterator_pipeline" => cq_pipeline::iterator::compile,
        "collect_pipeline" => cq_pipeline::collect::compile,
        _ => return None,
    };
    let pipeline: iterator_pipeline::Pipeline = syn::parse2(mac.tokens.clone()).unwrap();
    iterator_pipeline::contract::check(&pipeline).unwrap();
    Some(lower(&pipeline).unwrap())
}

struct LowerPipelines;
impl VisitMut for LowerPipelines {
    fn visit_stmt_mut(&mut self, stmt: &mut syn::Stmt) {
        if let syn::Stmt::Macro(m) = stmt {
            if let Some(expr) = lower_pipeline(&m.mac) {
                *stmt = syn::Stmt::Expr(expr, m.semi_token);
                return;
            }
        }
        syn::visit_mut::visit_stmt_mut(self, stmt);
    }
    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        if let syn::Expr::Macro(m) = expr {
            if let Some(target) = lower_pipeline(&m.mac) {
                *expr = target;
                return;
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}
fn write(directory: &std::path::Path, name: &str, file: &syn::File) {
    std::fs::write(
        directory.join(format!("{name}.rs")),
        prettyplease::unparse(file),
    )
    .unwrap();
}
fn boundaries(directory: &std::path::Path, prefix: &str, source: &cq::Module, planned: bool) {
    let ra = if planned {
        cq_compose::compile_cq_planned(source)
    } else {
        cq_compose::compile_cq(source)
    }
    .unwrap();
    let ix = if planned {
        cq_index::result::compile(&ra)
    } else {
        cq_index::result::compile(&ra)
    }
    .unwrap();
    let indexes: Vec<_> = ix
        .indexes
        .iter()
        .map(|index| {
            assert!(index.result_token.is_some());
            (
                index.relation.to_string(),
                index
                    .key_columns
                    .iter()
                    .map(|column| column.index)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let expected = if planned {
        vec![
            ("r0", vec![0, 1]),
            ("r4", vec![2]),
            ("r5", vec![1]),
            ("r6", vec![0, 1]),
            ("r6", vec![2]),
            ("r8", vec![0, 1]),
        ]
    } else {
        vec![("r1", vec![0, 1]), ("r3", vec![1])]
    };
    assert_eq!(
        indexes,
        expected
            .into_iter()
            .map(|(name, key)| (name.to_owned(), key))
            .collect::<Vec<_>>()
    );
    let staged = if planned {
        cq_physical::materialized::compile_materialized(
            &ix,
            &syn::parse_quote!(::mini_linq::collect_access),
        )
    } else {
        cq_physical::lazy::compile_lazy_indexed(&ix, &syn::parse_quote!(::mini_linq::pull))
    }
    .unwrap();
    let (cq_macro, ra_macro, index_macro): (syn::Path, syn::Path, syn::Path) = if planned {
        (
            syn::parse_quote!(::mini_linq::planned_query),
            syn::parse_quote!(::mini_linq::planned_relational_plan),
            syn::parse_quote!(::mini_linq::materialized),
        )
    } else {
        (
            syn::parse_quote!(::mini_linq::mini_linq),
            syn::parse_quote!(::mini_linq::relational_plan),
            syn::parse_quote!(::mini_linq::lazy_indexed),
        )
    };
    write(
        directory,
        &format!("{prefix}-01-cq"),
        &syn::parse2(quote!(#cq_macro! { #source })).unwrap(),
    );
    write(
        directory,
        &format!("{prefix}-02-relational"),
        &syn::parse2(quote!(#ra_macro! { #ra })).unwrap(),
    );
    write(
        directory,
        &format!("{prefix}-03-indexes"),
        &syn::parse2(quote!(#index_macro! { #ix })).unwrap(),
    );
    write(directory, &format!("{prefix}-04-access"), &staged);
    let mut pipelines = staged;
    ExposePipelines.visit_file_mut(&mut pipelines);
    write(directory, &format!("{prefix}-05-pipeline"), &pipelines);
    let mut rust = pipelines;
    LowerPipelines.visit_file_mut(&mut rust);
    let main: syn::Item = syn::parse_quote! {
        fn main() {
            let r = [(1,10,1,100),(1,10,1,101),(2,10,2,200),(3,20,1,300),(4,30,1,400),(5,40,1,500)];
            let s = [(10,1,1000),(10,2,2000),(20,1,3000),(40,1,4000),(50,1,5000)];
            let t = [(1000,7),(1000,8),(2000,9),(3000,-1),(5000,11),(6000,12)];
            let expected = vec![(1,7),(1,8),(2,9)];
            let actual = Overview::run(r,s,t);
            assert_eq!(actual, expected);
            assert_eq!(
                Overview::run(
                    r.into_iter().rev().chain(r),
                    s.into_iter().rev().chain(s),
                    t.into_iter().rev().chain(t),
                ),
                expected,
            );
            assert!(Overview::run([],s,t).is_empty());
            assert!(Overview::run(r,[],t).is_empty());
            assert!(Overview::run(r,s,[]).is_empty());
        }
    };
    rust.items.push(main);
    write(directory, &format!("{prefix}-06-rust"), &rust);
    let source_file = directory.join(format!("{prefix}-06-rust.rs"));
    let executable = directory.join(format!("{prefix}-run"));
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let build = std::process::Command::new(rustc)
        .args(["--edition=2024", "-Awarnings"])
        .arg(&source_file)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        std::process::Command::new(executable)
            .status()
            .unwrap()
            .success()
    );
}

fn oracle(r: &[(i32, i32, i32, i32)], s: &[(i32, i32, i32)], t: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut answer = BTreeSet::new();
    // Enumerate whole base tuples, independently of the compiler's RA and indexes.
    for &(x, k, tag, _) in r {
        for &(sk, stag, z) in s {
            for &(tz, y) in t {
                if k == sk && tag == stag && z == tz && y > 0 {
                    answer.insert((x, y));
                }
            }
        }
    }
    answer.into_iter().collect()
}

fn check_input(r: &[(i32, i32, i32, i32)], s: &[(i32, i32, i32)], t: &[(i32, i32)]) {
    let expected = oracle(r, s, t);
    assert_eq!(
        LazyOverview::run(r.iter().copied(), s.iter().copied(), t.iter().copied()),
        expected
    );
    assert_eq!(
        MaterializedOverview::run(r.iter().copied(), s.iter().copied(), t.iter().copied()),
        expected
    );
}

fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/example")
        });
    std::fs::create_dir_all(&directory).unwrap();
    let source: cq::Module = syn::parse_quote! {
        pub struct Overview;
        relation R(a:i32,b:i32,c:i32,d:i32);
        relation S(a:i32,b:i32,c:i32);
        relation T(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x,y) :- R(x,k,tag,unused), S(k,tag,z), T(z,y), if y > 0;
    };
    let (ra, report) = cq_compose::explain_cq_plan(
        &source,
        cq_plan::PlanningOptions {
            strategy: cq_plan::JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    std::fs::write(directory.join("report.txt"), format!("{report:#?}")).unwrap();
    cq_compose::check_cq_plan(&source, &ra, &report).unwrap();
    assert_eq!(
        report.join_tree.as_ref().unwrap().parent,
        [Some(1), Some(2), None]
    );
    assert_eq!(report.peak_arity, 4);
    assert_eq!(
        report.derivation,
        [
            JoinRule::Introduce { occurrence: 0 },
            JoinRule::Introduce { occurrence: 1 },
            JoinRule::Introduce { occurrence: 2 },
            JoinRule::Select {
                group: 2,
                predicate: 0
            },
            JoinRule::Begin,
            JoinRule::Up { child: 0 },
            JoinRule::Up { child: 1 },
            JoinRule::Down { child: 1 },
            JoinRule::Down { child: 0 },
            JoinRule::Freeze,
            JoinRule::Trim { group: 0 },
            JoinRule::Combine { left: 2, right: 1 },
            JoinRule::Trim { group: 2 },
            JoinRule::Combine { left: 2, right: 0 },
            JoinRule::Trim { group: 2 },
            JoinRule::Output { group: 2 },
        ]
    );
    let expected_ra: relational_plan::Plan = syn::parse_quote! {
        r0 = rename R { a -> x, b -> k, c -> tag, d -> unused };
        r1 = rename S { a -> k, b -> tag, c -> z };
        r2 = rename T { a -> z, b -> y };
        r3 = select r2 if (y > 0i32);
        r4 = semijoin r1 with r0;
        r5 = semijoin r3 with r4;
        r6 = semijoin r4 with r5;
        r7 = semijoin r0 with r6;
        r8 = project r7 keep { k, tag, x };
        r9 = natural_join r5 with r6;
        r10 = project r9 keep { k, tag, y };
        r11 = natural_join r10 with r8;
        r12 = project r11 keep { x, y };
        output r12 as answer(x, y);
    };
    assert_eq!(ra.plan, expected_ra);
    boundaries(&directory, "lazy", &source, false);
    boundaries(&directory, "materialized", &source, true);
    let r = [
        (1, 10, 1, 100),
        (1, 10, 1, 101),
        (2, 10, 2, 200),
        (3, 20, 1, 300),
        (4, 30, 1, 400),
        (5, 40, 1, 500),
    ];
    let s = [
        (10, 1, 1000),
        (10, 2, 2000),
        (20, 1, 3000),
        (40, 1, 4000),
        (50, 1, 5000),
    ];
    let t = [
        (1000, 7),
        (1000, 8),
        (2000, 9),
        (3000, -1),
        (5000, 11),
        (6000, 12),
    ];
    let expected = vec![(1, 7), (1, 8), (2, 9)];
    assert_eq!(oracle(&r, &s, &t), expected);
    check_input(&r, &s, &t);
    check_input(
        &r.into_iter().rev().chain(r).collect::<Vec<_>>(),
        &s.into_iter().rev().chain(s).collect::<Vec<_>>(),
        &t.into_iter().rev().chain(t).collect::<Vec<_>>(),
    );
    check_input(&[], &s, &t);
    check_input(&r, &[], &t);
    check_input(&r, &s, &[]);
    let (answer, stats) = MaterializedOverview::run_with_stats(r, s, t);
    assert_eq!(answer, expected);
    assert_eq!(
        stats.results,
        [
            ("r0", 4, 6),
            ("r1", 3, 5),
            ("r2", 2, 6),
            ("r3", 2, 5),
            ("r4", 3, 4),
            ("r5", 2, 3),
            ("r6", 3, 2),
            ("r7", 4, 3),
            ("r8", 3, 2),
            ("r9", 4, 3),
            ("r10", 3, 3),
            ("r11", 4, 3),
            ("r12", 2, 3),
        ]
    );
    assert_eq!(stats.index_rows, 19);
    assert_eq!(stats.join_pairs, 6);
    assert_eq!(stats.semijoin_probes, 20);
    let evidence = format!(
        "YA join tree R--S--T, rooted at T; separators {{k,tag}} and {{z}}.\n\
         Up0, Up1, Down1, Down0 and complete expected RA/indexes verified.\n\
         Answer {answer:?}\nResults: {:?}\n\
         Index rows {}; join pairs {}; semijoin probes {}\n\
         Two public routes and two fully lowered Rust executables pass five input presentations each.\n\
         Presentations: original, reversed with duplicates, empty R, empty S, empty T.\n",
        stats.results, stats.index_rows, stats.join_pairs, stats.semijoin_probes,
    );
    std::fs::write(directory.join("validation.txt"), &evidence).unwrap();
    print!("{evidence}");
}
