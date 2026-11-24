//! Join lab: hand-written join kernels measured against machine caps on
//! captured DOOP rounds and synthetic skew. See `DESIGN.md`.
pub mod assign;
pub mod caps;
pub mod dup;
pub mod filter;
pub mod hashdedup;
pub mod maint;
pub mod probe;
pub mod replay;
