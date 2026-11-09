//! Typed realization-stack transport, separate from the printed IR grammar.
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::braced;

use crate::{Context, Language, OutputConsumer};

mod kw {
    syn::custom_keyword!(realization);
    syn::custom_keyword!(physical);
    syn::custom_keyword!(structures);
    syn::custom_keyword!(sequence);
    syn::custom_keyword!(ordering);
    syn::custom_keyword!(cq);
    syn::custom_keyword!(vo);
}

/// An annotation parsed and checked against separately supplied upper programs.
///
/// `Base` borrows the upper program, or a tuple of upper-program references.
/// No upper program is cloned or embedded by this contract.
/// Authored streams parse upper programs first, then pass their references here.
/// `check` validates the actual seam; `check_rewrite` receives both source and
/// target bases so it can check the rewrite without an embedded upper program.
pub trait Annotation<Base>: ToTokens {
    fn parse(input: ParseStream<'_>, base: Base) -> syn::Result<Self>
    where
        Self: Sized;
    fn check(&self, base: Base) -> syn::Result<()>;
    fn check_rewrite(
        source_base: Base,
        source: &Self,
        target_base: Base,
        target: &Self,
    ) -> syn::Result<()>;
    fn output_consumer(&self) -> Option<&OutputConsumer> {
        None
    }
}

/// CQ and its valuation annotation, carried as separately printed programs.
pub struct Valuation<C, V> {
    pub cq: C,
    pub valuation: V,
}

impl<C, V> Valuation<C, V> {
    pub fn new(cq: C, valuation: V) -> Self {
        Self { cq, valuation }
    }
}

impl<C: Language, V: for<'a> Annotation<&'a C>> Valuation<C, V> {
    pub fn pass<Out: Language>(
        input: TokenStream,
        f: impl FnOnce(&C, &V, TokenStream) -> syn::Result<Out>,
    ) -> TokenStream {
        crate::pass::<Self, Out>(input, |source, args| {
            f(&source.cq, &source.valuation, args)
        })
    }

    pub fn rewrite(
        input: TokenStream,
        f: impl FnOnce(&C, &V, TokenStream) -> syn::Result<(C, V)>,
    ) -> TokenStream {
        crate::rewrite::<Self>(input, |source, args| {
            let (cq, valuation) = f(&source.cq, &source.valuation, args)?;
            Ok(Self::new(cq, valuation))
        })
    }
}

impl<C: Parse, V: for<'a> Annotation<&'a C>> Parse for Valuation<C, V> {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::ordering) {
            let cq = input.parse()?;
            let valuation = V::parse(input, &cq)?;
            return Ok(Self::new(cq, valuation));
        }
        input.parse::<kw::ordering>()?;
        let body;
        braced!(body in input);
        body.parse::<kw::cq>()?;
        let cq = program(&body)?;
        body.parse::<kw::vo>()?;
        let annotation;
        braced!(annotation in body);
        let valuation = V::parse(&annotation, &cq)?;
        if !annotation.is_empty() {
            return Err(annotation.error("unexpected tokens after VO annotation"));
        }
        if !body.is_empty() {
            return Err(body.error("expected a CQ + VO ordering stack"));
        }
        Ok(Self::new(cq, valuation))
    }
}

impl<C: ToTokens, V: ToTokens> ToTokens for Valuation<C, V> {
    fn to_tokens(&self, output: &mut TokenStream) {
        let Self { cq, valuation } = self;
        output.extend(quote!(ordering { cq { #cq } vo { #valuation } }));
    }
}

impl<C: Language, V: for<'a> Annotation<&'a C>> Language for Valuation<C, V> {
    fn check(&self) -> syn::Result<()> {
        self.cq.check()?;
        self.valuation.check(&self.cq)
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        V::check_rewrite(&source.cq, &source.valuation, &target.cq, &target.valuation)
    }
    fn context(&self) -> Context { self.cq.context() }
    fn program_tokens(&self) -> TokenStream { self.valuation.to_token_stream() }
    fn calibration_stage(&self) -> crate::calibration::Stage {
        crate::calibration::Stage::annotation(
            &self.valuation,
            vec![self.cq.to_token_stream().to_string()],
        )
    }
}

/// Physical and its Structures annotation, printed independently in continuations.
/// Authored entry consumes Physical followed by the Structures annotation.
pub struct Structures<P, S> {
    pub physical: P,
    pub structures: S,
}

impl<P, S> Structures<P, S> {
    pub fn new(physical: P, structures: S) -> Self {
        Self { physical, structures }
    }
}

impl<P: Language, S: for<'a> Annotation<&'a P>> Structures<P, S> {
    /// A consumer receives each program it can read as a separate argument.
    pub fn pass<Out: Language>(
        input: TokenStream,
        f: impl FnOnce(&P, &S, TokenStream) -> syn::Result<Out>,
    ) -> TokenStream {
        crate::pass::<Self, Out>(input, |source, args| {
            f(&source.physical, &source.structures, args)
        })
    }

    /// Return both programs, including any changes to the upper program.
    pub fn rewrite(
        input: TokenStream,
        f: impl FnOnce(&P, &S, TokenStream) -> syn::Result<(P, S)>,
    ) -> TokenStream {
        crate::rewrite::<Self>(input, |source, args| {
            let (physical, structures) = f(&source.physical, &source.structures, args)?;
            Ok(Self::new(physical, structures))
        })
    }
}

impl<P: Parse, S: for<'a> Annotation<&'a P>> Parse for Structures<P, S> {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::realization) {
            // Authored IR entry; a checked continuation always uses the envelope.
            let physical = input.parse()?;
            let structures = S::parse(input, &physical)?;
            return Ok(Self::new(physical, structures));
        }
        input.parse::<kw::realization>()?;
        let body;
        braced!(body in input);
        body.parse::<kw::physical>()?;
        let physical = program(&body)?;
        body.parse::<kw::structures>()?;
        let annotations;
        braced!(annotations in body);
        let structures = S::parse(&annotations, &physical)?;
        if !annotations.is_empty() {
            return Err(annotations.error("unexpected tokens after Structures annotation"));
        }
        if !body.is_empty() {
            return Err(body.error("expected a Physical + Structures realization stack"));
        }
        Ok(Self::new(physical, structures))
    }
}

impl<P: ToTokens, S: ToTokens> ToTokens for Structures<P, S> {
    fn to_tokens(&self, output: &mut TokenStream) {
        let Self { physical, structures } = self;
        output.extend(quote!(realization { physical { #physical } structures { #structures } }));
    }
}

impl<P: Language, S: for<'a> Annotation<&'a P>> Language for Structures<P, S> {
    fn check(&self) -> syn::Result<()> {
        // Formal rule ST-Realization: annotation references resolve in the supplied upper program.
        self.physical.check()?;
        self.structures.check(&self.physical)
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        S::check_rewrite(
            &source.physical,
            &source.structures,
            &target.physical,
            &target.structures,
        )
    }
    fn context(&self) -> Context { self.physical.context() }
    fn output_consumer(&self) -> Option<&OutputConsumer> { self.structures.output_consumer() }
    fn program_tokens(&self) -> TokenStream { self.structures.to_token_stream() }
    fn calibration_stage(&self) -> crate::calibration::Stage {
        crate::calibration::Stage::annotation(
            &self.structures,
            vec![self.physical.to_token_stream().to_string()],
        )
    }
}

/// Physical, Structures and the Sequence schedule, all separate programs.
/// Authored entry consumes Physical, Structures against Physical, and Sequence
/// against both preceding programs, in that order.
pub struct Sequence<P, S, Q> {
    pub physical: P,
    pub structures: S,
    pub sequence: Q,
}

impl<P, S, Q> Sequence<P, S, Q> {
    pub fn new(physical: P, structures: S, sequence: Q) -> Self {
        Self { physical, structures, sequence }
    }
}

impl<P, S, Q> Sequence<P, S, Q>
where
    P: Language,
    S: for<'a> Annotation<&'a P>,
    Q: for<'a> Annotation<(&'a P, &'a S)>,
{
    pub fn pass<Out: Language>(
        input: TokenStream,
        f: impl FnOnce(&P, &S, &Q, TokenStream) -> syn::Result<Out>,
    ) -> TokenStream {
        crate::pass::<Self, Out>(input, |source, args| {
            f(&source.physical, &source.structures, &source.sequence, args)
        })
    }

    pub fn rewrite(
        input: TokenStream,
        f: impl FnOnce(&P, &S, &Q, TokenStream) -> syn::Result<(P, S, Q)>,
    ) -> TokenStream {
        crate::rewrite::<Self>(input, |source, args| {
            let (physical, structures, sequence) =
                f(&source.physical, &source.structures, &source.sequence, args)?;
            Ok(Self::new(physical, structures, sequence))
        })
    }
}

impl<P, S, Q> Parse for Sequence<P, S, Q>
where
    P: Parse,
    S: for<'a> Annotation<&'a P>,
    Q: for<'a> Annotation<(&'a P, &'a S)>,
{
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::realization) {
            let physical = input.parse()?;
            let structures = S::parse(input, &physical)?;
            let sequence = Q::parse(input, (&physical, &structures))?;
            return Ok(Self::new(physical, structures, sequence));
        }
        input.parse::<kw::realization>()?;
        let body;
        braced!(body in input);
        body.parse::<kw::physical>()?;
        let physical = program(&body)?;
        body.parse::<kw::structures>()?;
        let annotations;
        braced!(annotations in body);
        let structures = S::parse(&annotations, &physical)?;
        if !annotations.is_empty() {
            return Err(annotations.error("unexpected tokens after Structures annotation"));
        }
        body.parse::<kw::sequence>()?;
        let annotations;
        braced!(annotations in body);
        let sequence = Q::parse(&annotations, (&physical, &structures))?;
        if !annotations.is_empty() {
            return Err(annotations.error("unexpected tokens after Sequence annotation"));
        }
        if !body.is_empty() {
            return Err(body.error("expected a Physical + Structures + Sequence realization stack"));
        }
        Ok(Self::new(physical, structures, sequence))
    }
}

impl<P: ToTokens, S: ToTokens, Q: ToTokens> ToTokens for Sequence<P, S, Q> {
    fn to_tokens(&self, output: &mut TokenStream) {
        let Self { physical, structures, sequence } = self;
        output.extend(quote!(realization {
            physical { #physical } structures { #structures } sequence { #sequence }
        }));
    }
}

impl<P, S, Q> Language for Sequence<P, S, Q>
where
    P: Language,
    S: for<'a> Annotation<&'a P>,
    Q: for<'a> Annotation<(&'a P, &'a S)>,
{
    fn check(&self) -> syn::Result<()> {
        self.physical.check()?;
        self.structures.check(&self.physical)?;
        self.sequence.check((&self.physical, &self.structures))
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        Q::check_rewrite(
            (&source.physical, &source.structures),
            &source.sequence,
            (&target.physical, &target.structures),
            &target.sequence,
        )
    }
    fn context(&self) -> Context { self.physical.context() }
    fn output_consumer(&self) -> Option<&OutputConsumer> { self.sequence.output_consumer() }
    fn program_tokens(&self) -> TokenStream { self.sequence.to_token_stream() }
    fn calibration_stage(&self) -> crate::calibration::Stage {
        crate::calibration::Stage::annotation(
            &self.sequence,
            vec![
                self.physical.to_token_stream().to_string(),
                self.structures.to_token_stream().to_string(),
            ],
        )
    }
}

fn program<L: Parse>(input: ParseStream<'_>) -> syn::Result<L> {
    let body;
    braced!(body in input);
    let program = body.parse()?;
    if !body.is_empty() {
        return Err(body.error("unexpected tokens after realization program"));
    }
    Ok(program)
}

