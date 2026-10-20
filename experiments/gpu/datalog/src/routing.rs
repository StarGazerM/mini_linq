//! Route named pipeline invocations without interpreting any pass arguments.
use cq_stage::{Configuration, Invocation, Pipeline};
use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Ident, Token, braced, parenthesized,
    parse::{Parse, ParseStream},
};

mod kw {
    syn::custom_keyword!(schedule);
    syn::custom_keyword!(pipeline);
    syn::custom_keyword!(hole);
    syn::custom_keyword!(rule);
    syn::custom_keyword!(delta);
    syn::custom_keyword!(seed);
    syn::custom_keyword!(source);
    syn::custom_keyword!(names);
    syn::custom_keyword!(pipelines);
    syn::custom_keyword!(collected);
    syn::custom_keyword!(pending);
    syn::custom_keyword!(task);
    syn::custom_keyword!(leveled_ratio);
}

#[derive(Clone)]
pub struct Named {
    pub config: Configuration,
    fields: BTreeSet<String>,
}
impl Parse for Named {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::pipeline>()?;
        let body;
        braced!(body in input);
        let tokens: TokenStream = body.parse()?;
        let config = syn::parse2(quote!(pipeline { #tokens }))?;
        // Groups (including every argument list) stay opaque. Only named phase
        // presence is needed because an explicit [] must not mean inheritance.
        let flat: Vec<_> = tokens.into_iter().collect();
        let fields = flat
            .windows(2)
            .filter_map(|pair| match pair {
                [TokenTree::Ident(name), TokenTree::Punct(eq)] if eq.as_char() == '=' => {
                    Some(name.to_string())
                }
                _ => None,
            })
            .collect();
        Ok(Self { config, fields })
    }
}
impl Named {
    pub fn merge(self, base: &Configuration) -> Configuration {
        let mut result = self.config;
        if result.plan.is_none() {
            result.plan = base.plan.clone();
        }
        if result.ix.is_none() {
            result.ix = base.ix.clone();
        }
        if result.lower.is_none() {
            result.lower = base.lower.clone();
        }
        if result.select.is_none() {
            result.select = base.select.clone();
        }
        if result.represent.is_none() {
            result.represent = base.represent.clone();
        }
        if result.stage.is_none() {
            result.stage = base.stage.clone();
        }
        if result.print.is_none() {
            result.print = base.print.clone();
        }
        if !self.fields.contains("physical") {
            result.physical = base.physical.clone();
        }
        if !self.fields.contains("structures") {
            result.structures = base.structures.clone();
        }
        if !self.fields.contains("surface") {
            result.surface = base.surface.clone();
        }
        result
    }
}

pub fn defaults() -> Configuration {
    syn::parse_quote! { pipeline {
        plan = ::cq_planners::order(le);
        ix = ::mini_linq::passes::index_demands;
        lower = ::mini_linq::passes::lower;
        select = ::mini_linq::passes::select;
        structures = [::cq_rewrites::sorted_runs, ::cq_rewrites::hashed_runs,
            ::cq_gpu::prefix_runs, ::cq_rewrites::retain_lookups,
            ::cq_rewrites::expand_lookups(256),
            ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::Plain, ::cq_gpu_runtime::Compacting>)];
        represent = ::mini_linq::passes::represent;
        stage = ::mini_linq::passes::stage;
        print = ::cq_gpu::print(tile(256,4), frontier(8), directory(rank));
    } }
}
fn path(invocation: &Invocation) -> String {
    invocation
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
pub fn validate(config: &Configuration) -> syn::Result<()> {
    // Structures pipelines are opaque policy choices. Their consuming passes
    // and the GPU printer check supported providers; do not require optimizers.
    if config
        .print
        .as_ref()
        .is_none_or(|print| path(print) != "cq_gpu::print")
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "CQ_GPU_DATALOG_PIPELINE: device holes require cq_gpu::print",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Selector {
    pub rule: usize,
    pub delta: Option<usize>,
}
impl Parse for Selector {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::rule>()?;
        let rule = input.parse::<syn::LitInt>()?.base10_parse()?;
        input.parse::<Token![,]>()?;
        let delta = if input.peek(kw::seed) {
            input.parse::<kw::seed>()?;
            None
        } else {
            input.parse::<kw::delta>()?;
            Some(input.parse::<syn::LitInt>()?.base10_parse()?)
        };
        Ok(Self { rule, delta })
    }
}
impl ToTokens for Selector {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let rule = syn::Index::from(self.rule);
        let kind = self
            .delta
            .map(|delta| {
                let delta = syn::Index::from(delta);
                quote!(delta #delta)
            })
            .unwrap_or_else(|| quote!(seed));
        tokens.extend(quote!(rule #rule, #kind));
    }
}

pub struct Advice {
    pub default: Configuration,
    pub overrides: Vec<(Selector, Named)>,
    pub leveled_ratio: u32,
}
impl Parse for Advice {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let leveled_ratio = if input.peek(kw::leveled_ratio) {
            input.parse::<kw::leveled_ratio>()?;
            let value;
            parenthesized!(value in input);
            let ratio = value.parse::<syn::LitInt>()?.base10_parse()?;
            if !value.is_empty() || ratio < 2 {
                return Err(value.error("leveled_ratio requires one integer at least two"));
            }
            input.parse::<Token![;]>()?;
            ratio
        } else {
            4
        };
        let default = if input.peek(kw::pipeline) {
            input.parse::<Named>()?.merge(&defaults())
        } else {
            defaults()
        };
        if input.peek(Token![;]) {
            input.parse::<Token![;]>()?;
        }
        let mut overrides = Vec::new();
        while input.peek(kw::hole) {
            input.parse::<kw::hole>()?;
            let selector;
            parenthesized!(selector in input);
            let selected = selector.parse()?;
            if !selector.is_empty() {
                return Err(selector.error("unexpected hole selector text"));
            }
            overrides.push((selected, input.parse()?));
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
            }
        }
        Ok(Self { default, overrides, leveled_ratio })
    }
}
pub struct Input {
    pub schedule: Invocation,
    pub advice: Advice,
    pub source: cq_lang_datalog::Program,
}
impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let schedule = if input.peek(kw::schedule) {
            input.parse::<kw::schedule>()?;
            input.parse::<Token![=]>()?;
            let pass = input.parse()?;
            input.parse::<Token![;]>()?;
            pass
        } else {
            syn::parse_quote!(::cq_gpu_datalog::schedule)
        };
        Ok(Self {
            schedule,
            advice: input.parse()?,
            source: input.parse()?,
        })
    }
}

pub fn config_tokens(config: &Configuration) -> TokenStream {
    let Configuration {
        plan,
        ix,
        lower,
        physical,
        select,
        structures,
        represent,
        stage,
        print,
        surface,
    } = config;
    let plan = plan.as_ref().map(|pass| quote!(plan = #pass;));
    let ix = ix.as_ref().map(|pass| quote!(ix = #pass;));
    let lower = lower.as_ref().map(|pass| quote!(lower = #pass;));
    let select = select.as_ref().map(|pass| quote!(select = #pass;));
    let represent = represent.as_ref().map(|pass| quote!(represent = #pass;));
    let stage = stage.as_ref().map(|pass| quote!(stage = #pass;));
    let print = print.as_ref().map(|pass| quote!(print = #pass;));
    quote!(pipeline { #plan #ix #lower physical = [#(#physical),*]; #select
        structures = [#(#structures),*]; #represent #stage #print surface = [#(#surface),*]; })
}
impl ToTokens for Advice {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ratio = self.leveled_ratio;
        tokens.extend(quote!(leveled_ratio(#ratio);));
        config_tokens(&self.default).to_tokens(tokens);
        for (selector, named) in &self.overrides {
            // Materializing inheritance changes no invocation or argument tokens.
            let config = config_tokens(&named.clone().merge(&self.default));
            tokens.extend(quote!(hole(#selector) #config;));
        }
    }
}

pub fn resolve(
    source: &cq_lang_datalog::Program,
    fork: &cq_datalog_fork::Fork,
    advice: Advice,
) -> syn::Result<Vec<Configuration>> {
    validate(&advice.default)?;
    let mut pipelines = vec![advice.default.clone(); fork.holes.len()];
    let variants = source.variants.as_ref().expect("checked schedule");
    let mut selected = BTreeSet::new();
    for (selector, named) in advice.overrides {
        let matches: Vec<_> = fork
            .context
            .strata
            .iter()
            .flat_map(|stratum| stratum.seeds.iter().chain(&stratum.steps))
            .filter(|rule| {
                variants[rule.variant].rule == selector.rule
                    && variants[rule.variant].delta == selector.delta
            })
            .map(|rule| rule.hole)
            .collect();
        if matches.len() != 1 {
            return Err(syn::Error::new_spanned(
                selector.to_token_stream(),
                "CQ_GPU_DATALOG_HOLE_SELECTOR: selector must name exactly one recorded schedule variant (rule and seed/delta occurrence)",
            ));
        }
        if !selected.insert(matches[0]) {
            return Err(syn::Error::new_spanned(
                selector.to_token_stream(),
                "CQ_GPU_DATALOG_HOLE_SELECTOR: ambiguous duplicate override for the same recorded hole",
            ));
        }
        let config = named.merge(&advice.default);
        validate(&config)?;
        pipelines[matches[0]] = config;
    }
    Ok(pipelines)
}

pub struct Pending {
    pub prefix: Pipeline,
    pub source: TokenStream,
}
pub struct State {
    pub source: cq_lang_datalog::Program,
    pub leveled_ratio: u32,
    pub names: Vec<Ident>,
    pub pipelines: Vec<Configuration>,
    pub collected: Vec<cq_lang_structures::Program>,
    pub pending: Vec<Pending>,
}
impl Parse for State {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::source>()?;
        let body;
        braced!(body in input);
        let source = body.parse()?;
        input.parse::<kw::leveled_ratio>()?;
        let leveled_ratio = input.parse::<syn::LitInt>()?.base10_parse()?;
        input.parse::<kw::names>()?;
        let body;
        braced!(body in input);
        let names = body
            .parse_terminated(Ident::parse, Token![,])?
            .into_iter()
            .collect();
        input.parse::<kw::pipelines>()?;
        let body;
        braced!(body in input);
        let mut pipelines = Vec::new();
        while !body.is_empty() {
            pipelines.push(body.parse::<Configuration>()?);
        }
        input.parse::<kw::collected>()?;
        let body;
        braced!(body in input);
        let mut collected = Vec::new();
        while !body.is_empty() {
            let one;
            braced!(one in body);
            collected.push(one.parse()?);
        }
        input.parse::<kw::pending>()?;
        let body;
        braced!(body in input);
        let mut pending = Vec::new();
        while !body.is_empty() {
            body.parse::<kw::task>()?;
            let one;
            braced!(one in body);
            let prefix = one.parse()?;
            one.parse::<kw::source>()?;
            let source;
            braced!(source in one);
            pending.push(Pending {
                prefix,
                source: source.parse()?,
            });
            if !one.is_empty() {
                return Err(one.error("unexpected pending hole text"));
            }
        }
        Ok(Self {
            source,
            leveled_ratio,
            names,
            pipelines,
            collected,
            pending,
        })
    }
}
impl ToTokens for State {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let source = &self.source;
        let ratio = self.leveled_ratio;
        let names = &self.names;
        let pipelines = self.pipelines.iter().map(config_tokens);
        let collected = self.collected.iter().map(|hole| quote!({ #hole }));
        let pending = self.pending.iter().map(|task| {
            let prefix = &task.prefix;
            let source = &task.source;
            quote!(task { #prefix source { #source } })
        });
        tokens.extend(quote!(source { #source } leveled_ratio #ratio names { #(#names),* }
            pipelines { #(#pipelines)* } collected { #(#collected)* } pending { #(#pending)* }));
    }
}

/// Preserve the fresh identifiers' hygiene across macro continuation boundaries.
/// The fork is reconstructed only at the two ends; no macro-global state is used.
pub fn context_names(
    context: &mut cq_datalog_fork::RoundContext,
    restore: Option<&[Ident]>,
) -> Vec<Ident> {
    let restored: BTreeMap<_, _> = restore
        .into_iter()
        .flatten()
        .map(|id| (id.to_string(), id))
        .collect();
    let mut names = Vec::new();
    let mut visit = |id: &mut Ident| {
        if let Some(original) = restored.get(&id.to_string()) {
            *id = (*original).clone();
        }
        names.push(id.clone());
    };
    for relation in &mut context.relations {
        visit(&mut relation.all);
        visit(&mut relation.seen);
        visit(&mut relation.next);
        if let Some(id) = &mut relation.delta_host {
            visit(id);
        }
    }
    for layout in context.layouts.iter_mut().chain(&mut context.delta_layouts) {
        visit(&mut layout.name);
        visit(&mut layout.host);
    }
    for stratum in &mut context.strata {
        for key in &mut stratum.keys {
            visit(&mut key.name);
        }
    }
    visit(&mut context.observer_name);
    visit(&mut context.round_name);
    names
}
