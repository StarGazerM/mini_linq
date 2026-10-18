//! Evaluate total scan predicates in chunks before exposing survivor rows.
use super::*;

impl Printer<'_> {
    pub(super) fn probe_stage(&self, name: &str) -> bool {
        self.physical.values().any(|definition| {
            let Operator::Probe(_, keyed) = &definition.operator else {
                return false;
            };
            let right = symbol_name(&keyed.right);
            let columns = sorted(&keyed.columns);
            symbol_name(&keyed.left) == name
                && !self.analysis.host_read(&right)
                && self.analysis.built.get(&right).is_some_and(|built| {
                    built
                        .iter()
                        .any(|(levels, _)| levels.len() == 1 && levels[0] == columns)
                })
                && self.infos[name].computed.is_empty()
                && !self
                    .analysis
                    .membership
                    .contains(&(right.clone(), columns.clone()))
                && self
                    .constant_key_condition(name, &right, &columns)
                    .is_none()
        })
    }

    pub(super) fn buffer_stage(&self, name: &str, rows: TokenStream) -> TokenStream {
        match self.analysis.stages.get(name) {
            Some(size) if !self.probe_stage(name) => {
                quote!(::cq_storage::stage::buffered::<#size, _>(#rows))
            }
            _ => rows,
        }
    }
    pub(super) fn staged_rows(&self, name: &str) -> Option<TokenStream> {
        let size = *self.analysis.stages.get(name)?;
        // Built probes own their existing batching adapter, including hosts
        // that offer Scan but no positional capability.
        if self.probe_stage(name) {
            return None;
        }
        let mut current = name.to_owned();
        let mut predicates = Vec::new();
        loop {
            if current != name
                && (self.analysis.retained.contains_key(&current)
                    || self.physical_analysis.canon.contains(&current)
                    || self.analysis.stages.contains_key(&current))
            {
                return None;
            }
            match &self.physical[&current].operator {
                Operator::Rename(_) => break,
                Operator::Project(project) => current = symbol_name(&project.input),
                Operator::Select(select)
                | Operator::Point(_, select)
                | Operator::Range(_, select) => {
                    predicates.push(select);
                    current = symbol_name(&select.input);
                }
                _ => return None,
            }
        }
        let origin = self.analysis.lineage.get(name)?;
        let host = Context::positional_host(&origin.base);
        let source = &self.hosts[&origin.base].value;
        // Total predicates may be evaluated together on every chunk row (ST-Total:
        // no ⊥, masks AND), so the chain costs one row fetch per row, not one
        // per predicate; the conjunction keeps written order and short-circuits.
        let mut bindings = Vec::new();
        let mut bound = std::collections::BTreeSet::new();
        let mut conditions = Vec::new();
        for select in predicates.into_iter().rev() {
            let input = symbol_name(&select.input);
            let lineage = self.analysis.lineage.get(&input)?;
            for column in cq_print::columns_used(&select.condition) {
                let ident = &self.infos[&input].bindings[&column];
                if !bound.insert(ident.to_string()) {
                    continue;
                }
                let source_column = &lineage.columns[&column];
                let field = self.analysis.host_layouts[&origin.base]
                    .iter()
                    .position(|c| c == source_column)?;
                let field = syn::Index::from(field);
                bindings.push(quote!(let #ident = __cq_stage_row.#field;));
            }
            conditions.push(self.relational_condition(&select.condition));
        }
        let fetch = (!bindings.is_empty()).then(|| quote!(
            let __cq_stage_row = ::cq_storage::PositionalRows::row_at(#host, __cq_id).expect("host-issued live ID");
        ));
        let ids = quote!(::core::iter::Iterator::flat_map(
            ::cq_storage::stage::for_chunks::<#size, _>(#host),
            move |mut __cq_chunk| {
                __cq_chunk.select_with(move |__cq_id| { #fetch #(#bindings)* true #(&& (#conditions))* });
                __cq_chunk
            }
        ));
        let rows = self.positional_rows(name, ids);
        Some(quote!({
            let #host = { use ::cq_storage::PositionalRows as _; #source.positional_handle() };
            #rows
        }))
    }
}
