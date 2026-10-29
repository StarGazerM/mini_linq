//! Layout and reconstruction of explicitly recorded filtered projections.
use super::*;
use structures::syntax::constraint::Term;

fn literal_reference(ty: &syn::Type, value: &Expr) -> TokenStream {
    // Reconstructed literals outlive every query input lifetime.
    struct Static;
    impl syn::visit_mut::VisitMut for Static {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            *lifetime = syn::parse_quote!('static);
        }
    }
    let mut ty = ty.clone();
    syn::visit_mut::VisitMut::visit_type_mut(&mut Static, &mut ty);
    if matches!(
        value,
        Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(_),
            ..
        })
    ) {
        quote!({
            static __CQ_CONSTANT: ::std::sync::LazyLock<
                <#ty as ::cq_storage::StringLiteral>::Storage
            > = ::std::sync::LazyLock::new(|| <#ty as ::cq_storage::StringLiteral>::storage(#value));
            <#ty as ::cq_storage::StringLiteral>::reference(&*__CQ_CONSTANT)
        })
    } else {
        quote!({ const __CQ_CONSTANT: #ty = #value; &__CQ_CONSTANT })
    }
}

impl Context<'_> {
    /// Logical reader keys omit constant-bound columns in physical storage.
    pub fn stored_key(&self, result: &str, columns: &[String]) -> Vec<String> {
        columns
            .iter()
            .filter(|column| {
                !self.analysis.filtered.get(result).is_some_and(|f| {
                    f.constants
                        .iter()
                        .any(|b| symbol_name(&b.column) == **column)
                })
            })
            .cloned()
            .collect()
    }

    /// A selected constant comes from program text or the captured parameter.
    /// Literal values have static storage, so borrowed answers retain their
    /// ordinary types and never borrow a block-local reconstructed value.
    pub fn constant_value(&self, result: &str, column: &str) -> Option<TokenStream> {
        let binding = self
            .analysis
            .filtered
            .get(result)?
            .constants
            .iter()
            .find(|b| symbol_name(&b.column) == column)?;
        let ty = &self.infos[result].types[column];
        let reference = match &binding.value {
            Term::Literal(value) => literal_reference(ty, value),
            Term::Closed(_) => expressions::term_reference(&binding.value, &self.parameters),
            Term::Variable(_) => unreachable!("checked constant binding"),
        };
        Some(if self.infos[result].computed.contains(column) {
            quote!(::core::clone::Clone::clone(#reference))
        } else {
            reference
        })
    }

    /// Columns removed from a reader's key still constrain its supplied row.
    pub fn constant_key_condition(
        &self,
        left: &str,
        right: &str,
        columns: &[String],
    ) -> Option<TokenStream> {
        let filtered = self.analysis.filtered.get(right)?;
        let tests: Vec<_> = filtered.constants.iter().filter_map(|binding| {
            let column = symbol_name(&binding.column);
            if !columns.contains(&column) { return None; }
            let value = self.access(left, &column);
            let value = if self.infos[left].computed.contains(&column) { quote!(&#value) } else { quote!(#value) };
            let constant = expressions::term_reference(&binding.value, &self.parameters);
            Some(if matches!(&binding.value, Term::Literal(l) if matches!(l.as_ref(), Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. }))) {
                quote!(::cq_storage::relational_string_eq(#value, #constant))
            } else {
                quote!(::cq_storage::relational_eq(#value, #constant))
            })
        }).collect();
        (!tests.is_empty()).then(|| quote!(true #(&& #tests)*))
    }
}
