//! Printing checked S1 scan decisions. Logical rows are never reconstructed.
use super::*;
use structures::blocks::{Prune, Strategy, Test};
use structures::syntax::constraint::{Comparison, Constraint, Term};

impl Context<'_> {
    pub(crate) fn prepare_blocked_sources(&mut self) {
        for scan in self.encoded.blocks.scans.values() {
            let source = &self.encoded.obligations.sources[&scan.source];
            let binding = &self.encoded.blocks.sources[&scan.source];
            let host = Self::positional_host(&source.relation);
            for column in binding.columns.values() {
                let slot = column.slot as usize;
                let value_type = &self.hosts[&source.relation].types[slot];
                let declaration = &self.encoded.obligations.declarations[&column.declaration];
                let family = match declaration.family {
                    structures::encoded::Family::Plain => quote!(::cq_storage::scan::Plain),
                    structures::encoded::Family::Bytes => quote!(::cq_storage::scan::Bytes),
                    structures::encoded::Family::IndexByteAddressable => unreachable!("checked S1 source"),
                };
                let width = match declaration.carrier {
                    structures::encoded::Carrier::U8 => quote!(::cq_storage::scan::Width::W8),
                    structures::encoded::Carrier::U16 => quote!(::cq_storage::scan::Width::W16),
                    structures::encoded::Carrier::U32 => quote!(::cq_storage::scan::Width::W32),
                    structures::encoded::Carrier::U64 => quote!(::cq_storage::scan::Width::W64),
                };
                let (namespace, version) = structures::blocks::runtime_domain(declaration, column);
                let block_rows = binding.block_rows as usize;
                let summary = binding.summary == structures::blocks::Summary::MinMax;
                self.statements.push(syn::parse_quote! {
                    {
                        let __cq_column: &::cq_storage::scan::ScanColumn<#value_type, #family> = #host.column::<#slot>();
                        ::core::assert_eq!(__cq_column.block_rows().get(), #block_rows, "blocked source geometry");
                        ::core::assert_eq!(__cq_column.declared_domain(), Some(::cq_storage::scan::format::Domain {
                            id: ::cq_storage::scan::format::DomainId { namespace: #namespace, version: #version },
                            carrier: #width, equality: true, order: true,
                        }), "blocked source domain");
                        if #summary { ::core::assert!(__cq_column.summaries().is_some(), "blocked source summary capability"); }
                    }
                });
            }
        }
    }
    pub fn blocked_rows(&self, result: &str, range: Option<TokenStream>) -> TokenStream {
        let scan = &self.encoded.blocks.scans[result];
        let source = &self.encoded.obligations.sources[&scan.source];
        let host = Self::positional_host(&source.relation);
        let range = range.unwrap_or_else(|| quote!(0..#host.len()));
        let run = self.blocked_runs.get(result).map_or(&[][..], Vec::as_slice);
        let (preparation, configure) = self.blocked_run(run);
        self.positional_rows(result, quote!({
            #(#preparation)*
            #host.scan_positions(#range, move |__cq_tile| { #(#configure)* })
        }))
    }

    fn blocked_slot(&self, representation: structures::encoded::RepresentationId) -> usize {
        let representation = &self.encoded.records.representations[&representation];
        self.encoded.blocks.sources[&representation.source].columns[&representation.column].slot as usize
    }

    /// Consume PartitionK's checked global-code key read. The value is used
    /// once by routing; it is neither a logical reconstruction nor a row.
    pub fn blocked_route_key(&self, result: &str, column: &str) -> Option<TokenStream> {
        let representation = *self.blocked_routing.get(result)?.get(column)?;
        let field = &self.encoded.records.representations[&representation];
        let source = &self.encoded.obligations.sources[&field.source];
        let host = Self::positional_host(&source.relation);
        let structures::Repr::Positions { bases, .. } = &self.analysis.repr[result] else { unreachable!("checked routing position carrier") };
        let base = bases.iter().position(|base| symbol_name(&base.base) == source.relation).expect("checked routing source");
        let position = self.position_ident(result, base);
        let slot = self.blocked_slot(representation);
        Some(quote!(#host.read_ordinal::<#slot>(#position)))
    }

    fn blocked_run(&self, names: &[String]) -> (Vec<TokenStream>, Vec<TokenStream>) {
        let mut preparation = Vec::new();
        let mut configure = Vec::new();
        for name in names {
            let (bindings, operation) = self.blocked_predicate(name);
            preparation.extend(bindings);
            configure.push(operation);
        }
        (preparation, configure)
    }

    /// Canonical identity is the complete logical key, never source position.
    /// Cache only demanded global ordinal keys once, retain owned computed
    /// fields in their original carrier, and return one representative carrier.
    pub fn blocked_canon_rows(&self, result: &str, rows: TokenStream, parallel: bool) -> Option<TokenStream> {
        let read = self.encoded.blocks.canonical_keys.get(result)?;
        let pattern = self.bind_columns(result, self.heading(result), true);
        let keys: Vec<_> = read.fields.iter().map(|(column, field)| {
            let representation = &self.encoded.records.representations[&field.representation];
            let source = &self.encoded.obligations.sources[&representation.source];
            let host = Self::positional_host(&source.relation);
            let structures::Repr::Positions { bases, .. } = &self.analysis.repr[result] else { unreachable!("checked canonical position carrier") };
            let base = bases.iter().position(|base| symbol_name(&base.base) == source.relation).expect("checked canonical occurrence");
            let position = self.position_ident(result, base);
            let slot = self.blocked_slot(field.representation);
            (column.clone(), quote!(#host.read_ordinal::<#slot>(#position)))
        }).collect();
        let ordinals = keys.iter().map(|(_, value)| value);
        let key = |binding: TokenStream| {
            let values: Vec<_> = self.heading(result).iter().map(|column| {
                if let Some(index) = keys.iter().position(|(name, _)| name == column) {
                    let index = crate::index(index);
                    quote!(&#binding.#index)
                } else {
                    self.access(result, column)
                }
            }).collect();
            crate::group(&values)
        };
        let left = key(quote!(__cq_left_keys));
        let right = key(quote!(__cq_right_keys));
        let compare = quote!(|(__cq_left_keys, __cq_left_row), (__cq_right_keys, __cq_right_row)| {
            let __cq_left = { let #pattern = __cq_left_row; #left };
            let __cq_right = { let #pattern = __cq_right_row; #right };
            ::cq_storage::require::leibniz(&__cq_left);
            ::core::cmp::Ord::cmp(&__cq_left, &__cq_right)
        });
        let provider = match self.analysis.canonical.get(result) {
            Some(structures::CanonSource::Rows(provider)) => &provider.ty,
            None => &self.analysis.head_dedup[result].ty,
            Some(_) => unreachable!("checked blocked comparator rows"),
        };
        let keyed = quote!(::core::iter::Iterator::map(#rows, move |__cq_row| {
            let __cq_keys = { let #pattern = &__cq_row; (#(#ordinals,)*) };
            (__cq_keys, __cq_row)
        }));
        let sorted = if parallel { quote!(<#provider>::par_sort(#keyed, #compare)) } else { quote!(<#provider>::sort(#keyed, #compare)) };
        Some(quote!(::core::iter::Iterator::map(#sorted, |(_, __cq_row)| __cq_row)))
    }

    /// Emit the selected field action, including injective value-code witnesses
    /// at an external consumer. Logical reads remain owned scoped values.
    pub fn blocked_read(&self, consumer: &str, result: &str, column: &str) -> Option<TokenStream> {
        let reads = &self.encoded.blocks.reads;
        let read = reads.get(consumer).or_else(|| {
            ["carrier", "read"].into_iter().find_map(|role| {
                reads.get(&format!("output:{consumer}:{role}"))
                    .filter(|read| read.fields.contains_key(column))
            })
        })?;
        let field = read.fields.get(column)?;
        let representation = &self.encoded.records.representations[&field.representation];
        let source = &self.encoded.obligations.sources[&representation.source];
        let host = Self::positional_host(&source.relation);
        let structures::Repr::Positions { bases, .. } = &self.analysis.repr[result] else { unreachable!("checked blocked read carrier") };
        let base = bases.iter().position(|base| symbol_name(&base.base) == source.relation).expect("checked retained blocked occurrence");
        let position = self.position_ident(result, base);
        let slot = self.blocked_slot(field.representation);
        Some(match field.action {
            structures::blocks::ReadAction::Ordinal => quote!(#host.read_ordinal::<#slot>(#position)),
            structures::blocks::ReadAction::Owned | structures::blocks::ReadAction::BorrowedScoped => match (&read.consumer, read.purpose) {
                (structures::encoded::Consumer::OpaqueRust(consumer), structures::encoded::ReadPurpose::OpaqueRustRead) => {
                    // DECODE R: selected logical field is read by the recorded local or external opaque Rust consumer.
                    quote!(#host.read_value::<#slot>(#position, ::cq_storage::scan::LogicalRead::OpaqueRust { consumer: #consumer }))
                }
                (structures::encoded::Consumer::FinalAnswer(consumer), structures::encoded::ReadPurpose::FinalHostAnswer) => {
                    // DECODE P: selected logical field is published to the genuine final-answer consumer.
                    quote!(#host.read_value::<#slot>(#position, ::cq_storage::scan::LogicalRead::FinalAnswer { consumer: #consumer }))
                }
                _ => unreachable!("checked logical read authorization"),
            },
        })
    }

    pub fn blocked_rust_aliases(&self, consumer: &str, result: &str, expression: &Expr, environment: Option<&structures::syntax::algebra::Environment>) -> Vec<TokenStream> {
        columns_used_rust(expression, self.heading(result), environment).iter().map(|column| {
            let logical = &self.infos[result].bindings[column];
            if let Some(value) = self.blocked_read(consumer, result, column) {
                let owned = format_ident!("__cq_read_{}_{}", consumer, column, span = proc_macro2::Span::mixed_site());
                quote!(let #owned = #value; #[allow(unused_variables)] let #logical = &#owned;)
            } else {
                let value = self.access(result, column);
                if self.infos[result].computed.contains(column) {
                    quote!(#[allow(unused_variables)] let #logical = &#value;)
                } else {
                    quote!(#[allow(unused_variables)] let #logical = #value;)
                }
            }
        }).collect()
    }

    /// The filter retains the entire positional carrier, including owned Bind
    /// outputs. Its bounded mask caches only the current immutable source tile.
    pub fn blocked_filter_rows(&self, name: &str, rows: TokenStream) -> TokenStream {
        let predicate = &self.encoded.blocks.predicates[name];
        let source = &self.encoded.obligations.sources[&predicate.source];
        let host = Self::positional_host(&source.relation);
        let input = &predicate.input;
        if self.blocked_runs.values().any(|run| run.iter().any(|predicate| predicate == name))
            && !self.blocked_runs.contains_key(name)
        {
            return self.forward(name, input, rows);
        }
        let pattern = self.bind_columns(input, &[], true);
        let structures::Repr::Positions { bases, .. } = &self.analysis.repr[input] else { unreachable!("checked blocked predicate carrier") };
        let base = bases.iter().position(|base| symbol_name(&base.base) == source.relation).expect("checked predicate occurrence");
        let position = self.position_ident(input, base);
        let (preparation, configure) = self.blocked_run(&self.blocked_runs[name]);
        let filtered = quote!({ #(#preparation)* #host.filter_rows(#rows, move |#pattern| #position, move |__cq_tile| { #(#configure)* }) });
        self.forward(name, input, filtered)
    }

    fn blocked_predicate(&self, name: &str) -> (Vec<TokenStream>, TokenStream) {
        let predicate = &self.encoded.blocks.predicates[name];
        let source = &self.encoded.obligations.sources[&predicate.source];
        let host = Self::positional_host(&source.relation);
        let (Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select)) = &self.physical[name].operator else { unreachable!("checked blocked predicate") };
        let prune = match predicate.prune { Prune::None => quote!(::cq_storage::scan::PruneMode::None), Prune::MinMax => quote!(::cq_storage::scan::PruneMode::MinMax) };
        let test = match predicate.test { Test::Encoded => quote!(::cq_storage::scan::TestMode::Encoded), Test::DecodeCodes => quote!(::cq_storage::scan::TestMode::DecodeToCode) };
        let mut preparation = Vec::new();
        let configure = match predicate.strategy {
            Strategy::Window | Strategy::Point => {
                let column = predicate.columns.keys().next().expect("checked unary strategy");
                let slot = self.blocked_slot(predicate.columns[column]);
                let encode = |term: &Term| { let value = expressions::term_reference(term, &self.parameters); quote!(#host.encode_operand::<#slot>(#value)) };
                let value = match predicate.strategy {
                    Strategy::Window => {
                        let (_, bounds) = select.condition.range_bounds().expect("checked Window strategy");
                        let bound = |lower| {
                            let mut result = quote!(::core::ops::Bound::Unbounded);
                            for bound in bounds.iter().filter(|bound| bound.lower == lower) {
                                let value = encode(&bound.value);
                                let next = if bound.inclusive { quote!(::core::ops::Bound::Included(#value)) } else { quote!(::core::ops::Bound::Excluded(#value)) };
                                result = if lower { quote!(::cq_storage::intersect_lower(#result, #next)) } else { quote!(::cq_storage::intersect_upper(#result, #next)) };
                            }
                            result
                        };
                        let lo = bound(true); let hi = bound(false);
                        quote!(::cq_storage::scan::Predicate::Window { lo: #lo, hi: #hi })
                    }
                    Strategy::Point => {
                        let constants = structures::selection_constants(select);
                        let value = encode(&constants[column]);
                        quote!(::cq_storage::scan::Predicate::Point(#value))
                    }
                    Strategy::Formula => unreachable!(),
                };
                let id = format_ident!("__cq_block_predicate_{}", name);
                preparation.push(quote!(let #id = #value;));
                quote!(__cq_tile.intersect::<#slot>(&#id, #prune, #test);)
            }
            Strategy::Formula => {
                let code = |column: &str| format_ident!("__cq_block_code_{}_{}", name, column);
                let reads: Vec<_> = predicate.columns.iter().map(|(column, representation)| {
                    let id = code(column);
                    let slot = self.blocked_slot(*representation);
                    quote!(let #id = #host.read_ordinal::<#slot>(__cq_position);)
                }).collect();
                let mut compare = |left: &Term, comparison: Comparison, right: &Term| {
                    let column = match (left, right) { (Term::Variable(column), _) | (_, Term::Variable(column)) => symbol_name(column), _ => unreachable!("checked column predicate") };
                    let slot = self.blocked_slot(predicate.columns[&column]);
                    let mut operand = |term: &Term| {
                        if let Term::Variable(column) = term {
                            let id = code(&symbol_name(column));
                            quote!(#id)
                        } else {
                            let value = expressions::term_reference(term, &self.parameters);
                            let id = format_ident!("__cq_block_operand_{}_{}", name, preparation.len());
                            preparation.push(quote!(let #id = #host.encode_operand::<#slot>(#value);));
                            quote!(#id)
                        }
                    };
                    let left = operand(left); let right = operand(right);
                    comparison.expression(quote!(&(#left)), quote!(&(#right)))
                };
                let mut tests = Vec::new();
                for equality in &select.condition.equalities {
                    let right = match &equality.right { structures::syntax::EqualityValue::Variable(v) => Term::Variable(v.clone()), structures::syntax::EqualityValue::Literal(v) => Term::Literal(Box::new(v.clone())) };
                    tests.push(compare(&Term::Variable(equality.left.clone()), Comparison::Eq(Default::default()), &right));
                }
                for constraint in &select.condition.constraints { tests.push(block_constraint(constraint, &mut compare)); }
                quote!(__cq_tile.retain(|__cq_position| { #(#reads)* true #(&& (#tests))* });)
            }
        };
        (preparation, configure)
    }
}

fn block_constraint(constraint: &Constraint, compare: &mut impl FnMut(&Term, Comparison, &Term) -> TokenStream) -> TokenStream {
    match constraint {
        Constraint::Compare { left, comparison, right } => compare(left, *comparison, right),
        Constraint::Between { term, low, high } => { let low = compare(term, Comparison::Ge(Default::default()), low); let high = compare(term, Comparison::Le(Default::default()), high); quote!((#low) && (#high)) }
        Constraint::In { term, literals } => { let tests: Vec<_> = literals.iter().map(|literal| compare(term, Comparison::Eq(Default::default()), &Term::Literal(Box::new(literal.clone())))).collect(); quote!(false #(|| (#tests))*) }
        Constraint::Or(arms) => { let arms: Vec<_> = arms.iter().map(|arm| { let tests: Vec<_> = arm.iter().map(|c| block_constraint(c, compare)).collect(); quote!(true #(&& (#tests))*) }).collect(); quote!(false #(|| (#arms))*) }
    }
}
