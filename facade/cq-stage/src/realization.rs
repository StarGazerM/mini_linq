//! Typed realization-stack transport, separate from the printed IR grammar.
use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::braced;

use crate::{Context, Language, OutputConsumer};

mod kw {
    syn::custom_keyword!(realization);
    syn::custom_keyword!(physical);
    syn::custom_keyword!(structures);
    syn::custom_keyword!(sequence);
}

/// An annotation parsed and checked against separately supplied upper programs.
///
/// `Base` borrows the upper program, or a tuple of upper-program references.
/// No upper program is cloned or embedded by this contract.
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

/// Physical and its Structures annotation, printed independently in continuations.
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

impl<P: Language, S: Language, Q: Language> Sequence<P, S, Q> {
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

impl<P: Parse, S: Parse, Q: Parse> Parse for Sequence<P, S, Q> {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::realization) {
            let physical = input.fork().parse()?;
            let structures = input.fork().parse()?;
            return Ok(Self::new(physical, structures, input.parse()?));
        }
        input.parse::<kw::realization>()?;
        let body;
        braced!(body in input);
        body.parse::<kw::physical>()?;
        let physical = program(&body)?;
        body.parse::<kw::structures>()?;
        let structures = program(&body)?;
        body.parse::<kw::sequence>()?;
        let sequence = program(&body)?;
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

impl<P: Language, S: Language, Q: Language> Language for Sequence<P, S, Q> {
    fn check(&self) -> syn::Result<()> {
        check_embedding(&self.physical, &self.structures)?;
        check_embedding(&self.structures, &self.sequence)?;
        self.sequence.check()
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        Q::check_rewrite(&source.sequence, &target.sequence)
    }
    fn context(&self) -> Context { self.sequence.context() }
    fn output_consumer(&self) -> Option<&OutputConsumer> { self.sequence.output_consumer() }
    fn program_tokens(&self) -> TokenStream { self.sequence.to_token_stream() }
    fn calibration_stage(&self) -> crate::calibration::Stage {
        crate::calibration::Stage::of(&self.sequence)
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

// Sequence's temporary prefix bridge is removed by the Sequence de-embedding.
// Comparing token trees ignores spans, not syntax.
fn check_embedding(upper: &impl ToTokens, lower: &impl ToTokens) -> syn::Result<()> {
    let mut lower_tokens = lower.to_token_stream().into_iter();
    if upper.to_token_stream().into_iter().all(|upper| {
        lower_tokens.next().is_some_and(|lower| same_token(upper, lower))
    }) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(lower,
            "ST-Realization: separate program disagrees with the embedded program"))
    }
}

fn same_token(left: TokenTree, right: TokenTree) -> bool {
    match (left, right) {
        (TokenTree::Group(left), TokenTree::Group(right)) => {
            if left.delimiter() != right.delimiter() { return false; }
            let mut left = left.stream().into_iter();
            let mut right = right.stream().into_iter();
            loop {
                match (left.next(), right.next()) {
                    (None, None) => return true,
                    (Some(left), Some(right)) => {
                        if !same_token(left, right) { return false; }
                    }
                    _ => return false,
                }
            }
        }
        (TokenTree::Ident(left), TokenTree::Ident(right)) => left == right,
        (TokenTree::Punct(left), TokenTree::Punct(right)) =>
            left.as_char() == right.as_char() && left.spacing() == right.spacing(),
        (TokenTree::Literal(left), TokenTree::Literal(right)) => left.to_string() == right.to_string(),
        _ => false,
    }
}
