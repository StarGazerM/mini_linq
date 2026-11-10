use crate::typed::{self, Source};

// Tag owns the `tag` domain. Selected tags are encoded through that same
// domain, so the join compares codes of one dictionary; a selected tag absent
// from Tag maps to the non-member code and joins nothing.
cq_load::interned_relation_loaders! {
    relation Tag(tag: cq_load::StringCode, productkey: i64);
    relation Product(productkey: i64, name: cq_load::StringCode, price: cq_load::Decimal, released: cq_load::Date);
}

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let mut dictionaries = cq_load::Dictionaries::default();
    let Tag = &<relation_loader!(Tag)>::intern(
        typed::load::<typed::Tag>("typed_tagged_products", source),
        &mut dictionaries,
    )[..];
    let Selected = &typed::load::<typed::Selected>("typed_tagged_products", source)
        .into_iter()
        .map(|(tag,)| (cq_storage::domain::exact(dictionaries.domain("tag"), &tag),))
        .collect::<Vec<_>>()[..];
    let Product = &<relation_loader!(Product)>::intern(
        typed::load::<typed::TaggedProduct>("typed_tagged_products", source),
        &mut dictionaries,
    )[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Tag, Selected, Product)); }
        pub struct TypedTaggedProducts;
        relation Tag(tag: cq_load::StringCode, productkey: i64);
        relation Selected(tag: cq_load::StringCode);
        relation Product(productkey: i64, name: cq_load::StringCode, price: cq_load::Decimal, released: cq_load::Date);

        tagged_products(tag, productkey, price, released) :-
            Selected(tag), Tag(tag, productkey), Product(productkey, name, price, released);
    }.map(|(a,b,c,d)| vec![
        // DECODE P: publish the tag in the typed tagged-products answer.
        dictionaries.publish("tag", *a),
        b.to_string(), c.to_string(), d.to_string(),
    ]).collect();
    typed::check("typed_tagged_products", rows);
}
