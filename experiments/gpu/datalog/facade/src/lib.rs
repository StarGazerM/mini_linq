//! Whole-program GPU Datalog through the ordinary checked Datalog pipeline.
//!
//! Configure `storage_rewrites = [::cq_gpu_datalog::device_storage]`, then
//! `cq = pipeline { structures = [::cq_gpu_datalog::bind_inputs,
//! ::cq_gpu_sequence::bind_runs]; decompose = [::cq_gpu_sequence::decompose];
//! stage = ::cq_gpu_sequence::stage;
//! grade = ::cq_gpu_sequence::machine_grade;
//! print = ::cq_gpu_sequence::translate_arena; };`, register
//! `terminals = [::cq_gpu_sequence::translate_arena => batch]`, and select
//! `round = [::cq_gpu_datalog::bind_round(binding = 4)]` and
//! `print = ::cq_gpu_datalog::print_rounds`. The zero-based binding ordinal
//! identifies the first Structures rewrite in this queue (add one for each
//! preceding surface/physical rewrite). Checked construction records both the
//! binding packet and arena argument; printing never fills invocation arguments.
//! `bind_round(binding = 4, head_cursor)` certifies that the registered batch
//! terminal accepts the grouped output ABI. Only a replay-safe head whose
//! launches share one publication epoch receives `head; &mut *__cq_gpu_arena`
//! and returns `Option<CandidateBatch>` (`None` means appended). Other holes
//! keep the original batch ABI, as do all holes in expansion-counter builds.
//! For `manual(...)` or `observe(...)` printer hooks, list the hooked
//! versions in e.g. `bind_round(binding = 4, head_cursor, legacy("Load@3"))`.
//! This keeps every producer of that head in that phase on the original ABI;
//! unrelated heads and phases can still group. The printer rejects hooks on
//! marked terminals rather than replaying a callback or a hand-written hole.
//! `device_storage(symbols(T => Codec), ratio(4))` records optional codecs and
//! geometric run-maintenance advice; omitted ratio advice defaults to four.
//!
//! Inputs upload once; rule candidates, canonical admission, permutation and
//! publication remain on one device stream. Only count controls synchronize
//! during evaluation; rows download only at final output conversion. Supported
//! columns are i8/i16/i32/i64/u8/u16/u32 with at most 128 packed bits per row.
//! Round observer callbacks, recorded phase timing, host execution certificates and
//! unit-valued existential negation keys are rejected explicitly.
pub use cq_gpu_datalog_core::{bindings, print, storage};
pub use cq_gpu_datalog_macros::{bind_inputs, bind_round, device_storage, print_rounds};
