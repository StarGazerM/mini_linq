//! Parse one query into the typed CQ source IR and inspect its structure.

use cq_ir::{cq, symbol_name};

fn main() {
    let source: cq::Module = syn::parse2(quote::quote! {
        struct TriangleProgram;
        relation R(c0: i32, c1: i32);
        relation S(c0: i32, c1: i32);
        relation T(c0: i32, c1: i32);
        triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
    })
    .expect("the quoted CQ example must parse");

    println!(
        "result: {}/{}",
        symbol_name(&source.program.query.head.relation),
        source.program.query.head.args.len()
    );

    for (position, item) in source.program.query.body.iter().enumerate() {
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
