//! Cross-pass checks live here so each pass keeps only its two language dependencies.
#![cfg(all(
    feature = "hypergraph",
    feature = "index-demands",
    feature = "physical-lowering"
))]
// The shared interpreters import syntax through this name; use RA's public
// re-export without adding another dependency to the facade.
extern crate self as cq_syntax;
use cq_lang_cq as cq;
use cq_lang_ix as ix;
use cq_lang_physical as physical;
use cq_lang_ra as ra;
pub use cq_lang_ra::syntax::{RelationDecl, predicate, rust, symbol_name};

#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
#[allow(dead_code)]
#[path = "../../../test-support/relational/values.rs"]
mod values;

#[test]
fn reduced_plans_compute_the_authored_answers() {
    for case in cases::CASES {
        let source = cq::normal::normalize(&syn::parse_str(case.cq).unwrap()).unwrap();
        let ordered = cq_order::compile(&source, &cq_order::Options::default()).unwrap();
        let options = cq_reduce::Options {
            guards: cq_reduce::Pushdown::Atoms,
            semijoins: if cq::hypergraph::Analysis::of(&source).acyclic {
                cq_reduce::Reductions::Tree
            } else {
                cq_reduce::Reductions::None
            },
        };
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        let target = cq_elaborate::compile(&reduced).unwrap();
        let database = case
            .inputs
            .iter()
            .map(|(name, rows)| {
                (
                    (*name).into(),
                    rows.iter().map(|row| row.to_vec()).collect(),
                )
            })
            .collect();
        let expected: values::Rows = case.expected.iter().map(|row| row.to_vec()).collect();
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn reordered_named_demands_are_accepted_but_each_reader_needs_its_demand() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        l = rename L {a -> a}; r = rename R {a -> a, b -> b}; s = rename S {b -> b, c -> c};
        j0 = natural_join l with r; j1 = natural_join j0 with s;
        x = semijoin j1 with l; p = project x keep {a, c}; output p as answer(a, c);
    } indexes { membership = @l[0]; second = @s[0]; first = @r[0]; }};
    let policy: physical::policy::Policy = syn::parse_quote!(policy { L; R; S; });
    let target = cq_physical_lowering::compile(&source, &policy).unwrap();
    cq_index_policy::check(&source.ra, &source).unwrap();
    cq_physical_lowering::check(&source, &target, &policy).unwrap();
    for (missing, reader) in [(0, "x"), (1, "j1"), (2, "j0")] {
        let mut changed = source.clone();
        changed.indexes = source
            .indexes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != missing)
            .map(|(_, demand)| demand.clone())
            .collect();
        changed.indexes.push_punct(Default::default());
        ix::contract::check(&changed).unwrap();
        assert!(cq_index_policy::check(&source.ra, &changed).is_err());
        for result in [
            cq_physical_lowering::check(&changed, &target, &policy),
            cq_physical_lowering::compile(&changed, &policy).map(|_| ()),
        ] {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(&format!("`{reader}`")), "{error}");
        }
    }
}

#[test]
fn complete_program_parsers_reject_wrong_stage_payloads() {
    let cq = include_str!("../../../test-support/relational/scenarios/scan.cq");
    let ra = include_str!("../../../test-support/relational/scenarios/scan.ra");
    let ix = format!("{ra} indexes {{}}");
    assert!(syn::parse_str::<ix::Program>(cq).is_err());
    assert!(syn::parse_str::<ix::Program>(ra).is_err());
    assert!(syn::parse_str::<cq::Program>(&ix).is_err());
    assert!(syn::parse_str::<ra::Program>(&ix).is_err());
}
