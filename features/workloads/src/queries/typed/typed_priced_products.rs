use crate::typed::{self, Source};

cq_load::interned_relation_loaders! {
    relation Price(price: cq_load::Decimal, label: cq_load::StringCode);
    relation Product(price: cq_load::Decimal, productkey: i64, name: cq_load::StringCode, released: cq_load::Date);
}

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let mut dictionaries = cq_load::Dictionaries::default();
    let Price = &<relation_loader!(Price)>::intern(
        typed::load::<typed::Price>("typed_priced_products", source),
        &mut dictionaries,
    )[..];
    let Product = &<relation_loader!(Product)>::intern(
        typed::load::<typed::PricedProduct>("typed_priced_products", source),
        &mut dictionaries,
    )[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Price, Product)); }
        pub struct TypedPricedProducts;
        relation Price(price: cq_load::Decimal, label: cq_load::StringCode);
        relation Product(price: cq_load::Decimal, productkey: i64, name: cq_load::StringCode, released: cq_load::Date);

        priced_products(label, productkey, name, released) :-
            Price(price, label), Product(price, productkey, name, released);
    }.map(|(a,b,c,d)| vec![
        // DECODE P: publish the price label in the typed priced-products answer.
        dictionaries.publish("label", *a),
        b.to_string(),
        // DECODE P: publish the product name in the typed priced-products answer.
        dictionaries.publish("name", *c),
        d.to_string(),
    ]).collect();
    typed::check("typed_priced_products", rows);
}
