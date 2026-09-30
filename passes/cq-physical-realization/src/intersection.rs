//! Opt-in realization of one binary candidate intersection. Selection must
//! provide two-level built structures `[prefix][v]` for both relations.
//! No RA/VO order changes: only the enumerator for this one variable changes.
use super::*;

impl Printer<'_> {
    /// Try an adjacent probe/check pair before consuming either pipe.
    pub(super) fn intersection(
        &mut self,
        first: &physical::Definition,
        second: &physical::Definition,
    ) -> syn::Result<bool> {
        let Some((a, b, variable)) =
            physical::binary_intersection(first, second, self.physical_analysis)
        else {
            return Ok(false);
        };
        let ak = sorted(&a.columns);
        let bk: Vec<_> = sorted(&b.columns)
            .into_iter()
            .filter(|c| c != &variable)
            .collect();
        let eligible = |result: &Ident, prefix: &[String]| {
            let name = symbol_name(result);
            self.analysis.host_read(&name)
                || self
                    .built_levels(&name, prefix)
                    .is_some_and(|(levels, depth)| {
                        depth == 1 && levels.len() == 2 && levels[1] == [variable.clone()]
                    })
        };
        if !eligible(&a.right, &ak) || !eligible(&b.right, &bk) {
            return Ok(false);
        }
        let first_name = symbol_name(&first.name);
        let name = symbol_name(&second.name);
        self.register(&first_name);
        self.register(&name);
        let outer = self.scan(&a.left)?;
        let left = symbol_name(&a.left);
        let pattern = self.pattern(&left);
        let variable_ident = self.ident(&first_name, &variable);
        let row = self.pattern(&name);
        let candidate = |result: &Ident, prefix: &[String]| {
            let name = symbol_name(result);
            if self.analysis.host_read(&name) {
                let access = self.host_access(&name, prefix);
                (
                    self.host_candidates(&left, &access, true),
                    self.host_candidates(&left, &access, false),
                )
            } else {
                let (levels, _) = self.built_levels(&name, prefix).unwrap();
                let structure = self.built_ident(&name, levels);
                let owned = self.owned_key(&name, prefix);
                let fields = prefix.iter().map(|c| self.key_field(&left, c, owned));
                let key = quote!(&(#(#fields,)*));
                (
                    quote!(::cq_storage::Candidates::count(&#structure,#key)),
                    quote!(::cq_storage::Candidates::keys(&#structure,#key)),
                )
            }
        };
        let (count_a, keys_a) = candidate(&a.right, &ak);
        let (count_b, keys_b) = candidate(&b.right, &bk);
        let test = |result: &Ident| -> syn::Result<TokenStream> {
            let right = symbol_name(result);
            Ok(match self.keyed(result, self.heading(&right))? {
                Access::Host(access) => self.host_lookup(&first_name, &access, true),
                Access::Built(s) => {
                    self.chained_contains(&first_name, &s, &right, &self.levels_of(&right, &s))
                }
                Access::Prefix(s, l) => self.chained_contains(&first_name, &s, &right, &l),
            })
        };
        let test_a = test(&a.right)?;
        let test_b = test(&b.right)?;
        let branch = |choice: TokenStream, keys: TokenStream, test: TokenStream| {
            quote! {
                ::core::iter::Iterator::flat_map(::core::iter::IntoIterator::into_iter(#choice), move |()| {
                    ::core::iter::Iterator::filter_map(#keys, move |__cq_v| {
                        let #variable_ident = __cq_v.0;
                        if #test { Some(#row) } else { None }
                    })
                })
            }
        };
        let from_a = branch(quote!(__cq_choose_a.then_some(())), keys_a, test_b);
        let from_b = branch(quote!((!__cq_choose_a).then_some(())), keys_b, test_a);
        let mut rows = quote! {
            ::core::iter::Iterator::flat_map(#outer, move |#pattern| {
                let __cq_choose_a = #count_a <= #count_b;
                ::core::iter::Iterator::chain(#from_a,#from_b)
            })
        };
        let repr = match physical::ModeKind::from(&second.mode) {
            physical::ModeKind::Pipe => {
                if second.canon.is_some() {
                    rows = self.distinct(&second.name, rows);
                }
                Repr::Pipe(Some(rows))
            }
            physical::ModeKind::Let => {
                if second.canon.is_some() {
                    rows = self.distinct(&second.name, rows);
                }
                self.materialize(&name, second.canon.is_some(), rows)
            }
        };
        self.reprs.insert(first_name, Repr::Pipe(None));
        self.reprs.insert(name, repr);
        Ok(true)
    }
}
