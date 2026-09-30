::mini_linq::workload_query! {
    pub struct TypedPricedProducts;
    relation Price(price: Decimal, label: String);
    relation Product(price: Decimal, productkey: i64, name: String, released: Date);

    priced_products(label, productkey, name, released) :-
        Price(price, label), Product(price, productkey, name, released);
}
