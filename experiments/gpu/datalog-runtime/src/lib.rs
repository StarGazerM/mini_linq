//! GPU-resident packed Datalog tuples and immutable, disjoint accepted runs.
//! All device work uses the owning [`Pool`]'s legacy default stream.
pub mod pack;
pub mod runs;
pub use gpu_device_columns::{Buffer, Pool};
pub use pack::{Column, Packing, Word};
pub use runs::{AdmissionTimings, DeviceRuns, Run, RunView};
pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
