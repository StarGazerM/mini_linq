//! Reads of a value: hosts through declaration layout and type dispatch, built
//! structures through their key, merge sides through their order.

use super::*;

/// A host read through a rename, in the rename's attribute names.
pub struct HostAccess {
    pub value: Ident,
    pub prefix: bool,
    pub fallback: Ident,
    /// Key columns in declaration order for an eligible prefix.
    pub key: Vec<String>,
    /// Remaining columns in the host's declared order.
    pub payload: Vec<String>,
}

/// The value answering a keyed read.
pub enum Access {
    Host(HostAccess),
    Built(Ident),
    /// A curried structure read on a proper prefix of its levels: the
    /// structure and the prefix's levels.
    Prefix(Ident, Vec<Vec<String>>),
}

/// One side of a merge.
pub enum Side {
    Host(HostAccess),
    Built(Ident),
    Pipe(TokenStream),
}

impl<'a> Context<'a> {
    /// The host behind a rename, with its key and payload in the rename's
    /// attribute names.
    pub fn host_access(&self, result: &str, columns: &[String]) -> HostAccess {
        let host = &self.hosts[&self.physical_analysis.hosts[result]];
        let inverse: BTreeMap<_, _> = self.physical_analysis.renames[result]
            .iter()
            .map(|(t, s)| (s, t))
            .collect();
        let layout: Vec<String> = host.layout.iter().map(|c| inverse[c].clone()).collect();
        // Formal rule IF-Dispatch: Only declaration prefixes are eligible for native reads.
        let prefix = layout[..columns.len()].iter().all(|c| columns.contains(c));
        let key = if prefix {
            layout[..columns.len()].to_vec()
        } else {
            columns.to_vec()
        };
        let payload = layout
            .into_iter()
            .filter(|c| !columns.contains(c))
            .collect();
        HostAccess {
            value: host.value.clone(),
            prefix,
            key,
            payload,
            fallback: format_ident!("__cq_{}_fallback_{}", result, columns.join("_")),
        }
    }

    /// Enumerate borrowed columns in declaration order, then apply the rename.
    pub fn host_scan(&self, result: &str) -> TokenStream {
        let access = self.host_access(result, &[]);
        let value = &access.value;
        let row = self.pattern(result);
        let payload = access.payload.iter().map(|c| self.ident(result, c));
        quote! { ::core::iter::Iterator::map(::cq_storage::rows::Rows::rows(#value), move |(#(#payload,)*)| #row) }
    }

    /// Allocate one lazy fallback for each host read key, outside all probe closures.
    pub fn prepare_host(&mut self, result: &str) {
        if !self.analysis.host_read(result) {
            return;
        }
        let mut keys = BTreeMap::new();
        for (reader, columns) in self
            .physical_analysis
            .key_reads
            .get(result)
            .into_iter()
            .flatten()
        {
            let probe = matches!(self.physical[reader].operator, Operator::Probe(..));
            keys.entry(columns.clone())
                .and_modify(|p| *p |= probe)
                .or_insert(probe);
        }
        if self.intersect {
            for prefix in self.candidate_reads.get(result).into_iter().flatten() {
                keys.insert(prefix.clone(), true);
                keys.entry(self.heading(result).to_vec()).or_insert(false);
            }
        }
        for (columns, probe) in keys {
            let a = self.host_access(result, &columns);
            let fallback = a.fallback;
            let rows = self.host_scan(result);
            let pattern = self.pattern(result);
            let key = a.key.iter().map(|c| self.ident(result, c));
            let payload = a.payload.iter().map(|c| self.ident(result, c));
            let ty = if probe {
                quote!(::cq_storage::HashIndex<'_, _, _>)
            } else {
                quote!(::cq_storage::dispatch::KeySet<_>)
            };
            self.statements.push(syn::parse_quote! {
                let #fallback: ::cq_storage::dispatch::Lazy<#ty, _> = ::cq_storage::dispatch::Lazy::new(move || <#ty as ::cq_storage::Build<_>>::build(
                    ::core::iter::Iterator::map(#rows, |#pattern| ((#(#key,)*), (#(#payload,)*)))
                ));
            });
            self.owned.push(fallback);
        }
        let mut merged = std::collections::BTreeSet::new();
        for (_, columns) in self
            .physical_analysis
            .merge_reads
            .get(result)
            .into_iter()
            .flatten()
        {
            if !merged.insert(columns.clone()) {
                continue;
            }
            let access = self.host_access(result, columns);
            let value = &access.value;
            if access.prefix && access.key == *columns {
                let types = columns.iter().map(|c| &self.infos[result].types[c]);
                self.statements.push(
                    syn::parse_quote!(::cq_storage::require::ordered_key::<(#(#types,)*),_>(&#value);),
                );
            } else {
                let name = format_ident!("__cq_{}_merge_{}", result, columns.join("_"));
                let rows = self.host_scan(result);
                let pattern = self.pattern(result);
                let key = columns.iter().map(|c| self.ident(result, c));
                let payload = self
                    .heading(result)
                    .iter()
                    .filter(|c| !columns.contains(c))
                    .map(|c| self.ident(result, c));
                self.statements.push(syn::parse_quote! {
                    let #name = &<::std::collections::BTreeMap<_,::std::vec::Vec<_>> as ::cq_storage::Build<_>>::build(
                        ::core::iter::Iterator::map(#rows, |#pattern| ((#(#key,)*),(#(#payload,)*)))
                    );
                });
                self.bound = true;
            }
        }
        if let Some(reads) = self.physical_analysis.range_reads.get(result) {
            let column = &reads[0].1;
            let a = self.host_access(result, std::slice::from_ref(column));
            let fallback = format_ident!("__cq_{}_range", result);
            let rows = self.host_scan(result);
            let pattern = self.pattern(result);
            let key = self.ident(result, column);
            let all = a
                .key
                .iter()
                .chain(&a.payload)
                .map(|c| self.ident(result, c));
            self.statements.push(syn::parse_quote! {
                let #fallback = ::cq_storage::dispatch::Lazy::new(move || <::cq_storage::dispatch::RangeIndex<_, _> as ::cq_storage::Build<_>>::build(
                    ::core::iter::Iterator::map(#rows, |#pattern| (((*#key),), (#(#all,)*)))
                ));
            });
            let handle = format_ident!("__cq_{}_host", result);
            let value = a.value;
            let ty = &self.infos[result].types[column];
            self.statements.push(syn::parse_quote! { let #handle = ::cq_storage::dispatch::Host::<_, (#ty,), _, _>::new(#value, &#fallback); });
            self.owned.push(handle);
        }
    }

    /// The value answering a read of `result` keyed on sorted `columns`: a
    /// structure built on them, else a reusable host read.
    pub fn keyed(&self, result: &Ident, columns: &[String]) -> syn::Result<Access> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Access::Built(structure));
        }
        if let Some((levels, depth)) = self.built_levels(&name, columns) {
            let prefix = levels[..depth].to_vec();
            return Ok(Access::Prefix(self.built_ident(&name, levels), prefix));
        }
        if self.analysis.host_read(&name) {
            return Ok(Access::Host(self.host_access(&name, columns)));
        }
        error(
            result,
            format!(
                "no structure or reusable host read answers the read of `{name}` on {columns:?}"
            ),
        )
    }

    pub fn built(&self, name: &str, columns: &[String]) -> Option<Ident> {
        self.built_levels(name, columns)
            .filter(|(levels, depth)| levels.len() == *depth)
            .map(|(levels, _)| self.built_ident(name, levels))
    }

    /// A built structure whose leading levels' union is `columns`: its
    /// levels and how many of them a read on `columns` applies.
    pub fn built_levels(
        &self,
        name: &str,
        columns: &[String],
    ) -> Option<(&Vec<Vec<String>>, usize)> {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(name) else {
            return None;
        };
        structures.iter().find_map(|(levels, _)| {
            structures::prefix_depth(levels, columns).map(|depth| (levels, depth))
        })
    }

    pub fn built_ident(&self, name: &str, levels: &[Vec<String>]) -> Ident {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(name) else {
            unreachable!("built_levels found it");
        };
        structures
            .iter()
            .find(|(l, _)| l == levels)
            .map(|(_, s)| s.clone())
            .expect("built_levels found it")
    }

    pub fn host_key(&self, result: &str, access: &HostAccess) -> TokenStream {
        let key = access.key.iter().map(|c| self.ident(result, c));
        quote!(&(#(#key,)*))
    }

    pub fn host_lookup(&self, left: &str, access: &HostAccess, contains: bool) -> TokenStream {
        let value = &access.value;
        let fallback = &access.fallback;
        let key = self.host_key(left, access);
        let types = access.key.iter().map(|c| &self.infos[left].types[c]);
        let types: Vec<_> = types.collect();
        let receiver = if access.prefix {
            quote!(&&::cq_storage::dispatch::Host::<_, (#(#types,)*), _, _>::new(#value, &#fallback))
        } else {
            quote!(&::cq_storage::dispatch::Host::<_, (#(#types,)*), _, _>::new(#value, &#fallback))
        };
        if contains {
            quote!({ use ::cq_storage::dispatch::Contains as _; (#receiver).contains(#key) })
        } else {
            quote!({ use ::cq_storage::dispatch::Seek as _; (#receiver).seek(#key) })
        }
    }

    pub fn host_candidates(&self, left: &str, access: &HostAccess, count: bool) -> TokenStream {
        let value = &access.value;
        let fallback = &access.fallback;
        let key = self.host_key(left, access);
        let types = access.key.iter().map(|c| &self.infos[left].types[c]);
        let types: Vec<_> = types.collect();
        let receiver = if access.prefix {
            quote!(&&::cq_storage::dispatch::Host::<_, (#(#types,)*), _, _>::new(#value, &#fallback))
        } else {
            quote!(&::cq_storage::dispatch::Host::<_, (#(#types,)*), _, _>::new(#value, &#fallback))
        };
        let method = if count { quote!(count) } else { quote!(keys) };
        quote!({ use ::cq_storage::dispatch::CandidateRead as _; (#receiver).#method(#key) })
    }

    /// The cursor merging or ranging over `result` on sorted `columns`; a
    /// pipe sorted on them by its own merge is consumed here.
    pub fn ordered(
        &mut self,
        reader: &Ident,
        result: &Ident,
        columns: &[String],
    ) -> syn::Result<Side> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Side::Built(structure));
        }
        if self.analysis.host_read(&name) {
            let access = self.host_access(&name, columns);
            if !access.prefix || access.key != columns {
                return Ok(Side::Built(format_ident!(
                    "__cq_{}_merge_{}",
                    name,
                    columns.join("_")
                )));
            }
            return Ok(Side::Host(access));
        }
        if let Some(Operator::Merge(_, k)) = self.physical.get(&name).map(|d| &d.operator) {
            if sorted(&k.columns) == columns {
                if let Some(rows) = self.take_pipe(&name) {
                    return Ok(Side::Pipe(rows));
                }
            }
        }
        error(
            reader,
            format!(
                "no ordered structure, direct host read or sorted stream answers the read of `{name}` on {columns:?}"
            ),
        )
    }
}
