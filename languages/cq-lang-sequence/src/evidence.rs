//! Planning estimates and explicitly asserted relational key premises.
//!
//! ```text
//! evidence {
//!     count R = 100;
//!     key R [id];
//!     foreign_key S [rid] -> R [id];
//!     selectivity filtered = 1 / 10;
//!     bounds R.id = -5 ..= 100;
//!     sorted R [id];
//! }
//! ```
//! Subjects may name declared relations, Physical results, or Sequence results.
//! Column references use the subject's heading. Counts and selectivities also
//! accept scalar Sequence definitions. Contradictory estimates are retained;
//! scalar consumers use the last authored fact. A key is an author assertion:
//! false keys may change grouped answers when canonicalization is eliminated.

use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitInt, Token, braced, bracketed, parenthesized};

use crate::{Analysis, Program};

pub(crate) mod kw {
    syn::custom_keyword!(evidence);
    syn::custom_keyword!(count);
    syn::custom_keyword!(key);
    syn::custom_keyword!(foreign_key);
    syn::custom_keyword!(selectivity);
    syn::custom_keyword!(bounds);
    syn::custom_keyword!(sorted);
}

/// Ordered estimates and author-asserted keys, recorded with checked analysis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub facts: Vec<EvidenceFact>,
}

/// An exact rational estimate, not a proven filtering ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selectivity {
    pub numerator: u64,
    pub denominator: u64,
}

/// Keys assert unique physical rows per key; all other facts remain estimates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceFact {
    Count {
        subject: Ident,
        rows: usize,
    },
    Key {
        subject: Ident,
        columns: Vec<Ident>,
    },
    ForeignKey {
        subject: Ident,
        columns: Vec<Ident>,
        referenced: Ident,
        referenced_columns: Vec<Ident>,
    },
    Selectivity {
        subject: Ident,
        ratio: Selectivity,
    },
    Bounds {
        subject: Ident,
        column: Ident,
        lower: i128,
        upper: i128,
    },
    Sorted {
        subject: Ident,
        columns: Vec<Ident>,
    },
}

impl Evidence {
    /// Last recorded count for this subject; absence remains unknown.
    pub fn count(&self, subject: &str) -> Option<usize> {
        self.facts.iter().rev().find_map(|fact| match fact {
            EvidenceFact::Count {
                subject: name,
                rows,
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
            } if symbol_name(name) == subject => Some(*ratio),
            _ => None,
        })
    }

    /// Whether this exact column set has an author-asserted key premise.
    pub fn has_key(&self, subject: &str, columns: &[String]) -> bool {
        self.facts.iter().any(|fact| match fact {
            EvidenceFact::Key {
                subject: name,
                columns: key,
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
    pub(crate) fn check(&self, program: &Program, analysis: &Analysis) -> syn::Result<()> {
        for fact in &self.facts {
            let subject = match fact {
                EvidenceFact::Count { subject, .. }
                | EvidenceFact::Key { subject, .. }
                | EvidenceFact::ForeignKey { subject, .. }
                | EvidenceFact::Selectivity { subject, .. }
                | EvidenceFact::Bounds { subject, .. }
                | EvidenceFact::Sorted { subject, .. } => subject,
            };
            check_subject(program, analysis, subject)?;
            match fact {
                EvidenceFact::Key { columns, .. } | EvidenceFact::Sorted { columns, .. } => {
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

impl Parse for Evidence {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::evidence>()?;
        let body;
        braced!(body in input);
        let mut facts = Vec::new();
        while !body.is_empty() {
            facts.push(body.parse()?);
        }
        Ok(Self { facts })
    }
}

impl Parse for EvidenceFact {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        let fact = if lookahead.peek(kw::count) {
            input.parse::<kw::count>()?;
            let subject = input.parse()?;
            input.parse::<Token![=]>()?;
            Self::Count {
                subject,
                rows: input.parse::<LitInt>()?.base10_parse()?,
            }
        } else if lookahead.peek(kw::key) {
            input.parse::<kw::key>()?;
            Self::Key {
                subject: input.parse()?,
                columns: columns(input)?,
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
                subject,
                column,
                lower,
                upper,
            }
        } else if lookahead.peek(kw::sorted) {
            input.parse::<kw::sorted>()?;
            Self::Sorted {
                subject: input.parse()?,
                columns: columns(input)?,
            }
        } else if lookahead.peek(Ident) {
            let subject = input.parse()?;
            let body;
            parenthesized!(body in input);
            let columns = Punctuated::<Ident, Token![,]>::parse_terminated(&body)?
                .into_iter()
                .collect();
            input.parse::<kw::key>()?;
            Self::Key { subject, columns }
        } else {
            return Err(lookahead.error());
        };
        input.parse::<Token![;]>()?;
        Ok(fact)
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
        tokens.extend(match self {
            Self::Count { subject, rows } => quote! { count #subject = #rows; },
            Self::Key { subject, columns } => quote! { key #subject [#(#columns),*]; },
            Self::ForeignKey { subject, columns, referenced, referenced_columns } => {
                quote! { foreign_key #subject [#(#columns),*] -> #referenced [#(#referenced_columns),*]; }
            }
            Self::Selectivity { subject, ratio: Selectivity { numerator, denominator } } => {
                quote! { selectivity #subject = #numerator / #denominator; }
            }
            Self::Bounds { subject, column, lower, upper } => quote! { bounds #subject.#column = #lower ..= #upper; },
            Self::Sorted { subject, columns } => quote! { sorted #subject [#(#columns),*]; },
        });
    }
}
