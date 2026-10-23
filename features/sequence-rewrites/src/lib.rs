//! Checked, advice-driven Sequence scheduling passes, usable at the public
//! `cq_stage` pipeline boundary. The companion `cq-sequence-rewrite-passes`
//! library exposes the same transformations to tools without proc macros.
//!
//! | Macro | Advice | Effect |
//! |---|---|---|
//! | `store` | `d` | Materialize the named producer without changing its rows. |
//! | `store_level` | `d`, named list, `all`, or empty | Store selected flatten definitions (BE-Offsets). |
//! | `tile` | `d, n` or `d, n, items m` | Wrap in positive contiguous tiles (BE-Partition). Unknown-length stores need capacity. |
//! | `partition` | `d, n` or `d, n, items m` | Pair tile/flatten at the producer chain's storage input, preserving row shape and authored tiles. |
//! | `segment` | `d, n` or `d, n, items m` | Materialize a named flatten's offsets and pair tile/flatten around that store. |
//! | `choose` | `d, len x < k => a, else => b` | Select earlier equal-Structures-result definitions (BE-Choose). |
//! | `output` | `count`, `reserve`, or `bounded` | Set the final store protocol; bounded requires declared capacity. |
//! | `grade` | `machine, d at host.device, ...` | Add/update placements; preserve other grades on the same machine. |
//!
//! Every pass checks the source, target and construction-based support equality.
//! Grading records advice only: it neither resolves the machine nor certifies
//! placements. The facade's grade phase checks those against the target model.
//! Named `store_level` selections must all be flatten terms; `all` and empty
//! advice skip other definitions. The other passes require their stated advice.
//!
//! The macro protocol is `arguments { advice }; pipeline { next, ... };` followed
//! by a complete Sequence program. Core dependencies are public packaged APIs;
//! this workspace does not depend on storage, target translations or test hooks.
#![forbid(unsafe_code)]

use proc_macro::TokenStream;

macro_rules! sequence_rewrite {
    ($name:ident, $($invalid:tt)*) => {
        #[doc = concat!(
            "Checked `", stringify!($name), "` rewrite. Invalid advice is rejected:\n",
            "```compile_fail\n",
            "macro_rules! finish { ($($tokens:tt)*) => { () }; }\n",
            "cq_sequence_rewrites::", stringify!($name), "! {\n",
            "    arguments { ", stringify!($($invalid)*), " };\n",
            "    pipeline { finish };\n",
            "    physical {\n",
            "        relation R(a:i32);\n",
            "        r = pipe canon rename R@0 {a -> a};\n",
            "        return r as answer(a);\n",
            "    }\n",
            "    structures { r host scan = Reader []; r canon = Canon; }\n",
            "    sequence { a = rows r; b = flatten tile 8 a; d = store a; }\n",
            "}\n",
            "```"
        )]
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            cq_stage::rewrite::<cq_lang_sequence::Program>(
                input.into(),
                cq_sequence_rewrite_passes::$name,
            )
            .into()
        }
    };
}

sequence_rewrite!(store, missing);
sequence_rewrite!(store_level, a);
sequence_rewrite!(tile, a, 0);
sequence_rewrite!(partition, missing, 0);
sequence_rewrite!(segment, missing, 0);
sequence_rewrite!(choose, b, len r < 4 => a, else => missing);
sequence_rewrite!(output, bounded);
sequence_rewrite!(grade, custom, missing at host);
