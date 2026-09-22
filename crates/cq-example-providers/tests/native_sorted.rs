//! Restored native row-set contract; the removed lowering API remains required.
use cq_native_lowering::rows::sorted::{Options, compile};

#[path = "support/set_ops.rs"]
mod set_ops;

mod tests {
    super::set_ops::check_set_provider!();
}
