//! Parse one query into the typed CQ source IR and inspect its structure.

use cq::symbol_name;
use cq_lang_cq as cq;

fn source() -> cq::Program {
    syn::parse2(quote::quote! {
        struct TriangleProgram;
        relation R(c0: i32, c1: i32);
        relation S(c0: i32, c1: i32);
        relation T(c0: i32, c1: i32);
        triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
    })
    .expect("the quoted CQ example must parse")
}

fn main() {
    let source = source();
    cq::contract::check(&source).expect("the inspected triangle satisfies the CQ contract");

    println!(
        "result: {}/{}",
        symbol_name(&source.query.head.relation),
        source.query.head.args.len()
    );

    for (position, item) in source.query.body.iter().enumerate() {
        #[allow(irrefutable_let_patterns)]
        let cq::BodyItem::Positive { atom } = item else {
            continue;
        };
        let variables = atom
            .variables()
            .map(symbol_name)
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "body {position}: {}({variables})",
            symbol_name(&atom.relation)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::{Rows, cq_semantics, databases};
    use quote::ToTokens;

    #[test]
    fn public_cq_shape_and_token_roundtrip_preserve_the_inspected_query() {
        let source = source();
        cq::contract::check(&source).unwrap();
        assert_eq!(symbol_name(&source.name), "TriangleProgram");
        assert_eq!(symbol_name(&source.query.head.relation), "triangle");
        assert_eq!(source.query.head.args.len(), 3);
        let body: Vec<_> = source
            .query
            .body
            .iter()
            .map(|item| {
                let cq::BodyItem::Positive { atom } = item else {
                    panic!("triangle has only positive atoms");
                };
                (
                    symbol_name(&atom.relation),
                    atom.variables().map(symbol_name).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            body,
            vec![
                ("R".into(), vec!["x".into(), "y".into()]),
                ("S".into(), vec!["y".into(), "z".into()]),
                ("T".into(), vec!["z".into(), "x".into()]),
            ]
        );
        let roundtrip: cq::Program = syn::parse2(source.to_token_stream()).unwrap();
        assert_eq!(roundtrip, source);
    }

    #[test]
    fn inspected_triangle_agrees_with_public_corpus_and_independent_sets() {
        let source = source();
        let case = cq_testkit::CASES
            .iter()
            .find(|case| case.name == "triangle")
            .unwrap();
        assert_eq!(
            cq_semantics::evaluate_cq(&source, &case.database()),
            case.expected_rows()
        );
        for seed in 0..32 {
            let database = databases::binary_database(&source, seed);
            let mut expected = Rows::new();
            for r in &database["R"] {
                for s in &database["S"] {
                    for t in &database["T"] {
                        if r[1] == s[0] && s[1] == t[0] && t[1] == r[0] {
                            expected.insert(vec![r[0], r[1], s[1]]);
                        }
                    }
                }
            }
            assert_eq!(cq_semantics::evaluate_cq(&source, &database), expected);
        }
    }
}
