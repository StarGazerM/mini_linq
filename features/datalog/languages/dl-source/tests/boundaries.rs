use cq_lang_datalog_source::{Program, contract, check_rewrite};
use quote::ToTokens;
#[test]
fn models_are_explicit_complete_and_set_only() {
    let p: Program = syn::parse_quote!(struct P; relation E(x:i32) = (e); output E;);
    assert_eq!(p.models["E"],"set");
    assert!(p.to_token_stream().to_string().contains("model E set"));
    assert_eq!(p,syn::parse2::<Program>(p.to_token_stream()).unwrap());
    for model in ["bag","lattice","semiring"] {
        let text=format!("struct P; relation E(x:i32) = (e); model E {model}; output E;");
        assert!(contract::check(&syn::parse_str::<Program>(&text).unwrap()).is_err());
    }
    for text in ["model E set; model E set;", "model Missing set;"] {
        assert!(syn::parse_str::<Program>(&format!("struct P; relation E(x:i32) = (e); {text} output E;")).is_err());
    }
    let mut changed=p.clone(); changed.models.clear();
    assert!(contract::check(&changed).is_err());
    changed.models.insert("Missing".into(),syn::parse_quote!(set));
    assert!(contract::check(&changed).is_err());
    assert!(check_rewrite(&p,&changed).is_err());
}
#[test]
fn source_rejects_sugar_and_later_stage_text() {
    let p:Program=syn::parse_quote!(struct P; relation E(o:Option<i32>) = (e); relation A(x:i32); output A; A(x) :- E(o), if let Some(x) = *o;);
    assert!(contract::check(&p).unwrap_err().to_string().contains("surface expansion"));
    for suffix in ["strata {}","incremental {}","storage {}"] {
        assert!(syn::parse_str::<Program>(&format!("struct P; relation E(x:i32) = (e); output E; {suffix}")).is_err());
    }
}
