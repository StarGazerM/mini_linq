//! Opt-in realization of one binary candidate intersection. Selection must
//! provide two-level built structures `[prefix][v]` for each retained relation;
//! reusable hosts use their candidate and membership dispatch instead.
//! No RA/VO order changes: only the enumerator for this one variable changes.
use super::*;

impl Printer<'_> {
    /// Try an adjacent probe/check pair before consuming either pipe.
    pub(super) fn intersection(
        &mut self,
        first: &physical::Definition,
        second: &physical::Definition,
        realize: bool,
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
        let built_candidate = |result: &Ident, prefix: &[String]| {
            let name = symbol_name(result);
            self.analysis
                .built
                .get(&name)
                .into_iter()
                .flatten()
                .find(|(levels, _)| {
                    levels.len() == 2 && levels[0] == prefix && levels[1] == [variable.clone()]
                })
                .map(|(levels, _)| levels.clone())
        };
        let a_levels = built_candidate(&a.right, &ak);
        let b_levels = built_candidate(&b.right, &bk);
        if !(self.analysis.host_read(&symbol_name(&a.right)) || a_levels.is_some())
            || !(self.analysis.host_read(&symbol_name(&b.right)) || b_levels.is_some())
        {
            return Ok(false);
        }
        let first_name = symbol_name(&first.name);
        let name = symbol_name(&second.name);
        if self.analysis.expands.contains(&first_name)
            || self.analysis.expands.contains(&name)
            || self.forcing_witness(&first_name, &symbol_name(&a.right), &ak).is_some()
            || self.forcing_witness(&first_name, &symbol_name(&b.right), &bk).is_some()
        {
            return error(
                &first.name,
                "native smallest candidate selection does not support retained or segmented candidate cursors",
            );
        }
        self.register(&first_name);
        self.register(&name);
        self.profile_time_alias(&first_name, &name);
        let outer = self.scan(&a.left)?;
        let outer = self.profile_input(&first_name, outer);
        let left = symbol_name(&a.left);
        let pattern = self.bind_columns(&left, self.carried(&left), false);
        let variable_ident = self.access(&first_name, &variable);
        let row = self.carrier(&name, |c| self.access(&name, c));
        let candidate = |result: &Ident, prefix: &[String], levels: Option<&Vec<Vec<String>>>| {
            let name = symbol_name(result);
            if self.analysis.host_read(&name) {
                let access = self.host_access(&name, prefix);
                (
                    self.host_candidates(&left, &access, true),
                    self.host_candidates(&left, &access, false),
                )
            } else {
                let structure = self.built_ident(&name, levels.expect("checked candidate levels"));
                let owned = self.owned_key(&name, prefix);
                let fields = prefix.iter().map(|c| self.key_field(&left, c, owned));
                let key = quote!(&(#(#fields,)*));
                (
                    quote!(::cq_storage::Candidates::count(&#structure,#key)),
                    quote!(::cq_storage::Candidates::keys(&#structure,#key)),
                )
            }
        };
        let (count_a, keys_a) = candidate(&a.right, &ak, a_levels.as_ref());
        let (count_b, keys_b) = candidate(&b.right, &bk, b_levels.as_ref());
        let test = |result: &Ident| -> syn::Result<TokenStream> {
            let right = symbol_name(result);
            Ok(match self.keyed(result, self.heading(&right))? {
                Access::Host(access) => self.host_lookup(&first_name, &access, true, false),
                Access::Built(s) => {
                    self.chained_contains(&first_name, &s, &right, &self.levels_of(&right, &s))
                }
                Access::Prefix(s, l) => self.chained_contains(&first_name, &s, &right, &l),
            })
        };
        let test_a = test(&a.right)?;
        let test_b = test(&b.right)?;
        let branch = |choice: TokenStream, keys: TokenStream, test: TokenStream| {
            let keys = self.profile_output(&first_name, keys);
            let keys = self.profile_input(&name, keys);
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
            _ if !realize => Repr::Pipe(Some(self.profile_stream(&name, rows))),
            physical::ModeKind::Pipe => {
                if second.canon.is_some() {
                    rows = self.distinct(&second.name, rows);
                }
                Repr::Pipe(Some(self.profile_stream(&name, rows)))
            }
            physical::ModeKind::Let | physical::ModeKind::Exists => {
                if second.canon.is_some() {
                    rows = self.distinct(&second.name, rows);
                }
                let rows = self.profile_stream(&name, rows);
                self.materialize(&name, rows)
            }
        };
        self.reprs.insert(first_name, Repr::Pipe(None));
        self.reprs.insert(name, repr);
        Ok(true)
    }
}
