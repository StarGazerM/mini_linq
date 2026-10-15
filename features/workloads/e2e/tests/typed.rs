//! Run each authored typed query over CSV and Parquet fixtures.
use cq_workloads::typed::Source;

#[path = "typed/typed_line_filter.rs"]
mod typed_line_filter;
#[path = "typed/typed_customer_orders.rs"]
mod typed_customer_orders;
#[path = "typed/typed_calendar_shipping.rs"]
mod typed_calendar_shipping;
#[path = "typed/typed_priced_products.rs"]
mod typed_priced_products;
#[path = "typed/typed_tagged_products.rs"]
mod typed_tagged_products;
