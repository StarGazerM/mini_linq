#![recursion_limit = "1024"]
doop_shared::doop_ascent_engine!(
    sequential,
    ascent::ascent,
    doop_shared::doop_relation_storage
);
pub use sequential::run;
