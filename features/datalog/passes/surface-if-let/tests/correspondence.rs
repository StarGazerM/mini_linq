use cq_datalog_surface_if_let::{check, compile};
use cq_lang_datalog_source::{Program, contract};
use quote::{ToTokens, quote};
#[test]
fn sugar_expands_independently_and_mutations_fail() {
    let source = quote! {
        struct Matched;
        relation Input(o:Option<(i32,(i32,i32))>,k:Kind) = (input);
        relation Out(x:i32,y:i32); output Out;
        Out(x,y) :- Input(o,k), if let Some((x,(y,_))) = *o,
            if let Kind::A = *k, if x < y;
    };
    let expected: Program = syn::parse_quote! {
        struct Matched;
        relation Input(o:Option<(i32,(i32,i32))>,k:Kind) = (input);
        relation Out(x:i32,y:i32); output Out;
        Out(x,y) :- Input(o,k),
            for (x,y,) in match *o {
                Some((x,(y,_))) => ::core::option::Option::Some((x,y,)),
                _ => ::core::option::Option::None,
            },
            for () in match *k {
                Kind::A => ::core::option::Option::Some(()),
                _ => ::core::option::Option::None,
            }, if x < y;
    };
    check(&source, &expected.to_token_stream()).unwrap();
    let target = compile(&source).unwrap();
    assert_eq!(syn::parse2::<Program>(target.clone()).unwrap(), expected);
    check(&source, &target).unwrap();
    assert!(contract::check(&syn::parse2::<Program>(source.clone()).unwrap()).is_err());
    for mutate in [0, 1, 2, 3, 4, 5] {
        let mut p = expected.clone();
        match mutate {
            0 => p.rules[0].body[1] = syn::parse_quote!(for (x,y) in [(0,0)]),
            1 => {
                let first = p.rules[0].body[1].clone();
                p.rules[0].body[1] = p.rules[0].body[2].clone();
                p.rules[0].body[2] = first;
            }
            2 => p.rules[0].head.args[0] = syn::parse_quote!(y),
            3 => p.observer = Some(syn::parse_quote!(observer)),
            4 => {
                p.rules[0].body.pop();
            }
            _ => {
                p.models.insert("Out".into(), syn::parse_quote!(bag));
            }
        }
        assert!(check(&source, &p.to_token_stream()).is_err());
    }
    assert!(check(&source, &source).is_err());
}
#[test]
fn untouched_source_is_checked_identity() {
    let source = quote!(struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), if *x > 0;);
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let before: Program = syn::parse2(source).unwrap();
    assert_eq!(before, syn::parse2::<Program>(target).unwrap());
}
#[test]
fn if_let_inherits_generator_scope_and_fresh_binding_obligations() {
    for body in [
        "Input(o), if let Some(x) = *o",
        "Input(o), if let Some((x, _)) = *o",
        "Input(o), if let Some(x) = *o, let y = x",
    ] {
        let source = format!(
            "struct P; relation Input(o:Option<i32>) = (input); relation Out(x:i32); output Out; Out(x) :- {body};"
        );
        let output = compile(&source.parse().unwrap()).unwrap();
        contract::check(&syn::parse2::<Program>(output).unwrap()).unwrap();
    }
    for body in [
        "Input(o), let y = x, if let Some(x) = *o",
        "if let Some(x) = *o, Input(o)",
        "Input(o), if let Some(o) = *o",
        "Input(o), if let Some(x) = *o, if let Some(x) = *o",
        "Input(o), if let None = *o",
    ] {
        let source = format!(
            "struct P; relation Input(o:Option<i32>) = (input); relation Out(x:i32); output Out; Out(x) :- {body};"
        );
        assert!(compile(&source.parse().unwrap()).is_err(), "{body}");
    }
}
#[test]
fn later_stage_sugar_is_not_implicitly_expanded() {
    let text = quote!(struct P; relation E(o:Option<i32>) = (e); relation A(x:i32); output A;
        A(x) :- E(o), if let Some(x) = *o; strata {component [A];});
    assert!(compile(&text).is_err());
}
