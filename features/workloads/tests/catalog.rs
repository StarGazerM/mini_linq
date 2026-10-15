use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use cq_workloads::{
    GenerationConfig, QueryCase, Scale, Scenario, Suite,
    catalog::{CLASSIC, RETAIL},
    dataset::Dataset,
};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workloads crate must live directly under the repository root")
        .to_path_buf()
}

fn all_cases() -> impl Iterator<Item = &'static QueryCase> {
    CLASSIC.iter().chain(RETAIL)
}

fn canonical_artifact_path(case: &QueryCase, directory: &str, extension: &str) -> PathBuf {
    PathBuf::from("workloads")
        .join(directory)
        .join(case.suite.as_str())
        .join(format!("{}.{}", case.name, extension))
}

fn artifact_tree_files(directory: &str) -> BTreeSet<PathBuf> {
    fn visit(repository_root: &Path, directory: &Path, files: &mut BTreeSet<PathBuf>) {
        let entries = fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("could not read `{}`: {error}", directory.display()));

        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!(
                    "could not read an entry in `{}`: {error}",
                    directory.display()
                )
            });
            let path = entry.path();
            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("could not inspect `{}`: {error}", path.display()));

            if file_type.is_dir() {
                visit(repository_root, &path, files);
                continue;
            }

            assert!(
                file_type.is_file(),
                "artifact tree contains a non-file entry: `{}`",
                path.display()
            );
            let relative = path
                .strip_prefix(repository_root)
                .unwrap_or_else(|error| {
                    panic!(
                        "artifact `{}` is outside repository root `{}`: {error}",
                        path.display(),
                        repository_root.display()
                    )
                })
                .to_path_buf();
            assert!(
                files.insert(relative.clone()),
                "duplicate artifact path `{}`",
                relative.display()
            );
        }
    }

    let repository_root = repository_root();
    let mut files = BTreeSet::new();
    visit(
        &repository_root,
        &repository_root.join("workloads").join(directory),
        &mut files,
    );
    files
}

fn artifact_files(directory: &str, extension: &str) -> BTreeSet<PathBuf> {
    let files = artifact_tree_files(directory);
    for path in &files {
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some(extension),
            "artifact tree contains a file with the wrong extension: `{}`",
            path.display()
        );
    }
    files
}

const CLASSIC_NAMES: [&str; 10] = [
    "intersection",
    "cartesian-product",
    "oriented-chain",
    "claw",
    "fact-star",
    "triangle",
    "cycle-4",
    "spade",
    "barbell",
    "loomis-whitney-4",
];

const RETAIL_NAMES: [&str; 10] = [
    "retail_h03",
    "retail_h05",
    "retail_h07",
    "retail_h08",
    "retail_h09",
    "retail_h21_positive",
    "retail_d19",
    "retail_d27",
    "retail_d72_inner",
    "retail_d85",
];

#[test]
fn classic_catalog_has_exactly_ten_cases_in_stable_order() {
    assert_eq!(CLASSIC.len(), 10);
    assert_eq!(
        CLASSIC.iter().map(|case| case.name).collect::<Vec<_>>(),
        CLASSIC_NAMES
    );
    assert!(CLASSIC.iter().all(|case| case.suite == Suite::Classic));
}

#[test]
fn classic_catalog_names_are_unique_and_metadata_is_present() {
    let names = CLASSIC
        .iter()
        .map(|case| case.name)
        .collect::<BTreeSet<_>>();

    assert_eq!(names.len(), CLASSIC.len());
    assert!(
        CLASSIC
            .iter()
            .all(|case| !case.purpose.is_empty() && !case.scope_note.is_empty())
    );
}

#[test]
fn classic_catalog_sources_parse_and_satisfy_the_cq_contract() {
    for case in CLASSIC {
        let source = case
            .module()
            .unwrap_or_else(|error| panic!("classic case `{}` did not parse: {error}", case.name));
        cq_lang_cq::contract::check(&source).unwrap_or_else(|error| {
            panic!(
                "classic case `{}` violates the CQ contract: {error}",
                case.name
            )
        });
    }
}

#[test]
fn retail_catalog_has_exactly_ten_explicitly_scoped_cases() {
    assert_eq!(RETAIL.len(), 10);
    assert_eq!(
        RETAIL.iter().map(|case| case.name).collect::<Vec<_>>(),
        RETAIL_NAMES
    );
    assert!(RETAIL.iter().all(|case| case.suite == Suite::Retail));
    assert!(RETAIL.iter().all(|case| {
        case.scope_note.contains("Inspired only")
            && case
                .scope_note
                .contains("not a compliant TPC query or result")
    }));
}

#[test]
fn retail_catalog_sources_parse_and_satisfy_the_cq_contract() {
    let mut names = BTreeSet::new();
    for case in RETAIL {
        assert!(names.insert(case.name), "duplicate case `{}`", case.name);
        let source = case
            .module()
            .unwrap_or_else(|error| panic!("retail case `{}` did not parse: {error}", case.name));
        cq_lang_cq::contract::check(&source).unwrap_or_else(|error| {
            panic!(
                "retail case `{}` violates the CQ contract: {error}",
                case.name
            )
        });
    }
}

#[test]
fn rust_query_modules_are_exactly_the_catalog_and_own_the_typed_programs() {
    let repository_root = repository_root();
    let mut expected_paths = BTreeSet::from([
        PathBuf::from("workloads/src/queries/mod.rs"),
        PathBuf::from("workloads/src/queries/extended/mod.rs"),
        PathBuf::from("workloads/src/queries/typed/mod.rs"),
    ]);
    expected_paths.extend(cq_workloads::extended::CASES.iter().map(|case| {
        let path = PathBuf::from(case.rust_path);
        if path.is_absolute() {
            path.strip_prefix(&repository_root).unwrap().to_path_buf()
        } else {
            PathBuf::from("workloads").join(path)
        }
    }));
    for case in cq_workloads::tpch::CASES {
        assert!(
            expected_paths.insert(PathBuf::from(case.rust_path)),
            "duplicate standalone benchmark source {}",
            case.rust_path
        );
    }

    for case in all_cases() {
        assert_eq!(case.module().unwrap().query.head.args.len(), case.arity,
            "{} output arity metadata", case.name);
    }
    for (name, rust_path, schema) in all_cases()
        .map(|case| (case.name, case.rust_path, cq_workloads::schema::declarations(case.inputs)))
        .chain(cq_workloads::typed::CASES.iter()
            .map(|case| (case.name, case.rust_path, case.inputs())))
    {
        let source_path = PathBuf::from(rust_path);
        let expected_path = if source_path.is_absolute() {
            source_path
                .strip_prefix(&repository_root)
                .unwrap_or_else(|error| {
                    panic!(
                        "query module `{}` is outside repository root: {error}",
                        source_path.display()
                    )
                })
                .to_path_buf()
        } else {
            PathBuf::from("workloads").join(source_path)
        };
        assert!(
            expected_path.starts_with("workloads/src/queries"),
            "case `{}` points outside the Rust query tree: `{}`",
            name,
            expected_path.display(),
        );
        assert!(
            expected_paths.insert(expected_path.clone()),
            "multiple catalog entries point to `{}`",
            expected_path.display()
        );

        let checked_in =
            fs::read_to_string(repository_root.join(&expected_path)).unwrap_or_else(|error| {
                panic!(
                    "could not read Rust query module `{}`: {error}",
                    expected_path.display()
                )
            });
        syn::parse_file(&checked_in).unwrap_or_else(|error| {
            panic!("Rust case `{}` did not parse: {error}", expected_path.display())
        });
        assert_eq!(
            cq_workloads::authored::schema(rust_path).unwrap(), schema,
            "{name} loader metadata differs from its single literal compile! query",
        );
    }

    assert_eq!(
        artifact_files("src/queries", "rs"),
        expected_paths,
        "the Rust query tree and catalog must be an exact bijection"
    );
}

#[test]
fn authored_sql_files_are_exactly_the_catalog() {
    let repository_root = repository_root();
    let mut expected_paths = BTreeSet::new();
    expected_paths.extend(
        cq_workloads::extended::CASES
            .iter()
            .map(|case| PathBuf::from("workloads/sql/extended").join(format!("{}.sql", case.name))),
    );
    for path in cq_workloads::tpch::CASES
        .iter()
        .filter_map(|case| case.sql_path)
    {
        assert!(
            expected_paths.insert(PathBuf::from(path)),
            "duplicate standalone benchmark SQL peer {path}"
        );
    }
    for case in cq_workloads::typed::CASES {
        let path = PathBuf::from("workloads/sql/typed").join(format!("{}.sql", case.name));
        assert!(expected_paths.insert(path.clone()));
        assert_eq!(
            fs::read_to_string(repository_root.join(path)).unwrap(),
            case.sql
        );
    }

    for case in all_cases() {
        let expected_path = canonical_artifact_path(case, "sql", "sql");
        assert!(
            expected_paths.insert(expected_path.clone()),
            "multiple catalog entries map to `{}`",
            expected_path.display()
        );

        let checked_in =
            fs::read_to_string(repository_root.join(&expected_path)).unwrap_or_else(|error| {
                panic!(
                    "could not read SQL file `{}`: {error}",
                    expected_path.display()
                )
            });
        assert_eq!(
            checked_in,
            case.sql,
            "catalog SQL for `{}` differs from its authored file at `{}`",
            case.name,
            expected_path.display()
        );
    }

    assert_eq!(
        artifact_files("sql", "sql"),
        expected_paths,
        "the SQL artifact tree and catalog must be an exact bijection"
    );
}

#[test]
fn tpch_benchmark_inventory_covers_retail_and_matches_full_query_coverage() {
    use cq_workloads::tpch;

    assert_eq!(tpch::CASES.len(), 13);
    let retail = tpch::CASES
        .iter()
        .filter(|case| case.tpch_query.is_none())
        .map(|case| case.name)
        .collect::<BTreeSet<_>>();
    let mut expected_retail = RETAIL.iter().map(|case| case.name).collect::<BTreeSet<_>>();
    expected_retail.insert("retail_h07_block");
    assert_eq!(retail, expected_retail);
    let mut names = BTreeSet::new();
    for case in tpch::CASES {
        assert!(names.insert(case.name), "duplicate benchmark {}", case.name);
        let path = Path::new(case.rust_path);
        assert!(!path.is_absolute());
        assert_eq!(path.file_stem().unwrap(), case.name);
        let source = fs::read_to_string(repository_root().join(path)).unwrap();
        let parsed = syn::parse_file(&source).unwrap();
        if case.tpch_query.is_none() {
            assert!(path.starts_with("workloads/src/queries/retail/tpch"));
            assert!(case.sql_path.is_none(), "retail SQL is authored inline");
            let declarations = parsed
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Macro(item)
                        if item.mac.path.is_ident("retail_kernel")
                            || item
                                .mac
                                .path
                                .segments
                                .last()
                                .is_some_and(|s| s.ident == "retail_kernel") =>
                    {
                        Some(&item.mac)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(declarations.len(), 1);
            let prefix = |input: syn::parse::ParseStream<'_>| -> syn::Result<(String, String)> {
                input.parse::<syn::Token![$]>()?;
                input.parse::<syn::Token![;]>()?;
                let keyword: syn::Ident = input.parse()?;
                assert_eq!(keyword, "name");
                let name: syn::LitStr = input.parse()?;
                input.parse::<syn::Token![;]>()?;
                let keyword: syn::Ident = input.parse()?;
                assert_eq!(keyword, "sql");
                let sql: syn::LitStr = input.parse()?;
                let _: proc_macro2::TokenStream = input.parse()?;
                Ok((name.value(), sql.value()))
            };
            let (name, sql) =
                syn::parse::Parser::parse2(prefix, declarations[0].tokens.clone()).unwrap();
            assert_eq!(name, case.name);
            assert!(!sql.trim().is_empty(), "{} has no authored SQL", case.name);
        } else {
            assert!(path.starts_with("workloads/src/queries/tpch"));
            let constant = |name: &str| {
                parsed
                    .items
                    .iter()
                    .find_map(|item| match item {
                        syn::Item::Const(item) if item.ident == name => Some(&*item.expr),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{} has no {name} constant", case.name))
            };
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(name),
                ..
            }) = constant("NAME")
            else {
                panic!("{} must author its name as a string literal", case.name);
            };
            assert_eq!(name.value(), case.name);
            let syn::Expr::Macro(sql) = constant("SQL") else {
                panic!("{} must include its authored SQL peer", case.name);
            };
            assert!(sql.mac.path.is_ident("include_str"));
            let included: syn::LitStr = syn::parse2(sql.mac.tokens.clone()).unwrap();
            let sql_path = case
                .sql_path
                .expect("full query needs an authored SQL peer");
            assert!(Path::new(sql_path).starts_with("workloads/sql/tpch"));
            assert_eq!(
                repository_root()
                    .join(path.parent().unwrap())
                    .join(included.value())
                    .canonicalize()
                    .unwrap(),
                repository_root().join(sql_path).canonicalize().unwrap(),
                "{} includes SQL different from its registered peer",
                case.name,
            );
        }
    }

    let mut coverage_ids = BTreeSet::new();
    let mut implemented = BTreeSet::new();
    for row in tpch::COVERAGE.lines().skip(1) {
        let columns = row.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 5, "malformed coverage record {row}");
        let number: u8 = columns[0].strip_prefix('Q').unwrap().parse().unwrap();
        assert!(
            coverage_ids.insert(number),
            "duplicate coverage record {row}"
        );
        if columns[1] == "implemented" {
            assert_eq!(columns[2], "none");
            implemented.insert(number);
        } else {
            assert!(columns[1].starts_with("not_expressible_"));
            assert!(!columns[2].is_empty() && columns[2] != "none");
        }
    }
    assert_eq!(coverage_ids, (1..=22).collect());
    let full_queries = tpch::CASES
        .iter()
        .filter_map(|case| case.tpch_query)
        .collect::<BTreeSet<_>>();
    assert_eq!(full_queries, BTreeSet::from([6, 19]));
    assert_eq!(
        implemented, full_queries,
        "coverage and executable catalogue differ"
    );
}

#[test]
fn checked_in_tiny_scenarios_are_an_exact_fresh_generator_bijection() {
    let repository_root = repository_root();
    let mut expected_data_directories = BTreeSet::new();
    let mut expected_golden_paths = BTreeSet::new();
    let mut inapplicable = BTreeSet::new();

    for case in all_cases() {
        let module = case.module().unwrap();
        let output_arity = module.query.head.args.len();
        for scenario in Scenario::ALL {
            let config = GenerationConfig {
                scenario,
                scale: Scale::Tiny,
                seed: cq_workloads::seed_for(case),
            };
            let fresh = match cq_workloads::generate::generate(case, config) {
                Ok(dataset) => dataset,
                Err(cq_workloads::generate::GenerateError::ScenarioNotApplicable {
                    scenario,
                    ..
                }) => {
                    inapplicable.insert((case.name, scenario.as_str()));
                    continue;
                }
                Err(error) => panic!(
                    "could not regenerate `{}` scenario `{}`: {error}",
                    case.name,
                    scenario.as_str(),
                ),
            };

            let data_directory = PathBuf::from("workloads/data")
                .join(case.suite.as_str())
                .join(case.name)
                .join(scenario.as_str())
                .join(Scale::Tiny.as_str());
            assert!(
                expected_data_directories.insert(data_directory.clone()),
                "duplicate tiny data directory `{}`",
                data_directory.display()
            );
            let checked_in = Dataset::read_from_dir(&module, repository_root.join(&data_directory))
                .unwrap_or_else(|error| {
                    panic!(
                        "could not read checked-in data `{}`: {error}",
                        data_directory.display()
                    )
                });
            checked_in
                .validate_provenance(case.suite, case.name, config)
                .unwrap();
            assert_eq!(
                checked_in,
                fresh,
                "checked-in data for `{}` scenario `{}` is stale",
                case.name,
                scenario.as_str(),
            );

            let golden_path = PathBuf::from("workloads/golden")
                .join(case.suite.as_str())
                .join(case.name)
                .join(scenario.as_str())
                .join("tiny.csv");
            assert!(
                expected_golden_paths.insert(golden_path.clone()),
                "duplicate tiny golden path `{}`",
                golden_path.display()
            );
            let rows = cq_workloads::dataset::read_result_rows(
                repository_root.join(&golden_path),
                output_arity,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "could not read checked-in golden `{}`: {error}",
                    golden_path.display()
                )
            });
            assert_eq!(
                rows.is_empty(),
                scenario != Scenario::Coverage,
                "`{}` scenario `{}` has the wrong empty/nonempty golden shape",
                case.name,
                scenario.as_str(),
            );
        }
    }

    assert_eq!(
        inapplicable,
        BTreeSet::from([("cartesian-product", "no-match")])
    );
    assert_eq!(expected_data_directories.len(), 59);
    assert_eq!(expected_golden_paths.len(), 59);
    for case in cq_workloads::extended::CASES {
        for scenario in Scenario::ALL {
            expected_data_directories.insert(
                PathBuf::from("workloads/data/extended")
                    .join(case.name)
                    .join(scenario.as_str())
                    .join("tiny"),
            );
            expected_golden_paths.insert(
                PathBuf::from("workloads/golden/extended")
                    .join(case.name)
                    .join(scenario.as_str())
                    .join("tiny.csv"),
            );
        }
    }

    let actual_data_directories = artifact_tree_files("data")
        .into_iter()
        .filter_map(|path| {
            let parent = path.parent()?.to_path_buf();
            (parent.file_name()?.to_str()? == Scale::Tiny.as_str()).then_some(parent)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_data_directories, expected_data_directories,
        "checked-in tiny data directories must exactly match applicable catalog scenarios"
    );

    let actual_golden_paths = artifact_tree_files("golden")
        .into_iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("tiny.csv"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_golden_paths, expected_golden_paths,
        "checked-in tiny golden files must exactly match applicable catalog scenarios"
    );
}
