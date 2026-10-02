use cq_lang_physical::{self as ir, Choice, Operator};

fn main() -> syn::Result<()> {
    let source: ir::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32);
        relation R(b:i32,c:i32);
        relation S(a:i32,c:i32);
        l = pipe rename L {a -> a,b -> b};
        r = pipe rename R {b -> b,c -> c};
        candidate = pipe probe first [b] l,r;
        s = pipe rename S {a -> a,c -> c};
        checked = pipe canon probe first [a,c] candidate,s;
        return checked as answer(a,b,c);
    });
    let mut target = source.clone();
    let renamed = target.program.definitions.remove(3);
    target.program.definitions.insert(2, renamed);
    if let Operator::Probe(_, key) = &mut target.program.definitions[3].operator {
        key.choice = Some(Choice::Smallest(Default::default()));
    }
    ir::contract::check(&source)?;
    ir::contract::check(&target)?;
    eprintln!("both Physical contracts accept the independent rename hoist");
    ir::check_rewrite(&source, &target)
}
