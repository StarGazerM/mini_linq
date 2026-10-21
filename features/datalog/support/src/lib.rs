//! IR-independent utilities for checked Datalog refinements.
//! Identifier allocation visits opaque Rust tokens as well as relation syntax.
//! Column helpers preserve written order and do not choose a storage policy.
use cq_syntax::{Atom, rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident};

/// Report a failed obligation at the supplied syntax.
pub fn ensure(ok: bool, at: impl ToTokens, message: &str) -> syn::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(at, message))
    }
}

/// Collect normalized identifiers recursively, including opaque Rust groups.
pub fn collect_names(tokens: TokenStream, names: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(id) => {
                names.insert(symbol_name(&id));
            }
            TokenTree::Group(group) => collect_names(group.stream(), names),
            _ => {}
        }
    }
}

/// Deterministic, role-local fresh names reserved against all source tokens.
/// Each role resumes after its last candidate; occupied names are never retried.
pub struct Fresh {
    used: BTreeSet<String>,
    next: BTreeMap<String, usize>,
}
impl Fresh {
    pub fn new(source: impl ToTokens) -> Self {
        let mut used = BTreeSet::new();
        collect_names(source.into_token_stream(), &mut used);
        Self {
            used,
            next: BTreeMap::new(),
        }
    }

    pub fn reserve(&mut self, id: &Ident) -> bool {
        self.used.insert(symbol_name(id))
    }

    pub fn id(&mut self, role: &str) -> Ident {
        if !self.next.contains_key(role) {
            self.next.insert(role.to_owned(), 0);
        }
        let next = self.next.get_mut(role).unwrap();
        loop {
            let name = format!("__cq_datalog_{role}_{next}");
            *next += 1;
            if self.used.insert(name.clone()) {
                return Ident::new(&name, Span::mixed_site());
            }
        }
    }
}

/// Variable columns of an atom, in written order.
pub fn columns(atom: &Atom) -> Vec<usize> {
    atom.args
        .iter()
        .enumerate()
        .filter_map(|(i, arg)| rust::variable(arg).map(|_| i))
        .collect()
}

/// Constant/expression filters, excluding variable and wildcard columns.
pub fn filters(atom: &Atom) -> Vec<(usize, Expr)> {
    atom.args
        .iter()
        .enumerate()
        .filter(|(_, arg)| {
            rust::variable(arg).is_none() && !matches!(rust::unparen(arg), Expr::Infer(_))
        })
        .map(|(i, arg)| (i, arg.clone()))
        .collect()
}

/// Stable key-first permutation and key length for the currently bound names.
/// Constants are keys; unbound variables and wildcards are payload columns.
pub fn key_columns(atom: &Atom, bound: &BTreeSet<String>) -> (Vec<usize>, usize) {
    let mut columns = vec![0; atom.args.len()];
    let mut key = 0;
    let mut payload = columns.len();
    for (column, arg) in atom.args.iter().enumerate() {
        let is_key = match rust::variable(arg) {
            Some(id) => bound.contains(&symbol_name(id)),
            None => !matches!(rust::unparen(arg), Expr::Infer(_)),
        };
        if is_key {
            columns[key] = column;
            key += 1;
        } else {
            payload -= 1;
            columns[payload] = column;
        }
    }
    // Payload was filled from the back so one buffer suffices.
    columns[key..].reverse();
    (columns, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn freshness_visits_nested_tokens_and_normalizes_raw_names() {
        let mut fresh = Fresh::new(quote! {
            outer({ __cq_datalog_row_0; r#__cq_datalog_row_2; r#reserved; })
        });
        assert!(!fresh.reserve(&syn::parse_quote!(reserved)));
        assert!(fresh.reserve(&syn::parse_quote!(__cq_datalog_row_1)));
        assert!(!fresh.reserve(&syn::parse_quote!(__cq_datalog_row_1)));
        assert_eq!(fresh.id("row"), "__cq_datalog_row_3");
        assert_eq!(fresh.id("other"), "__cq_datalog_other_0");
        assert_eq!(fresh.id("row"), "__cq_datalog_row_4");
    }

    #[test]
    fn role_counters_skip_late_reservations_without_restarting() {
        let mut fresh = Fresh::new(quote! {
            __cq_datalog_row_2;
            r#__cq_datalog_other_0;
        });
        assert_eq!(fresh.id("row"), "__cq_datalog_row_0");
        assert_eq!(fresh.id("other"), "__cq_datalog_other_1");
        assert!(fresh.reserve(&syn::parse_quote!(__cq_datalog_row_1)));
        assert_eq!(fresh.id("row"), "__cq_datalog_row_3");
        assert!(fresh.reserve(&syn::parse_quote!(__cq_datalog_row_5)));
        assert_eq!(fresh.id("row"), "__cq_datalog_row_4");
        assert_eq!(fresh.id("row"), "__cq_datalog_row_6");
        assert_eq!(fresh.id("other"), "__cq_datalog_other_2");
    }

    #[test]
    fn bound_and_constant_columns_keep_written_order() {
        let atom: Atom = syn::parse_quote!(R(x, 7, y, _, r#x, (8 + 1), (_)));
        assert_eq!(columns(&atom), vec![0, 2, 4]);
        assert_eq!(
            filters(&atom),
            vec![(1, syn::parse_quote!(7)), (5, syn::parse_quote!((8 + 1)))]
        );
        let bound = BTreeSet::from([String::from("x")]);
        assert_eq!(key_columns(&atom, &bound), (vec![0, 1, 4, 5, 2, 3, 6], 4));
        assert_eq!(
            key_columns(&atom, &BTreeSet::new()),
            (vec![1, 5, 0, 2, 3, 4, 6], 2)
        );
    }

    #[test]
    fn empty_and_entirely_bound_atoms() {
        let empty: Atom = syn::parse_quote!(R());
        assert_eq!(key_columns(&empty, &BTreeSet::new()), (vec![], 0));
        let atom: Atom = syn::parse_quote!(R(x, 7));
        assert_eq!(
            key_columns(&atom, &BTreeSet::from([String::from("x")])),
            (vec![0, 1], 2)
        );
        assert!(ensure(true, &atom, "valid").is_ok());
        assert_eq!(
            ensure(false, &atom, "invalid").unwrap_err().to_string(),
            "invalid"
        );
    }
}
