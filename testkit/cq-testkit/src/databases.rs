//! Deterministic binary-domain tables for small conformance inputs.
//!
//! Seed 0 is empty and seed 1 is full. Other seeds sample tuples using explicit
//! wrapping arithmetic. Enumeration grows exponentially with input arity.
use crate::{Database, cq};

/// Generate schema-shaped tables of zeroes and ones with stable seeded sampling.
///
/// Input arities must be smaller than `usize::BITS`.
pub fn binary_database(program: &cq::Program, seed: u64) -> Database {
    binary_relations(
        program
            .inputs()
            .map(|input| (cq_syntax::symbol_name(&input.name), input.columns.len())),
        seed,
    )
}

/// Generate the same finite-domain data directly from a declared relation schema.
pub fn binary_relations(
    relations: impl IntoIterator<Item = (String, usize)>,
    seed: u64,
) -> Database {
    let mut random = seed;
    relations
        .into_iter()
        .map(|(name, width)| {
            assert!(width < usize::BITS as usize);
            let rows = (0_usize..(1 << width))
                .filter_map(|bits| {
                    random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let include = match seed {
                        0 => false,
                        1 => true,
                        _ => (random >> 61) <= seed % 8,
                    };
                    include.then(|| {
                        (0..width)
                            .map(|column| ((bits >> column) & 1) as i32)
                            .collect()
                    })
                })
                .collect();
            (name, rows)
        })
        .collect()
}
