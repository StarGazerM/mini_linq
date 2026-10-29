use std::collections::BTreeSet;

use cq_lang_physical as physical;

#[test]
fn rust_reads_distinguishes_columns_from_closed_captures_without_recognition() {
    for (expression, expected) in [
        ("compute(x, capture)", vec!["x"]),
        ("compute(capture)", vec![]),
        ("opaque!(capture)", vec!["x"]),
    ] {
        let program: physical::Program = syn::parse_str(&format!(
            "physical {{
                relation R(x: HostType);
                r = pipe rename R@0 {{ x -> x }};
                b = pipe bind r with y: Output = ({expression});
                return canon b as answer(x, y);
            }}"
        ))
        .unwrap();
        let analysis = physical::contract::check(&program).unwrap();
        assert_eq!(
            analysis.rust_reads(&program.program.definitions[0].operator),
            None
        );
        assert_eq!(
            analysis.rust_reads(&program.program.definitions[1].operator),
            Some(
                expected
                    .into_iter()
                    .map(String::from)
                    .collect::<BTreeSet<_>>()
            ),
        );
    }
}

#[test]
fn rust_reads_respects_the_recorded_lexical_environment() {
    let program: physical::Program = syn::parse_quote! { physical {
        relation R(x: HostType, z: HostType);
        r = pipe rename R@0 { x -> x, z -> z };
        g = pipe guard r if (opaque!()) scope[x];
        return canon g as answer(x, z);
    }};
    let analysis = physical::contract::check(&program).unwrap();
    assert_eq!(
        analysis.rust_reads(&program.program.definitions[1].operator),
        Some(BTreeSet::from([String::from("x")])),
    );
}
