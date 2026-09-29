//! Deterministic binary-domain databases shared by independent test consumers.
use cq_lang_cq as cq;

pub fn binary_database(
    program: &cq::Program,
    seed: u64,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<Vec<i32>>> {
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
