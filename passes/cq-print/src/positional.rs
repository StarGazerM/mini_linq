//! Emission of checked positional decisions. No host or provider is inferred here.
use super::*;
use structures::{KeyOwnership, MembershipSummary, ReadPredicate, Summary};

impl Context<'_> {
    pub fn join_filter_ident(&self, scan: &str) -> Ident {
        format_ident!("__cq_{}_join_filter", scan, span = proc_macro2::Span::mixed_site())
    }
    /// Formal rule IF-JoinFilter: Filter scan positions conservatively and retain the covering exact probe unchanged.
    pub fn join_filter_ids(&self, name: &str) -> Option<TokenStream> {
        let origin = self.analysis.lineage.get(name)?;
        let filter = self.analysis.join_filters.get(&origin.occurrence)?;
        let id = self.join_filter_ident(&origin.occurrence);
        let host = format_ident!("__cq_positional_{}", origin.base, span = proc_macro2::Span::mixed_site());
        let marker = &filter.marker;
        Some(quote!(::cq_storage::BlockRead::read(#host, <#marker>::default(), #id)))
    }
    fn combined_join_filter_ids(
        &self,
        name: &str,
        input: &str,
        predicate: &Ident,
    ) -> Option<TokenStream> {
        let origin = self.analysis.lineage.get(name)?;
        let filter = self.analysis.join_filters.get(&origin.occurrence)?;
        // Replace only an unevaluated rename/project position stream. A prior
        // predicate remains authoritative and must not be scanned a second time.
        let mut current = input.to_owned();
        loop {
            match &self.physical[&current].operator {
                Operator::Rename(_) => break,
                Operator::Project(p) => current = symbol_name(&p.input),
                _ => return None,
            }
        }
        let id = self.join_filter_ident(&origin.occurrence);
        let host = format_ident!("__cq_positional_{}", origin.base, span = proc_macro2::Span::mixed_site());
        let marker = &self.analysis.reads[name].marker;
        let key_marker = &filter.marker;
        Some(quote!(::cq_storage::read_with_filter(#host, <#marker>::default(), #predicate, <#key_marker>::default(), #id)))
    }
    pub fn keyset_ident(&self, name: &str, key: &[String]) -> Ident {
        format_ident!(
            "__cq_{}_keys_{}",
            name,
            key.join("_"),
            span = proc_macro2::Span::mixed_site()
        )
    }
    pub fn keyset_contains(&self, left: &str, right: &str, key: &[String]) -> Option<TokenStream> {
        let set = self
            .analysis
            .keysets
            .get(&(right.to_owned(), key.to_vec()))?;
        let id = self.keyset_ident(right, key);
        let values = key
            .iter()
            .map(|column| {
                let value = self.ident(left, column);
                if self.infos[left].computed.contains(column) {
                    quote!(&#value)
                } else {
                    quote!(#value)
                }
            })
            .collect::<Vec<_>>();
        let key = if matches!(set.ownership, KeyOwnership::Owned(_)) {
            values[0].clone()
        } else {
            quote!(&(#(#values,)*))
        };
        Some(quote!(::cq_storage::ExactMembership::contains(#id, #key)))
    }
    /// Map a host row to this reader's checked logical layout without cloning.
    pub fn positional_pattern(&self, name: &str) -> TokenStream {
        let origin = &self.analysis.lineage[name];
        let binders = self.analysis.host_layouts[&origin.base].iter().map(|c| {
            origin
                .columns
                .iter()
                .find(|(_, source)| *source == c)
                .filter(|(logical, _)| self.heading(name).contains(logical))
                .map_or_else(
                    || quote!(_),
                    |(logical, _)| self.ident(name, logical).to_token_stream(),
                )
        });
        quote!((#(#binders,)*))
    }
    pub fn selection_rows(&self, name: &str, selection: &Ident) -> TokenStream {
        let pattern = self.positional_pattern(name);
        let row = self.pattern(name);
        quote!(::core::iter::Iterator::map(#selection.rows(), move |#pattern| #row))
    }
    pub fn positional_rows(&self, name: &str, ids: TokenStream) -> TokenStream {
        let host = format_ident!(
            "__cq_positional_{}",
            self.analysis.lineage[name].base,
            span = proc_macro2::Span::mixed_site()
        );
        let pattern = self.positional_pattern(name);
        let row = self.pattern(name);
        let rows = quote!(::core::iter::Iterator::map(#ids, move |__cq_id| {
            let #pattern = ::cq_storage::PositionalRows::row_at(#host, __cq_id).expect("host-issued live ID");
            #row
        }));
        rows
    }

    pub fn read_predicate(&self, name: &str) -> TokenStream {
        let read = &self.analysis.reads[name];
        match &read.predicate {
            ReadPredicate::Membership {
                build,
                key,
                summary,
                ..
            } => {
                let right = symbol_name(build);
                let columns = sorted(&key.columns);
                let id = self.keyset_ident(&right, &columns);
                match summary {
                    MembershipSummary::Exact(_) => quote!(::cq_storage::Membership(#id)),
                    MembershipSummary::Row(_) => {
                        quote!(::cq_storage::RowPredicate(move |__cq_key: &_| ::cq_storage::ExactMembership::contains(#id, __cq_key)))
                    }
                }
            }
            ReadPredicate::Physical(_) => {
                let (Operator::Point(_, s) | Operator::Range(_, s)) = &self.physical[name].operator
                else {
                    unreachable!()
                };
                let column = structures::condition_column(&self.physical[name].operator)
                    .expect("checked physical read");
                let string = structures::ungroup_type(&self.infos[name].types[&column])
                    == &syn::parse_quote!(String);
                let value = |term: &structures::syntax::constraint::Term| {
                    let reference = expressions::term_reference(term, &self.parameters);
                    if string {
                        quote!(::cq_storage::StringValue::string_value(#reference))
                    } else {
                        // A correlated parameter may itself be borrowed. Coerce
                        // to the declared key reference before copying the scalar;
                        // one syntactic dereference would leave Point<&K>.
                        let ty = &self.infos[name].types[&column];
                        quote!({ let __cq_value: &#ty = #reference; *__cq_value })
                    }
                };
                if let Some((_, bounds)) = s.condition.range_bounds() {
                    let bound = |lower: bool| {
                        let mut result = quote!(::core::ops::Bound::Unbounded);
                        for b in bounds.iter().filter(|b| b.lower == lower) {
                            let v = value(&b.value);
                            let next = if b.inclusive {
                                quote!(::core::ops::Bound::Included(#v))
                            } else {
                                quote!(::core::ops::Bound::Excluded(#v))
                            };
                            result = if lower {
                                quote!(::cq_storage::intersect_lower(#result, #next))
                            } else {
                                quote!(::cq_storage::intersect_upper(#result, #next))
                            };
                        }
                        result
                    };
                    let low = bound(true);
                    let high = bound(false);
                    quote!(::cq_storage::Window { low: #low, high: #high })
                } else {
                    // Constant extraction is checked by selection. Conjuncts on
                    // the same column still run: an inconsistent point is empty.
                    let constants = structures::selection_constants(s);
                    let v = value(&constants[&column]);
                    quote!(::cq_storage::Point(#v))
                }
            }
        }
    }
    /// Name a predicate and check its host. Retained reads borrow one value;
    /// pipe expressions construct their owned predicate inside each traversal.
    pub fn prepare_read(&mut self, name: &str) -> Ident {
        let id = format_ident!(
            "__cq_{}_predicate",
            name,
            span = proc_macro2::Span::mixed_site()
        );
        let read = &self.analysis.reads[name];
        let host = format_ident!(
            "__cq_positional_{}",
            symbol_name(&read.origin.base),
            span = proc_macro2::Span::mixed_site()
        );
        let marker = &read.marker;
        self.statements.push(
            syn::parse_quote!(::cq_storage::require::block_read(#host, <#marker>::default());),
        );
        if self.physical_analysis.modes[name] != physical::ModeKind::Pipe {
            let predicate = self.read_predicate(name);
            self.statements
                .push(syn::parse_quote!(let #id = #predicate;));
        }
        id
    }
    /// Formal rule IF-Positional: Read recorded predicates as positions before fetching survivors.
    pub fn read_ids(&mut self, name: &str) -> syn::Result<TokenStream> {
        let read = &self.analysis.reads[name];
        let host = format_ident!(
            "__cq_positional_{}",
            symbol_name(&read.origin.base),
            span = proc_macro2::Span::mixed_site()
        );
        let marker = &read.marker;
        let marker = quote!(<#marker>::default());
        let input = match &self.physical[name].operator {
            Operator::Point(_, s) | Operator::Range(_, s) => symbol_name(&s.input),
            Operator::Semi(_, k) => symbol_name(&k.left),
            _ => unreachable!(),
        };
        let predicate = self.prepare_read(name);
        let combined = self.combined_join_filter_ids(name, &input, &predicate);
        let existing = match self.reprs.get_mut(&input) {
            Some(Repr::Positions(ids)) => Some(ids.take().ok_or_else(|| {
                syn::Error::new_spanned(&self.physical[name].name, "positional pipe read twice")
            })?),
            Some(Repr::Selection { selection, .. }) => Some(quote!(#selection.positions())),
            Some(Repr::Host) => self.join_filter_ids(name),
            _ => {
                return error(
                    &self.physical[name].name,
                    "positional read requires a recorded positional source",
                );
            }
        };
        let mut ids = if let Some(ids) = combined {
            ids
        } else if let Some(ids) = existing {
            quote!(::core::iter::Iterator::filter(#ids, move |&__cq_id| ::cq_storage::residual(::cq_storage::ReadPredicate::test(&#predicate, &::cq_storage::Column::key(#marker, #host, __cq_id)))))
        } else if self.physical_analysis.modes[name] == physical::ModeKind::Pipe {
            quote!(::cq_storage::BlockRead::read_owned(#host, #marker, #predicate))
        } else {
            quote!(::cq_storage::BlockRead::read(#host, #marker, &#predicate))
        };
        // A point adapter may cover only one conjunct. Test the complete
        // Physical predicate before exposing IDs to any subsequent reducer.
        if let Operator::Point(_, s) = &self.physical[name].operator {
            if s.condition.equalities.len() + s.condition.constraints.len() != 1 {
                let read = &self.analysis.reads[name];
                let fields = self.analysis.host_layouts[&symbol_name(&read.origin.base)]
                    .iter()
                    .map(|c| {
                        read.origin
                            .columns
                            .iter()
                            .find(|m| symbol_name(&m.column) == *c)
                            .map_or_else(
                                || quote!(_),
                                |m| self.ident(name, &symbol_name(&m.logical)).to_token_stream(),
                            )
                    });
                let condition = self.relational_condition(&s.condition);
                ids = quote!(::core::iter::Iterator::filter(#ids, move |&__cq_id| {
                    #[allow(unused_variables)] let (#(#fields,)*) = ::cq_storage::PositionalRows::row_at(#host, __cq_id).expect("host-issued live ID");
                    ::cq_storage::residual(#condition)
                }));
            }
        }
        if self.physical_analysis.modes[name] == physical::ModeKind::Pipe {
            // A retained union may replay this pipe for several index keys.
            // Each traversal must own its predicate, not move one shared value.
            let value = self.read_predicate(name);
            Ok(quote!({ let #predicate = #value; #ids }))
        } else {
            Ok(ids)
        }
    }
    /// Construct the named selection through exactly its checked read spine.
    /// Formal rule IF-Selection: Construct immutable snapshots through the recorded read spine.
    pub fn selection_source(&mut self, name: &str) -> syn::Result<TokenStream> {
        let s = self.analysis.selections[name].clone();
        self.snapshot(name, name, &s.ty, &s.summary)
    }
    fn snapshot(
        &mut self,
        name: &str,
        root: &str,
        ty: &syn::Type,
        summary: &Summary,
    ) -> syn::Result<TokenStream> {
        let summary = match summary {
            Summary::None(_) => quote!(::cq_storage::SelectionSummary::None),
            Summary::PerBlock(_) => quote!(::cq_storage::SelectionSummary::PerBlock),
        };
        self.snapshot_with(name, root, ty, &summary)
    }
    fn snapshot_with(
        &mut self,
        name: &str,
        root: &str,
        ty: &syn::Type,
        summary: &TokenStream,
    ) -> syn::Result<TokenStream> {
        let read = &self.analysis.reads[name];
        let host = format_ident!(
            "__cq_positional_{}",
            symbol_name(&read.origin.base),
            span = proc_macro2::Span::mixed_site()
        );
        let marker = read.marker.clone();
        let input = match &self.physical[name].operator {
            Operator::Point(_, s) | Operator::Range(_, s) => symbol_name(&s.input),
            Operator::Semi(_, k) => symbol_name(&k.left),
            _ => return error(&self.physical[name].name, "unsupported selection spine"),
        };
        let predicate = self.read_predicate(name);
        let mut input = input;
        while let Operator::Project(p) = &self.physical[&input].operator {
            self.take_positions(&input);
            input = symbol_name(&p.input);
        }
        let previous = match self.reprs.get(&input) {
            Some(Repr::Selection { selection, .. }) => Some(selection.to_token_stream()),
            Some(Repr::Host) => None,
            Some(Repr::Positions(_)) => Some(self.snapshot_with(&input, root, ty, summary)?),
            _ => {
                return error(
                    &self.physical[name].name,
                    "selection cannot bypass a retained or opaque input",
                );
            }
        };
        let result = if let Some(previous) = previous {
            quote!({ let __cq_previous = #previous; __cq_previous.narrow(<#marker>::default(), &#predicate, #summary) })
        } else {
            quote!(<#ty>::select(#host, <#marker>::default(), &#predicate, #summary))
        };
        if name != root {
            self.take_positions(name);
        }
        Ok(result)
    }
    pub fn take_positions(&mut self, name: &str) -> Option<TokenStream> {
        match self.reprs.get_mut(name) {
            Some(Repr::Positions(ids)) => ids.take(),
            _ => None,
        }
    }
}
