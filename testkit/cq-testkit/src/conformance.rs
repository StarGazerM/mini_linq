//! Executable answer conformance: ignore enumeration order, not multiplicity.
//!
//! `assert_answers_equal` is dependency-light and also serves external targets.
//! Enable `sequence-conformance` for the CPU translation driver. Each target's
//! machine and catalog drive the shared planner before translation.

use std::collections::BTreeMap;
use std::fmt::Debug;

/// Compare answer rows after the answer map. Canonical outputs compare as sets;
/// other outputs compare as multisets. Report the first differing row and counts.
pub fn assert_answers_equal<Row: Ord + Debug>(
    reference_name: &str,
    reference: impl IntoIterator<Item = Row>,
    target_name: &str,
    actual: impl IntoIterator<Item = Row>,
    canonical: bool,
) {
    fn counts<Row: Ord>(
        rows: impl IntoIterator<Item = Row>,
        canonical: bool,
    ) -> BTreeMap<Row, usize> {
        let mut counts = BTreeMap::new();
        for row in rows {
            let count = counts.entry(row).or_default();
            if canonical {
                *count = 1;
            } else {
                *count += 1;
            }
        }
        counts
    }
    let reference = counts(reference, canonical);
    let actual = counts(actual, canonical);
    for row in reference.keys().chain(actual.keys()) {
        let expected = reference.get(row).copied().unwrap_or(0);
        let observed = actual.get(row).copied().unwrap_or(0);
        assert_eq!(
            expected, observed,
            "answer divergence: {reference_name} vs {target_name}; row {row:?}; \
             {reference_name} multiplicity {expected}, {target_name} multiplicity {observed}"
        );
    }
}

#[cfg(feature = "sequence-conformance")]
pub use cpu::*;

#[cfg(feature = "sequence-conformance")]
mod cpu {
    use super::assert_answers_equal;
    use crate::Database;
    use cq_lang_sequence::{Analysis, Catalog, Program, Term};
    use cq_syntax::symbol_name;
    use quote::{ToTokens, quote};

    /// CPU translations available to the executable testkit.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Target {
        Sequential,
        Tasks,
    }

    /// Plan a Sequence program with the target's exported machine and catalog.
    /// Authored decomposition is retained; missing decisions come only from the
    /// shared planner, never from harness-specific store or grading preparation.
    pub fn plan(program: &Program, target: Target) -> syn::Result<(Program, Analysis)> {
        plan_with_catalog(program, target, &Catalog::default())
    }

    /// Plan using the target catalog extended with caller-owned runtime tokens.
    pub fn plan_with_catalog(
        program: &Program,
        target: Target,
        additions: &Catalog,
    ) -> syn::Result<(Program, Analysis)> {
        let (machine, mut catalog) = match target {
            Target::Sequential => (cq_sequence_pull::machine(), cq_sequence_pull::catalog()),
            Target::Tasks => (cq_sequence_tasks::machine(), cq_sequence_tasks::catalog()),
        };
        catalog
            .providers
            .extend(additions.providers.iter().cloned());
        catalog.codecs.extend(additions.codecs.iter().cloned());
        catalog.canons.extend(additions.canons.iter().cloned());
        let planned = cq_sequence_plan::plan(program, &machine, &catalog)?;
        let analysis = cq_lang_sequence::check_grading(&planned.program, &machine)?;
        Ok((planned.program, analysis))
    }

    /// Whether the final answer stream has an explicit global canonical boundary.
    /// Partition-local canonicalization does not establish global uniqueness.
    fn output_is_canonical(program: &Program, analysis: &Analysis) -> bool {
        fn canonical(
            term: &Term,
            dimensions: usize,
            program: &Program,
            analysis: &Analysis,
        ) -> bool {
            match term {
                Term::Canon(_) => dimensions == 1,
                Term::Store { inner, .. } => canonical(inner, dimensions, program, analysis),
                Term::Tile { inner, .. } => canonical(inner, dimensions - 1, program, analysis),
                Term::Flatten(inner) => canonical(inner, dimensions + 1, program, analysis),
                Term::Ref(name) => program
                    .sequence
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *name)
                    .is_some_and(|definition| {
                        canonical(
                            &definition.term,
                            analysis.shapes[&symbol_name(name)].len(),
                            program,
                            analysis,
                        )
                    }),
                _ => false,
            }
        }
        let Some(last) = program.sequence.definitions.last() else {
            return false;
        };
        if let Some(result) = analysis.results.get(&symbol_name(&last.name)) {
            if analysis.structures.head_dedup.contains_key(result) {
                return true;
            }
            // A canonical valuation stream may still project to repeated
            // answers. Only an injective answer map preserves its uniqueness.
            let head = &program.structures.physical.program.head;
            if !analysis.structures.physical.headings[result]
                .iter()
                .all(|column| {
                    head.args
                        .iter()
                        .filter_map(cq_syntax::rust::variable)
                        .any(|variable| symbol_name(variable) == *column)
                })
            {
                return false;
            }
        }
        program
            .sequence
            .definitions
            .last()
            .is_some_and(|definition| {
                canonical(
                    &definition.term,
                    analysis.shapes[&symbol_name(&definition.name)].len(),
                    program,
                    analysis,
                )
            })
    }

    /// Compare translations on deterministic empty, full and sampled binary-domain
    /// databases. Use `assert_conformant_on` for an authored corpus database.
    pub fn assert_conformant(program: &Program, targets: &[Target]) {
        for seed in [0, 1, 7] {
            let database = crate::databases::binary_relations(
                program
                    .structures
                    .physical
                    .program
                    .declarations
                    .iter()
                    .map(|relation| (symbol_name(&relation.name), relation.columns.len())),
                seed,
            );
            assert_conformant_on(program, targets, &database);
        }
    }

    /// Compare translations using exactly one shared finite integer database.
    pub fn assert_conformant_on(program: &Program, targets: &[Target], database: &Database) {
        let borrowed: Vec<_> = database
            .iter()
            .map(|(name, rows)| {
                (
                    name.as_str(),
                    rows.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                )
            })
            .collect();
        let inputs: Vec<_> = borrowed
            .iter()
            .map(|(name, rows)| (*name, rows.as_slice()))
            .collect();
        assert_conformant_with_inputs(program, targets, &inputs);
    }

    /// Compare translations on raw authored inputs (the same representation as
    /// `Case::inputs`), retaining duplicate input rows.
    pub fn assert_conformant_with_inputs(
        program: &Program,
        targets: &[Target],
        inputs: &[(&str, &[&[i32]])],
    ) {
        let declarations = program
            .structures
            .physical
            .program
            .declarations
            .iter()
            .map(|relation| {
                let name = &relation.name;
                let types: Vec<_> = relation.columns.iter().map(|column| &column.ty).collect();
                let rows = inputs
                    .iter()
                    .find(|(input, _)| *input == symbol_name(name))
                    .unwrap_or_else(|| panic!("missing conformance input {name}"))
                    .1;
                let values = rows.iter().map(|row| {
                    assert_eq!(row.len(), types.len(), "input width for {name}");
                    quote!((#(#row as #types,)*))
                });
                quote!(let #name: Vec<(#(#types,)*)> = vec![#(#values),*];)
            })
            .collect::<Vec<_>>();
        assert_conformant_with_source(program, targets, &quote!(#(#declarations)*));
    }

    /// Execute a shared caller-authored input setup, for example the corpus's
    /// map-host fixtures. Setup is inserted unchanged for every translation.
    pub fn assert_conformant_with_source(
        program: &Program,
        targets: &[Target],
        setup: &proc_macro2::TokenStream,
    ) {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../runtime/cq-storage/src/lib.rs");
        let runtime = crate::rustc::Runtime::new(&source, false);
        assert_conformant_with_runtime(program, targets, setup, &runtime, &Catalog::default());
    }

    /// Reuse a rustc driver with the caller's exact linked provider dependencies.
    /// Caller-owned provider, codec and canon tokens extend each target catalog.
    /// Like `Runtime`, this is a serial driver: do not run simultaneous queries
    /// against one runtime directory.
    pub fn assert_conformant_with_runtime(
        program: &Program,
        targets: &[Target],
        setup: &proc_macro2::TokenStream,
        runtime: &crate::rustc::Runtime,
        catalog: &Catalog,
    ) {
        assert!(
            targets.len() >= 2,
            "conformance requires at least two targets"
        );
        let mut reference: Option<(Target, Vec<String>)> = None;
        for &target in targets {
            let (graded, analysis) = plan_with_catalog(program, target, catalog)
                .unwrap_or_else(|error| panic!("{target:?} planning rejected: {error}"));
            let expression = match target {
                Target::Sequential => cq_sequence_pull::translate(&graded, &analysis),
                Target::Tasks => cq_sequence_tasks::translate(&graded, &analysis),
            }
            .unwrap_or_else(|error| panic!("{target:?} translation failed: {error}"));
            let generated = quote! {
                #![allow(non_snake_case, unused_variables)]
                fn main() {
                    #setup
                    for row in #expression {
                        println!("{:?}", row);
                    }
                }
            };
            eprintln!("conformance: compiling {target:?}");
            let output = runtime.run(&generated.to_token_stream().to_string());
            let rows: Vec<_> = String::from_utf8(output.stdout)
                .expect("UTF-8 answer rows")
                .lines()
                .map(str::to_owned)
                .collect();
            if let Some((first, expected)) = &reference {
                assert_answers_equal(
                    &format!("{first:?}"),
                    expected.iter(),
                    &format!("{target:?}"),
                    rows.iter(),
                    output_is_canonical(&graded, &analysis),
                );
            } else {
                reference = Some((target, rows));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::assert_answers_equal;

    #[test]
    fn order_is_not_observable() {
        assert_answers_equal(
            "pull",
            [vec![1], vec![2], vec![1]],
            "tasks",
            [vec![1], vec![1], vec![2]],
            false,
        );
    }

    #[test]
    fn canonical_answers_compare_as_sets() {
        assert_answers_equal("pull", [vec![1], vec![1]], "tasks", [vec![1]], true);
    }

    #[test]
    #[should_panic(expected = "answer divergence: pull vs tasks; row [1]")]
    fn noncanonical_multiplicity_is_observable() {
        assert_answers_equal("pull", [vec![1], vec![1]], "tasks", [vec![1]], false);
    }

    #[test]
    #[should_panic(expected = "answer divergence: pull vs tasks; row [2]")]
    fn additional_answer_is_observable() {
        assert_answers_equal("pull", [vec![1]], "tasks", [vec![1], vec![2]], true);
    }
}
