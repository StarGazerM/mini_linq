//! Shared emission of checked position-payload boundaries and exact residuals.
use super::*;
use structures::blocks::{Bridge, IndexKind, Join, Operand, Side};
use structures::syntax::constraint::{Comparison, Term};

impl Context<'_> {
    pub(crate) fn blocked_position_slot(&self, result: &str, occurrence: &str) -> usize {
        structures::blocks::position_slot(self.physical_definitions, self.analysis, result, occurrence)
            .expect("checked source occurrence position")
    }

    pub(crate) fn blocked_index(&self, result: &str, levels: &[Vec<String>]) -> Option<&Join> {
        self.encoded.blocks.joins.values().find(|join| join.right == result && join.binding.levels == levels)
    }

    /// Only the recorded forward bridge is used: no inverse codec or type inspection.
    pub fn blocked_operand(&self, result: &str, operand: &Operand) -> TokenStream {
        match operand {
            Operand::Blocked { representation, position } => {
                let field = &self.encoded.records.representations[representation];
                let source = &self.encoded.obligations.sources[&field.source];
                let host = Self::positional_host(&source.relation);
                let slot = self.blocked_slot(*representation);
                let position = self.position_ident(result, *position);
                quote!(#host.read_ordinal::<#slot>(#position))
            }
            Operand::Host { occurrence, position, slot, bridge: Bridge::OrderKey64, .. } => {
                let source = &self.physical_analysis.hosts[occurrence];
                let host = Self::positional_host(source);
                let position = self.position_ident(result, *position);
                let slot = crate::index(*slot);
                quote!({
                    let __cq_value = ::cq_storage::PositionalRows::row_at(#host, #position).expect("host-issued live ID").#slot;
                    let __cq_ordinal: u64 = ::cq_storage::OrderKey::ordinal(__cq_value);
                    __cq_ordinal
                })
            }
            Operand::Owned { column } => self.access(result, column),
        }
    }

    pub(crate) fn blocked_index_payload(&self, result: &str, join: &Join) -> TokenStream {
        let positions = join.payload.positions.iter().map(|occurrence| {
            self.position_ident(result, self.blocked_position_slot(result, occurrence))
        });
        let owned = join.payload.owned.iter().map(|column| self.access(result, column));
        quote!((#(#positions,)* #(#owned,)*))
    }

    pub(crate) fn blocked_index_key(&self, result: &str, join: &Join, columns: &[String]) -> TokenStream {
        let fields = columns.iter().map(|column| {
            let key = join.keys.iter().find(|key| &key.column == column).expect("checked index key");
            self.blocked_operand(result, &key.right)
        });
        quote!((#(#fields,)*))
    }

    /// Existing provider and build scope are retained; only key/payload adapters differ.
    pub fn blocked_probe_rows(&self, name: &str, rows: TokenStream) -> TokenStream {
        let join = &self.encoded.blocks.joins[name];
        let left = &join.left;
        let right = &join.right;
        let mut columns = self.carried(left).to_vec();
        columns.extend(join.owned.iter().filter(|column| self.heading(left).contains(column)).cloned());
        let pattern = self.bind_columns(left, &columns, false);
        let keys: Vec<_> = join.keys.iter().map(|key| {
            let id = format_ident!("__cq_join_key_{}_{}", name, key.column);
            let value = self.blocked_operand(left, &key.left);
            (key.column.clone(), id, value)
        }).collect();
        let bindings = keys.iter().map(|(_, id, value)| quote!(let #id = #value;)).collect::<Vec<_>>();
        let key = |level: &[String]| {
            let ids = level.iter().map(|column| &keys.iter().find(|(name, _, _)| name == column).expect("checked probe key").1);
            quote!(&(#(#ids,)*))
        };
        let first = key(&join.binding.levels[0]);
        let handle = match join.binding.kind {
            IndexKind::Host => {
                let columns = sorted(&match &self.physical[name].operator { Operator::Probe(_, keyed) => keyed.columns.clone(), _ => unreachable!("checked probe") });
                let fallback = self.host_access(right, None, &columns).fallback;
                quote!(#fallback.get())
            }
            IndexKind::Built => {
                let structure = self.built_ident(right, &join.binding.levels);
                quote!(#structure)
            }
        };
        let mut lookup = quote!(::cq_storage::IndexRead::lookup(&#handle, #first));
        for level in &join.binding.levels[1..] {
            let key = key(level);
            lookup = quote!(::core::iter::Iterator::flat_map(#lookup, move |__cq_level| ::cq_storage::IndexRead::lookup(&__cq_level, #key)));
        }
        let mut positions: Vec<_> = join.positions.iter().collect();
        positions.sort_by_key(|position| position.output);
        let positions = positions.into_iter().map(|position| match position.side {
            Side::Left => self.position_ident(left, position.input).to_token_stream(),
            Side::Right => {
                let slot = crate::index(position.input);
                quote!(__cq_payload.#slot)
            }
        });
        let owned = join.owned.iter().map(|column| {
            if let Some(slot) = join.payload.owned.iter().position(|field| field == column) {
                let slot = crate::index(join.payload.positions.len() + slot);
                quote!(::core::clone::Clone::clone(&__cq_payload.#slot))
            } else {
                let value = self.access(left, column);
                quote!(::core::clone::Clone::clone(&#value))
            }
        });
        let witnesses = self.witness_fields(name);
        let row = quote!((#(#positions,)* #(#owned,)* #(#witnesses,)*));
        let borrowed_pattern = self.bind_columns(left, self.carried(left), true);
        if self.analysis.expands.contains(name) {
            let batch = self.analysis.stages.get(name).copied().unwrap_or(1);
            return quote!(::core::iter::Iterator::map(
                ::cq_storage::stage::segmented::<#batch, _, _, _>(
                    #rows, move |#borrowed_pattern| { #(#bindings)* #lookup }
                ),
                move |(#pattern, __cq_payload)| #row
            ));
        }
        if matches!(join.binding.kind, IndexKind::Built)
            && join.binding.levels.len() == 1
            && self.infos[left].computed.is_empty()
            && self.carried_witnesses[left].is_empty()
            && !self.analysis.membership.contains(&(right.clone(), join.binding.levels.concat()))
            && self.constant_key_condition(left, right, &join.binding.levels[0]).is_none()
        {
            let batch = self.analysis.stages.get(left).copied().unwrap_or(1);
            let key = key(&join.binding.levels[0]);
            return quote!(::core::iter::Iterator::map(
                ::cq_storage::IndexRead::probe::<#batch, _, _>(
                    #handle, #rows, move |#borrowed_pattern| { #(#bindings)* *#key }
                ),
                move |(#pattern, __cq_payload)| #row
            ));
        }
        quote!(::core::iter::Iterator::flat_map(#rows, move |#pattern| {
            #(#bindings)*
            ::core::iter::Iterator::map(#lookup, move |__cq_payload| #row)
        }))
    }

    pub fn blocked_carrier_filter_rows(&self, name: &str, rows: TokenStream) -> TokenStream {
        let predicate = &self.encoded.blocks.carrier_predicates[name];
        let input = &predicate.input;
        let (Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select)) = &self.physical[name].operator else { unreachable!("checked carrier predicate") };
        let columns: Vec<_> = predicate.operands.keys().cloned().collect();
        let pattern = self.bind_columns(input, &columns, true);
        let code = |column: &str| format_ident!("__cq_carrier_code_{}_{}", name, column);
        let reads = predicate.operands.iter().map(|(column, operand)| {
            let id = code(column);
            let value = self.blocked_operand(input, operand);
            quote!(let #id = #value;)
        }).collect::<Vec<_>>();
        let mut preparation = Vec::new();
        let mut compare = |left: &Term, comparison: Comparison, right: &Term| {
            let column = match (left, right) { (Term::Variable(column), _) | (_, Term::Variable(column)) => symbol_name(column), _ => unreachable!("checked carrier comparison") };
            let mut operand = |term: &Term| {
                if let Term::Variable(column) = term {
                    let column = symbol_name(column);
                    let id = code(&column);
                    if matches!(predicate.operands[&column], Operand::Owned { .. }) {
                        quote!(*#id)
                    } else {
                        quote!(#id)
                    }
                } else {
                    let value = expressions::term_reference(term, &self.parameters);
                    let value = match &predicate.operands[&column] {
                        Operand::Blocked { representation, .. } => {
                            let field = &self.encoded.records.representations[representation];
                            let host = Self::positional_host(&self.encoded.obligations.sources[&field.source].relation);
                            let slot = self.blocked_slot(*representation);
                            quote!(#host.encode_operand::<#slot>(#value))
                        }
                        Operand::Host { .. } => {
                            let ty = &self.infos[input].types[&column];
                            quote!({ let __cq_value: &#ty = #value; let __cq_code: u64 = ::cq_storage::OrderKey::ordinal(__cq_value); __cq_code })
                        }
                        Operand::Owned { .. } => quote!(#value),
                    };
                    let id = format_ident!("__cq_carrier_operand_{}_{}", name, preparation.len());
                    preparation.push(quote!(let #id = #value;));
                    if matches!(predicate.operands[&column], Operand::Owned { .. }) {
                        quote!(*#id)
                    } else {
                        quote!(#id)
                    }
                }
            };
            let left = operand(left);
            let right = operand(right);
            comparison.expression(quote!(&(#left)), quote!(&(#right)))
        };
        let mut tests = Vec::new();
        for equality in &select.condition.equalities {
            let right = match &equality.right { structures::syntax::EqualityValue::Variable(value) => Term::Variable(value.clone()), structures::syntax::EqualityValue::Literal(value) => Term::Literal(Box::new(value.clone())) };
            tests.push(compare(&Term::Variable(equality.left.clone()), Comparison::Eq(Default::default()), &right));
        }
        for constraint in &select.condition.constraints { tests.push(crate::blocks::block_constraint(constraint, &mut compare)); }
        let rows = quote!({ #(#preparation)* ::core::iter::Iterator::filter(#rows, move |#pattern| { #(#reads)* ::cq_storage::residual(true #(&& (#tests))*) }) });
        self.forward(name, input, rows)
    }
}
