use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Compiler {
    directory: PathBuf,
    dependencies: PathBuf,
}
impl Compiler {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("provider-contract-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        Self {
            directory,
            dependencies: std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned(),
        }
    }
    fn compile(&self, name: &str, declarations: &str, expression: &str) -> Output {
        let input = self.directory.join(format!("{name}.rs"));
        std::fs::write(
            &input,
            format!(
                r#"
#![allow(dead_code, unused_variables)]
{declarations}
fn main() {{
    let left = [(1_i64,), (2,)];
    let right = [(1_i64, 10_i64), (1, 11), (2, 20)];
    let _answers: Vec<_> = ({expression}).collect();
}}
"#
            ),
        )
        .unwrap();
        let mut rustc = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
        rustc
            .args(["--edition=2024", "--emit=metadata", "--crate-name", name])
            .arg("--out-dir")
            .arg(&self.directory)
            .arg("-L")
            .arg(format!("dependency={}", self.dependencies.display()));
        for (name, extension) in [
            ("cq_rewrites", std::env::consts::DLL_EXTENSION),
            ("mini_linq", "rlib"),
            ("cq_storage", "rlib"),
            ("cq_load", "rlib"),
        ] {
            // Runtime providers must share the exact Cargo-linked trait identity,
            // even when another feature workspace built a newer artifact.
            let path = match name {
                "cq_storage" => {
                    cq_testkit::rustc::linked_dependency::<cq_storage::Exists>(
                        name,
                        "cq_storage::Exists",
                    )
                    .1
                }
                "cq_load" => {
                    cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
                        name,
                        "cq_load::ColumnAt<0>",
                    )
                    .1
                }
                _ => artifact(&self.dependencies, name, extension),
            };
            rustc
                .arg("--extern")
                .arg(format!("{name}={}", path.display()));
        }
        rustc.arg(input).output().unwrap()
    }
}
impl Drop for Compiler {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn artifact(directory: &Path, name: &str, extension: &str) -> PathBuf {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("lib{name}-"))
                && path.extension().is_some_and(|value| value == extension)
        })
        .max_by_key(|path| std::fs::metadata(path).unwrap().modified().unwrap())
        .unwrap_or_else(|| panic!("missing Cargo-built artifact for {name}"))
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn rejected(output: Output, obligation: &str) {
    assert!(
        !output.status.success(),
        "false {obligation} declaration compiled"
    );
}

fn provider(copy: bool, exact_size: bool, keys: bool) -> String {
    let derive = if copy { "#[derive(Clone, Copy)]" } else { "" };
    let exact = if exact_size {
        "impl<V: Copy> ExactSizeIterator for Cursor<'_, V> {}"
    } else {
        ""
    };
    let enumeration = if keys {
        r#"
impl<K, V> cq_storage::Keys for Provider<K, V> {
    type Key = K;
    type Iter<'a> = std::collections::btree_map::Keys<'a, K, Vec<V>> where Self: 'a;
    fn distinct_keys(&self) -> Self::Iter<'_> { self.0.keys() }
    fn key_count(&self) -> usize { self.0.len() }
}
"#
    } else {
        ""
    };
    format!(
        r#"
struct Provider<K, V>(std::collections::BTreeMap<K, Vec<V>>);
type Alias<K, V> = Provider<K, V>;
impl<K: Ord, V, I: Iterator<Item=(K,V)>> cq_storage::Build<I> for Provider<K,V> {{
    fn build(input: I) -> Self {{
        let mut groups: std::collections::BTreeMap<K, Vec<V>> = Default::default();
        for (key, value) in input {{ groups.entry(key).or_default().push(value); }}
        Self(groups)
    }}
}}
{derive}
struct Cursor<'a, V: Copy> {{ rows: &'a [V] }}
impl<V: Copy> Iterator for Cursor<'_, V> {{
    type Item = V;
    fn next(&mut self) -> Option<V> {{
        let (first, rest) = self.rows.split_first()?;
        self.rows = rest;
        Some(*first)
    }}
    fn size_hint(&self) -> (usize, Option<usize>) {{ (self.rows.len(), Some(self.rows.len())) }}
}}
{exact}
impl<'a, K: Ord, V: Copy> cq_storage::IndexRead for &'a Provider<K,V> {{
    type Key = K;
    type Item = V;
    type Cursor = Cursor<'a,V>;
    fn lookup(&self, key: &K) -> Self::Cursor {{
        Cursor {{ rows: self.0.get(key).map_or(&[], Vec::as_slice) }}
    }}
}}
impl<K: Ord, V> cq_storage::ExactMembership<K> for Provider<K,V> {{
    fn contains(&self, key: &K) -> bool {{ self.0.contains_key(key) }}
}}
{enumeration}
"#
    )
}

/// Custom test tokens. Their facts admit every read, so rustc alone checks
/// the spelled types' traits.
const CUSTOM: &str = "
    custom = Provider<_, _> {
        reads [exact, range, membership, ordered], supports [retain, expand, keys], build typed
    };
    set_alias = SetAlias<_> { reads [membership], build typed };
    keyset_alias = KeySetAlias<_> { reads [membership], build typed };
";

/// Retype advice installing `provider` (`@token as Type ...`) at `selector`.
fn retype(selector: &str, provider: &str) -> String {
    format!("cq_rewrites::retype(providers {{ {CUSTOM} }} {selector} => {provider})")
}

fn query(provider: &str, support: &str, advice: &str, ordered: bool) -> String {
    let retype = retype("r", &format!("{provider} {support}"));
    let operator = if ordered { "merge" } else { "probe first" };
    let semi = if ordered {
        ""
    } else {
        "s = pipe semi eager [x] l,r;"
    };
    let outer = if ordered { "l" } else { "s" };
    format!(
        r#"
mini_linq::passes::select! {{
    pipeline {{
        {retype},
        {advice}
        mini_linq::passes::sequence, mini_linq::passes::sequential,
        mini_linq::passes::translate_pull
    }};
    physical {{
        relation L(a:i64) = (&left); relation R(a:i64,b:i64) = (&right);
        l = pipe rename L@0 {{a -> x}};
        r = let rename R@1 {{a -> x,b -> y}};
        {semi}
        p = pipe {operator} [x] {outer},r;
        out = let canon project p keep {{x,y}};
        return out as answer(x,y);
    }}
}}
"#
    )
}

#[test]
fn declared_support_is_checked_even_without_an_optimization_consumer() {
    let compiler = Compiler::new();
    let plain = query("@custom as Alias<_,_>", "", "", false);
    // Ordinary exact lookup does not require Copy, exact-size, or key enumeration.
    success(compiler.compile("ordinary", &provider(false, false, false), &plain));
    success(compiler.compile(
        "valid",
        &provider(true, true, true),
        &query(
            "@custom as Alias<_,_>",
            "supports [expand, keys]",
            "cq_rewrites::retain_lookups, cq_rewrites::expand_lookups(2),",
            false,
        ),
    ));
    for (name, declarations, support, obligation) in [
        (
            "noncopy",
            provider(false, true, true),
            "supports [retain]",
            "Copy",
        ),
        (
            "unknown_length",
            provider(true, false, true),
            "supports [expand]",
            "ExactSizeIterator",
        ),
        (
            "no_keys",
            provider(true, true, false),
            "supports [keys]",
            "Keys",
        ),
    ] {
        rejected(
            compiler.compile(
                name,
                &declarations,
                &query("@custom as Alias<_,_>", support, "", false),
            ),
            obligation,
        );
    }
}

#[test]
fn ordinary_read_obligations_reject_builtins_and_aliases_uniformly() {
    let compiler = Compiler::new();
    let aliases = "type SetAlias<T> = std::collections::HashSet<T>; type KeySetAlias<K> = cq_storage::AdaptiveKeySet<K, cq_storage::BorrowedI64TupleCodec, 64>;";
    for (name, ty) in [
        (
            "set_payload",
            "@std_hash_set as ::std::collections::HashSet<_>",
        ),
        ("alias_payload", "@set_alias as SetAlias<_>"),
        ("keyset_payload", "@keyset_alias as KeySetAlias<_>"),
    ] {
        rejected(
            compiler.compile(name, aliases, &query(ty, "", "", false)),
            "IndexRead",
        );
    }
    for (name, ty) in [
        ("unordered", "@custom as Provider<_,_>"),
        ("unordered_alias", "@custom as Alias<_,_>"),
    ] {
        rejected(
            compiler.compile(name, &provider(true, true, true), &query(ty, "", "", true)),
            "OrderedRead",
        );
    }
}

#[test]
fn host_ordering_and_interval_summaries_require_real_runtime_traits() {
    let compiler = Compiler::new();
    let aliases = "type SetAlias<T> = std::collections::HashSet<T>;";
    let ordered = |ty: Option<&str>| {
        let native = ty.is_none();
        let retype = ty
            .map(|ty| format!("{},", retype("ordered", ty)))
            .unwrap_or_default();
        let hosts = if native {
            r#"
            let left = std::collections::BTreeMap::from([((1_i64,), vec![()])]);
            let right = std::collections::BTreeMap::from([((1_i64,), vec![(10_i64,), (11,)])]);
        "#
        } else {
            ""
        };
        let host_reads = if native {
            "host [a] reads [exact, membership, ordered]"
        } else {
            ""
        };
        format!(
            r#"{{ {hosts}
mini_linq::passes::select! {{
    pipeline {{ {retype}
        mini_linq::passes::sequence, mini_linq::passes::sequential,
        mini_linq::passes::translate_pull }};
    physical {{
        relation L(a:i64) = (&left) {host_reads}; relation R(a:i64,b:i64) = (&right) {host_reads};
        l = pipe rename L@0 {{a -> x}};
        r = pipe rename R@1 {{a -> x,b -> y}};
        p = let canon merge [x] l,r;
        return p as answer(x,y);
    }}
}}.map(|(x,y)| (*x,*y)).collect::<Vec<_>>().into_iter()
}}"#
        )
    };
    success(compiler.compile("host_ordered", aliases, &ordered(None)));
    rejected(
        compiler.compile(
            "host_unordered_array",
            aliases,
            &ordered(Some("@set_alias as SetAlias<_>")),
        ),
        "OrderedRead",
    );

    let interval = |ty: &str| {
        let retype = retype("kind(membership)", ty);
        format!(
            r#"{{
        let left = [(1_i64, 10_i64), (2, 20)];
        let right = [(1_i64, 1_i64), (2, 2)];
        mini_linq::passes::select! {{
            arguments {{ positional(L,R) }};
            pipeline {{ {retype},
                mini_linq::passes::sequence, mini_linq::passes::sequential,
                mini_linq::passes::translate_pull }};
            physical {{
                relation L(a:i64,b:i64) = (&left); relation R(b:i64,c:i64) = (&right);
                l = pipe rename L@0 {{a -> x,b -> y}};
                r = pipe rename R@1 {{b -> x,c -> z}};
                s = let point select r if (z = 1);
                t = let semi eager [x] l,s;
                u = pipe semi eager [x] s,t;
                j = pipe probe first [x] u,t;
                out = let canon project j keep {{x,y}};
                return out as answer(x,y);
            }}
        }}.map(|(x,y)| (*x,*y)).collect::<Vec<_>>().into_iter()
    }}"#
        )
    };
    success(compiler.compile(
        "exact_interval",
        aliases,
        &interval(
            "@adaptive_keyset as ::cq_storage::AdaptiveKeySet<_, ::cq_storage::I64Codec, 64>",
        ),
    ));
    rejected(
        compiler.compile(
            "point_only_interval_alias",
            aliases,
            &interval("@set_alias as SetAlias<_>"),
        ),
        "ExactIntervalMembership",
    );
    let codec_alias = "type WrongCodec = cq_storage::BorrowedI64TupleCodec;";
    // The direct typed build requires Build; an incompatible codec prevents
    // that implementation from accepting the recorded input key.
    rejected(
        compiler.compile(
            "wrong_key_codec_alias",
            codec_alias,
            &interval("@adaptive_keyset as ::cq_storage::AdaptiveKeySet<_, WrongCodec, 64>"),
        ),
        "Build<",
    );
}

#[test]
fn candidate_directory_requires_candidates_not_just_matching_level_syntax() {
    let compiler = Compiler::new();
    let expression = |ty: &str| {
        let retype = retype("r", ty);
        format!(
            r#"{{
        let lrows = [(1_i64, 2_i64)];
        let rrows = [(2_i64, 3_i64)];
        let srows = [(1_i64, 3_i64)];
        mini_linq::passes::select! {{
            pipeline {{ {retype},
                mini_linq::passes::sequence, mini_linq::passes::sequential,
                mini_linq::passes::translate_pull }};
            physical {{
                relation L(a:i64,b:i64) = (&lrows);
                relation R(b:i64,c:i64) = (&rrows);
                relation S(a:i64,c:i64) = (&srows);
                l = pipe rename L@0 {{a -> a,b -> b}};
                r = let rename R@1 {{b -> b,c -> c}};
                s = let rename S@2 {{a -> a,c -> c}};
                candidate = pipe probe smallest [b] l,r;
                checked = let canon semi eager [a,c] candidate,s;
                return checked as answer(a,b,c);
            }}
        }}.map(|(a,b,c)| (*a,*b,*c)).collect::<Vec<_>>().into_iter()
    }}"#
        )
    };
    success(compiler.compile(
        "valid_directory",
        "",
        &expression(
            "@trie(@hash_set as ::cq_storage::HashSet<_>) as ::cq_storage::Trie<'_, _, ::cq_storage::HashSet<_>>",
        ),
    ));
    rejected(
        compiler.compile(
            "invalid_directory_alias",
            "type SetAlias<K> = std::collections::HashSet<K>;",
            &expression("@set_alias as SetAlias<_>"),
        ),
        "Candidates",
    );
}

#[test]
fn marker_aliases_must_bind_the_recorded_column_slot() {
    let compiler = Compiler::new();
    let point = r#"
mini_linq::passes::sequence! {
    pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
    physical {
        relation R(a:i64,b:i64) = (&right);
        r = pipe rename R@0 {a -> a,b -> b};
        selected = pipe point select r if (a = 1);
        out = let canon project selected keep {a,b};
        return out as answer(a,b);
    }
    structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        selected read = positional base R lineage {a -> a,b -> b}
            column a marker MarkerAlias predicate physical;
        out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
    }
}"#;
    let filtered = r#"{
        let left = [(1_i64, 99_i64)];
        mini_linq::passes::sequence! {
            pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
            physical {
                relation L(k:i64,payload:i64) = (&left);
                relation R(k:i64,v:i64) = (&right);
                l = pipe rename L@0 {k -> k,payload -> p};
                r = let rename R@1 {k -> k,v -> v};
                joined = pipe probe first [k] l,r;
                out = let canon project joined keep {k,p,v};
                return out as answer(k,p,v);
            }
            structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; radix_run = ::cq_storage::RadixRun<_, _> { reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter, run_filter, shared, items 2 [rows ::std::vec::Vec<_>, columns ::cq_storage::TupleColumns<_>], statistics_build [rows, max_group], statistics [ rows: rows occurrences exact run_finalize stat_rows host required build overlapping, keys: keys distinct_keys exact run_finalize stat_keys host optional build single, max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host optional build single, runs: runs runs exact run_finalize stat_runs host required build overlapping ] }; dense_membership = ::cq_storage::DenseMembership<_, 16777216> { reads [membership], build typed, shared }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                l host scan = @host_scan as ::core::iter::Map<_,_> [];
                r host scan = @host_scan as ::core::iter::Map<_,_> [];
                r = @radix_run as ::cq_storage::RadixRun<_,_> [k] supports [keys] items rows;
                l join_filter = @dense_membership as ::cq_storage::DenseMembership<i64,64> build r [k] covering joined marker MarkerAlias policy always;
                out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
            }
        }.map(|(k,p,v)| (*k,*p,*v)).collect::<Vec<_>>().into_iter()
    }"#;
    for (name, expression) in [("point_marker", point), ("filter_marker", filtered)] {
        success(compiler.compile(
            &format!("{name}_valid"),
            "type MarkerAlias = cq_load::ColumnAt<0>;",
            expression,
        ));
        // Both columns are i64. Key-type checking alone cannot detect this swap.
        rejected(
            compiler.compile(
                &format!("{name}_wrong_slot"),
                "type MarkerAlias = cq_load::ColumnAt<1>;",
                expression,
            ),
            "ColumnSlot",
        );
    }
}

/// IF-HostDecl: a declared host read is one direct call. A host whose type
/// lacks the declared capability, or answers another key type, is rejected by
/// rustc naming the missing trait; no fallback tier hides the mismatch.
#[test]
fn declared_host_reads_require_the_declared_trait_and_key() {
    let compiler = Compiler::new();
    let query = |host: &str, declaration: &str, body: &str| {
        format!(
            r#"{{ {host}
mini_linq::compile! {{ pipeline {{ plan = mini_linq::passes::plan(no_reduce); }}
    struct Q; relation left(a:i64); relation right(a:i64,b:i64) {declaration};
    {body}
}}.map(|(a,)| *a).collect::<Vec<_>>().into_iter()
}}"#
        )
    };
    let map = "let right = std::collections::HashMap::from([((1_i64,), vec![(10_i64,)])]);";
    let narrow = "let right = std::collections::HashMap::from([((1_i32,), vec![(10_i64,)])]);";
    let probe = "answer(b) :- left(a), right(a,b);";
    let semi = "answer(a) :- left(a), !right(a,_);";
    let exact = "host [a] reads [exact, membership]";
    success(compiler.compile("declared_map", "", &query(map, exact, probe)));
    success(compiler.compile("declared_set", "", &query(map, exact, semi)));
    // The slice declares no keyed read: it is scanned into a fallback.
    success(compiler.compile("undeclared_slice", "", &query("", "", probe)));
    for (name, host, body, obligation) in [
        ("slice_exact", "", probe, "IndexRead"),
        ("slice_membership", "", semi, "ExactMembership"),
        ("narrow_key_exact", narrow, probe, "IndexRead"),
        ("narrow_key_membership", narrow, semi, "ExactMembership"),
    ] {
        rejected(
            compiler.compile(name, "", &query(host, exact, body)),
            obligation,
        );
    }
}
