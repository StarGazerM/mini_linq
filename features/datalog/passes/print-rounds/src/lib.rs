//! Sequential terminal for the checked Round language.
use cq_lang_datalog_round::Program;
pub fn compile(source: &Program) -> syn::Result<syn::Expr> {
    cq_datalog_print::compile(source)
}
