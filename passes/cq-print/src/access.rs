//! Reads of a value: hosts through declaration layout and type dispatch, built
//! structures through their key, merge sides through their order.

use super::*;

/// A host read through a rename, in the rename's attribute names.
pub struct HostAccess {
    pub value: Ident,
    pub prefix: bool,
    pub fallback: Ident,
    /// Computed prefix keys cannot borrow for the lifetime of host rows.
    pub owned_key: bool,
    /// Key columns in declaration order for an eligible prefix.
    pub key: Vec<String>,
    /// Declared host key types, used when a generator's item type is inferred.
    pub key_types: Vec<syn::Type>,
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
    pub fn fallback_type(&self, result: &str, reader: &str, columns: &[String]) -> &syn::Type {
        let mut columns = columns.to_vec();
        columns.sort();
        &self.analysis.fallbacks[&(result.to_owned(), reader.to_owned(), columns)]
    }

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
        let owned_key = self.owned_key(result, columns);
        let key_types = key
            .iter()
            .map(|column| self.infos[result].types[column].clone())
            .collect();
        HostAccess {
            value: host.value.clone(),
            prefix,
            key,
            key_types,
            payload,
            fallback: format_ident!("__cq_{}_fallback_{}", result, columns.join("_")),
            owned_key,
        }
    }

    /// Enumerate borrowed columns in declaration order, then apply the rename.
    pub fn host_scan(&self, result: &str) -> TokenStream {
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
                    if matches!(
                        literal,
                        Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(_),
                            ..
                        })
                    ) {
                        quote!(::cq_storage::relational_string_eq(*#binding, &#literal))
                    } else {
                        quote!(::cq_storage::relational_eq(*#binding, &#literal))
                    }
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
                let ty = self.fallback_type(result, "scan", &[]);
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
                let prefix = host
                    .layout
                    .iter()
                    .take_while(|column| literals.contains_key(*column))
                    .count();
                let native_key = prefix > 0 && (0..prefix).all(|i| {
                    !matches!(literals[&host.layout[i]], Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. }))
                        || matches!(&host.types[i], syn::Type::Reference(reference)
                            if matches!(&*reference.elem, syn::Type::Path(path) if path.path.is_ident("str")))
                });
                let rows = if native_key {
                    let types = &host.types[..prefix];
                    let values = host.layout[..prefix].iter().map(|column| literals[column]);
                    quote!({
                        use ::cq_storage::dispatch::PointRead as _;
                        (&&::cq_storage::dispatch::PointHost::<_, (#(#types,)*), _>::new(
                            #value,
                            move || {
                                let __cq_scan: #ty = ::core::iter::Iterator::map(
                                    ::cq_storage::rows::Rows::rows(#value),
                                    ::core::convert::identity
                                );
                                __cq_scan
                            }
                        )).point(&(#(#values,)*))
                    })
                } else {
                    quote!(::cq_storage::rows::Rows::rows(#value))
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
        let access = self.host_access(result, &[]);
        let value = &access.value;
        let row = self.carrier(result, |c| self.access(result, c));
        let payload = access.payload.iter().map(|c| {
            if self.carried(result).contains(c) {
                self.access(result, c)
            } else {
                quote!(_)
            }
        });
        let ty = self.fallback_type(result, "scan", &[]);
        quote! { { let __cq_scan: #ty = ::core::iter::Iterator::map(::cq_storage::rows::Rows::rows(#value), move |(#(#payload,)*)| #row); __cq_scan } }
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
            let ty = self
                .analysis
                .fallbacks
                .iter()
                .find_map(|((n, rd, cs), ty)| {
                    (n == result
                        && cs == &columns
                        && rd != "scan"
                        && !self.physical.get(rd).is_some_and(|d| {
                            matches!(
                                d.operator,
                                Operator::Merge(..) | Operator::Point(..) | Operator::Range(..)
                            )
                        }))
                    .then_some(ty)
                })
                .expect("checked fallback");
            let capture = self.carries_positions(result).then(|| quote!(move));
            let pairs = quote!(::core::iter::Iterator::map(#rows, #capture |#pattern| ((#(#key,)*), (#(#payload,)*))));
            let ty = ty.clone();
            let build_name = format!("{result}.fallback.{}", columns.join("_"));
            self.profile_label(
                &build_name,
                if probe { "INDEX_BUILD" } else { "KEYSET_BUILD" },
                &columns.join(","),
                vec![result.into()],
            );
            let pairs = self.profile_inserting(&build_name, pairs);
            let build = self.profile_build(
                &build_name,
                quote!(<#ty as ::cq_storage::Build<_>>::build(#pairs)),
            );
            self.statements.push(syn::parse_quote! {
                let #fallback: ::cq_storage::dispatch::Lazy<#ty, _> = ::cq_storage::dispatch::Lazy::new(move || #build);
            });
            self.owned.push(fallback);
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
            let access = self.host_access(result, columns);
            let value = &access.value;
            if access.prefix && access.key == *columns {
                let types = columns.iter().map(|c| &self.infos[result].types[c]);
                self.statements.push(
                    syn::parse_quote!(::cq_storage::require::ordered_key::<(#(#types,)*),_>(&#value);),
                );
            } else {
                let name = format_ident!("__cq_{}_merge_{}", result, columns.join("_"));
                let ty = self.fallback_type(result, reader, columns);
                let rows = self.profile_stream(result, self.host_scan(result));
                let pattern = self.bind_columns(result, self.carried(result), false);
                let key = columns.iter().map(|c| self.access(result, c));
                let payload = self
                    .heading(result)
                    .iter()
                    .filter(|c| !columns.contains(c))
                    .map(|c| self.access(result, c));
                let capture = self.carries_positions(result).then(|| quote!(move));
                self.statements.push(syn::parse_quote! {
                    let #name = &<#ty as ::cq_storage::Build<_>>::build(
                        ::core::iter::Iterator::map(#rows, #capture |#pattern| ((#(#key,)*),(#(#payload,)*)))
                    );
                });
                self.bound = true;
            }
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

    fn host_key_types<'b>(
        &'b self,
        left: &str,
        access: &'b HostAccess,
    ) -> impl Iterator<Item = &'b syn::Type> {
        let types = &self.infos[left].types;
        let generated = !self.generator_expressions.is_empty();
        access
            .key
            .iter()
            .zip(&access.key_types)
            .map(move |(column, host)| {
                let ty = &types[column];
                // Autoref dispatch must know its key type before selecting Seek.
                // A generator has no surface annotation; its joined host supplies
                // the same logical key type without constraining iterator items.
                if generated && matches!(ty, syn::Type::Infer(_)) {
                    host
                } else {
                    ty
                }
            })
    }

    pub fn host_lookup(&self, left: &str, access: &HostAccess, contains: bool) -> TokenStream {
        let value = &access.value;
        let fallback = &access.fallback;
        let key = self.host_key(left, access);
        let types = self.host_key_types(left, access);
        let types: Vec<_> = types.collect();
        let receiver = if access.prefix && !access.owned_key {
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
        let types = self.host_key_types(left, access);
        let types: Vec<_> = types.collect();
        let receiver = if access.prefix && !access.owned_key {
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
