//! Whole-program GPU Datalog through the ordinary checked Datalog pipeline.
//!
//! Configure `storage_rewrites = [::cq_gpu_datalog::device_storage]`, then
//! `cq = pipeline { structures = [::cq_gpu_datalog::bind_inputs,
//! ::cq_gpu_sequence::bind_runs]; decompose = [::cq_gpu_sequence::plan];
//! grade = ::cq_gpu_sequence::machine_grade;
//! print = ::cq_gpu_sequence::translate_arena; };`, register
//! `terminals = [::cq_gpu_sequence::translate_arena => batch]`, and select
//! `print = ::cq_gpu_datalog::print_rounds`. The printer owns arena arguments.
//!
//! Inputs upload once; rule candidates, canonical admission, permutation and
//! publication remain on one device stream. Only count controls synchronize
//! during evaluation; rows download only at final output conversion. Supported
//! columns are i8/i16/i32/i64/u8/u16/u32 with at most 128 packed bits per row.
//! Observer callbacks, recorded phase timing, host execution certificates and
//! unit-valued existential negation keys are rejected explicitly.
pub use cq_gpu_datalog_macros::{bind_inputs, device_storage, print_rounds};
pub use cq_gpu_datalog_core::{print, storage};
