use cq_lang_physical as ir;

fn main() -> syn::Result<()> {
    let physical: ir::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L@0 {a -> a,b -> b};
        r = pipe rename R@1 {b -> b,c -> c};
        s0 = pipe rename S@2 {a -> a,c -> c};
        s = let project s0 keep {a,c};
        candidate = pipe probe smallest [b] l,r;
        checked = pipe canon semi eager [a,c] candidate,s;
        return checked as answer(a,b,c);
    });
    ir::contract::check(&physical)?;
    eprintln!("Physical accepts the mixed host/retained smallest intersection");
    cq_structure_selection::compile(&physical)?;
    Ok(())
}
