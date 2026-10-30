//! Joining triangle kernel: written fine planner (160 configurations).
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/full_matrix.rs"]
mod full_matrix;
#[macro_use]
#[path = "../../../src/queries/classic/triangle.rs"]
mod triangle;

full_matrix!([written, fine]; merge_hosts: all);
