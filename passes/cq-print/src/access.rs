//! Reads of a value: hosts through their recorded native read or fallback,
//! built structures through their key, merge sides through their order.
//! Every recorded read prints as exactly one call (IF-Dispatch).

use super::*;

/// A host read through a rename, in the rename's attribute names.
pub struct HostAccess {
    pub value: Ident,
    /// The recorded native read (STR-NativeHost); otherwise the read uses
    /// its recorded fallback.
    pub native: Option<structures::Native>,
    pub fallback: Ident,
    /// Checked full-row-key probes observe existence, not witness payloads.
    pub membership: bool,
    /// This read uses a declared immutable cursor contract and a borrowed fallback.
    pub retained: bool,
    /// The actual selected host cursor must also expose an exact finite length.
    pub expand: bool,
    /// Computed prefix keys cannot borrow for the lifetime of host rows.
    pub owned_key: bool,
    /// Key columns: a native read's recorded order, else declaration order
    /// for a declaration prefix and sorted order otherwise.
    pub key: Vec<String>,
    /// Declared host key types, used when a generator's item type is inferred.
    pub key_types: Vec<syn::Type>,
    /// Remaining columns in the host's declared order.
    pub payload: Vec<String>,
}

/// The value answering a keyed read.
pub enum Access {
    /// The checked empty-key read enumerates the retained whole relation.
    Rows,
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
    /// The recorded host fallback provider of this reader and key.
    pub fn fallback_provider(
        &self,
        result: &str,
        reader: cq_lang_structures::ReaderRef<String>,
        columns: &[String],
    ) -> &cq_lang_structures::Provider {
        let mut columns = columns.to_vec();
        columns.sort();
        &self.analysis.fallbacks[&(result.to_owned(), reader, columns)]
    }

    /// The printed spelling of the recorded host fallback provider.
    pub fn fallback_type(
        &self,
        result: &str,
        reader: cq_lang_structures::ReaderRef<String>,
        columns: &[String],
    ) -> &syn::Type {
        &self.fallback_provider(result, reader, columns).ty
    }

    /// The host behind a rename read by `reader` on `columns`, with its key
    /// and payload in the rename's attribute names and the recorded native
    /// read, if any.
    pub fn host_access(
        &self,
        result: &str,
        reader: Option<cq_lang_structures::ReaderRef<String>>,
        columns: &[String],
    ) -> HostAccess {
        let host = &self.hosts[&self.physical_analysis.hosts[result]];
        let inverse: BTreeMap<_, _> = self.physical_analysis.renames[result]
            .iter()
            .map(|(t, s)| (s, t))
            .collect();
        let layout: Vec<String> = host.layout.iter().map(|c| inverse[c].clone()).collect();
        let mut sorted_columns = columns.to_vec();
        sorted_columns.sort();
        let native = reader.and_then(|reader| {
            self.analysis
                .natives
                .get(&(result.to_owned(), reader, sorted_columns))
                .cloned()
        });
        let key = if let Some(native) = &native {
            native.key.clone()
        } else if layout[..columns.len()].iter().all(|c| columns.contains(c)) {
            layout[..columns.len()].to_vec()
        } else {
            columns.to_vec()
        };
        let payload = layout
            .into_iter()
            .filter(|c| !columns.contains(c))
            .collect();
        let owned_key = self.owned_key(result, columns);
        let key_types = key
            .iter()
            .map(|column| self.infos[result].types[column].clone())
            .collect();
        let (retained, expand) = match &native {
            Some(native) => (
                native.supports(structures::ProviderCapability::Retain),
                native.supports(structures::ProviderCapability::Expand),
            ),
            None => {
                let support = |capability| {
                    self.analysis.fallbacks.keys().any(|(name, reader, key)| {
                        name == result
                            && key == columns
                            && reader.result().is_some_and(|reader| {
                                self.analysis.provider_support(
                                    result,
                                    columns,
                                    Some(reader),
                                    capability,
                                )
                            })
                    })
                };
                (
                    support(structures::ProviderCapability::Retain),
                    support(structures::ProviderCapability::Expand),
                )
            }
        };
        HostAccess {
            value: host.value.clone(),
            membership: self
                .analysis
                .membership
                .contains(&(result.to_owned(), columns.to_vec())),
            native,
            retained,
            expand,
            key,
            key_types,
            payload,
            fallback: format_ident!("__cq_{}_fallback_{}", result, columns.join("_")),
            owned_key,
        }
    }

    /// Enumerate borrowed columns in declaration order, then apply the rename.
    pub fn host_scan(&self, result: &str) -> TokenStream {
        self.host_scan_from(result, None)
    }

    /// Apply the recorded host rename to an optionally borrowed tile iterator.
    pub fn host_scan_from(&self, result: &str, source: Option<TokenStream>) -> TokenStream {
        if self.encoded.blocks.scans.contains_key(result) {
            return self.blocked_rows(result, source);
        }
        if let Operator::Rename(rename) = &self.physical[result].operator {
            // Literal predicates belong only to this row source. Retained rows
            // and indices consume its projected tuples through ordinary builds.
            if let Some(constants) = &rename.constants {
                let host = &self.hosts[&self.physical_analysis.hosts[result]];
                let value = &host.value;
                let bindings = host
                    .layout
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format_ident!("__cq_source_{i}"))
                    .collect::<Vec<_>>();
                let tests = constants.equalities.iter().map(|equality| {
                    let position = host
                        .layout
                        .iter()
                        .position(|column| *column == symbol_name(&equality.left))
                        .expect("checked occurrence column");
                    let binding = &bindings[position];
                    let structures::syntax::EqualityValue::Literal(literal) = &equality.right
                    else {
                        unreachable!("checked occurrence literal")
                    };
                    quote!(::cq_storage::relational_eq(*#binding, &#literal))
                });
                let row = self.carrier(result, |column| {
                    let source = &self.physical_analysis.renames[result][column];
                    let position = host
                        .layout
                        .iter()
                        .position(|column| column == source)
                        .expect("checked occurrence mapping");
                    bindings[position].to_token_stream()
                });
                let ty = self.fallback_type(result, cq_lang_structures::ReaderRef::Scan, &[]);
                let literals: BTreeMap<_, _> = constants
                    .equalities
                    .iter()
                    .map(|equality| {
                        let structures::syntax::EqualityValue::Literal(literal) = &equality.right
                        else {
                            unreachable!("checked occurrence literal")
                        };
                        (symbol_name(&equality.left), literal)
                    })
                    .collect();
                // A recorded native scan reads the leading literal constants by
                // one point read; the residual filter still tests every literal.
                let native =
                    self.analysis
                        .natives
                        .iter()
                        .find_map(|((name, reader, _), native)| {
                            (name == result
                                && matches!(reader, cq_lang_structures::ReaderRef::Scan))
                            .then_some(native)
                        });
                let rows = match (native, source.clone()) {
                    (Some(native), None) => {
                        let types = native.key.iter().map(|column| {
                            &host.types[host
                                .layout
                                .iter()
                                .position(|c| c == column)
                                .expect("checked native scan column")]
                        });
                        let values = native.key.iter().map(|column| literals[column]);
                        quote!({
                            let __cq_key: (#(#types,)*) = (#(#values,)*);
                            ::cq_storage::dispatch::point(#value, &__cq_key)
                        })
                    }
                    (_, source) => {
                        source.unwrap_or_else(|| quote!(::cq_storage::rows::Rows::rows(#value)))
                    }
                };
                return quote! {{
                    let __cq_scan: #ty = ::core::iter::Iterator::map(
                        ::core::iter::Iterator::filter(
                            #rows,
                            move |(#(#bindings,)*)| true #(&& #tests)*
                        ),
                        move |(#(#bindings,)*)| #row
                    );
                    __cq_scan
                }};
            }
        }
        if let Some(ids) = self.join_filter_ids(result) {
            return self.positional_rows(result, ids);
        }
        if self.carries_positions(result) {
            let host = Self::positional_host(&self.physical_analysis.hosts[result]);
            let ids = quote!(::cq_storage::PositionalRows::positions(#host));
            return self.positional_rows(result, ids);
        }
        let access = self.host_access(result, None, &[]);
        let value = &access.value;
        let row = self.carrier(result, |c| self.access(result, c));
        let payload = access.payload.iter().map(|c| {
            if self.carried(result).contains(c) {
                self.access(result, c)
            } else {
                quote!(_)
            }
        });
        let ty = self.fallback_type(result, cq_lang_structures::ReaderRef::Scan, &[]);
        let rows = source.unwrap_or_else(|| quote!(::cq_storage::rows::Rows::rows(#value)));
        quote! { { let __cq_scan: #ty = ::core::iter::Iterator::map(#rows, move |(#(#payload,)*)| #row); __cq_scan } }
    }

    /// Allocate one lazy fallback for each host read key, outside all probe closures.
    pub fn prepare_host(&mut self, result: &str) {
        self.native_guards(result);
        if !self.analysis.host_read(result) {
            return;
        }
        // One lazy fallback per key that a recorded fallback serves; native
        // reads (STR-NativeHost) build nothing.
        let mut keys: BTreeMap<Vec<String>, (bool, &structures::Provider)> = BTreeMap::new();
        for ((name, reader, columns), provider) in &self.analysis.fallbacks {
            if name != result {
                continue;
            }
            let probe = match reader {
                cq_lang_structures::ReaderRef::Scan => continue,
                cq_lang_structures::ReaderRef::Candidates if self.intersect => true,
                cq_lang_structures::ReaderRef::Membership if self.intersect => false,
                cq_lang_structures::ReaderRef::Candidates
                | cq_lang_structures::ReaderRef::Membership => continue,
                cq_lang_structures::ReaderRef::Result(reader) => {
                    match &self.physical[reader].operator {
                        Operator::Merge(..) | Operator::Point(..) | Operator::Range(..) => continue,
                        Operator::Probe(..) => true,
                        _ => false,
                    }
                }
            };
            keys.entry(columns.clone())
                .and_modify(|(p, _)| *p |= probe)
                .or_insert((probe, provider));
        }
        for (columns, (probe, ty)) in keys {
            let a = self.host_access(result, None, &columns);
            let fallback = a.fallback;
            let rows = self.profile_stream(result, self.host_scan(result));
            let pattern = self.bind_columns(result, self.carried(result), false);
            let key = a.key.iter().map(|c| {
                let ident = self.access(result, c);
                if a.owned_key {
                    quote!(::core::clone::Clone::clone(#ident))
                } else {
                    ident.to_token_stream()
                }
            });
            let payload = a.payload.iter().map(|c| self.access(result, c));
            let capture = self.carries_positions(result).then(|| quote!(move));
            let pairs = quote!(::core::iter::Iterator::map(#rows, #capture |#pattern| ((#(#key,)*), (#(#payload,)*))));
            let ty = ty.ty.clone();
            let build_name = format!("{result}.fallback.{}", columns.join("_"));
            self.profile_label(
                &build_name,
                if probe { "INDEX_BUILD" } else { "KEYSET_BUILD" },
                &columns.join(","),
                vec![result.into()],
            );
            let pairs = self.profile_inserting(&build_name, pairs);
            let construct = quote!(<#ty as ::cq_storage::Build<_>>::build(#pairs));
            let declared_type = ty;
            let build = self.profile_build(&build_name, construct);
            let mut guards = Vec::new();
            for ((name, selected, key), selected_provider) in &self.analysis.fallbacks {
                if name == result && key == &columns {
                    guards.extend(self.provider_guards(
                        result,
                        std::slice::from_ref(&columns),
                        selected_provider,
                        Some(selected),
                        quote!(&__cq_provider),
                    ));
                }
            }
            // The recorded fallback's declared contracts are proved on it;
            // native reads carry their own `require::native_*` guards.
            let build = quote!({
                let __cq_provider = #build;
                #(#guards)*
                __cq_provider
            });
            if a.retained {
                self.statements.push(syn::parse_quote! {
                    let #fallback: &::cq_storage::dispatch::Lazy<#declared_type, _> = &::cq_storage::dispatch::Lazy::new(move || #build);
                });
                self.bound = true;
            } else {
                self.statements.push(syn::parse_quote! {
                    let #fallback: ::cq_storage::dispatch::Lazy<#declared_type, _> = ::cq_storage::dispatch::Lazy::new(move || #build);
                });
                self.owned.push(fallback);
            }
        }
        let mut merged = std::collections::BTreeSet::new();
        for (reader, columns) in self
            .physical_analysis
            .merge_reads
            .get(result)
            .into_iter()
            .flatten()
        {
            if !merged.insert(columns.clone()) {
                continue;
            }
            let selected = cq_lang_structures::ReaderRef::Result(reader.clone());
            let access = self.host_access(result, Some(selected.clone()), columns);
            let value = &access.value;
            if access.native.is_some() {
                let types = columns.iter().map(|c| &self.infos[result].types[c]);
                self.statements.push(
                    syn::parse_quote!(::cq_storage::require::ordered_key::<(#(#types,)*),_>(&#value);),
                );
            } else {
                let name = format_ident!("__cq_{}_merge_{}", result, columns.join("_"));
                let provider = self.fallback_provider(
                    result,
                    cq_lang_structures::ReaderRef::Result(reader.into()),
                    columns,
                );
                let ty = &provider.ty;
                let rows = self.profile_stream(result, self.host_scan(result));
                let pattern = self.bind_columns(result, self.carried(result), false);
                let key = columns.iter().map(|c| self.access(result, c));
                let payload = self
                    .heading(result)
                    .iter()
                    .filter(|c| !columns.contains(c))
                    .map(|c| self.access(result, c));
                let capture = self.carries_positions(result).then(|| quote!(move));
                let guards = self.provider_guards(
                    result,
                    std::slice::from_ref(columns),
                    provider,
                    Some(&selected),
                    quote!(#name),
                );
                self.statements.push(syn::parse_quote! {
                    let #name = &<#ty as ::cq_storage::Build<_>>::build(
                        ::core::iter::Iterator::map(#rows, #capture |#pattern| ((#(#key,)*),(#(#payload,)*)))
                    );
                });
                self.statements.extend(guards);
                self.statements
                    .push(syn::parse_quote!(::cq_storage::require::ordered(#name);));
                self.bound = true;
            }
        }
    }

    /// The value answering `reader`'s read of `result` keyed on sorted
    /// `columns`: a structure built on them, else the recorded host read.
    pub fn keyed(&self, reader: &str, result: &Ident, columns: &[String]) -> syn::Result<Access> {
        self.keyed_by(
            cq_lang_structures::ReaderRef::Result(reader.to_owned()),
            result,
            columns,
        )
    }

    /// [`Self::keyed`] for a host reader role (intersection candidates and
    /// membership).
    pub fn keyed_by(
        &self,
        reader: cq_lang_structures::ReaderRef<String>,
        result: &Ident,
        columns: &[String],
    ) -> syn::Result<Access> {
        let name = symbol_name(result);
        if self
            .analysis
            .read_accesses
            .get(&name)
            .and_then(|reads| reads.get(columns))
            == Some(&structures::ReadAccess::Whole)
        {
            return Ok(Access::Rows);
        }
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Access::Built(structure));
        }
        if let Some((levels, depth)) = self.built_levels(&name, columns) {
            let prefix = levels[..depth].to_vec();
            return Ok(Access::Prefix(self.built_ident(&name, levels), prefix));
        }
        if self.analysis.host_read(&name) {
            return Ok(Access::Host(self.host_access(&name, Some(reader), columns)));
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
    /// levels and how many of them a read on `columns` applies. Complete
    /// keys take precedence over candidate prefixes, as in Structures.
    pub fn built_levels(
        &self,
        name: &str,
        columns: &[String],
    ) -> Option<(&Vec<Vec<String>>, usize)> {
        let Some(Repr::Let { structures, .. } | Repr::Selection { structures, .. }) =
            self.reprs.get(name)
        else {
            return None;
        };
        structures
            .iter()
            .filter_map(|(levels, _)| {
                structures::prefix_depth(levels, columns).map(|depth| (levels, depth))
            })
            .min_by_key(|(levels, depth)| (levels.len() != *depth, levels.len()))
    }

    pub fn built_ident(&self, name: &str, levels: &[Vec<String>]) -> Ident {
        let Some(Repr::Let { structures, .. } | Repr::Selection { structures, .. }) =
            self.reprs.get(name)
        else {
            unreachable!("built_levels found it");
        };
        structures
            .iter()
            .find(|(l, _)| l == levels)
            .map(|(_, s)| s.clone())
            .expect("built_levels found it")
    }

    pub fn host_key(&self, result: &str, access: &HostAccess) -> TokenStream {
        // A computed binding lives only for its prefix iteration. Its
        // fallback therefore owns key values, while payloads remain borrowed.
        let key = access
            .key
            .iter()
            .map(|c| self.key_field(result, c, access.owned_key));
        quote!(&(#(#key,)*))
    }

    fn host_key_types<'b>(&'b self, access: &'b HostAccess) -> impl Iterator<Item = &'b syn::Type> {
        access.key_types.iter()
    }

    /// The recorded native host value: the host itself, or its explicitly
    /// indexed declaration slot.
    pub fn native_value(&self, access: &HostAccess) -> TokenStream {
        let value = &access.value;
        match access.native.as_ref().and_then(|native| native.slot) {
            Some(slot) => quote!(::cq_storage::dispatch::ColumnHost::<_, #slot>::new(#value)),
            None => quote!(#value),
        }
    }

    /// The borrowed key of a native read: query columns are borrowed for the
    /// one call; the runtime owns the key once (`OwnKey`).
    fn native_key(&self, left: &str, access: &HostAccess) -> TokenStream {
        if !access.owned_key {
            return self.host_key(left, access);
        }
        let types: Vec<_> = self.host_key_types(access).collect();
        let fields = access.key.iter().zip(&types).map(|(column, ty)| {
            let value = self.access(left, column);
            // Pull filters borrow computed fields; push sinks own them.
            // Borrow fixes one logical reference in either representation.
            quote!(::core::borrow::Borrow::<#ty>::borrow(&#value))
        });
        quote!(&(#(#fields,)*))
    }

    /// A retained handle whose cursor borrows stable query storage: the
    /// recorded native read's key-normalizing handle or the shared fallback.
    pub fn host_retained_read(&self, access: &HostAccess) -> TokenStream {
        let fallback = &access.fallback;
        let types: Vec<_> = self.host_key_types(access).collect();
        let (read, proof) = match (&access.native, access.expand) {
            (Some(_), true) => {
                let value = self.native_value(access);
                (
                    quote!(::cq_storage::dispatch::expanded::<_, (#(&#types,)*)>(#value)),
                    quote!(::cq_storage::provider_contract::require_expand(&__cq_read);),
                )
            }
            (Some(_), false) => {
                let value = self.native_value(access);
                (
                    quote!(::cq_storage::dispatch::retained::<_, (#(&#types,)*)>(#value)),
                    quote!(::cq_storage::provider_contract::require_retain(&__cq_read);),
                )
            }
            (None, true) => (
                quote!(#fallback.get()),
                quote!(::cq_storage::provider_contract::require_expand(&__cq_read);),
            ),
            (None, false) => (
                quote!(#fallback.get()),
                quote!(::cq_storage::provider_contract::require_retain(&__cq_read);),
            ),
        };
        quote!({
            let __cq_read = #read;
            #proof
            __cq_read
        })
    }

    pub fn host_lookup(
        &self,
        left: &str,
        access: &HostAccess,
        contains: bool,
        negative: bool,
    ) -> TokenStream {
        let value = &access.value;
        if negative && access.key.is_empty() {
            // An empty anti key asks only whether the host has a witness.
            // No keyed storage is needed, even for hosts without membership.
            return quote!(::core::iter::Iterator::next(
                &mut ::cq_storage::rows::Rows::rows(#value)
            ).is_some());
        }
        if access.retained && !contains {
            let read = self.host_retained_read(access);
            let key = self.host_key(left, access);
            return quote!(::cq_storage::provider_contract::retain(#read, #key));
        }
        let membership = contains || access.membership;
        let test = if let Some(native) = &access.native {
            let value = self.native_value(access);
            let key = self.native_key(left, access);
            match native.read {
                structures::syntax::HostRead::Membership if native.borrowed => {
                    quote!(::cq_storage::dispatch::contains_borrowed(#value, #key))
                }
                structures::syntax::HostRead::Membership => {
                    quote!(::cq_storage::dispatch::contains(#value, #key))
                }
                structures::syntax::HostRead::Exact if membership => {
                    quote!(::core::iter::Iterator::next(&mut ::cq_storage::dispatch::seek(#value, #key)).is_some())
                }
                structures::syntax::HostRead::Exact => {
                    return quote!(::cq_storage::dispatch::seek(#value, #key));
                }
                _ => unreachable!("checked native lookup capability"),
            }
        } else {
            let fallback = &access.fallback;
            if negative && access.owned_key {
                // The fallback owns its keys; the probe borrows the query's.
                let key = self.native_key(left, access);
                quote!(::cq_storage::ExactMembership::contains(
                    #fallback.get(),
                    &::cq_storage::rows::OwnKey::own_key(#key)
                ))
            } else if membership {
                let key = self.host_key(left, access);
                quote!(::cq_storage::ExactMembership::contains(#fallback.get(), #key))
            } else {
                let key = self.host_key(left, access);
                return quote!(::cq_storage::IndexRead::lookup(#fallback.get(), #key));
            }
        };
        if contains {
            test
        } else {
            quote!(::core::iter::IntoIterator::into_iter((#test).then_some(())))
        }
    }

    pub fn host_candidates(&self, left: &str, access: &HostAccess, count: bool) -> TokenStream {
        let fallback = &access.fallback;
        let key = self.host_key(left, access);
        match (&access.native, count) {
            (Some(_), true) => {
                let value = self.native_value(access);
                let key = self.native_key(left, access);
                quote!(::cq_storage::dispatch::candidate_count(#value, #key))
            }
            (Some(_), false) => {
                let value = self.native_value(access);
                let key = self.native_key(left, access);
                quote!(::cq_storage::dispatch::candidate_keys(#value, #key))
            }
            (None, true) => quote!(::cq_storage::Candidates::count(#fallback.get(), #key)),
            (None, false) => quote!(::cq_storage::Candidates::keys(#fallback.get(), #key)),
        }
    }

    /// One `require::native_*` guard per recorded native read of `result`,
    /// outside all closures: a host whose type lacks the declared capability
    /// fails here, naming the trait (IF-HostDecl).
    pub fn native_guards(&mut self, result: &str) {
        let natives: Vec<_> = self
            .analysis
            .natives
            .iter()
            .filter(|((name, _, _), _)| name == result)
            .map(|((_, reader, columns), native)| (reader.clone(), columns.clone(), native.clone()))
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        for (reader, columns, native) in natives {
            if matches!(reader, cq_lang_structures::ReaderRef::Scan) {
                let host = &self.hosts[&self.physical_analysis.hosts[result]];
                let value = &host.value;
                let types = native.key.iter().map(|column| {
                    &host.types[host
                        .layout
                        .iter()
                        .position(|c| c == column)
                        .expect("checked native scan column")]
                });
                self.statements.push(syn::parse_quote!(
                    ::cq_storage::require::native_point::<(#(#types,)*), _>(&#value);
                ));
                continue;
            }
            let point = matches!(&reader, cq_lang_structures::ReaderRef::Result(name)
                if matches!(self.physical[name].operator, Operator::Point(..)));
            if !seen.insert((native.read, native.key.clone(), point)) {
                continue;
            }
            let access = self.host_access(result, Some(reader), &columns);
            let value = self.native_value(&access);
            let types = native.key.iter().map(|c| &self.infos[result].types[c]);
            use structures::syntax::HostRead;
            let guard: Stmt = match native.read {
                HostRead::Exact if point => syn::parse_quote!(
                    ::cq_storage::require::native_point::<(#(#types,)*), _>(&#value);
                ),
                HostRead::Exact => syn::parse_quote!(
                    ::cq_storage::require::native_exact::<(#(#types,)*), _>(&#value);
                ),
                HostRead::Membership if native.borrowed => syn::parse_quote!(
                    ::cq_storage::require::native_membership::<(#(&#types,)*), _>(&#value);
                ),
                HostRead::Membership => syn::parse_quote!(
                    ::cq_storage::require::native_membership::<(#(#types,)*), _>(&#value);
                ),
                HostRead::Candidates => syn::parse_quote!(
                    ::cq_storage::require::native_candidates::<(#(#types,)*), _>(&#value);
                ),
                HostRead::Range => syn::parse_quote!(
                    ::cq_storage::require::native_range::<(#(&#types,)*), _>(&#value);
                ),
                HostRead::Ordered => syn::parse_quote!(
                    ::cq_storage::require::ordered_key::<(#(#types,)*), _>(&#value);
                ),
            };
            self.statements.push(guard);
        }
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
            let access = self.host_access(
                &name,
                Some(cq_lang_structures::ReaderRef::Result(symbol_name(reader))),
                columns,
            );
            if access.native.is_none() {
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
