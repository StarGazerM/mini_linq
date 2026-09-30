//! One iterator adapter per physical operator.

use super::*;

impl<'a> Printer<'a> {
    /// The rows of one physical operator, in the result's heading order.
    pub(super) fn produce(
        &mut self,
        name: &str,
        definition: &physical::Definition,
    ) -> syn::Result<TokenStream> {
        match &definition.operator {
            Operator::Unit(_) => Ok(quote!(::core::iter::once(()))),
            Operator::Rename(_) => Ok(self.host_scan(name)),
            Operator::Select(select) => {
                let input = self.scan(&select.input)?;
                let pattern = self.borrowed_pattern(&symbol_name(&select.input));
                let condition = deref_predicates(&select.condition);
                Ok(quote! {
                    ::core::iter::Iterator::filter(
                        #input,
                        move |#[allow(unused_variables)] #pattern| #condition
                    )
                })
            }
            Operator::Range(_, ranged) => {
                self.range(name, &definition.name, &ranged.column, ranged)
            }
            Operator::Extend(extend) => {
                let input = self.scan(&extend.input)?;
                let input_name = symbol_name(&extend.input);
                let pattern = self.pattern(&input_name);
                let shadows = self.shadows(&input_name);
                let names = extend.columns.iter().map(|c| &c.name);
                let types = extend.columns.iter().map(|c| &c.ty);
                let value = &extend.value;
                let row = self.pattern(name);
                Ok(quote! {
                    ::core::iter::Iterator::map(#input, move |#pattern| {
                        let (#(#names,)*): (#(#types,)*) = { #(#shadows)* #value };
                        #row
                    })
                })
            }
            Operator::Project(p) => {
                let project = &p.input;
                let input = self.scan(project)?;
                let input_name = symbol_name(project);
                let kept = self.heading(name);
                let binders = self.heading(&input_name).iter().map(|c| {
                    if kept.contains(c) {
                        self.ident(&input_name, c).to_token_stream()
                    } else {
                        quote!(_)
                    }
                });
                let row = self.pattern(name);
                Ok(quote! {
                    ::core::iter::Iterator::map(#input, move |(#(#binders,)*)| #row)
                })
            }
            Operator::Probe(_, keyed) => self.probe(name, keyed),
            Operator::Semi(_, keyed) => self.semi(keyed),
            Operator::Merge(_, keyed) => self.merge(name, &definition.name, keyed),
        }
    }

    /// For every outer row, enter the right side at the join key and append
    /// the payload of every match.
    pub(super) fn probe(
        &mut self,
        name: &str,
        keyed: &physical::Keyed,
    ) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let outer = self.scan(&keyed.left)?;
        let left = symbol_name(&keyed.left);
        let right = symbol_name(&keyed.right);
        let outer_pattern = self.pattern(&left);
        let access = self.keyed(&keyed.right, &columns)?;
        let (lookup, payload, host): (TokenStream, Vec<String>, bool) = match &access {
            Access::Host(access) => (
                self.host_lookup(&left, access, false),
                access.payload.clone(),
                false,
            ),
            Access::Prefix(structure, levels) => {
                let all = self.levels_of(&right, structure);
                // The opt-in binary case has exactly one remaining key,
                // which is the entire payload of this prefix read.
                if !self.endpoints.intersect
                    || levels.len() != 1
                    || all.len() != 2
                    || all[1].len() != 1
                    || all.concat().len() != self.heading(&right).len()
                {
                    return error(
                        &keyed.right,
                        "a probe needs the payload; it reads a curried structure only at its full depth",
                    );
                }
                let owned = self.owned_key(&right, &levels[0]);
                let key = levels[0].iter().map(|c| self.key_field(&left, c, owned));
                (
                    quote!(::cq_storage::Candidates::keys(&#structure, &(#(#key,)*))),
                    all[1].clone(),
                    false,
                )
            }
            Access::Built(structure) => {
                let lookup = self.chained_lookup(&left, structure, &right, &columns);
                (
                    lookup,
                    self.built_payload(&right, &self.levels_of(&right, structure)),
                    false,
                )
            }
        };
        // Payload fields are read by position: a host payload is a borrowed
        // tuple, a built payload a value; field access copies the `&T`
        // column out of either.
        let row = self.row(name, |c| {
            match payload.iter().position(|p| p == c && !columns.contains(p)) {
                Some(i) => {
                    let i = index(i);
                    if host {
                        quote!(&__cq_v.#i)
                    } else {
                        quote!(__cq_v.#i)
                    }
                }
                None => self.ident(&left, c).to_token_stream(),
            }
        });
        Ok(quote! {
            ::core::iter::Iterator::flat_map(#outer, move |#outer_pattern| {
                ::core::iter::Iterator::map(#lookup, move |__cq_v| #row)
            })
        })
    }

    /// Keep the rows whose key the right side contains.
    pub(super) fn semi(&mut self, keyed: &physical::Keyed) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let input = self.scan(&keyed.left)?;
        let left = symbol_name(&keyed.left);
        let binders = self.heading(&left).iter().map(|c| {
            if columns.contains(c) {
                self.ident(&left, c).to_token_stream()
            } else {
                quote!(_)
            }
        });
        let test = match self.keyed(&keyed.right, &columns)? {
            Access::Host(access) => self.host_lookup(&left, &access, true),
            Access::Built(structure) => {
                let levels = self.levels_of(&symbol_name(&keyed.right), &structure);
                self.chained_contains(&left, &structure, &symbol_name(&keyed.right), &levels)
            }
            Access::Prefix(structure, levels) => {
                self.chained_contains(&left, &structure, &symbol_name(&keyed.right), &levels)
            }
        };
        Ok(quote! {
            ::core::iter::Iterator::filter(#input, move |&(#(#binders,)*)| #test)
        })
    }

    /// Co-iterate both sides on the sorted key columns.
    pub(super) fn merge(
        &mut self,
        name: &str,
        reader: &Ident,
        keyed: &physical::Keyed,
    ) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let left = self.ordered(reader, &keyed.left, &columns)?;
        let right = self.ordered(reader, &keyed.right, &columns)?;
        if [&keyed.left, &keyed.right].iter().any(|result| {
            let result = symbol_name(result);
            !self.analysis.host_read(&result) && self.owned_key(&result, &columns)
        }) {
            return Ok(self.merge_owned_keys(name, keyed, &columns, &left, &right));
        }
        let native = matches!(
            (&left, &right),
            (Side::Host(_), Side::Host(_)) | (Side::Built(_), Side::Built(_))
        );
        let mut fields: BTreeMap<String, TokenStream> = BTreeMap::new();
        let mut cursors = Vec::new();
        for (side, result, item) in [
            (&left, &keyed.left, format_ident!("__cq_l")),
            (&right, &keyed.right, format_ident!("__cq_r")),
        ] {
            let result = symbol_name(result);
            let payload: Vec<String> = match side {
                Side::Host(access) => access.payload.clone(),
                Side::Built(_) | Side::Pipe(_) => self
                    .heading(&result)
                    .iter()
                    .filter(|c| !columns.contains(c))
                    .cloned()
                    .collect(),
            };
            for (i, column) in payload.iter().enumerate() {
                let i = index(i);
                let field = match side {
                    Side::Host(_) => quote!(&#item.#i),
                    Side::Built(_) | Side::Pipe(_) => quote!(#item.#i),
                };
                fields.insert(column.clone(), field);
            }
            let cursor = match side {
                Side::Host(access) => {
                    let value = &access.value;
                    let cursor = quote!(::cq_storage::OrderedRead::ordered(&#value));
                    if native {
                        cursor
                    } else {
                        let key = (0..columns.len()).map(index).map(|i| quote!(&__cq_k.#i));
                        quote! {
                            ::cq_storage::sorted_by(::core::iter::Iterator::map(
                                #cursor,
                                |(__cq_k, __cq_v)| ((#(#key,)*), __cq_v)
                            ))
                        }
                    }
                }
                Side::Built(structure) => {
                    let cursor = quote!(::cq_storage::OrderedRead::ordered(&#structure));
                    if native {
                        cursor
                    } else {
                        let key = (0..columns.len()).map(index).map(|i| quote!(__cq_k.#i));
                        quote! {
                            ::cq_storage::sorted_by(::core::iter::Iterator::map(
                                #cursor,
                                |(__cq_k, __cq_v)| ((#(#key,)*), __cq_v)
                            ))
                        }
                    }
                }
                Side::Pipe(rows) => {
                    let pattern = self.pattern(&result);
                    let key = columns.iter().map(|c| self.ident(&result, c));
                    let rest = payload.iter().map(|c| self.ident(&result, c));
                    quote! {
                        ::cq_storage::sorted_by(::core::iter::Iterator::map(
                            #rows,
                            move |#pattern| ((#(#key,)*), (#(#rest,)*))
                        ))
                    }
                }
            };
            cursors.push(cursor);
        }
        for (i, column) in columns.iter().enumerate() {
            let i = index(i);
            let field = match (&left, native) {
                (Side::Host(_), true) => quote!(&__cq_k.#i),
                _ => quote!(__cq_k.#i),
            };
            fields.insert(column.clone(), field);
        }
        let row = self.row(name, |c| fields[c].clone());
        let (left, right) = (&cursors[0], &cursors[1]);
        Ok(quote! {
            ::core::iter::Iterator::map(
                ::cq_storage::merge(#left, #right),
                move |(__cq_k, __cq_l, __cq_r)| #row
            )
        })
    }

    /// Computed bindings and borrowed host columns compare through owned
    /// keys, while each cursor carries its original row separately.
    fn merge_owned_keys(
        &self,
        name: &str,
        keyed: &physical::Keyed,
        columns: &[String],
        left: &Side,
        right: &Side,
    ) -> TokenStream {
        let mut cursors = Vec::new();
        for (side, result) in [(left, &keyed.left), (right, &keyed.right)] {
            let result = symbol_name(result);
            let pattern = self.pattern(&result);
            let key = columns.iter().map(|c| self.key_field(&result, c, true));
            let pairs = match side {
                Side::Pipe(rows) => {
                    quote!(::core::iter::Iterator::map(#rows, move |#pattern| ((#(#key,)*), #pattern)))
                }
                Side::Host(access) => {
                    let value = &access.value;
                    let row = self.row(&result, |c| {
                        if let Some(i) = columns.iter().position(|k| k == c) {
                            let i = index(i);
                            quote!(&__cq_k.#i)
                        } else {
                            let i = index(access.payload.iter().position(|p| p == c).unwrap());
                            quote!(&__cq_v.#i)
                        }
                    });
                    quote!(::core::iter::Iterator::map(::cq_storage::OrderedRead::ordered(&#value), |(__cq_k, __cq_v)| {
                        let #pattern = #row; ((#(#key,)*), #pattern)
                    }))
                }
                Side::Built(structure) => {
                    let payload = if self.analysis.host_read(&result) {
                        self.heading(&result)
                            .iter()
                            .filter(|c| !columns.contains(c))
                            .cloned()
                            .collect()
                    } else {
                        self.built_payload(&result, &self.levels_of(&result, structure))
                    };
                    let row = self.row(&result, |c| {
                        if let Some(i) = payload.iter().position(|p| p == c) {
                            let i = index(i);
                            quote!(__cq_v.#i)
                        } else {
                            let i = index(columns.iter().position(|k| k == c).unwrap());
                            quote!(__cq_k.#i)
                        }
                    });
                    quote!(::core::iter::Iterator::map(::cq_storage::OrderedRead::ordered(&#structure), |(__cq_k, __cq_v)| {
                        let #pattern = #row; ((#(#key,)*), #pattern)
                    }))
                }
            };
            cursors.push(quote!(::cq_storage::sorted_by(#pairs)));
        }
        let left_name = symbol_name(&keyed.left);
        let right_name = symbol_name(&keyed.right);
        let row = self.row(name, |c| {
            if let Some(i) = self.heading(&left_name).iter().position(|p| p == c) {
                let i = index(i);
                quote!(__cq_l.#i)
            } else {
                let i = index(
                    self.heading(&right_name)
                        .iter()
                        .position(|p| p == c)
                        .unwrap(),
                );
                quote!(__cq_r.#i)
            }
        });
        let (left, right) = (&cursors[0], &cursors[1]);
        quote!(::core::iter::Iterator::map(::cq_storage::merge(#left, #right), move |(_, __cq_l, __cq_r)| #row))
    }

    /// Enter an ordered value at the select's lower bound on `column` and
    /// leave it at the upper bound; the other conjuncts remain a filter.
    pub(super) fn range(
        &mut self,
        name: &str,
        reader: &Ident,
        column: &Ident,
        select: &physical::Ranged,
    ) -> syn::Result<TokenStream> {
        let input = symbol_name(&select.input);
        let column_name = symbol_name(column);
        let window = window(&select.condition, &column_name);
        if window.low.is_none() && window.high.is_none() {
            return error(
                column,
                format!(
                    "`{reader}` ranges over `{column}`, which the condition does not compare with an integer literal"
                ),
            );
        }
        let rows = if self.analysis.host_read(&input) {
            let access = self.host_access(&input, std::slice::from_ref(&column_name));
            let handle = format_ident!("__cq_{}_host", input);
            let bound = |value: Option<i32>| {
                value.map_or_else(
                    || quote!(None),
                    |n| {
                        let n = proc_macro2::Literal::i32_unsuffixed(n);
                        quote!(Some(&(#n,)))
                    },
                )
            };
            let low = bound(window.low);
            let high = bound(window.high);
            let binders = access
                .key
                .iter()
                .chain(&access.payload)
                .map(|c| self.ident(&input, c));
            let row = self.pattern(name);
            quote!({
                use ::cq_storage::dispatch::Range as _;
                ::core::iter::Iterator::flat_map(::core::iter::once(()), move |()| {
                    ::core::iter::Iterator::map((&&#handle).range(#low, #high), move |(#(#binders,)*)| #row)
                })
            })
        } else {
            let Side::Built(structure) =
                self.ordered(reader, &select.input, std::slice::from_ref(&column_name))?
            else {
                return error(reader, "range needs a host rename or built let");
            };
            let seek = window.low.map(|low| {
                let low = proc_macro2::Literal::i32_unsuffixed(low);
                quote!(::cq_storage::OrderedCursor::seek(&mut __cq_cursor, &&(&#low,));)
            });
            let cursor = match window.high {
                Some(high) => {
                    let high = proc_macro2::Literal::i32_unsuffixed(high);
                    quote!(::core::iter::Iterator::take_while(__cq_cursor, |(__cq_k,_)| *__cq_k.0 <= #high))
                }
                None => quote!(__cq_cursor),
            };
            let payload: Vec<_> = self
                .heading(&input)
                .iter()
                .filter(|c| **c != column_name)
                .collect();
            let row = self.row(name, |c| {
                if c == column_name {
                    quote!(__cq_k.0)
                } else {
                    let i = index(payload.iter().position(|p| *p == c).unwrap());
                    quote!(__cq_v.#i)
                }
            });
            quote!({
                let mut __cq_cursor = ::cq_storage::OrderedRead::ordered(&#structure);
                #seek
                ::core::iter::Iterator::map(#cursor, |(__cq_k,__cq_v)| #row)
            })
        };
        if window.residual.is_empty() {
            return Ok(rows);
        }
        let pattern = self.borrowed_pattern(&input);
        let residual = window
            .residual
            .iter()
            .map(deref_predicates)
            .reduce(|left, right| syn::parse_quote!((#left) && (#right)))
            .expect("nonempty residual");
        Ok(quote! {
            ::core::iter::Iterator::filter(
                #rows,
                move |#[allow(unused_variables)] #pattern| #residual
            )
        })
    }
}
