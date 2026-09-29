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
        let candidate = |result: &Ident, prefix: &[String]| {
            let name = symbol_name(result);
            let (levels, depth) = self.built_levels(&name, prefix)?;
            (depth == 1 && levels.len() == 2 && levels[1] == [variable.clone()])
                .then(|| self.built_ident(&name, levels))
        };
        let (Some(sa), Some(sb)) = (candidate(&a.right, &ak), candidate(&b.right, &bk)) else {
            return Ok(false);
        };
        let first_name = symbol_name(&first.name);
        let name = symbol_name(&second.name);
        self.register(&first_name);
        self.register(&name);
        let outer = self.scan(&a.left)?;
        let left = symbol_name(&a.left);
        let pattern = self.pattern(&left);
        let key = |columns: &[String]| {
            let fields = columns.iter().map(|c| self.ident(&left, c));
            quote!(&(#(#fields,)*))
        };
        let ka = key(&ak);
        let kb = key(&bk);
        let variable_ident = self.ident(&first_name, &variable);
        let row = self.pattern(&name);
        let test_a = self.chained_contains(
            &first_name,
            &sa,
            &self.levels_of(&symbol_name(&a.right), &sa),
        );
        let test_b = self.chained_contains(
            &first_name,
            &sb,
            &self.levels_of(&symbol_name(&b.right), &sb),
        );
        // Option's empty iterator disables the unchosen branch without a
        // box or per-prefix collection. Both branches yield the same row.
        let branch = |choice: TokenStream, key: &TokenStream, test: &TokenStream| {
            quote! {
                ::core::iter::Iterator::flat_map(
                    ::core::iter::IntoIterator::into_iter(#choice),
                    move |__cq_candidates| {
                        ::core::iter::Iterator::filter_map(
                            ::cq_storage::Candidates::keys(&__cq_candidates, #key),
                            move |__cq_v| {
                                let #variable_ident = __cq_v.0;
                                if #test { ::core::option::Option::Some(#row) }
                                else { ::core::option::Option::None }
                            }
                        )
                    }
                )
            }
        };
        let from_a = branch(quote!(__cq_from_a), &ka, &test_b);
        let from_b = branch(quote!(__cq_from_b), &kb, &test_a);
        let mut rows = quote! {{
            ::cq_storage::require::candidates(#sa);
            ::cq_storage::require::candidates(#sb);
            ::core::iter::Iterator::flat_map(#outer, move |#pattern| {
                let __cq_choose_a = ::cq_storage::Candidates::count(&#sa, #ka)
                    <= ::cq_storage::Candidates::count(&#sb, #kb);
                let __cq_from_a = if __cq_choose_a { Some(#sa) } else { None };
                let __cq_from_b = if __cq_choose_a { None } else { Some(#sb) };
                ::core::iter::Iterator::chain(#from_a, #from_b)
            })
        }};
        let repr = match physical::ModeKind::from(&second.mode) {
            physical::ModeKind::Pipe => {
                if second.canon.is_some() {
                    rows = self.distinct(&second.name, rows);
                }
                Repr::Pipe(Some(rows))
            }
            physical::ModeKind::Let => self.materialize(&name, second.canon.is_some(), rows),
            physical::ModeKind::Alias => unreachable!("a check is not an alias"),
        };
        self.reprs.insert(first_name, Repr::Pipe(None));
        self.reprs.insert(name, repr);
        Ok(true)
    }
}
