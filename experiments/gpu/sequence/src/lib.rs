//! Checked Sequence-to-CUDA translation and named pipeline adapters.
//!
//! `bind_runs` binds explicit native `DeviceLayout<K, V>` inputs before Sequence
//! lowering. Finish a graded raw-candidate dispatch with
//! `translate_arena(&mut arena)` to return a `gpu_datalog_runtime::CandidateBatch`.
//! Its live rows use declared head-column widths, most-significant column first;
//! the batch retains its arena lease and run input owners. Recorded output
//! counting/reservation and metadata-driven capacity retries remain in effect.
//! Terminal canon, group, fold, and host-only output are rejected by this terminal;
//! intermediate publications retain their existing semantics.

pub use cq_gpu_sequence_core::{catalog, machine, translate, translate_arena};
pub use cq_gpu_sequence_core::runs::record_representation;
pub use cq_gpu_sequence_macros::{bind_runs, capture_constants, decompose, machine_grade, select_catalog, stage, translate, translate_arena};
