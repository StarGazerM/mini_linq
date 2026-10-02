use cq_ext_support::{
    Fresh, Leaf, Rules, check_fresh, check_leaves, distinct, ensure, hoist, same_symbol,
};
use cq_lang_cq::Program;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub holes: Vec<Program>,
    pub leaves: Vec<Leaf>,
    pub shared_distinct: bool,
}

fn domain(source: &Rules) -> syn::Result<()> {
    for query in source.queries() {
        ensure(
            same_symbol(&query.head.relation, &source.first.query.head.relation),
            query,
            "union rules must have the same head relation",
        )?;
        ensure(
            query.head.args.len() == source.first.query.head.args.len(),
            query,
            "union heads must have the same arity",
        )?;
    }
    if let Some(output) = source.first.output() {
        output.check_result()?;
    }
    Ok(())
}

pub fn compile(source: &Rules) -> syn::Result<Context> {
    domain(source)?;
    let (declarations, leaves) = hoist(&source.first, &mut Fresh::new(source));
    let holes = source
        .queries()
        .map(|query| {
            let mut hole = source.first.clone();
            hole.declarations = declarations.clone();
            hole.query = query.clone();
            hole
        })
        .collect();
    Ok(Context {
        holes,
        leaves,
        shared_distinct: true,
    })
}

/// Rule bijection, unchanged heads/bodies/schema, and one shared distinct sink.
/// Core validity is checked after caller-selected surface expansion, not here.
pub fn check(source: &Rules, target: &Context) -> syn::Result<()> {
    domain(source)?;
    check_fresh(source, target.leaves.iter().map(|leaf| &leaf.binding))?;
    ensure(
        target.shared_distinct,
        source,
        "union needs one shared distinct sink",
    )?;
    ensure(
        target.holes.len() == source.queries().count(),
        source,
        "missing or extra disjunct",
    )?;
    for (query, hole) in source.queries().zip(&target.holes) {
        check_leaves(&source.first, &hole.declarations, &target.leaves)?;
        let mut erased = hole.clone();
        erased.declarations = source.first.declarations.clone();
        erased.query = source.first.query.clone();
        ensure(erased == source.first, hole, "hole header changed")?;
        ensure(hole.query == *query, hole, "disjunct changed")?;
    }
    Ok(())
}

pub fn render(context: &Context, pipeline: &cq_stage::Pipeline) -> syn::Result<TokenStream> {
    let mut fresh = Fresh::new(
        context
            .holes
            .iter()
            .map(ToTokens::to_token_stream)
            .collect::<TokenStream>(),
    );
    let leaves = &context.leaves;
    let mut holes = context.holes.iter();
    let first = holes.next().expect("a parsed union has a rule");
    let first = pipeline.clone().start(first.to_token_stream())?;
    let mut stream = quote!(::core::iter::once_with(|| #first).flatten());
    for hole in holes {
        let invocation = pipeline.clone().start(hole.to_token_stream())?;
        stream = quote!(#stream.chain(::core::iter::once_with(|| #invocation).flatten()));
    }
    let filtered = distinct(stream, &mut fresh, "union_seen");
    let result = if leaves.is_empty() {
        filtered
    } else {
        // Local owned leaves cannot be borrowed by an escaping lazy closure.
        quote!(#filtered.collect::<::std::vec::Vec<_>>().into_iter())
    };
    Ok(quote!({
        #(#leaves)*
        #result
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Rules {
        syn::parse_quote! {
            struct U; relation A(x:i32) = (make()); relation B(x:i32);
            answer(x) :- A(x), arbitrary!{opaque tokens}; answer(x) :- B(x);
        }
    }
    #[test]
    fn testkit_checks_disjunct_correspondence() {
        cq_testkit::PassCase {
            source: source(),
            expected_next: None,
            compile: Box::new(compile),
            check: Box::new(check),
            mutations: vec![Box::new(|context: &Context| {
                let mut changed = context.clone();
                changed.holes.pop();
                changed
            })],
            answers: None,
        }
        .run(None)
        .unwrap();
    }
    #[test]
    fn correspondence_accepts_passthrough_and_rejects_four_mutations() {
        let source = source();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        let mut changed = target.clone();
        changed.holes.pop();
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.holes[1].query.head = syn::parse_quote!(answer(1));
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.shared_distinct = false;
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.leaves[0].expression = syn::parse_quote!(other());
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn incompatible_heads_are_rejected() {
        for text in [
            "struct U; answer(x) :- A(x); other(x) :- B(x);",
            "struct U; answer(x) :- A(x); answer(x,y) :- B(x,y);",
        ] {
            assert!(compile(&syn::parse_str(text).unwrap()).is_err());
        }
    }
}
