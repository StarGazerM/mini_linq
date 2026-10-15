use crate::typed::{self, Source};

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let Price = &typed::load::<typed::Price>("typed_priced_products", source)[..];
    let Product = &typed::load::<typed::PricedProduct>("typed_priced_products", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Price, Product)); }
        pub struct TypedPricedProducts;
        relation Price(price: cq_load::Decimal, label: String);
        relation Product(price: cq_load::Decimal, productkey: i64, name: String, released: cq_load::Date);

        priced_products(label, productkey, name, released) :-
            Price(price, label), Product(price, productkey, name, released);
    }.map(|(a,b,c,d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()]).collect();
    typed::check("typed_priced_products", rows);
}
