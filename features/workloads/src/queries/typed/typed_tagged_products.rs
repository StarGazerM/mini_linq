use crate::typed::{self, Source};

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let Tag = &typed::load::<typed::Tag>("typed_tagged_products", source)[..];
    let Selected = &typed::load::<typed::Selected>("typed_tagged_products", source)[..];
    let Product = &typed::load::<typed::TaggedProduct>("typed_tagged_products", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Tag, Selected, Product)); }
        pub struct TypedTaggedProducts;
        relation Tag(tag: String, productkey: i64);
        relation Selected(tag: String);
        relation Product(productkey: i64, name: String, price: cq_load::Decimal, released: cq_load::Date);

        tagged_products(tag, productkey, price, released) :-
            Selected(tag), Tag(tag, productkey), Product(productkey, name, price, released);
    }.map(|(a,b,c,d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()]).collect();
    typed::check("typed_tagged_products", rows);
}
