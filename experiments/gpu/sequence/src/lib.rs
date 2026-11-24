//! Checked Sequence-to-CUDA translation and named pipeline adapters.
//!
//! `bind_runs` binds declared full-key `RadixLayout<K, (), W, u64, ()>` set inputs before Sequence
//! lowering. Finish a graded raw-candidate dispatch with
//! `translate_arena(&mut arena)` to return a `gpu_datalog_runtime::CandidateBatch`.
//! Its live rows use declared head-column widths, most-significant column first;
//! the batch retains its arena lease and run input owners. Recorded output
//! counting/reservation and metadata-driven capacity retries remain in effect.
//! Terminal canon, group, fold, and host-only output are rejected by this terminal;
//! intermediate publications retain their existing semantics.
//! `translate_arena(head; &mut arena)` requests GPU-HeadCursor. It returns
//! `None` after a grouped append (or a closed gate), otherwise `Some(batch)`
//! from the ordinary arena terminal. The binding owns grouped completion.
//! `translate_arena(frontier; &mut arena)` requires GPU-FrontierResume for
//! the final dispatch. Use `frontier; head;` to request both refinements.
//! An ineligible frontier is a translation error.

pub use cq_gpu_sequence_core::runs::record_representation;
pub use cq_gpu_sequence_core::{
    bind_structures, catalog, machine, providers, translate, translate_arena, translate_arena_head, translate_with_origins,
    CaptureHandle, DeviceCaptureBindings,
};
pub use cq_gpu_sequence_macros::{
    bind_runs, capture_constants, decompose, machine_grade, select_catalog, stage, translate,
    translate_arena,
};
