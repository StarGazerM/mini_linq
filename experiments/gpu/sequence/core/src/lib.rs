//! Checked Sequence-to-CUDA translation over resident column providers.

use cq_lang_sequence::{LevelNode, Machine};

mod emit;
mod reduce;
mod rows;

pub use emit::translate;

/// Host publication over separated, dynamically scheduled device blocks.
/// Each block is collaborative; lanes are sequential leaves (no warp level).
pub fn machine() -> Machine {
    Machine {
        name: syn::parse_quote!(gpu),
        root: LevelNode {
            name: syn::parse_quote!(host),
            collaborative: false,
            dynamic: false,
            children: vec![LevelNode {
                name: syn::parse_quote!(device),
                collaborative: false,
                dynamic: true,
                children: vec![LevelNode {
                    name: syn::parse_quote!(block),
                    collaborative: true,
                    dynamic: false,
                    children: vec![LevelNode {
                        name: syn::parse_quote!(lane),
                        collaborative: false,
                        dynamic: false,
                        children: vec![],
                    }],
                }],
            }],
        },
    }
}
