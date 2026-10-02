//! Pipeline descriptions shared by the executable corpus and command drivers.
//! Every stage below writes its decision into checked target IR.
pub use cq_lang_physical as ir;
pub use cq_planners_plan::Planner;
use proc_macro2::TokenStream;
use quote::quote;

pub const PLAN_ARGUMENTS: [&str; 9] = [
    "written, coarse",
    "written, fine",
    "hypergraph",
    "prefix connected, coarse",
    "prefix connected, fine",
    "prefix greedy, coarse",
    "prefix greedy, fine",
    "elimination, coarse",
    "elimination, fine",
];

#[derive(Clone, Copy, Debug, Default)]
pub struct Pipeline {
    pub plan: usize,
    pub merge: bool,
    pub canon: bool,
    pub smallest: bool,
    pub lazy: bool,
    pub ordered_dedup: bool,
    pub sorted_payload: bool,
    pub push: bool,
}

impl Pipeline {
    pub fn physical(self, source: &cq_lang_cq::Program) -> syn::Result<cq_lang_physical::Program> {
        let ra = cq_planners_plan::compile(source, Planner::ALL[self.plan])?;
        let ix = cq_index_policy::compile(&ra)?;
        self.rewrite_physical(&cq_physical_lowering::compile(&ix)?)
    }

    pub fn rewrite_physical(
        self,
        source: &cq_lang_physical::Program,
    ) -> syn::Result<cq_lang_physical::Program> {
        let mut target = source.clone();
        for (enabled, rewrite) in [
            (
                self.canon,
                cq_rewrite_passes::canon_prefixes
                    as fn(
                        &cq_lang_physical::Program,
                        TokenStream,
                    ) -> syn::Result<cq_lang_physical::Program>,
            ),
            (self.smallest, cq_rewrite_passes::smallest_intersections),
            (self.lazy, cq_rewrite_passes::lazy_semijoins),
            (self.merge, cq_rewrite_passes::merge_joins),
        ] {
            if enabled {
                target = rewrite(&target, TokenStream::new())?;
            }
        }
        Ok(target)
    }

    pub fn configuration(self, source: &cq_lang_cq::Program) -> syn::Result<TokenStream> {
        let arguments: TokenStream = PLAN_ARGUMENTS[self.plan].parse()?;
        let mut physical = Vec::new();
        if self.canon {
            physical.push(quote!(::cq_rewrites::canon_prefixes));
        }
        if self.smallest {
            physical.push(quote!(::cq_rewrites::smallest_intersections));
        }
        if self.lazy {
            physical.push(quote!(::cq_rewrites::lazy_semijoins));
        }
        if self.merge {
            physical.push(quote!(::cq_rewrites::merge_joins));
        }
        let mut structures = Vec::new();
        if self.ordered_dedup {
            let target = cq_structure_selection::compile(&self.physical(source)?)?;
            if target.structures.entries.iter().any(|e| {
                matches!(
                    e.structure,
                    cq_lang_structures::Structure::Dedup { .. }
                        | cq_lang_structures::Structure::HeadDedup { .. }
                )
            }) {
                structures
                    .push(quote!(::cq_rewrites::retype(dedup => ::std::collections::BTreeSet<_>)));
            }
        }
        if self.sorted_payload {
            structures.push(quote!(::cq_workload_passes::sorted_payload));
        }
        let printer = if self.push {
            quote!(::own_push::push)
        } else {
            quote!(::mini_linq::passes::print_pull)
        };
        Ok(quote!(pipeline {
            plan = ::cq_planners::order(#arguments);
            physical = [#(#physical),*];
            structures = [#(#structures),*];
            print = #printer;
        }))
    }

    /// A native leading merge requires an ordered host of exactly that prefix.
    /// Push explicitly refuses merge and smallest, even when pull can print them.
    pub fn rejection(
        self,
        source: &cq_lang_cq::Program,
        host: usize,
    ) -> syn::Result<Option<&'static str>> {
        self.rejection_physical(&self.physical(source)?, host)
    }

    pub fn rejection_physical(
        self,
        target: &cq_lang_physical::Program,
        host: usize,
    ) -> syn::Result<Option<&'static str>> {
        if self.push
            && target.program.definitions.iter().any(|d| {
                matches!(d.operator, cq_lang_physical::Operator::Merge(..))
                    || matches!(&d.operator, cq_lang_physical::Operator::Probe(_, k)
                    if matches!(k.choice, Some(cq_lang_physical::Choice::Smallest(_))))
            })
        {
            return Ok(Some(
                "push printer refuses ordered merge or smallest intersection",
            ));
        }
        let structures = cq_structure_selection::compile(target)?;
        let analysis = cq_lang_structures::contract::check(&structures)?;
        for (result, reads) in &analysis.physical.merge_reads {
            if !analysis.host_read(result) {
                continue;
            }
            let host_name = &analysis.physical.hosts[result];
            let inverse: std::collections::BTreeMap<_, _> = analysis.physical.renames[result]
                .iter()
                .map(|(t, s)| (s, t))
                .collect();
            let layout: Vec<_> = analysis.host_layouts[host_name]
                .iter()
                .map(|c| inverse[c].clone())
                .collect();
            for (_, columns) in reads {
                if layout[..columns.len()] == *columns && (host != 2 || columns.len() != 1) {
                    return Ok(Some(if host == 2 {
                        "leading declaration prefix"
                    } else {
                        "OrderedRead"
                    }));
                }
            }
        }
        Ok(None)
    }
}
