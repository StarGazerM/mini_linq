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
            let path = artifact(&self.dependencies, name, extension);
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
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "false {obligation} declaration compiled"
    );
    assert!(
        error.contains(obligation),
        "expected the {obligation} obligation, got {error}"
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

fn query(ty: &str, support: &str, advice: &str, ordered: bool) -> String {
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
        cq_rewrites::retype(r => {ty} {support}),
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
        out = pipe canon project p keep {{x,y}};
        return out as answer(x,y);
    }}
}}
"#
    )
}

#[test]
fn declared_support_is_checked_even_without_an_optimization_consumer() {
    let compiler = Compiler::new();
    let plain = query("Alias<_,_>", "", "", false);
    // Ordinary exact lookup does not require Copy, exact-size, or key enumeration.
    success(compiler.compile("ordinary", &provider(false, false, false), &plain));
    success(compiler.compile(
        "valid",
        &provider(true, true, true),
        &query(
            "Alias<_,_>",
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
                &query("Alias<_,_>", support, "", false),
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
        ("set_payload", "::std::collections::HashSet<_>"),
        ("alias_payload", "SetAlias<_>"),
        ("keyset_payload", "KeySetAlias<_>"),
    ] {
        rejected(
            compiler.compile(name, aliases, &query(ty, "", "", false)),
            "IndexRead",
        );
    }
    for (name, ty) in [
        ("unordered", "Provider<_,_>"),
        ("unordered_alias", "Alias<_,_>"),
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
    let ordered = |ty: &str, native: bool| {
        let hosts = if native {
            r#"
            let left = std::collections::BTreeMap::from([((1_i64,), vec![()])]);
            let right = std::collections::BTreeMap::from([((1_i64,), vec![(10_i64,), (11,)])]);
        "#
        } else {
            ""
        };
        format!(
            r#"{{ {hosts}
mini_linq::passes::select! {{
    pipeline {{ cq_rewrites::retype(ordered => {ty}),
        mini_linq::passes::sequence, mini_linq::passes::sequential,
        mini_linq::passes::translate_pull }};
    physical {{
        relation L(a:i64) = (&left); relation R(a:i64,b:i64) = (&right);
        l = pipe rename L@0 {{a -> x}};
        r = pipe rename R@1 {{a -> x,b -> y}};
        p = pipe canon merge [x] l,r;
        return p as answer(x,y);
    }}
}}.map(|(x,y)| (*x,*y)).collect::<Vec<_>>().into_iter()
}}"#
        )
    };
    success(compiler.compile(
        "host_ordered",
        aliases,
        &ordered("::std::collections::BTreeMap<_, ::std::vec::Vec<_>>", true),
    ));
    // An unused fallback type must not impose bounds on a direct native read.
    success(compiler.compile(
        "host_native_with_alias",
        aliases,
        &ordered("SetAlias<_>", true),
    ));
    rejected(
        compiler.compile(
            "host_unordered_array",
            aliases,
            &ordered("SetAlias<_>", false),
        ),
        "OrderedRead",
    );

    let interval = |ty: &str| {
        format!(
            r#"{{
        let left = [(1_i64, 10_i64), (2, 20)];
        let right = [(1_i64, 1_i64), (2, 2)];
        mini_linq::passes::select! {{
            arguments {{ positional(L,R) }};
            pipeline {{ cq_rewrites::retype(kind(membership) => {ty}),
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
                out = pipe canon project j keep {{x,y}};
                return out as answer(x,y);
            }}
        }}.map(|(x,y)| (*x,*y)).collect::<Vec<_>>().into_iter()
    }}"#
        )
    };
    success(compiler.compile(
        "exact_interval",
        aliases,
        &interval("::cq_storage::AdaptiveKeySet<_, ::cq_storage::I64Codec, 64>"),
    ));
    rejected(
        compiler.compile(
            "point_only_interval_alias",
            aliases,
            &interval("SetAlias<_>"),
        ),
        "ExactIntervalMembership",
    );
    let codec_alias = "type WrongCodec = cq_storage::BorrowedI64TupleCodec;";
    // build! selects the generic Build obligation through BuildProvider; an
    // incompatible codec prevents that implementation from being available.
    rejected(
        compiler.compile(
            "wrong_key_codec_alias",
            codec_alias,
            &interval("::cq_storage::AdaptiveKeySet<_, WrongCodec, 64>"),
        ),
        "BuildProvider",
    );
}

#[test]
fn candidate_directory_requires_candidates_not_just_matching_level_syntax() {
    let compiler = Compiler::new();
    let expression = |ty: &str| {
        format!(
            r#"{{
        let lrows = [(1_i64, 2_i64)];
        let rrows = [(2_i64, 3_i64)];
        let srows = [(1_i64, 3_i64)];
        mini_linq::passes::select! {{
            pipeline {{ cq_rewrites::retype(r => {ty}),
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
                checked = pipe canon semi eager [a,c] candidate,s;
                return checked as answer(a,b,c);
            }}
        }}.map(|(a,b,c)| (*a,*b,*c)).collect::<Vec<_>>().into_iter()
    }}"#
        )
    };
    success(compiler.compile(
        "valid_directory",
        "",
        &expression("::cq_storage::Trie<'_, _, ::cq_storage::HashSet<_>>"),
    ));
    rejected(
        compiler.compile(
            "invalid_directory_alias",
            "type SetAlias<K> = std::collections::HashSet<K>;",
            &expression("SetAlias<_>"),
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
        out = pipe canon project selected keep {a,b};
        return out as answer(a,b);
    }
    structures {
        r host scan = ::core::iter::Map<_,_> [];
        selected read = positional base R lineage {a -> a,b -> b}
            column a marker MarkerAlias predicate physical;
        out canon = ::std::collections::HashSet<_>;
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
                out = pipe canon project joined keep {k,p,v};
                return out as answer(k,p,v);
            }
            structures {
                l host scan = ::core::iter::Map<_,_> [];
                r host scan = ::core::iter::Map<_,_> [];
                r = ::cq_storage::RadixRun<_,_> [k] supports [keys];
                l join_filter = ::cq_storage::AdaptiveKeySet<i64,::cq_storage::I64Codec,64>
                    build r [k] covering joined marker MarkerAlias;
                out canon = ::std::collections::HashSet<_>;
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
