//! GPU-resident packed Datalog tuples and immutable, disjoint accepted runs.
//! All device work uses the owning [`Pool`]'s legacy default stream.
//! [`DeviceLayout`] retains typed, immutable run inputs and uploads only their
//! erased descriptors. [`CandidateArena`] leases packed output lanes to Sequence
//! terminals; [`CandidateBatch`] retains those lanes and input owners without a
//! candidate-row download. Signed packing preserves numeric lexicographic order.
pub mod arena;
pub mod layout;
pub mod pack;
pub mod runs;
pub use gpu_device_columns::{Buffer, Pool};
pub use pack::{Column, Packing, Word};
pub use runs::{AdmissionTimings, DeviceRuns, Run, RunView};
pub use arena::{concatenate_batches, CandidateArena, CandidateBatch, CandidateLease};
pub use layout::{device, DeviceLayout, PackedField, PackedRunView, RunsView, Scalar, ScalarTuple};
pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
