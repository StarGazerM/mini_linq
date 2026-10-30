//! Opt-in realization of a k-way candidate intersection. Selection supplies
//! two-level built structures `[prefix][v]` for each retained relation;
//! reusable hosts use their candidate and membership dispatch instead.
//! No RA/VO order changes: only the enumerator for this one variable changes.
use super::*;

impl Printer<'_> {
    /// Realize the entire adjacent checked chain before consuming any pipe.
    pub(super) fn intersection(
        &mut self,
        definitions: &[physical::Definition],
        realize: bool,
    ) -> syn::Result<usize> {
        let Some(intersection) = physical::intersection(definitions, self.physical_analysis) else {
            return Ok(0);
        };
        let keys = &intersection.keys;
        let variable = &intersection.column;
        let definitions = &definitions[..keys.len()];
        let first = &definitions[0];
        let last = definitions.last().unwrap();
        let prefixes: Vec<Vec<_>> = keys
            .iter()
            .map(|key| {
                sorted(&key.columns)
                    .into_iter()
                    .filter(|c| c != variable)
                    .collect()
            })
            .collect();
        let levels: Vec<_> = keys
            .iter()
            .zip(&prefixes)
            .map(|(key, prefix)| {
                self.analysis
                    .built
                    .get(&symbol_name(&key.right))
                    .into_iter()
                    .flatten()
                    .find(|(levels, _)| {
                        levels.len() == 2 && levels[0] == *prefix && levels[1] == [variable.clone()]
                    })
                    .map(|(levels, _)| levels.clone())
            })
            .collect();
        if keys.iter().zip(&levels).any(|(key, levels)| {
            !self.analysis.host_read(&symbol_name(&key.right)) && levels.is_none()
        }) {
            return Ok(0);
        }
        let first_name = symbol_name(&first.name);
        let name = symbol_name(&last.name);
        if definitions.iter().any(|definition| {
            self.analysis
                .expands
                .contains(&symbol_name(&definition.name))
        }) || keys.iter().zip(&prefixes).any(|(key, prefix)| {
            self.forcing_witness(&first_name, &symbol_name(&key.right), prefix)
                .is_some()
        }) {
            return error(
                &first.name,
                "native smallest candidate selection does not support retained or segmented candidate cursors",
            );
        }
        for definition in definitions {
            let member = symbol_name(&definition.name);
            self.register(&member);
            if member != name {
                self.profile_time_alias(&member, &name);
            }
        }
        let outer = self.scan(&keys[0].left)?;
        let outer = self.profile_input(&first_name, outer);
        let left = symbol_name(&keys[0].left);
        let pattern = self.bind_columns(&left, self.carried(&left), false);
        let variable_ident = self.access(&first_name, variable);
        let row = self.carrier(&name, |c| self.access(&name, c));
        let mut counts = Vec::with_capacity(keys.len());
        let mut enumerators = Vec::with_capacity(keys.len());
        let mut tests = Vec::with_capacity(keys.len());
        for ((key, prefix), levels) in keys.iter().zip(&prefixes).zip(&levels) {
            let right = symbol_name(&key.right);
            let (count, candidates) = if self.analysis.host_read(&right) {
                let access = self.host_access(&right, prefix);
                (
                    self.host_candidates(&left, &access, true),
                    self.host_candidates(&left, &access, false),
                )
            } else {
                let structure =
                    self.built_ident(&right, levels.as_ref().expect("checked candidate levels"));
                let owned = self.owned_key(&right, prefix);
                let fields = prefix.iter().map(|c| self.key_field(&left, c, owned));
                let key = quote!(&(#(#fields,)*));
                (
                    quote!(::cq_storage::Candidates::count(&#structure,#key)),
                    quote!(::cq_storage::Candidates::keys(&#structure,#key)),
                )
            };
            counts.push(count);
            enumerators.push(candidates);
            tests.push(match self.keyed(&key.right, self.heading(&right))? {
                Access::Host(access) => self.host_lookup(&first_name, &access, true, false),
                Access::Built(s) => {
                    self.chained_contains(&first_name, &s, &right, &self.levels_of(&right, &s))
                }
                Access::Prefix(s, l) => self.chained_contains(&first_name, &s, &right, &l),
            });
        }
        // Preserve the established binary emission byte-for-byte. The k-way
        // extension is additive; existing two-provider plans keep their code.
        let selection = if keys.len() == 2 {
            let a = &counts[0];
            let b = &counts[1];
            quote!(let __cq_choose_a = #a <= #b;)
        } else {
            let first_count = &counts[0];
            let comparisons = counts.iter().enumerate().skip(1).map(|(index, count)| {
                let minimum = if index + 1 == keys.len() {
                    quote!(_)
                } else {
                    quote!(__cq_min)
                };
                quote! {
                    let __cq_count = #count;
                    let (#minimum, __cq_choice) = if __cq_count < __cq_min {
                        (__cq_count, #index)
                    } else {
                        (__cq_min, __cq_choice)
                    };
                }
            });
            quote! {
                let __cq_choice = 0usize;
                let __cq_min = #first_count;
                #(#comparisons)*
            }
        };
        let mut branches = enumerators.into_iter().enumerate().map(|(index, candidates)| {
            let candidates = self.profile_output(&first_name, candidates);
            let candidates = self.profile_input(&name, candidates);
            let (enabled, membership) = if keys.len() == 2 {
                let enabled = if index == 0 {
                    quote!(__cq_choose_a.then_some(()))
                } else {
                    quote!((!__cq_choose_a).then_some(()))
                };
                (enabled, tests[1 - index].clone())
            } else {
                let membership = tests.iter().enumerate().filter_map(|(other, test)| (other != index).then_some(test));
                (quote!((__cq_choice == #index).then_some(())), quote!(true #(&& #membership)*))
            };
            quote! {
                ::core::iter::Iterator::flat_map(::core::iter::IntoIterator::into_iter(#enabled), move |()| {
                    ::core::iter::Iterator::filter_map(#candidates, move |__cq_v| {
                        let #variable_ident = __cq_v.0;
                        if #membership { Some(#row) } else { None }
                    })
                })
            }
        });
        let mut candidates = branches.next().unwrap();
        for branch in branches {
            candidates = quote!(::core::iter::Iterator::chain(#candidates, #branch));
        }
        let mut rows = quote! {
            ::core::iter::Iterator::flat_map(#outer, move |#pattern| {
                #selection
                #candidates
            })
        };
        let repr = match physical::ModeKind::from(&last.mode) {
            _ if !realize => Repr::Pipe(Some(self.profile_stream(&name, rows))),
            physical::ModeKind::Pipe => {
                if last.canon.is_some() {
                    rows = self.distinct(&last.name, rows);
                }
                Repr::Pipe(Some(self.profile_stream(&name, rows)))
            }
            physical::ModeKind::Let | physical::ModeKind::Exists => {
                if last.canon.is_some() {
                    rows = self.distinct(&last.name, rows);
                }
                let rows = self.profile_stream(&name, rows);
                self.materialize(&name, rows)
            }
        };
        for definition in &definitions[..definitions.len() - 1] {
            self.reprs
                .insert(symbol_name(&definition.name), Repr::Pipe(None));
        }
        self.reprs.insert(name, repr);
        Ok(keys.len())
    }
}
