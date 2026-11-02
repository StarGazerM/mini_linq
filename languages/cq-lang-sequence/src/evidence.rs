//! Planning estimates and explicitly trusted relational key premises.
//!
//! ```text
//! evidence from authored("schema contract and workload estimates") {
//!     count R = 100;
//!     planning_key R [id];
//!     trusted_key R [id];
//!     foreign_key S [rid] -> R [id];
//!     selectivity filtered = 1 / 10;
//!     bounds R.id = -5 ..= 100;
//!     sorted R [id];
//! }
//! ```
//! Every fact retains its source when mapped or appended. Counts, planning
//! keys, and other estimates are advice, not correctness certificates.
//! Only an explicitly authored `trusted_key` asserts uniqueness; a false
//! trusted premise may change answers when canonicalization is eliminated.

use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitInt, LitStr, Token, braced, bracketed, parenthesized};

use crate::{Analysis, Program};

pub(crate) mod kw {
    syn::custom_keyword!(evidence);
    syn::custom_keyword!(count);
    syn::custom_keyword!(planning_key);
    syn::custom_keyword!(trusted_key);
    syn::custom_keyword!(from);
    syn::custom_keyword!(authored);
    syn::custom_keyword!(vo);
    syn::custom_keyword!(foreign_key);
    syn::custom_keyword!(selectivity);
    syn::custom_keyword!(bounds);
    syn::custom_keyword!(sorted);
}

/// Ordered, sourced estimates and explicitly trusted authored key premises.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub facts: Vec<EvidenceFact>,
}

/// Original evidence owner, preserved across renaming and composition.
/// Runtime build publications have separate typed references, not these labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceSource {
    Authored { provenance: LitStr },
    Vo { provenance: LitStr },
}

impl EvidenceSource {
    pub fn provenance(&self) -> &LitStr {
        match self {
            Self::Authored { provenance } | Self::Vo { provenance } => provenance,
        }
    }

    fn check(&self) -> syn::Result<()> {
        if self.provenance().value().trim().is_empty() {
            return Err(syn::Error::new_spanned(
                self.provenance(),
                "evidence: source provenance must be nonempty",
            ));
        }
        Ok(())
    }
}

/// An exact rational estimate, not a proven filtering ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selectivity {
    pub numerator: u64,
    pub denominator: u64,
}

/// Only TrustedKey is a semantic premise; every other variant is planning advice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceFact {
    Count {
        source: EvidenceSource,
        subject: Ident,
        rows: usize,
    },
    PlanningKey {
        source: EvidenceSource,
        subject: Ident,
        columns: Vec<Ident>,
    },
    TrustedKey {
        source: EvidenceSource,
        subject: Ident,
        columns: Vec<Ident>,
    },
    ForeignKey {
        source: EvidenceSource,
        subject: Ident,
        columns: Vec<Ident>,
        referenced: Ident,
        referenced_columns: Vec<Ident>,
    },
    Selectivity {
        source: EvidenceSource,
        subject: Ident,
        ratio: Selectivity,
    },
    Bounds {
        source: EvidenceSource,
        subject: Ident,
        column: Ident,
        lower: i128,
        upper: i128,
    },
    Sorted {
        source: EvidenceSource,
        subject: Ident,
        columns: Vec<Ident>,
    },
}

impl Evidence {
    /// Rename evidence subjects without relabeling their original source.
    /// Column headings are unchanged; the resulting references still require WF checking.
    pub fn map_subjects(mut self, mut map: impl FnMut(&Ident) -> Ident) -> Self {
        for fact in &mut self.facts {
            let subject = match fact {
                EvidenceFact::Count { subject, .. }
                | EvidenceFact::PlanningKey { subject, .. }
                | EvidenceFact::TrustedKey { subject, .. }
                | EvidenceFact::ForeignKey { subject, .. }
                | EvidenceFact::Selectivity { subject, .. }
                | EvidenceFact::Bounds { subject, .. }
                | EvidenceFact::Sorted { subject, .. } => subject,
            };
            *subject = map(subject);
            if let EvidenceFact::ForeignKey { referenced, .. } = fact {
                *referenced = map(referenced);
            }
        }
        self
    }

    /// Last recorded count for this subject; absence remains unknown.
    pub fn count(&self, subject: &str) -> Option<usize> {
        self.facts.iter().rev().find_map(|fact| match fact {
            EvidenceFact::Count {
                subject: name,
                rows,
                ..
            } if symbol_name(name) == subject => Some(*rows),
            _ => None,
        })
    }

    /// Last recorded ratio for this subject; even a false ratio is just advice.
    pub fn selectivity(&self, subject: &str) -> Option<Selectivity> {
        self.facts.iter().rev().find_map(|fact| match fact {
            EvidenceFact::Selectivity {
                subject: name,
                ratio,
                ..
            } if symbol_name(name) == subject => Some(*ratio),
            _ => None,
        })
    }

    /// Whether this exact column set has an explicitly trusted authored premise.
    pub fn has_trusted_key(&self, subject: &str, columns: &[String]) -> bool {
        self.facts.iter().any(|fact| match fact {
            EvidenceFact::TrustedKey {
                subject: name,
                columns: key,
                ..
            } => {
                symbol_name(name) == subject
                    && key.len() == columns.len()
                    && key
                        .iter()
                        .all(|column| columns.contains(&symbol_name(column)))
            }
            _ => false,
        })
    }

    /// Planning keys never establish uniqueness for a semantic rewrite.
    pub fn has_planning_key(&self, subject: &str, columns: &[String]) -> bool {
        self.facts.iter().any(|fact| match fact {
            EvidenceFact::PlanningKey {
                subject: name,
                columns: key,
                ..
            } => {
                symbol_name(name) == subject
                    && key.len() == columns.len()
                    && key
                        .iter()
                        .all(|column| columns.contains(&symbol_name(column)))
            }
            _ => false,
        })
    }

    /// Check grammar invariants and references, never the truth of a fact.
    // Formal rule WF-SequenceEvidence: Require structured nonempty provenance and valid references; VO planning advice never supplies a trusted semantic key premise.
    pub(crate) fn check(&self, program: &Program, analysis: &Analysis) -> syn::Result<()> {
        for fact in &self.facts {
            fact.source().check()?;
            if matches!(
                fact,
                EvidenceFact::TrustedKey {
                    source: EvidenceSource::Vo { .. },
                    ..
                }
            ) {
                return Err(syn::Error::new_spanned(
                    fact,
                    "evidence: VO planning advice cannot supply a trusted key premise",
                ));
            }
            let subject = match fact {
                EvidenceFact::Count { subject, .. }
                | EvidenceFact::PlanningKey { subject, .. }
                | EvidenceFact::TrustedKey { subject, .. }
                | EvidenceFact::ForeignKey { subject, .. }
                | EvidenceFact::Selectivity { subject, .. }
                | EvidenceFact::Bounds { subject, .. }
                | EvidenceFact::Sorted { subject, .. } => subject,
            };
            check_subject(program, analysis, subject)?;
            match fact {
                EvidenceFact::PlanningKey { columns, .. }
                | EvidenceFact::TrustedKey { columns, .. }
                | EvidenceFact::Sorted { columns, .. } => {
                    check_columns(program, analysis, subject, columns)?;
                }
                EvidenceFact::ForeignKey {
                    columns,
                    referenced,
                    referenced_columns,
                    ..
                } => {
                    if columns.len() != referenced_columns.len() {
                        return Err(syn::Error::new_spanned(
                            subject,
                            "evidence: foreign key column arities differ",
                        ));
                    }
                    check_subject(program, analysis, referenced)?;
                    check_columns(program, analysis, subject, columns)?;
                    check_columns(program, analysis, referenced, referenced_columns)?;
                }
                EvidenceFact::Bounds { column, .. } => {
                    check_columns(program, analysis, subject, std::slice::from_ref(column))?;
                }
                EvidenceFact::Selectivity { ratio, .. } if ratio.denominator == 0 => {
                    return Err(syn::Error::new_spanned(
                        subject,
                        "evidence: selectivity denominator must be nonzero",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn check_subject(program: &Program, analysis: &Analysis, subject: &Ident) -> syn::Result<()> {
    let name = symbol_name(subject);
    if analysis.lengths.contains_key(&name)
        || analysis.structures.physical.headings.contains_key(&name)
        || program
            .structures
            .physical
            .program
            .declarations
            .iter()
            .any(|relation| symbol_name(&relation.name) == name)
    {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            subject,
            "evidence: unknown subject",
        ))
    }
}

fn check_columns(
    program: &Program,
    analysis: &Analysis,
    subject: &Ident,
    columns: &[Ident],
) -> syn::Result<()> {
    let name = symbol_name(subject);
    // Sequence names take precedence, matching term reference resolution.
    let result = analysis
        .results
        .get(&name)
        .map(String::as_str)
        .unwrap_or(&name);
    for (index, column) in columns.iter().enumerate() {
        let column_name = symbol_name(column);
        if columns[..index]
            .iter()
            .any(|previous| symbol_name(previous) == column_name)
        {
            return Err(syn::Error::new_spanned(
                column,
                "evidence: duplicate column",
            ));
        }
        let present =
            if analysis.lengths.contains_key(&name) && !analysis.results.contains_key(&name) {
                false
            } else if let Some(heading) = analysis.structures.physical.headings.get(result) {
                heading.contains(&column_name)
            } else {
                program
                    .structures
                    .physical
                    .program
                    .declarations
                    .iter()
                    .find(|relation| symbol_name(&relation.name) == result)
                    .is_some_and(|relation| {
                        relation
                            .columns
                            .iter()
                            .any(|candidate| symbol_name(&candidate.name) == column_name)
                    })
            };
        if !present {
            return Err(syn::Error::new_spanned(
                column,
                "evidence: unknown subject column",
            ));
        }
    }
    Ok(())
}

fn columns(input: ParseStream<'_>) -> syn::Result<Vec<Ident>> {
    let body;
    bracketed!(body in input);
    Ok(Punctuated::<Ident, Token![,]>::parse_terminated(&body)?
        .into_iter()
        .collect())
}

fn signed_integer(input: ParseStream<'_>) -> syn::Result<i128> {
    let negative = input.peek(Token![-]);
    if negative {
        input.parse::<Token![-]>()?;
    }
    let literal: LitInt = input.parse()?;
    let magnitude: u128 = literal.base10_parse()?;
    if negative && magnitude == (1u128 << 127) {
        return Ok(i128::MIN);
    }
    let value = i128::try_from(magnitude).map_err(|_| {
        syn::Error::new_spanned(literal, "evidence bound exceeds signed 128-bit range")
    })?;
    Ok(if negative { -value } else { value })
}

impl Parse for EvidenceSource {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        let authored = if lookahead.peek(kw::authored) {
            input.parse::<kw::authored>()?;
            true
        } else if lookahead.peek(kw::vo) {
            input.parse::<kw::vo>()?;
            false
        } else {
            return Err(lookahead.error());
        };
        let body;
        parenthesized!(body in input);
        let provenance = body.parse()?;
        if !body.is_empty() {
            return Err(body.error("evidence: expected one provenance string"));
        }
        let source = if authored {
            Self::Authored { provenance }
        } else {
            Self::Vo { provenance }
        };
        source.check()?;
        Ok(source)
    }
}

impl ToTokens for EvidenceSource {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Authored { provenance } => quote!(authored(#provenance)),
            Self::Vo { provenance } => quote!(vo(#provenance)),
        });
    }
}

impl Parse for Evidence {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::evidence>()?;
        let source = if input.peek(kw::from) {
            input.parse::<kw::from>()?;
            Some(input.parse::<EvidenceSource>()?)
        } else {
            None
        };
        let body;
        braced!(body in input);
        let mut facts = Vec::new();
        while !body.is_empty() {
            facts.push(EvidenceFact::parse_with_source(&body, source.as_ref())?);
        }
        Ok(Self { facts })
    }
}

impl EvidenceFact {
    pub fn source(&self) -> &EvidenceSource {
        match self {
            Self::Count { source, .. }
            | Self::PlanningKey { source, .. }
            | Self::TrustedKey { source, .. }
            | Self::ForeignKey { source, .. }
            | Self::Selectivity { source, .. }
            | Self::Bounds { source, .. }
            | Self::Sorted { source, .. } => source,
        }
    }

    fn parse_with_source(
        input: ParseStream<'_>,
        inherited: Option<&EvidenceSource>,
    ) -> syn::Result<Self> {
        let source = if input.peek(kw::from) {
            input.parse::<kw::from>()?;
            input.parse()?
        } else {
            inherited
                .cloned()
                .ok_or_else(|| input.error("evidence: fact requires an explicit source"))?
        };
        let lookahead = input.lookahead1();
        let fact = if lookahead.peek(kw::count) {
            input.parse::<kw::count>()?;
            let subject = input.parse()?;
            input.parse::<Token![=]>()?;
            let rows = input.parse::<LitInt>()?.base10_parse()?;
            Self::Count {
                source,
                subject,
                rows,
            }
        } else if lookahead.peek(kw::planning_key) || lookahead.peek(kw::trusted_key) {
            let trusted = input.peek(kw::trusted_key);
            if trusted {
                input.parse::<kw::trusted_key>()?;
            } else {
                input.parse::<kw::planning_key>()?;
            }
            let subject = input.parse()?;
            let columns = columns(input)?;
            if trusted {
                Self::TrustedKey {
                    source,
                    subject,
                    columns,
                }
            } else {
                Self::PlanningKey {
                    source,
                    subject,
                    columns,
                }
            }
        } else if lookahead.peek(kw::foreign_key) {
            input.parse::<kw::foreign_key>()?;
            let subject = input.parse()?;
            let columns = columns(input)?;
            input.parse::<Token![->]>()?;
            let referenced = input.parse()?;
            let referenced_columns = self::columns(input)?;
            if columns.len() != referenced_columns.len() {
                return Err(input.error("evidence: foreign key column arities differ"));
            }
            Self::ForeignKey {
                source,
                subject,
                columns,
                referenced,
                referenced_columns,
            }
        } else if lookahead.peek(kw::selectivity) {
            input.parse::<kw::selectivity>()?;
            let subject = input.parse()?;
            input.parse::<Token![=]>()?;
            let numerator = input.parse::<LitInt>()?.base10_parse()?;
            input.parse::<Token![/]>()?;
            let denominator = input.parse::<LitInt>()?.base10_parse()?;
            if denominator == 0 {
                return Err(input.error("evidence: selectivity denominator must be nonzero"));
            }
            Self::Selectivity {
                source,
                subject,
                ratio: Selectivity {
                    numerator,
                    denominator,
                },
            }
        } else if lookahead.peek(kw::bounds) {
            input.parse::<kw::bounds>()?;
            let subject = input.parse()?;
            input.parse::<Token![.]>()?;
            let column = input.parse()?;
            input.parse::<Token![=]>()?;
            let lower = signed_integer(input)?;
            input.parse::<Token![..=]>()?;
            let upper = signed_integer(input)?;
            Self::Bounds {
                source,
                subject,
                column,
                lower,
                upper,
            }
        } else if lookahead.peek(kw::sorted) {
            input.parse::<kw::sorted>()?;
            let subject = input.parse()?;
            let columns = columns(input)?;
            Self::Sorted {
                source,
                subject,
                columns,
            }
        } else {
            return Err(lookahead.error());
        };
        input.parse::<Token![;]>()?;
        Ok(fact)
    }
}

impl Parse for EvidenceFact {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Self::parse_with_source(input, None)
    }
}

impl ToTokens for Evidence {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let facts = &self.facts;
        tokens.extend(quote! { evidence { #(#facts)* } });
    }
}

impl ToTokens for EvidenceFact {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let source = self.source();
        tokens.extend(quote!(from #source));
        tokens.extend(match self {
            Self::Count { subject, rows, .. } => quote! { count #subject = #rows; },
            Self::PlanningKey { subject, columns, .. } => quote! { planning_key #subject [#(#columns),*]; },
            Self::TrustedKey { subject, columns, .. } => quote! { trusted_key #subject [#(#columns),*]; },
            Self::ForeignKey { subject, columns, referenced, referenced_columns, .. } => {
                quote! { foreign_key #subject [#(#columns),*] -> #referenced [#(#referenced_columns),*]; }
            }
            Self::Selectivity { subject, ratio: Selectivity { numerator, denominator }, .. } => {
                quote! { selectivity #subject = #numerator / #denominator; }
            }
            Self::Bounds { subject, column, lower, upper, .. } => quote! { bounds #subject.#column = #lower ..= #upper; },
            Self::Sorted { subject, columns, .. } => quote! { sorted #subject [#(#columns),*]; },
        });
    }
}
