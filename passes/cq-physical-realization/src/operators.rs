//! One iterator adapter per physical operator.

use super::*;

impl<'a> Printer<'a> {
    fn arm_row(&self, name: &str, operand: &str) -> TokenStream {
        self.carrier(name, |column| {
            let value = self.access(operand, column);
            if self.infos[name].computed.contains(column)
                && !self.infos[operand].computed.contains(column)
            {
                quote!(::core::clone::Clone::clone(#value))
            } else {
                value.to_token_stream()
            }
        })
    }
    fn union(&mut self, name: &str, union: &physical::Union) -> syn::Result<TokenStream> {
        if union.fused.is_some() {
            return self.fused_union(name, union);
        }
        let mut chain = None;
        for operand in &union.operands {
            let rows = self.scan(operand)?;
            let operand = symbol_name(operand);
            let pattern = self.bind_columns(&operand, self.carried(name), false);
            let row = self.arm_row(name, &operand);
            let rows = quote!(::core::iter::Iterator::map(#rows, move |#pattern| #row));
            chain = Some(match chain {
                None => rows,
                Some(previous) => quote!(::core::iter::Iterator::chain(#previous,#rows)),
            });
        }
        let rows = chain.expect("contracted union");
        // A hoisted relation over F must retain all distinct F tuples. The
        // witness stop applies to a unit heading, or inside a correlated arm.
        Ok(
            if matches!(union.witness, Some(physical::Witness::Any(_)))
                && self.heading(name).is_empty()
            {
                quote!(::core::iter::Iterator::take(#rows,1))
            } else {
                rows
            },
        )
    }
    fn define_arm(
        &mut self,
        result: &str,
        group: &physical::Fused,
        defined: &mut std::collections::BTreeSet<String>,
    ) -> syn::Result<()> {
        if !group.members.contains(result) || !defined.insert(result.to_owned()) {
            return Ok(());
        }
        let definition = self.physical[result];
        if !matches!(&definition.operator,Operator::Union(u) if u.fused.is_some()) {
            for operand in definition.operator.operands() {
                self.define_arm(&symbol_name(operand), group, defined)?;
            }
        } else {
            let Operator::Union(union) = &definition.operator else {
                unreachable!()
            };
            let prefix = &union
                .origin
                .as_ref()
                .expect("fused origin")
                .over
                .as_ref()
                .expect("fused prefix")
                .prefix;
            self.define_arm(&symbol_name(prefix), group, defined)?;
        }
        let ids = self.idents.get(result).cloned();
        self.define(definition)?;
        if let Some(ids) = ids {
            self.idents.insert(result.to_owned(), ids);
        }
        Ok(())
    }
    fn fused_union(&mut self, name: &str, union: &physical::Union) -> syn::Result<TokenStream> {
        let group = self.physical_analysis.fused[name].clone();
        let prefix_id = union
            .origin
            .as_ref()
            .expect("fused origin")
            .over
            .as_ref()
            .expect("fused prefix")
            .prefix
            .clone();
        let outer = self.scan(&prefix_id)?;
        let prefix_ids = self.idents[&group.prefix].clone();
        let free: std::collections::BTreeSet<_> = union
            .origin
            .as_ref()
            .expect("fused origin")
            .free
            .iter()
            .map(symbol_name)
            .collect();
        let prefix_columns = self.heading(&group.prefix).to_vec();
        for column in &prefix_columns {
            if !free.contains(column) {
                self.idents.get_mut(&group.prefix).unwrap().insert(
                    column.clone(),
                    format_ident!(
                        "__cq_prefix_{}_{}",
                        name,
                        column,
                        span = proc_macro2::Span::mixed_site()
                    ),
                );
            }
        }
        let pattern = self.bind_columns(&group.prefix, self.carried(&group.prefix), false);
        // Arm rows carry the hidden prefix names; only their source Rust scopes
        // introduce lexical aliases when those names are actually in scope.
        let hidden = self.idents[&group.prefix].clone();
        let mut saved = BTreeMap::new();
        for member in &group.members {
            self.register(member);
            saved.insert(member.clone(), self.idents[member].clone());
            for column in &prefix_columns {
                if self.idents[member].contains_key(column) && !free.contains(column) {
                    self.idents
                        .get_mut(member)
                        .unwrap()
                        .insert(column.clone(), hidden[column].clone());
                }
            }
        }
        let previous = self
            .active_prefixes
            .insert(group.prefix.clone(), pattern.clone());
        let mut defined = std::collections::BTreeSet::new();
        let mut arms = Vec::new();
        let bound = self.bound;
        for operand in &union.operands {
            let start = self.statements.len();
            let operand_name = symbol_name(operand);
            self.define_arm(&operand_name, &group, &mut defined)?;
            let rows = self.scan(operand)?;
            let operand_pattern = self.bind_columns(&operand_name, self.carried(name), false);
            let row = self.arm_row(name, &operand_name);
            let rows = quote!(::core::iter::Iterator::map(#rows,move |#operand_pattern| #row));
            let rows = if matches!(union.witness, Some(physical::Witness::Any(_))) {
                quote!(::core::iter::Iterator::take(#rows,1))
            } else {
                rows
            };
            let statements = self.statements.split_off(start);
            arms.push((statements, rows));
        }
        self.bound = bound;
        self.idents.insert(group.prefix.clone(), prefix_ids);
        for (member, ids) in saved {
            self.idents.insert(member, ids);
        }
        if let Some(previous) = previous {
            self.active_prefixes.insert(group.prefix.clone(), previous);
        } else {
            self.active_prefixes.remove(&group.prefix);
        }
        let fallback = union.origin.as_ref().is_some_and(|o| o.fallback.is_some());
        let rows = if fallback {
            let mut next = quote!(::std::vec::Vec::new());
            for (statements, rows) in arms.into_iter().rev() {
                next = quote!({
                    #(#statements)*
                    let __cq_arm = ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows);
                    if __cq_arm.is_empty() { #next } else { __cq_arm }
                });
            }
            quote!(::core::iter::IntoIterator::into_iter(#next))
        } else {
            let mut chain = None;
            for (statements, rows) in arms {
                let value = if statements.is_empty() {
                    rows
                } else {
                    quote!(::core::iter::IntoIterator::into_iter(::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows)))
                };
                let arm = quote!(::core::iter::Iterator::flatten(::core::iter::once_with(move || {
                    #(#statements)*
                    #value
                })));
                chain = Some(match chain {
                    None => arm,
                    Some(previous) => quote!(::core::iter::Iterator::chain(#previous,#arm)),
                });
            }
            let chain = chain.expect("union arms");
            if matches!(union.witness, Some(physical::Witness::Any(_))) {
                quote!(::core::iter::Iterator::take(#chain,1))
            } else {
                chain
            }
        };
        Ok(quote!(::core::iter::Iterator::flat_map(#outer,move |#pattern| { #rows })))
    }
    /// The rows of one physical operator, in the result's heading order.
    /// Formal rule P-For: Fan out generator Bind through its finite iterator, retaining prefix values.
    pub(super) fn produce(
        &mut self,
        name: &str,
        definition: &physical::Definition,
    ) -> syn::Result<TokenStream> {
        if let Some(rows) = self.staged_rows(name) {
            return Ok(rows);
        }
        if self.analysis.reads.contains_key(name) {
            let ids = self.read_ids(name)?;
            return Ok(self.buffer_stage(name, self.positional_rows(name, ids)));
        }
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) =
            &definition.operator
        {
            let input_name = symbol_name(&select.input);
            if self.carries_positions(&input_name) {
                let rows = self.scan(&select.input)?;
                let columns = cq_print::columns_used(&select.condition)
                    .into_iter()
                    .collect::<Vec<_>>();
                let pattern = self.bind_columns(&input_name, &columns, true);
                let aliases = self.position_aliases(&input_name, &columns);
                let condition = self.relational_condition(&select.condition);
                let rows = quote!(::core::iter::Iterator::filter(#rows, move |#pattern| {
                    #(#aliases)*
                    ::cq_storage::residual(#condition)
                }));
                return Ok(self.forward(name, &input_name, rows));
            }
        }
        if let Operator::Guard(guard) = &definition.operator {
            let input_name = symbol_name(&guard.input);
            if self.carries_positions(&input_name) {
                let rows = self.scan(&guard.input)?;
                let pattern = self.rust_borrowed_pattern(
                    &input_name,
                    &guard.condition,
                    guard.environment.as_ref(),
                );
                let aliases =
                    self.rust_aliases(&input_name, &guard.condition, guard.environment.as_ref());
                let condition = &guard.condition;
                let rows = quote!(::core::iter::Iterator::filter(#rows, move |#pattern| {
                    #(#aliases)*
                    ::cq_storage::residual(#condition)
                }));
                return Ok(self.forward(name, &input_name, rows));
            }
        }
        let rows = match &definition.operator {
            Operator::Union(union) => self.union(name, union),
            Operator::Anti(_, keyed) => self.membership(keyed, true),
            Operator::Unit(_) => Ok(quote!(::core::iter::once(()))),
            Operator::Rename(_) => Ok(self.host_scan(name)),
            Operator::Select(select) => {
                let input = self.scan(&select.input)?;
                let columns = cq_print::columns_used(&select.condition)
                    .into_iter()
                    .collect::<Vec<_>>();
                let pattern = self.bind_columns(&symbol_name(&select.input), &columns, true);
                let condition = self.relational_condition(&select.condition);
                Ok(quote! {
                    ::core::iter::Iterator::filter(
                        #input,
                        move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition)
                    )
                })
            }
            Operator::Point(_, select) => self.point(name, select),
            Operator::Range(_, select) => {
                if let Some(rows) = self.host_range(name, select) {
                    return Ok(self.forward(name, &symbol_name(&select.input), rows));
                }
                let input = self.scan(&select.input)?;
                let columns = cq_print::columns_used(&select.condition)
                    .into_iter()
                    .collect::<Vec<_>>();
                let pattern = self.bind_columns(&symbol_name(&select.input), &columns, true);
                let condition = self.relational_condition(&select.condition);
                Ok(
                    quote!(::core::iter::Iterator::filter(#input, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition))),
                )
            }
            Operator::Guard(guard) => {
                let input = self.scan(&guard.input)?;
                let pattern = self.rust_borrowed_pattern(
                    &symbol_name(&guard.input),
                    &guard.condition,
                    guard.environment.as_ref(),
                );
                let condition = &guard.condition;
                Ok(quote! {
                    ::core::iter::Iterator::filter(#input, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition))
                })
            }
            Operator::Bind(bind) => {
                let input = self.scan(&bind.input)?;
                let input_name = symbol_name(&bind.input);
                let previous = self.idents[&input_name].clone();
                if let Some(environment) = &bind.environment {
                    let names: std::collections::BTreeSet<_> =
                        environment.variables.iter().map(symbol_name).collect();
                    for column in self.carried(&input_name).to_vec() {
                        if !names.contains(&column) {
                            self.idents.get_mut(&input_name).unwrap().insert(
                                column.clone(),
                                format_ident!(
                                    "__cq_bind_{}_{}",
                                    name,
                                    column,
                                    span = proc_macro2::Span::mixed_site()
                                ),
                            );
                        }
                    }
                }
                let mut used = cq_print::columns_used_rust(
                    &bind.expression,
                    self.heading(&input_name),
                    bind.environment.as_ref(),
                );
                used.extend(self.carried(name).iter().cloned());
                let pattern =
                    self.bind_columns(&input_name, &used.into_iter().collect::<Vec<_>>(), false);
                let shadows =
                    self.rust_aliases(&input_name, &bind.expression, bind.environment.as_ref());
                let binding = &bind.pattern;
                let annotation = &bind.annotation;
                let value = self
                    .generator_expressions
                    .get(name)
                    .unwrap_or(&bind.expression);
                // Fan out each prefix through its finite iterator,
                // moving generated values and retaining the prefix for subsequent items.
                let generator = bind.for_token.is_some();
                let row = self.carrier(name, |column| {
                    if self.heading(&input_name).contains(&column.to_owned()) {
                        let value = self.access(&input_name, column);
                        if generator && self.infos[&input_name].computed.contains(column) {
                            quote!(::core::clone::Clone::clone(&#value))
                        } else {
                            value
                        }
                    } else {
                        self.access(name, column)
                    }
                });
                let result = if generator {
                    let item = format_ident!(
                        "__cq_generated_item",
                        span = proc_macro2::Span::mixed_site()
                    );
                    // Prepared captures are shared references, including when
                    // this adapter is nested inside a fused arm's move closure.
                    quote! {
                        ::core::iter::Iterator::flat_map(#input, move |#pattern| {
                            ::core::iter::Iterator::map(
                                ::core::iter::IntoIterator::into_iter({ #(#shadows)* #value }),
                                move |#item| {
                                    #[allow(unused_variables)]
                                    let #binding #annotation = #item;
                                    #row
                                }
                            )
                        })
                    }
                } else {
                    quote! {
                        ::core::iter::Iterator::map(#input, move |#pattern| {
                            // A dead output still evaluates its potentially partial Rust expression.
                            #[allow(unused_variables)]
                            let #binding #annotation = { #(#shadows)* #value };
                            #row
                        })
                    }
                };
                self.idents.insert(input_name, previous);
                Ok(result)
            }
            Operator::Project(p) => {
                let project = &p.input;
                let input = self.scan(project)?;
                let input_name = symbol_name(project);
                let kept = self.carried(name);
                let pattern = self.bind_columns(&input_name, kept, false);
                let row = self.carrier(name, |c| self.access(&input_name, c));
                Ok(quote! {
                    ::core::iter::Iterator::map(#input, move |#pattern| #row)
                })
            }
            Operator::Probe(_, keyed) => self.probe(name, keyed),
            Operator::Semi(_, keyed) => self.semi(keyed),
            Operator::Merge(_, keyed) => self.merge(name, &definition.name, keyed),
        }?;
        let input = match &definition.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => Some(&s.input),
            Operator::Guard(g) => Some(&g.input),
            Operator::Semi(_, k) | Operator::Anti(_, k) => Some(&k.left),
            _ => None,
        };
        let rows = match input {
            Some(input) => self.forward(name, &symbol_name(input), rows),
            None => rows,
        };
        Ok(self.buffer_stage(name, rows))
    }

    /// Lookahead is a checked Structures decision, never a printer policy.
    fn lookahead(&self, name: &str) -> usize {
        self.analysis.stages.get(name).copied().unwrap_or(1)
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
                if !self.intersect
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
        let mut used = columns.clone();
        used.extend(
            self.carried(name)
                .iter()
                .filter(|c| !payload.contains(c) && self.constant_value(&right, c).is_none())
                .cloned(),
        );
        let outer_pattern = self.bind_columns(&left, &used, false);
        // Payload fields are read by position: a host payload is a borrowed
        // tuple, a built payload a value; field access copies the `&T`
        // column out of either.
        let row = self.carrier(name, |c| {
            match payload.iter().position(|p| p == c && !columns.contains(p)) {
                Some(i) => {
                    let i = index(i);
                    if host {
                        quote!(&__cq_v.#i)
                    } else {
                        quote!(__cq_v.#i)
                    }
                }
                None => self
                    .constant_value(&right, c)
                    .filter(|_| !columns.iter().any(|key| key == c))
                    .unwrap_or_else(|| self.access(&left, c)),
            }
        });
        // The checked probe is unchanged: batch only borrowed outer columns,
        // preserve outer/bucket order, and leave curried/custom scalar access
        // to the IndexRead default. No predicate moves across a Rust boundary.
        if let Access::Built(structure) = &access {
            let levels = self.levels_of(&right, structure);
            if levels.len() == 1
                && self.infos[&left].computed.is_empty()
                && !self
                    .analysis
                    .membership
                    .contains(&(right.clone(), columns.clone()))
                && self
                    .constant_key_condition(&left, &right, &columns)
                    .is_none()
            {
                let batch = self.lookahead(&left);
                let owned = self.owned_key(&right, &levels[0]);
                let stored_key = self.stored_key(&right, &levels[0]);
                let key = stored_key.iter().map(|c| self.key_field(&left, c, owned));
                if self.carries_positions(&left) {
                    let key_pattern = self.bind_columns(&left, &stored_key, true);
                    return Ok(quote! {
                        ::core::iter::Iterator::map(
                            ::cq_storage::IndexRead::probe::<#batch, _, _>(#structure, #outer, move |#key_pattern| (#(#key,)*)),
                            move |(#outer_pattern, __cq_v)| #row
                        )
                    });
                }
                let key_binders = self.carried(&left).iter().map(|c| {
                    if stored_key.contains(c) {
                        self.access(&left, c)
                    } else {
                        quote!(_)
                    }
                });
                let row_binders = self.carried(&left).iter().map(|c| {
                    if self.carried(name).contains(c)
                        && (columns.contains(c)
                            || (!payload.contains(c) && self.constant_value(&right, c).is_none()))
                    {
                        self.access(&left, c)
                    } else {
                        quote!(_)
                    }
                });
                return Ok(quote! {
                    ::core::iter::Iterator::map(
                        ::cq_storage::IndexRead::probe::<#batch, _, _>(#structure, #outer, move |&(#(#key_binders,)*)| (#(#key,)*)),
                        move |((#(#row_binders,)*), __cq_v)| #row
                    )
                });
            }
        }
        Ok(quote! {
            ::core::iter::Iterator::flat_map(#outer, move |#outer_pattern| {
                ::core::iter::Iterator::map(#lookup, move |__cq_v| #row)
            })
        })
    }

    /// Keep the rows whose key the right side contains.
    pub(super) fn semi(&mut self, keyed: &physical::Keyed) -> syn::Result<TokenStream> {
        self.membership(keyed, false)
    }
    fn membership(&mut self, keyed: &physical::Keyed, negative: bool) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let input = self.scan(&keyed.left)?;
        let left = symbol_name(&keyed.left);
        let pattern = self.bind_columns(&left, &columns, true);
        let test = if let Some(test) =
            self.keyset_contains(&left, &symbol_name(&keyed.right), &columns)
        {
            test
        } else {
            match self.keyed(&keyed.right, &columns)? {
                Access::Host(access) => self.host_lookup(&left, &access, true),
                Access::Built(structure) => {
                    let levels = self.levels_of(&symbol_name(&keyed.right), &structure);
                    self.chained_contains(&left, &structure, &symbol_name(&keyed.right), &levels)
                }
                Access::Prefix(structure, levels) => {
                    self.chained_contains(&left, &structure, &symbol_name(&keyed.right), &levels)
                }
            }
        };
        let test = if negative { quote!(!(#test)) } else { test };
        Ok(quote! {
            ::core::iter::Iterator::filter(#input, move |#pattern| ::cq_storage::residual(#test))
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
        let left = if self.active_prefixes.contains_key(&symbol_name(&keyed.left)) {
            Side::Pipe(self.scan(&keyed.left)?)
        } else {
            self.ordered(reader, &keyed.left, &columns)?
        };
        let right = if self
            .active_prefixes
            .contains_key(&symbol_name(&keyed.right))
        {
            Side::Pipe(self.scan(&keyed.right)?)
        } else {
            self.ordered(reader, &keyed.right, &columns)?
        };
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
                    .carried(&result)
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
                    let pattern = self.bind_columns(&result, self.carried(&result), false);
                    let key = columns.iter().map(|c| self.access(&result, c));
                    let rest = payload.iter().map(|c| self.access(&result, c));
                    quote! {
                        ::cq_storage::sorted_by(::core::iter::Iterator::map(
                            #rows,
                            move |#pattern| ((#(#key,)*), (#(#rest,)*))
                        ))
                    }
                }
            };
            let cursor = if matches!(side, Side::Host(_)) {
                self.profile_output(&result, cursor)
            } else {
                cursor
            };
            cursors.push(self.profile_input(name, cursor));
        }
        for (i, column) in columns.iter().enumerate() {
            let i = index(i);
            let field = match (&left, native) {
                (Side::Host(_), true) => quote!(&__cq_k.#i),
                _ => quote!(__cq_k.#i),
            };
            fields.insert(column.clone(), field);
        }
        let row = self.carrier(name, |c| fields[c].clone());
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
            let pattern = self.bind_columns(&result, self.carried(&result), false);
            let key = columns.iter().map(|c| self.key_field(&result, c, true));
            let pairs = match side {
                Side::Pipe(rows) => {
                    quote!(::core::iter::Iterator::map(#rows, move |#pattern| ((#(#key,)*), #pattern)))
                }
                Side::Host(access) => {
                    let value = &access.value;
                    let row = self.carrier(&result, |c| {
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
                    let row = self.carrier(&result, |c| {
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
            let pairs = if matches!(side, Side::Host(_)) {
                self.profile_output(&result, pairs)
            } else {
                pairs
            };
            let cursor = quote!(::cq_storage::sorted_by(#pairs));
            cursors.push(self.profile_input(name, cursor));
        }
        let left_name = symbol_name(&keyed.left);
        let right_name = symbol_name(&keyed.right);
        let row = self.carrier(name, |c| {
            if let Some(i) = self.carried(&left_name).iter().position(|p| p == c) {
                let i = index(i);
                quote!(__cq_l.#i)
            } else {
                let i = index(
                    self.carried(&right_name)
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

    /// A single compile-time key uses native prefix lookup or a scan/filter.
    /// Neither path creates a retained structure for this demand.
    fn point(&mut self, name: &str, select: &cq::algebra::Select) -> syn::Result<TokenStream> {
        let input_name = symbol_name(&select.input);
        let condition = self.relational_condition(&select.condition);
        let columns = cq_print::columns_used(&select.condition)
            .into_iter()
            .collect::<Vec<_>>();
        let pattern = self.bind_columns(&input_name, &columns, true);
        if !self.analysis.host_read(&input_name) {
            let rows = self.scan(&select.input)?;
            return Ok(
                quote!(::core::iter::Iterator::filter(#rows, move |#[allow(unused_variables)] #pattern| ::cq_storage::residual(#condition))),
            );
        }
        Ok(self
            .host_point(name, select)
            .expect("checked host point input"))
    }
}
