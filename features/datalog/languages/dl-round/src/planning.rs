use super::*;

impl Hole {
    /// Formal rule DL-StepAdvice: mechanically serialize recorded occurrence facts.
    /// Serialize recorded occurrence advice into a fresh invocation queue.
    /// Normalization is a deterministic identity translation, not a planner.
    pub fn invocation_pipeline(&self) -> syn::Result<cq_stage::Pipeline> {
        let mut pipeline = self.pipeline.clone();
        let Some(planning) = &self.planning else {
            return Ok(pipeline);
        };
        if !planning.authored && planning.known.is_none() {
            return Ok(pipeline);
        }
        let plan = self.plan.ok_or_else(|| {
            syn::Error::new_spanned(self, "planning facts require a planning invocation")
        })?;
        let pass = pipeline
            .passes
            .get_mut(plan)
            .ok_or_else(|| syn::Error::new_spanned(self, "missing planning invocation"))?;
        ensure(
            pass.arguments.is_empty(),
            self,
            "recorded planning facts conflict with raw planning arguments",
        )?;
        let mut arguments = TokenStream::new();
        if planning.authored {
            let normal = cq::normal::normalize(&self.cq)?;
            let atoms: Vec<_> = normal
                .query
                .body
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    if let BodyItem::Positive { atom } = item {
                        Some((index, &atom.relation))
                    } else {
                        None
                    }
                })
                .collect();
            ensure(
                atoms.len() == planning.order.len(),
                self,
                "normalization must preserve recorded positive occurrences",
            )?;
            let mapped: std::collections::BTreeMap<_, _> = planning
                .order
                .iter()
                .copied()
                .zip(atoms.iter().copied())
                .collect();
            let selector =
                |occurrence: usize, columns: Option<&Vec<Ident>>| -> syn::Result<TokenStream> {
                    let &(index, relation) = mapped.get(&occurrence).ok_or_else(|| {
                        syn::Error::new_spanned(self, "advice names no positive occurrence")
                    })?;
                    let index = syn::Index::from(index);
                    let columns = columns.map(|columns| quote!((#(#columns),*)));
                    Ok(quote!(#relation @ #index #columns))
                };
            let order = if planning.steps.is_empty() {
                planning
                    .order
                    .iter()
                    .map(|&occurrence| selector(occurrence, None))
                    .collect::<syn::Result<Vec<_>>>()?
            } else {
                planning
                    .steps
                    .iter()
                    .map(|step| selector(step.occurrence, step.columns.as_ref()))
                    .collect::<syn::Result<Vec<_>>>()?
            };
            let no_reduce = (!planning.steps.is_empty() || !planning.smallest.is_empty())
                .then(|| quote!(no_reduce,));
            arguments.extend(quote!(#no_reduce order(#(#order),*)));
            if !planning.smallest.is_empty() {
                let sites = planning
                    .smallest
                    .iter()
                    .map(|site| {
                        let atoms = site
                            .iter()
                            .map(|&occurrence| selector(occurrence, None))
                            .collect::<syn::Result<Vec<_>>>()?;
                        Ok(quote!(smallest(#(#atoms),*)))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                let next = pipeline.passes.get(self.lower + 1).ok_or_else(|| {
                    syn::Error::new_spanned(
                        self,
                        "smallest advice requires a post-lowering invocation",
                    )
                })?;
                pipeline.passes[self.lower + 1] =
                    syn::parse_quote!(::cq_datalog::passes::smallest_before(#next; #(#sites),*));
            }
        }
        if let Some(level) = planning.known {
            let Some(BodyItem::Negation(negative)) = self.cq.query.body.iter().last() else {
                return Err(syn::Error::new_spanned(
                    self,
                    "known level requires a final admission negation",
                ));
            };
            let name = &negative.atom.relation;
            let clause = syn::Index::from(self.cq.query.body.len() - 1);
            let level = syn::Index::from(level);
            if !arguments.is_empty() {
                arguments.extend(quote!(,));
            }
            arguments.extend(quote!(known(#name @ #clause at #level)));
        }
        pipeline.passes[plan].arguments = arguments;
        Ok(pipeline)
    }
}
