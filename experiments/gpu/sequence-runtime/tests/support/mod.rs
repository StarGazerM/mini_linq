//! The auxiliary geometry record generated programs print today.
use cq_gpu_sequence_runtime::{Auxiliary, Geometry};

const ELEMENT: Geometry = Geometry {
    block_threads: 256,
    shared_bytes: 0,
};

pub const AUX: Auxiliary = Auxiliary {
    selection: ELEMENT,
    offsets: ELEMENT,
    range: ELEMENT,
    canon: ELEMENT,
    group: ELEMENT,
    group_reduce: Geometry {
        block_threads: 256,
        shared_bytes: 4096,
    },
    bounds: ELEMENT,
};
