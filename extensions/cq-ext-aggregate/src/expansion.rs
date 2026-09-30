use std::collections::BTreeSet;

use cq_ext_support::{Fresh, ensure, same_symbol};
use cq_lang_cq::Program;
use cq_syntax::{rust, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Ident, Token, parse::Parse};

mod kw {
    syn::custom_keyword!(by);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Count,
    Sum,
    Min,
    Max,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measure {
    pub name: Ident,
    pub operation: Operation,
    pub column: Option<Ident>,
}
impl ToTokens for Measure {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            name,
            operation,
            column,
        } = self;
        let op = Ident::new(
            match operation {
                Operation::Count => "count",
                Operation::Sum => "sum",
                Operation::Min => "min",
                Operation::Max => "max",
            },
            name.span(),
        );
        tokens.extend(quote!(#name = #op(#column);));
    }
}
#[derive(Clone)]
pub struct Source {
    pub keys: Vec<Ident>,
    pub measures: Vec<Measure>,
    pub rows: Program,
}
impl Parse for Source {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::by>()?;
        let keys;
        syn::parenthesized!(keys in input);
        let keys = syn::punctuated::Punctuated::<Ident, Token![,]>::parse_terminated(&keys)?
            .into_iter()
            .collect();
        input.parse::<Token![;]>()?;
        let mut measures = Vec::new();
        while !input.peek(Token![struct]) && !input.peek(Token![pub]) {
            let name = input.parse()?;
            input.parse::<Token![=]>()?;
            let operator: Ident = input.parse()?;
            let operation = match operator.to_string().as_str() {
                "count" => Operation::Count,
                "sum" => Operation::Sum,
                "min" => Operation::Min,
                "max" => Operation::Max,
                _ => {
                    return Err(syn::Error::new_spanned(
                        operator,
                        "expected count, sum, min, or max",
                    ));
                }
            };
            let arguments;
            syn::parenthesized!(arguments in input);
            let column = if operation == Operation::Count {
                None
            } else {
                Some(arguments.parse()?)
            };
            ensure(
                arguments.is_empty(),
                &operator,
                "aggregate argument count mismatch",
            )?;
            input.parse::<Token![;]>()?;
            measures.push(Measure {
                name,
                operation,
                column,
            });
        }
        Ok(Self {
            keys,
            measures,
            rows: input.parse()?,
        })
    }
}
impl ToTokens for Source {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            keys,
            measures,
            rows,
        } = self;
        tokens.extend(quote!(by (#(#keys),*); #(#measures)* #rows));
    }
}
#[derive(Clone)]
pub struct Context {
    pub hole: Program,
    pub keys: Vec<Ident>,
    pub measures: Vec<Measure>,
}
fn domain(source: &Source) -> syn::Result<()> {
    ensure(
        !source.keys.is_empty() && !source.measures.is_empty(),
        source,
        "group-by needs at least one key and measure",
    )?;
    let columns: BTreeSet<_> = source
        .rows
        .query
        .head
        .variables()
        .map(symbol_name)
        .collect();
    ensure(
        columns.len() == source.rows.query.head.args.len(),
        &source.rows.query.head,
        "aggregate rows must have distinct bare columns",
    )?;
    let mut names = BTreeSet::new();
    for key in &source.keys {
        ensure(
            columns.contains(&symbol_name(key)),
            key,
            "group key is absent from the row head",
        )?;
        ensure(names.insert(symbol_name(key)), key, "duplicate group key")?;
    }
    for measure in &source.measures {
        ensure(
            names.insert(symbol_name(&measure.name)),
            measure,
            "duplicate group/measure name",
        )?;
        if let Some(column) = &measure.column {
            ensure(
                columns.contains(&symbol_name(column)),
                column,
                "aggregate column is absent from the row head",
            )?;
        }
    }
    Ok(())
}
pub fn compile(source: &Source) -> syn::Result<Context> {
    domain(source)?;
    Ok(Context {
        hole: source.rows.clone(),
        keys: source.keys.clone(),
        measures: source.measures.clone(),
    })
}
/// The row hole is unchanged; key layout, measure order, operands and operations
/// exactly match the authored fold specification. Rust arithmetic is trusted.
#[cfg_attr(not(test), allow(dead_code))]
pub fn check(source: &Source, target: &Context) -> syn::Result<()> {
    domain(source)?;
    ensure(
        target.hole == source.rows,
        &target.hole,
        "aggregate row hole changed",
    )?;
    ensure(target.keys == source.keys, source, "group layout changed")?;
    ensure(
        target.measures == source.measures,
        source,
        "aggregate fold specification changed",
    )
}
pub fn render(context: &Context) -> TokenStream {
    let Context {
        hole,
        keys,
        measures,
    } = context;
    let columns: Vec<_> = hole
        .query
        .head
        .args
        .iter()
        .filter_map(rust::variable)
        .map(|id| {
            if keys.iter().any(|key| same_symbol(key, id))
                || measures.iter().any(|m| {
                    m.column
                        .as_ref()
                        .is_some_and(|column| same_symbol(column, id))
                })
            {
                quote!(#id)
            } else {
                quote!(_)
            }
        })
        .collect();
    let mut fresh = Fresh::new(quote!(#hole #(#measures)* #(#keys)*));
    let groups = fresh.id("groups");
    let values = fresh.id("measures");
    let key = fresh.id("group_key");
    let entry = fresh.id("group_entry");
    let initial = measures.iter().map(|measure| {
        if let Some(column) = &measure.column {
            quote!(::core::clone::Clone::clone(#column))
        } else {
            quote!(1usize)
        }
    });
    let updates = measures.iter().enumerate().map(|(i, measure)| {
        let index = syn::Index::from(i);
        let column = &measure.column;
        match measure.operation {
            Operation::Count => quote!(#values.#index += 1usize;),
            Operation::Sum => quote!(#values.#index += ::core::clone::Clone::clone(#column);),
            Operation::Min => quote!(if #column < &#values.#index { #values.#index = ::core::clone::Clone::clone(#column); }),
            Operation::Max => quote!(if #column > &#values.#index { #values.#index = ::core::clone::Clone::clone(#column); }),
        }
    });
    quote!({
        let mut #groups = ::std::collections::HashMap::new();
        for (#(#columns,)*) in mini_linq::aggregate_hole! { #hole } {
            let #key = (#(::core::clone::Clone::clone(#keys),)*);
            match #groups.entry(#key) {
                ::std::collections::hash_map::Entry::Vacant(#entry) => { #entry.insert((#(#initial,)*)); }
                ::std::collections::hash_map::Entry::Occupied(mut #entry) => {
                    let #values = #entry.get_mut(); #(#updates)*
                }
            }
        }
        ::cq_storage::GroupedAnswers::collect_from(#groups)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source {
        syn::parse_quote! {
            by (d); n = count(); s = sum(a); lo = min(a); hi = max(a);
            struct G; relation Sale(d:i32,id:i32,a:i32); rows(d,id,a) :- Sale(d,id,a);
        }
    }
    #[test]
    fn correspondence_rejects_row_layout_and_operation_mutations() {
        let source = source();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        let mut changed = target.clone();
        changed.keys = vec![syn::parse_quote!(a)];
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.measures[1].operation = Operation::Max;
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.hole.query.head = syn::parse_quote!(rows(d, a));
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.measures.swap(0, 1);
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn malformed_group_specs_are_rejected() {
        for prefix in [
            "by (); n=count();",
            "by (missing); n=count();",
            "by (d,d); n=count();",
            "by (d); d=count();",
            "by (d); n=sum(missing);",
        ] {
            assert!(
                compile(
                    &syn::parse_str(&format!(
                        "{prefix} struct G; relation S(d:i32,a:i32); rows(d,a) :- S(d,a);"
                    ))
                    .unwrap()
                )
                .is_err()
            );
        }
        assert!(
            syn::parse_str::<Source>("by(d); n=median(a); struct G; rows(d,a) :- S(d,a);").is_err()
        );
    }
}
