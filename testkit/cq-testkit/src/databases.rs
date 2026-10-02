//! Deterministic binary-domain tables for small conformance inputs.
//!
//! Seed 0 is empty and seed 1 is full. Other seeds sample tuples using explicit
//! wrapping arithmetic. Enumeration grows exponentially with input arity.
use crate::{Database, cq};

/// Generate schema-shaped tables of zeroes and ones with stable seeded sampling.
///
/// Input arities must be smaller than `usize::BITS`.
pub fn binary_database(program: &cq::Program, seed: u64) -> Database {
    assert!(
        program
            .inputs()
            .all(|input| input.columns.len() < usize::BITS as usize)
    );
    let mut random = seed;
    program
        .inputs()
        .map(|input| {
            let rows = (0_usize..(1 << input.columns.len()))
                .filter_map(|bits| {
                    random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let include = match seed {
                        0 => false,
                        1 => true,
                        _ => (random >> 61) <= seed % 8,
                    };
                    include.then(|| {
                        (0..input.columns.len())
                            .map(|column| ((bits >> column) & 1) as i32)
                            .collect()
                    })
                })
                .collect();
            (cq_syntax::symbol_name(&input.name), rows)
        })
        .collect()
}
