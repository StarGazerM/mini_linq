//! Sequential terminal for the checked Round language.
//! Hole invocation queues retain Round's checked lower packets. Their CQ entry
//! is fact-free; the core lower continuation supplies the exact factful envelope.
use cq_lang_datalog_round::Program;
pub fn compile(source: &Program) -> syn::Result<syn::Expr> {
    cq_datalog_print::compile(source)
}
