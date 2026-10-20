use cq_lang_datalog_incremental::{Program,contract,check_rewrite,strata,source};
use quote::{quote,ToTokens};
#[test]
fn generators_round_trip_and_survive_schedule_correspondence() {
    let source:source::Program=syn::parse_quote! {
        struct Generated; relation Seed(x:i32) = (seeds); relation Reach(x:i32); output Reach;
        Reach(y) :- Seed(x), for (y,_) in [(*x,())];
        Reach(y) :- Reach(x), if *x < 3, for y in [*x+1,*x+1];
    };
    let strata:strata::Program=syn::parse2(quote!(#source strata {component [Reach];})).unwrap();
    let p:Program=syn::parse2(quote!(#strata incremental {
        scheme seminaive_covering; delta Reach = Recent;
        variant 0 seed {Reach(y) :- Seed(x), for (y,_) in [(*x,())];}
        variant 1 delta 0 {Reach(y) :- Recent(x), if *x < 3, for y in [*x+1,*x+1];}
    })).unwrap();
    source::contract::check(&source).unwrap(); strata::contract::check(&strata).unwrap(); contract::check(&p).unwrap();
    assert_eq!(source,syn::parse2::<source::Program>(source.to_token_stream()).unwrap());
    assert_eq!(strata,syn::parse2::<strata::Program>(strata.to_token_stream()).unwrap());
    assert_eq!(p,syn::parse2::<Program>(p.to_token_stream()).unwrap());
    let mut changed=p.clone(); changed.variants[1].query.body[2]=syn::parse_quote!(for y in [*x+2]);
    assert!(check_rewrite(&p,&changed).is_err());
}
#[test]
fn authored_if_let_variants_are_rejected_without_surface_expansion() {
    let p:Program=syn::parse_quote! {
        struct Matched; relation Seed(o:Option<i32>) = (seeds); relation Reach(x:i32); output Reach;
        Reach(x) :- Seed(o), for (x,) in match *o {Some(x)=>Some((x,)),_=>None};
        strata {component [Reach];}
        incremental {scheme seminaive_covering; delta Reach = Recent;
            variant 0 seed {Reach(x) :- Seed(o), if let Some(x) = *o;}
        }
    };
    assert!(contract::check(&p).unwrap_err().to_string().contains("surface expansion"));
}
