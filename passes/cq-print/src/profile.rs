//! Optional measurement tokens. Ordinary printer paths return tokens unchanged.
use super::*;

/// Printer-local metadata; no runtime dependency or Cargo feature on the pass.
pub struct Profile {
    handle: Ident,
    labels: Vec<(String, String, String, Vec<String>)>,
    aliases: Vec<(usize, usize)>,
    time_aliases: Vec<(usize, usize)>,
}
impl Context<'_> {
    pub fn enable_profile(&mut self, module: &structures::Program) {
        fn names(tokens: TokenStream, out: &mut std::collections::BTreeSet<String>) {
            for token in tokens {
                match token {
                    proc_macro2::TokenTree::Ident(id) => {
                        out.insert(symbol_name(&id));
                    }
                    proc_macro2::TokenTree::Group(group) => names(group.stream(), out),
                    _ => {}
                }
            }
        }
        let mut used = std::collections::BTreeSet::new();
        names(module.to_token_stream(), &mut used);
        let mut suffix = 0usize;
        while used.contains(&format!("__cq_profile_{suffix}")) {
            suffix += 1;
        }
        let handle = format_ident!(
            "__cq_profile_{}",
            suffix,
            span = proc_macro2::Span::mixed_site()
        );
        self.profile = Some(Profile {
            handle: handle.clone(),
            labels: Vec::new(),
            aliases: Vec::new(),
            time_aliases: Vec::new(),
        });
        self.owned.push(handle);
        let definitions: Vec<_> = self.physical.values().copied().collect();
        for d in definitions {
            let name = symbol_name(&d.name);
            let kind = d
                .operator
                .to_token_stream()
                .to_string()
                .split_whitespace()
                .next()
                .unwrap()
                .to_uppercase();
            let key = match &d.operator {
                Operator::Rename(r) => symbol_name(&r.relation),
                Operator::Point(_, select) => select
                    .condition
                    .constant_keys()
                    .map(|keys| keys.keys().cloned().collect::<Vec<_>>().join(","))
                    .unwrap_or_default(),
                Operator::Range(_, select) => select
                    .condition
                    .range_bounds()
                    .map(|(column, _)| column)
                    .unwrap_or_default(),
                Operator::Probe(_, k)
                | Operator::Semi(_, k)
                | Operator::Anti(_, k)
                | Operator::Merge(_, k) => sorted(&k.columns).join(","),
                _ => self.heading(&name).join(","),
            };
            let mut inputs: Vec<String> = d
                .operator
                .operands()
                .iter()
                .map(|i| symbol_name(i))
                .collect();
            if let Operator::Union(union) = &d.operator {
                if union.fused.is_some() {
                    if let Some(over) = union.origin.as_ref().and_then(|o| o.over.as_ref()) {
                        let prefix = symbol_name(&over.prefix);
                        if !inputs.contains(&prefix) {
                            inputs.push(prefix);
                        }
                    }
                }
            }
            self.profile_label(&name, &kind, &key, inputs);
        }
        for (name, definition) in &self.physical {
            if let Operator::Merge(_, keyed) = &definition.operator {
                for input in [&keyed.left, &keyed.right] {
                    let input = symbol_name(input);
                    if self.analysis.host_read(&input) {
                        let i = self.profile_index(&input).unwrap();
                        let j = self.profile_index(name).unwrap();
                        self.profile.as_mut().unwrap().time_aliases.push((i, j));
                    }
                }
            }
        }
        for name in self.analysis.reads.keys() {
            if let Some(input) = self.physical[name].operator.operands().first() {
                let input = symbol_name(input);
                if matches!(self.physical[&input].operator, Operator::Rename(_)) {
                    let i = self.profile_index(&input).unwrap();
                    let j = self.profile_index(name).unwrap();
                    self.profile.as_mut().unwrap().time_aliases.push((i, j));
                }
            }
        }
    }
    pub fn profile_handle(&self) -> &Ident {
        &self.profile.as_ref().expect("enabled profiling").handle
    }
    pub fn profile_label(
        &mut self,
        name: &str,
        kind: &str,
        key: &str,
        inputs: Vec<String>,
    ) -> usize {
        let Some(p) = &mut self.profile else {
            return 0;
        };
        if let Some(i) = p.labels.iter().position(|l| l.0 == name) {
            return i;
        }
        p.labels
            .push((name.into(), kind.into(), key.into(), inputs));
        p.labels.len() - 1
    }
    pub fn profile_alias(&mut self, name: &str, source: &str) {
        if let (Some(i), Some(j)) = (self.profile_index(name), self.profile_index(source)) {
            self.profile.as_mut().unwrap().aliases.push((i, j));
        }
    }
    /// Fused Physical results share the measured span of their combined stream.
    pub fn profile_time_alias(&mut self, name: &str, source: &str) {
        if let (Some(i), Some(j)) = (self.profile_index(name), self.profile_index(source)) {
            self.profile.as_mut().unwrap().time_aliases.push((i, j));
        }
    }
    pub fn profile_index(&self, name: &str) -> Option<usize> {
        self.profile
            .as_ref()?
            .labels
            .iter()
            .position(|l| l.0 == name)
    }
    pub fn profile_stream(&self, name: &str, rows: TokenStream) -> TokenStream {
        match self.profile_index(name) {
            Some(i) => {
                let profile = self.profile_handle();
                quote!(#profile.stream(#i,#rows))
            }
            None => rows,
        }
    }
    pub fn profile_output(&self, name: &str, rows: TokenStream) -> TokenStream {
        match self.profile_index(name) {
            Some(i) => {
                let profile = self.profile_handle();
                quote!(#profile.output(#i,#rows))
            }
            None => rows,
        }
    }
    pub fn profile_input(&self, name: &str, rows: TokenStream) -> TokenStream {
        match self.profile_index(name) {
            Some(i) => {
                let profile = self.profile_handle();
                quote!(#profile.input(#i,#rows))
            }
            None => rows,
        }
    }
    pub fn profile_inserting(&self, name: &str, rows: TokenStream) -> TokenStream {
        match self.profile_index(name) {
            Some(i) => {
                let profile = self.profile_handle();
                quote!(#profile.inserting(#i,#rows))
            }
            None => rows,
        }
    }
    pub fn profile_build(&self, name: &str, value: TokenStream) -> TokenStream {
        match self.profile_index(name) {
            Some(i) => {
                let profile = self.profile_handle();
                quote!(#profile.build(#i,|| #value))
            }
            None => value,
        }
    }
    pub fn profile_predicate(&self, name: &str, predicate: TokenStream) -> TokenStream {
        let Some(index) = self.profile_index(name) else {
            return predicate;
        };
        let operands = self.physical[name].operator.operands();
        let input = operands.first().map(|i| symbol_name(i));
        let source = input
            .as_ref()
            .filter(|n| matches!(self.reprs.get(*n), Some(Repr::Host)))
            .and_then(|n| self.profile_index(n));
        let source = match source {
            Some(i) => quote!(Some(#i)),
            None => quote!(None),
        };
        let profile = self.profile_handle();
        quote!(#profile.predicate(#index,#source,#predicate))
    }
    /// Append after all labels are known, then move before generated work.
    pub fn profile_registration(&self) -> Option<Stmt> {
        let p = self.profile.as_ref()?;
        let profile = &p.handle;
        let labels=p.labels.iter().map(|(result,kind,key,inputs)|quote!(::cq_storage::counters::OperatorLabel {result:#result,kind:#kind,key:#key,inputs:&[#(#inputs),*]}));
        let aliases = p
            .aliases
            .iter()
            .map(|(i, j)| quote!(#profile.alias(#i,#j);));
        let times = p
            .time_aliases
            .iter()
            .map(|(i, j)| quote!(#profile.share_time(#i,#j);));
        Some(syn::parse_quote!(let #profile = {
            let #profile = ::cq_storage::counters::OperatorProgram::new(&[#(#labels),*]);
            #(#aliases)*
            #(#times)*
            #profile
        };))
    }
}
