::mini_linq::workload_query! {
    pub struct TypedTaggedProducts;
    relation Tag(tag: String, productkey: i64);
    relation Selected(tag: String);
    relation Product(productkey: i64, name: String, price: Decimal, released: Date);

    tagged_products(tag, productkey, price, released) :-
        Selected(tag), Tag(tag, productkey), Product(productkey, name, price, released);
}
