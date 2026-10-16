//! Checked Datalog-to-Datalog scheduling passes.
//!
//! `stratify::compile` records a dependency-ordered exact SCC partition.
//! `seminaive::compile` records fresh declared delta relations and one variant
//! per changing positive position (or one seed for a non-changing rule).
//! Each pass has an independent source/target check: target language contracts
//! certify the decisions, and correspondence preserves the source and existing
//! decisions. Neither check reruns its compiler or requires canonical names or
//! an arbitrary order between independent components.
//! Ordered output lists and constant selections in negative atoms are source
//! decisions: both passes preserve them verbatim through printing and replay.

pub mod seminaive;
pub mod stratify;

use quote::ToTokens;

fn ensure(ok: bool, at: impl ToTokens, message: &str) -> syn::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(at, message))
    }
}
