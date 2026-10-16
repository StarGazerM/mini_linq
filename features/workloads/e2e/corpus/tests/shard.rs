//! One shard of the accepted corpus configurations; build.rs deals every
//! accepted invocation into exactly one `shard_NNN` target.
include!(concat!(
    env!("OUT_DIR"),
    "/",
    env!("CARGO_CRATE_NAME"),
    ".rs"
));
