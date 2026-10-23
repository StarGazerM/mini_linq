use cq_rewrite_passes::retain_streams;
use cq_testkit::physical as ir;
use proc_macro2::TokenStream;
use quote::quote;

fn chain() -> ir::Program {
    syn::parse_quote! { physical {
        relation A(a:i64); relation B(a:i64,b:i64); relation C(b:i64,c:i64);
        a = pipe rename A@0 {a -> x};
        b = let rename B@1 {a -> x,b -> y};
        p = pipe probe first [x] a,b;
        c = let rename C@2 {b -> y,c -> z};
        q = pipe canon probe first [y] p,c;
        return q as answer(x,y,z);
    }}
}

/// EXT-RetainStream: only the named stream's mode changes; host renames, retained
/// results and a missing selection are rejected.
#[test]
fn retains_only_named_streams_and_rejects_hosts_lets_and_unnamed() {
    let source = chain();
    let actual = retain_streams(&source, quote!(p)).unwrap();
    let mut expected = source.clone();
    expected.program.definitions[2].mode = ir::Mode::Let(Default::default());
    assert_eq!(actual, expected);
    ir::check_rewrite(&source, &actual).unwrap();
    for selection in [quote!(a), quote!(b), quote!(missing), TokenStream::new()] {
        assert!(retain_streams(&source, selection).is_err());
    }
}

#[test]
fn hoisting_relational_builds_preserves_the_checked_graph() {
    let source = chain();
    let actual = cq_rewrite_passes::hoist(&source, quote!(c)).unwrap();
    assert_eq!(actual.program.definitions[0].name, "c");
    ir::check_rewrite(&source, &actual).unwrap();
    assert_eq!(
        cq_rewrite_passes::hoist(&actual, quote!(c)).unwrap(),
        actual
    );
    assert!(cq_rewrite_passes::hoist(&source, quote!(missing)).is_err());
    assert!(cq_rewrite_passes::hoist(&source, TokenStream::new()).is_err());
}

#[test]
fn hoisting_closes_dependencies_without_dropping_operators() {
    let source = chain();
    let actual = cq_rewrite_passes::hoist(&source, quote!(q)).unwrap();
    assert_eq!(actual, source);
    ir::check_rewrite(&source, &actual).unwrap();
}

#[test]
fn hoisting_refuses_an_opaque_dependency() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        r = pipe rename R@0 {a -> x};
        guarded = pipe guard r if (opaque(x));
        projected = pipe canon project guarded keep {x};
        return projected as answer(x);
    }};
    let error = cq_rewrite_passes::hoist(&source, quote!(projected)).unwrap_err();
    assert!(error.to_string().contains("opaque Rust dependency"));
}

#[test]
fn retained_prefix_preserves_answers_and_duplicates_until_canon() {
    let a = [(1i64,), (2,), (1,)];
    let b = [(1i64, 10i64), (1, 11), (2, 20), (3, 30)];
    let c = [(10i64, 100i64), (11, 110), (20, 200), (10, 100)];
    macro_rules! run {
        ($($physical:tt)*) => {{
            let mut rows: Vec<_> = mini_linq::compile! {
                pipeline { physical = [$($physical)*]; }
                struct Q;
                relation A(a:i64) = (&a); relation B(a:i64,b:i64) = (&b);
                relation C(b:i64,c:i64) = (&c);
                answer(x,y,z) :- A(x), B(x,y), C(y,z);
            }
            .map(|(x, y, z)| (*x, *y, *z))
            .collect();
            rows.sort_unstable();
            rows
        }};
    }
    let streamed = run!();
    let retained = run!(::cq_rewrites::retain_streams(r2));
    assert_eq!(streamed, vec![(1, 10, 100), (1, 11, 110), (2, 20, 200)]);
    assert_eq!(retained, streamed);
}
