//! Checked token-stream pass protocol.
//!
//! A pass receives `pipeline { Path(arguments), Path, ... }; <program>`.
//! [`pass`] parses and checks its source, calls the transformation with the
//! current pass's opaque argument tokens, checks the target, and invokes the
//! next macro. Arguments are delivered as `arguments { ... };` before the
//! pipeline. [`rewrite`] additionally checks language correspondence.
//!
//! The facade accepts the named configuration below. This crate knows phase
//! names, but does not resolve macro types. Omitted single phases use caller-supplied
//! defaults; omitted pass lists are empty. Sequence targets must opt in together.
//! ```text
//! pipeline {
//!   plan = Path[(tokens)]; ix = Path; lower = Path;
//!   physical = [Path[(tokens)], ...]; select = Path;
//!   structures = [Path[(tokens)], ...]; represent = Path; stage = Path;
//!   sequence = Path; decompose = [Path[(tokens)], ...]; grade = Path; print = Path;
//!   surface = [Path[(tokens)], ...];
//! }
//! ```
//! Planning options use that same invocation argument slot, for example
//! `plan = mini_linq::passes::plan(no_reduce);` opts out of the facade's
//! default Hypergraph semijoin reduction without adding a phase-level setting.
//! Surface passes receive raw CQ tokens before the CQ contract. They expand
//! authored `name!{...}` clauses. The planner then parses and checks CQ.
//!
//! Physical records `pipe`, `let`, `exists`, `canon`, `probe first` or
//! `probe smallest`, `merge`, `semi eager` or `semi lazy`, `point`, and `range`.
//! Structures records built Rust types and levels, each host read's fallback
//! type, and each canonical boundary's dedup type, including computed heads.
//! A printer consumes these decisions through the checked pass queue.
//!
//! ```text
//! structures {
//!   result = ::cq_storage::HashIndex<'_, _, _> [key];
//!   host_result host reader = ::cq_storage::dispatch::KeySet<_> [key];
//!   host_result host scan = ::core::iter::Map<_, _> [];
//!   result canon = ::std::collections::HashSet<_>;
//!   returned head = ::std::collections::HashSet<_>;
//!   retained rows = ::std::vec::Vec<_>;
//! }
//! ```
//! Host fallbacks are recorded per reader/key, including the initial scan of
//! retained host renames. `candidates`/`membership` record supplemental smallest
//! intersection reads. Lazy membership selects `KeySet<_, false>` by default;
//! `exists` retains unit rows in `::cq_storage::Exists`, consuming the full input.
//!
//! Rewrite checks are exported by the languages: Physical preserves RA′ erasure
//! and admissible demand supplies; Structures preserves Physical and discharges
//! every read and boundary. RA′ uses the existing tableau/Rust-chain proof. VO
//! preserves CQ, valuation erasure and existing reductions under its coverage
//! and Before contract. IX preserves RA′. Contracted CQ rewrites are identity;
//! surface expansions operate on raw tokens before that boundary. Known invalid
//! standard representations fail contracts; rustc checks arbitrary user types
//! against the generated Build, read and dedup obligations.
//!
//! Restricted context facts enter only through [`pass_with_context`] (the
//! lower/select adapters). Checked continuations carry a separate
//! `context { consumer = canonical, host_set R discharged };` envelope after
//! arguments. Its facts must equal the source's recorded facts; ordinary passes
//! cannot add facts. Printing an IR does not print this envelope, so factful
//! re-entry requires an explicit exact envelope from the enclosing compiler.
//! Lower/select arguments introduce facts only into a fact-free source; matching
//! arguments never replace an envelope. Fact-free expansions retain their
//! original token protocol, and pass arguments remain opaque.

use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Token, braced, parenthesized};

mod kw {
    syn::custom_keyword!(pipeline);
    syn::custom_keyword!(arguments);
    syn::custom_keyword!(context);
    syn::custom_keyword!(consumer);
    syn::custom_keyword!(canonical);
    syn::custom_keyword!(host_set);
    syn::custom_keyword!(discharged);
}

/// Public contract implemented by each IR language. Rust expressions terminate
/// the pipeline; raw token streams are used only by surface expansion passes.
pub trait Language: Parse + ToTokens {
    fn check(&self) -> syn::Result<()>;
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()>;
    /// Restricted facts recorded in this program, not ambient Rust assumptions.
    fn context(&self) -> Context {
        Context::default()
    }
    fn terminal() -> bool {
        false
    }
}

impl Language for TokenStream {
    fn check(&self) -> syn::Result<()> {
        Ok(())
    }
    fn check_rewrite(_: &Self, _: &Self) -> syn::Result<()> {
        Ok(())
    }
}
impl Language for syn::Expr {
    fn check(&self) -> syn::Result<()> {
        Ok(())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        if source == target {
            Ok(())
        } else {
            Err(syn::Error::new_spanned(
                target,
                "Rust expression rewrites require their own semantic check",
            ))
        }
    }
    fn terminal() -> bool {
        true
    }
}

/// Facts whose provenance must survive the checked pass queue.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub consumer_canonical: bool,
    pub discharged_host_sets: BTreeSet<String>,
}
impl Context {
    pub fn is_empty(&self) -> bool {
        !self.consumer_canonical && self.discharged_host_sets.is_empty()
    }

    pub fn is_subset(&self, other: &Self) -> bool {
        (!self.consumer_canonical || other.consumer_canonical)
            && self
                .discharged_host_sets
                .is_subset(&other.discharged_host_sets)
    }

    fn extend(&mut self, other: &Self) {
        self.consumer_canonical |= other.consumer_canonical;
        self.discharged_host_sets
            .extend(other.discharged_host_sets.iter().cloned());
    }
}
impl Parse for Context {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        use syn::ext::IdentExt;

        let mut context = Self::default();
        while !input.is_empty() {
            if input.peek(kw::consumer) {
                input.parse::<kw::consumer>()?;
                input.parse::<Token![=]>()?;
                input.parse::<kw::canonical>()?;
                if context.consumer_canonical {
                    return Err(input.error("CTX-Entry: duplicate consumer context fact"));
                }
                context.consumer_canonical = true;
            } else {
                input.parse::<kw::host_set>()?;
                let host: syn::Ident = input.parse()?;
                input.parse::<kw::discharged>()?;
                if !context
                    .discharged_host_sets
                    .insert(host.unraw().to_string())
                {
                    return Err(syn::Error::new_spanned(
                        host,
                        "CTX-Entry: duplicate host context fact",
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(context)
    }
}
impl ToTokens for Context {
    fn to_tokens(&self, output: &mut TokenStream) {
        let mut comma = false;
        if self.consumer_canonical {
            output.extend(quote!(consumer = canonical));
            comma = true;
        }
        for host in &self.discharged_host_sets {
            if comma {
                output.extend(quote!(,));
            }
            let host: syn::Ident = syn::parse_str(host)
                .unwrap_or_else(|_| syn::Ident::new_raw(host, proc_macro2::Span::call_site()));
            output.extend(quote!(host_set #host discharged));
            comma = true;
        }
    }
}

/// A macro path with argument tokens whose interpretation belongs to that pass.
#[derive(Clone, Debug)]
pub struct Invocation {
    pub path: syn::Path,
    pub arguments: TokenStream,
}
impl Parse for Invocation {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse()?;
        let arguments = if input.peek(syn::token::Paren) {
            let contents;
            parenthesized!(contents in input);
            contents.parse()?
        } else {
            TokenStream::new()
        };
        Ok(Self { path, arguments })
    }
}
impl ToTokens for Invocation {
    fn to_tokens(&self, output: &mut TokenStream) {
        let path = &self.path;
        let args = &self.arguments;
        if args.is_empty() {
            output.extend(quote!(#path));
        } else {
            output.extend(quote!(#path(#args)));
        }
    }
}

/// Remaining passes, in execution order. Each continuation consumes one entry.
#[derive(Clone, Debug, Default)]
pub struct Pipeline {
    pub passes: Vec<Invocation>,
}
impl Parse for Pipeline {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::pipeline>()?;
        let contents;
        braced!(contents in input);
        let passes = contents
            .parse_terminated(Invocation::parse, Token![,])?
            .into_iter()
            .collect();
        Ok(Self { passes })
    }
}
impl ToTokens for Pipeline {
    fn to_tokens(&self, output: &mut TokenStream) {
        let passes = &self.passes;
        output.extend(quote!(pipeline { #(#passes),* }));
    }
}
impl Pipeline {
    /// Start this pipeline with unchecked tokens (surface expansion is allowed).
    /// This does not authorize recorded facts: a factful payload still requires
    /// an exact context envelope at the receiving pass (CTX-Entry).
    pub fn start(self, program: TokenStream) -> syn::Result<TokenStream> {
        self.start_with_context(program, Context::default())
    }

    fn start_with_context(
        mut self,
        program: TokenStream,
        context: Context,
    ) -> syn::Result<TokenStream> {
        if self.passes.is_empty() {
            return Err(syn::Error::new_spanned(
                program,
                "pipeline requires a final printer",
            ));
        }
        let next = self.passes.remove(0);
        let path = next.path;
        let args = next.arguments;
        let context = if context.is_empty() {
            TokenStream::new()
        } else {
            quote!(context { #context };)
        };
        Ok(if args.is_empty() {
            quote!(#path! { #context #self ; #program })
        } else {
            quote!(#path! { arguments { #args }; #context #self ; #program })
        })
    }
}

struct Input<L> {
    args: TokenStream,
    context: Option<Context>,
    pipeline: Pipeline,
    program: L,
}
impl<L: Parse> Parse for Input<L> {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let args = if input.peek(kw::arguments) {
            input.parse::<kw::arguments>()?;
            let contents;
            braced!(contents in input);
            let tokens = contents.parse()?;
            input.parse::<Token![;]>()?;
            tokens
        } else {
            TokenStream::new()
        };
        let context = if input.peek(kw::context) {
            input.parse::<kw::context>()?;
            let contents;
            braced!(contents in input);
            let context = contents.parse()?;
            input.parse::<Token![;]>()?;
            Some(context)
        } else {
            None
        };
        let pipeline = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self {
            args,
            context,
            pipeline,
            program: input.parse()?,
        })
    }
}

/// Parse In, check its contract, run f, check Out, then invoke the next pass.
/// Arguments are opaque to this crate; a pass records their meaning in its IR.
/// Formal rule ST-Next: Parse and transform a payload, then emit the selected next macro invocation.
/// Formal rule ST-Context: Expand the pass invocation in its enclosing Rust context.
pub fn pass<In: Language, Out: Language>(
    input: TokenStream,
    f: impl FnOnce(&In, TokenStream) -> syn::Result<Out>,
) -> TokenStream {
    run(input, |_| Ok(Context::default()), f, |_, _| Ok(()))
}

/// Introduce facts named by a lower/select argument packet into a fact-free source.
///
/// Unlike [`pass`], this entry point may introduce facts, but every authorized
/// fact must be recorded in the target. Already-factful sources require an exact
/// context envelope, even when arguments name the same facts, and packets cannot
/// add facts to those sources. The argument parser is supplied by the lower/select
/// implementation, so ordinary pass packets stay opaque.
/// Formal rule CTX-Entry: Admit recorded restricted facts only with an exact context envelope.
pub fn pass_with_context<In: Language, Out: Language>(
    input: TokenStream,
    authorize: impl FnOnce(TokenStream) -> syn::Result<Context>,
    f: impl FnOnce(&In, TokenStream) -> syn::Result<Out>,
) -> TokenStream {
    run(input, authorize, f, |_, _| Ok(()))
}

/// Run an L → L pass and the language's exported correspondence check.
pub fn rewrite<L: Language>(
    input: TokenStream,
    f: impl FnOnce(&L, TokenStream) -> syn::Result<L>,
) -> TokenStream {
    run(input, |_| Ok(Context::default()), f, L::check_rewrite)
}

fn run<In: Language, Out: Language>(
    input: TokenStream,
    authorize: impl FnOnce(TokenStream) -> syn::Result<Context>,
    f: impl FnOnce(&In, TokenStream) -> syn::Result<Out>,
    correspondence: impl FnOnce(&In, &Out) -> syn::Result<()>,
) -> TokenStream {
    let result = (|| {
        let input: Input<In> = syn::parse2(input)?;
        let source_context = input.program.context();
        if input
            .context
            .as_ref()
            .is_some_and(|context| context != &source_context)
        {
            return Err(syn::Error::new_spanned(
                &input.program,
                "CTX-Entry: continuation context must exactly match the source's recorded facts",
            ));
        }
        if !source_context.is_empty() && input.context.is_none() {
            return Err(syn::Error::new_spanned(
                &input.program,
                "CTX-Entry: recorded context facts require an explicit exact context envelope",
            ));
        }
        let authorized = authorize(input.args.clone())?;
        if !source_context.is_empty() && !authorized.is_subset(&source_context) {
            return Err(syn::Error::new_spanned(
                &input.program,
                "CTX-Entry: lower/select argument packets may introduce facts only into a fact-free source",
            ));
        }
        input.program.check()?;
        let target = f(&input.program, input.args)?;
        target.check()?;
        correspondence(&input.program, &target)?;
        let target_context = target.context();
        let mut allowed = source_context;
        allowed.extend(&authorized);
        if !target_context.is_subset(&allowed) {
            return Err(syn::Error::new_spanned(
                &target,
                "CTX-Entry: only lower/select argument packets may introduce context facts",
            ));
        }
        if !authorized.is_subset(&target_context) {
            return Err(syn::Error::new_spanned(
                &target,
                "CTX-Entry: lower/select must record every authorized context fact",
            ));
        }
        if input.pipeline.passes.is_empty() {
            if Out::terminal() {
                Ok(target.to_token_stream())
            } else {
                Err(syn::Error::new_spanned(
                    target,
                    "pipeline ended before its Rust printer",
                ))
            }
        } else if Out::terminal() {
            Err(syn::Error::new_spanned(
                target,
                "Rust printer must be the final pass",
            ))
        } else {
            input
                .pipeline
                .start_with_context(target.to_token_stream(), target_context)
        }
    })();
    result.unwrap_or_else(syn::Error::into_compile_error)
}

/// Parse the named facade grammar. Missing phases inherit from `defaults`.
#[derive(Clone, Debug, Default)]
pub struct Configuration {
    pub plan: Option<Invocation>,
    pub ix: Option<Invocation>,
    pub lower: Option<Invocation>,
    pub physical: Vec<Invocation>,
    pub select: Option<Invocation>,
    pub structures: Vec<Invocation>,
    pub represent: Option<Invocation>,
    pub stage: Option<Invocation>,
    pub sequence: Option<Invocation>,
    pub decompose: Vec<Invocation>,
    pub grade: Option<Invocation>,
    pub print: Option<Invocation>,
    pub surface: Vec<Invocation>,
}
impl Parse for Configuration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::pipeline>()?;
        let contents;
        braced!(contents in input);
        let mut config = Self::default();
        let mut seen = std::collections::BTreeSet::new();
        while !contents.is_empty() {
            let name: syn::Ident = contents.parse()?;
            if !seen.insert(name.to_string()) {
                return Err(syn::Error::new_spanned(name, "duplicate pipeline phase"));
            }
            contents.parse::<Token![=]>()?;
            match name.to_string().as_str() {
                "plan" => config.plan = Some(contents.parse()?),
                "ix" => config.ix = Some(contents.parse()?),
                "lower" => config.lower = Some(contents.parse()?),
                "select" => config.select = Some(contents.parse()?),
                "represent" => config.represent = Some(contents.parse()?),
                "stage" => config.stage = Some(contents.parse()?),
                "sequence" => config.sequence = Some(contents.parse()?),
                "grade" => config.grade = Some(contents.parse()?),
                "print" => config.print = Some(contents.parse()?),
                "physical" | "structures" | "surface" | "decompose" => {
                    let list;
                    syn::bracketed!(list in contents);
                    let entries = list
                        .parse_terminated(Invocation::parse, Token![,])?
                        .into_iter()
                        .collect();
                    match name.to_string().as_str() {
                        "physical" => config.physical = entries,
                        "structures" => config.structures = entries,
                        "decompose" => config.decompose = entries,
                        _ => config.surface = entries,
                    }
                }
                _ => return Err(syn::Error::new_spanned(name, "unknown pipeline phase")),
            }
            contents.parse::<Token![;]>()?;
        }
        Ok(config)
    }
}
impl Configuration {
    pub fn resolve(self, defaults: Self) -> syn::Result<Pipeline> {
        let default_sequence_target = defaults.sequence.is_some() && defaults.grade.is_some();
        let sequence = self.sequence.or(defaults.sequence);
        let grade = self.grade.or(defaults.grade);
        let sequence_target = sequence.is_some() || grade.is_some() || !self.decompose.is_empty();
        let explicit_print = self.print.is_some();
        let print = self.print.or(defaults.print);
        let terminal = print.as_ref().and_then(|p| p.path.segments.last());
        if sequence_target
            && (sequence.is_none()
                || grade.is_none()
                || (!explicit_print && !default_sequence_target)
                || terminal.is_some_and(|p| p.ident == "print_pull" || p.ident == "push"))
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "sequence, grade and an explicit Sequence-consuming print must be selected together",
            ));
        }
        if !sequence_target && terminal.is_some_and(|p| p.ident == "translate_pull") {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "translate_pull requires sequence and grade phases",
            ));
        }
        let mut passes = self.surface;
        let missing =
            || syn::Error::new(proc_macro2::Span::call_site(), "missing pipeline default");
        passes.push(self.plan.or(defaults.plan).ok_or_else(missing)?);
        passes.push(self.ix.or(defaults.ix).ok_or_else(missing)?);
        passes.push(self.lower.or(defaults.lower).ok_or_else(missing)?);
        passes.extend(self.physical);
        passes.push(self.select.or(defaults.select).ok_or_else(missing)?);
        passes.extend(self.structures);
        passes.push(self.represent.or(defaults.represent).ok_or_else(missing)?);
        passes.push(self.stage.or(defaults.stage).ok_or_else(missing)?);
        passes.extend(sequence);
        passes.extend(self.decompose);
        passes.extend(grade);
        passes.push(print.ok_or_else(missing)?);
        Ok(Pipeline { passes })
    }
}
