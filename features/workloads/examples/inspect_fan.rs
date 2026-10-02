use quote::ToTokens;
fn main() {
    let source = syn::parse_quote! {
        struct Triangle;
        relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(a:i32,c:i32);
        answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
    };
    let pipeline = cq_workloads::pipelines::Pipeline {
        plan: 1,
        canon: true,
        smallest: true,
        ..Default::default()
    };
    let physical = pipeline.physical(&source).unwrap();
    println!("{}", physical.to_token_stream());
    println!("active smallest choices: {}", physical.program.definitions.iter().filter(|d|
        matches!(&d.operator, cq_workloads::pipelines::ir::Operator::Probe(_, k) if matches!(k.choice, Some(cq_workloads::pipelines::ir::Choice::Smallest(_))))).count());
}
