#![recursion_limit = "1024"]
#[macro_export]
macro_rules! relation_storage {
    ($rows:expr) => {
        $rows.into_iter().collect()
    };
}
doop_shared::doop_ascent_engine!(workers, ascent::ascent_par, crate::relation_storage);
pub use workers::run;
