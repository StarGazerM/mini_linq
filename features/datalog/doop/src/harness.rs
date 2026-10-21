use doop_shared::harness::{Engine, Harness, Result};

pub const ENGINES: &[Engine] = &[
    #[cfg(feature = "mini")]
    Engine {
        name: "mini",
        evaluate: doop_mini::run,
        threads: || 1,
    },
    #[cfg(feature = "mini-fx")]
    Engine {
        name: "mini-fx",
        evaluate: doop_mini_fx::run,
        threads: || 1,
    },
    #[cfg(feature = "ascent")]
    Engine {
        name: "ascent",
        evaluate: doop_ascent::run,
        threads: || 1,
    },
];

pub fn main(args: impl IntoIterator<Item = String>) -> Result<()> {
    let harness = Harness {
        engines: ENGINES,
        defaults: &[
            #[cfg(feature = "mini")]
            "mini",
            #[cfg(feature = "ascent")]
            "ascent",
        ],
        reference: "ascent",
        checked: &["mini", "mini-fx"],
        options: &[],
        usage: "usage: doop <facts_dir> [--engine mini|mini-fx|ascent|both] [--check]",
    };
    let options = harness.parse(args)?;
    harness.execute(&options).map_err(Into::into)
}
