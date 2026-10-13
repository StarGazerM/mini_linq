//! Checked token-stream pass protocol.
//!
//! A pass receives `pipeline { Path(arguments), Path, ... }; <program>`.
//! [`pass`] parses and checks its source, calls the transformation with the
//! current pass's opaque argument tokens, checks the target, and invokes the
//! next macro. Arguments are delivered as `arguments { ... };` before the
//! pipeline. [`rewrite`] additionally checks language correspondence.
//!
//! The facade accepts the named configuration below. This crate knows phase
//! names, but no pass names. Omitted single phases use caller-supplied defaults;
//! omitted pass lists are empty.
//! ```text
//! pipeline {
//!   plan = Path[(tokens)]; ix = Path; lower = Path;
//!   physical = [Path[(tokens)], ...]; select = Path;
//!   structures = [Path[(tokens)], ...]; print = Path;
//!   surface = [Path[(tokens)], ...];
//! }
//! ```
//! Planning options use that same invocation argument slot, for example
//! `plan = mini_linq::passes::plan_written(no_reduce);` opts out of the facade's
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

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Token, braced, parenthesized};

mod kw {
    syn::custom_keyword!(pipeline);
    syn::custom_keyword!(arguments);
}

/// Public contract implemented by each IR language. Rust expressions terminate
/// the pipeline; raw token streams are used only by surface expansion passes.
pub trait Language: Parse + ToTokens {
    fn check(&self) -> syn::Result<()>;
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()>;
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
    pub fn start(mut self, program: TokenStream) -> syn::Result<TokenStream> {
        if self.passes.is_empty() {
            return Err(syn::Error::new_spanned(
                program,
                "pipeline requires a final printer",
            ));
        }
        let next = self.passes.remove(0);
        let path = next.path;
        let args = next.arguments;
        Ok(if args.is_empty() {
            quote!(#path! { #self ; #program })
        } else {
            quote!(#path! { arguments { #args }; #self ; #program })
        })
    }
}

struct Input<L> {
    args: TokenStream,
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
        let pipeline = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self {
            args,
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
    run(input, f, |_, _| Ok(()))
}

/// Run an L → L pass and the language's exported correspondence check.
pub fn rewrite<L: Language>(
    input: TokenStream,
    f: impl FnOnce(&L, TokenStream) -> syn::Result<L>,
) -> TokenStream {
    run(input, f, L::check_rewrite)
}

fn run<In: Language, Out: Language>(
    input: TokenStream,
    f: impl FnOnce(&In, TokenStream) -> syn::Result<Out>,
    correspondence: impl FnOnce(&In, &Out) -> syn::Result<()>,
) -> TokenStream {
    let result = (|| {
        let input: Input<In> = syn::parse2(input)?;
        input.program.check()?;
        let target = f(&input.program, input.args)?;
        target.check()?;
        correspondence(&input.program, &target)?;
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
            input.pipeline.start(target.to_token_stream())
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
                "print" => config.print = Some(contents.parse()?),
                "physical" | "structures" | "surface" => {
                    let list;
                    syn::bracketed!(list in contents);
                    let entries = list
                        .parse_terminated(Invocation::parse, Token![,])?
                        .into_iter()
                        .collect();
                    match name.to_string().as_str() {
                        "physical" => config.physical = entries,
                        "structures" => config.structures = entries,
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
        let mut passes = self.surface;
        let missing =
            || syn::Error::new(proc_macro2::Span::call_site(), "missing pipeline default");
        passes.push(self.plan.or(defaults.plan).ok_or_else(missing)?);
        passes.push(self.ix.or(defaults.ix).ok_or_else(missing)?);
        passes.push(self.lower.or(defaults.lower).ok_or_else(missing)?);
        passes.extend(self.physical);
        passes.push(self.select.or(defaults.select).ok_or_else(missing)?);
        passes.extend(self.structures);
        passes.push(self.print.or(defaults.print).ok_or_else(missing)?);
        Ok(Pipeline { passes })
    }
}

/// Legacy single-stage library composition, independent of the pipeline facade.
pub fn stage<S: Parse, T: ToTokens>(
    input: TokenStream,
    next: Option<syn::Path>,
    lower: impl FnOnce(&S) -> syn::Result<T>,
) -> TokenStream {
    (|| {
        let source = syn::parse2(input)?;
        let target = lower(&source)?;
        Ok(match next {
            Some(next) => quote!(#next! { #target }),
            None => target.to_token_stream(),
        })
    })()
    .unwrap_or_else(syn::Error::into_compile_error)
}
