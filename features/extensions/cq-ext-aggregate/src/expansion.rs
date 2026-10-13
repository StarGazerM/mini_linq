use std::collections::BTreeSet;

use cq_ext_support::{
    Fresh, Leaf, check_fresh, check_leaves, ensure, hoist, same_symbol,
};
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub hole: Program,
    pub leaves: Vec<Leaf>,
    pub keys: Vec<Ident>,
    pub measures: Vec<Measure>,
    /// Literal-bound group columns prove that at most one group can exist.
    pub constant_group: bool,
    /// Computed head columns are owned; relation columns are borrowed.
    pub owned: BTreeSet<String>,
    /// The grouped fold's hashing policy is part of the checked context.
    pub hasher: syn::Type,
    /// A stock terminal establishes the checked Structures canonical-return fact.
    pub canonical_stream: bool,
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

fn owned_columns(rows: &Program) -> BTreeSet<String> {
    fn collect(body: &cq_syntax::CommaList<cq_lang_cq::BodyItem>, output: &mut BTreeSet<String>) {
        for item in body {
            match item {
                cq_lang_cq::BodyItem::Let(binding) => {
                    output.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                }
                cq_lang_cq::BodyItem::Block(block) => {
                    for branch in &block.branches {
                        collect(&branch.body, output);
                    }
                }
                _ => {}
            }
        }
    }
    let mut output = BTreeSet::new();
    collect(&rows.query.body, &mut output);
    output
}

fn constant_group(source: &Source) -> bool {
    source.keys.iter().all(|key| source.rows.query.body.iter().any(|item| {
        matches!(item, cq_lang_cq::BodyItem::Let(binding)
            if matches!(&binding.pattern, syn::Pat::Ident(pattern) if same_symbol(&pattern.ident, key))
                && matches!(&binding.expression, syn::Expr::Lit(_)))
    }))
}
pub fn compile(source: &Source) -> syn::Result<Context> {
    domain(source)?;
    let (declarations, leaves) = hoist(&source.rows, &mut Fresh::new(source));
    let mut hole = source.rows.clone();
    hole.declarations = declarations;
    Ok(Context {
        hole,
        leaves,
        keys: source.keys.clone(),
        measures: source.measures.clone(),
        constant_group: constant_group(source),
        owned: owned_columns(&source.rows),
        hasher: syn::parse_quote!(::cq_storage::HashState),
        canonical_stream: false,
    })
}
/// Erasing shared leaf bindings restores the row query; key layout, measure
/// order, operands and operations match the authored fold specification.
pub fn check(source: &Source, target: &Context) -> syn::Result<()> {
    domain(source)?;
    check_fresh(source, target.leaves.iter().map(|leaf| &leaf.binding))?;
    check_leaves(&source.rows, &target.hole.declarations, &target.leaves)?;
    let mut erased = target.hole.clone();
    erased.declarations = source.rows.declarations.clone();
    ensure(
        erased == source.rows,
        &target.hole,
        "aggregate row hole changed",
    )?;
    ensure(target.constant_group == constant_group(source), source, "constant-group proof changed")?;
    ensure(target.owned == owned_columns(&source.rows), source, "aggregate ownership proof changed")?;
    ensure(target.hasher == syn::parse_quote!(::cq_storage::HashState), source, "aggregate hashing policy changed")?;
    ensure(target.keys == source.keys, source, "group layout changed")?;
    ensure(
        target.measures == source.measures,
        source,
        "aggregate fold specification changed",
    )
}

fn canonical_stream(pipeline: &cq_stage::Pipeline) -> bool {
    pipeline.passes.last().is_some_and(|pass| {
        let path = pass.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>();
        path == ["mini_linq", "passes", "print_pull"] && pass.arguments.is_empty()
    })
}

pub fn compile_with_pipeline(source: &Source, pipeline: &cq_stage::Pipeline) -> syn::Result<Context> {
    let mut target = compile(source)?;
    target.canonical_stream = canonical_stream(pipeline);
    Ok(target)
}

/// Formal rule EXT-SetFold: Fold the checked canonical core stream exactly once;
/// an unknown terminal retains a fast-hashed set boundary.
/// Formal rule EXT-ConstantGroup: Literal-bound group columns use an optional
/// accumulator, preserving empty-input and set-valued aggregate semantics.
pub fn check_with_pipeline(source: &Source, target: &Context, pipeline: &cq_stage::Pipeline) -> syn::Result<()> {
    check(source, target)?;
    ensure(target.canonical_stream == canonical_stream(pipeline), source, "aggregate canonical-stream proof changed")
}
pub fn render(context: &Context, pipeline: &cq_stage::Pipeline) -> syn::Result<TokenStream> {
    let Context {
        hole,
        leaves,
        keys,
        measures,
        constant_group,
        owned,
        hasher,
        canonical_stream,
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
    let reference = |column: &Ident| {
        if owned.contains(&symbol_name(column)) { quote!(&#column) } else { quote!(#column) }
    };
    let mut fresh = Fresh::new(quote!(#hole #(#measures)* #(#keys)*));
    let groups = fresh.id("groups");
    let values = fresh.id("measures");
    let key = fresh.id("group_key");
    let entry = fresh.id("group_entry");
    let initial = measures.iter().map(|measure| {
        if let Some(column) = &measure.column {
            let column = reference(column);
            quote!(::core::clone::Clone::clone(#column))
        } else {
            quote!(1usize)
        }
    });
    let updates = measures.iter().enumerate().map(|(i, measure)| {
        let index = syn::Index::from(i);
        let column = measure.column.as_ref().map(&reference);
        match measure.operation {
            Operation::Count => quote!(#values.#index += 1usize;),
            Operation::Sum => quote!(#values.#index += ::core::clone::Clone::clone(#column);),
            Operation::Min => quote!(if #column < &#values.#index { #values.#index = ::core::clone::Clone::clone(#column); }),
            Operation::Max => quote!(if #column > &#values.#index { #values.#index = ::core::clone::Clone::clone(#column); }),
        }
    });
    let invocation = pipeline.clone().start(hole.to_token_stream())?;
    // The stock terminal printer checks the Structures canonical-return fact.
    // An arbitrary caller-selected printer may repeat rows, so it retains an
    // explicit set boundary rather than inheriting a proof it did not establish.
    ensure(*canonical_stream == self::canonical_stream(pipeline), hole, "aggregate canonical-stream proof changed")?;
    let rows = if *canonical_stream { invocation } else {
        let seen = fresh.id("aggregate_seen");
        let row = fresh.id("aggregate_row");
        quote!({
            let mut #seen = ::std::collections::HashSet::<_, #hasher>::default();
            #invocation.filter(move |#row| #seen.insert(::core::clone::Clone::clone(#row)))
        })
    };
    let key_fields = keys.iter().map(|column| {
        let column = reference(column);
        quote!(::core::clone::Clone::clone(#column))
    });
    let fold = if *constant_group {
        quote! {
            let mut #groups = ::core::option::Option::None;
            for (#(#columns,)*) in #rows {
                match &mut #groups {
                    ::core::option::Option::None => {
                        #groups = ::core::option::Option::Some(((#(#key_fields,)*), (#(#initial,)*)));
                    }
                    ::core::option::Option::Some((_, #values)) => { #(#updates)* }
                }
            }
            ::cq_storage::SingleGroup::from_option(#groups)
        }
    } else {
        quote! {
            let mut #groups = ::std::collections::HashMap::<_, _, #hasher>::default();
            for (#(#columns,)*) in #rows {
                let #key = (#(#key_fields,)*);
                match #groups.entry(#key) {
                    ::std::collections::hash_map::Entry::Vacant(#entry) => { #entry.insert((#(#initial,)*)); }
                    ::std::collections::hash_map::Entry::Occupied(mut #entry) => {
                        let #values = #entry.get_mut(); #(#updates)*
                    }
                }
            }
            ::cq_storage::GroupedAnswers::collect_from(#groups)
        }
    };
    Ok(quote!({
        #(#leaves)*
        #fold
    }))
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
    fn testkit_checks_fold_correspondence() {
        cq_testkit::PassCase {
            source: source(),
            expected_next: None,
            compile: Box::new(compile),
            check: Box::new(check),
            mutations: vec![Box::new(|context: &Context| {
                let mut changed = context.clone();
                changed.measures[1].operation = Operation::Max;
                changed
            })],
            answers: None,
        }
        .run(None)
        .unwrap();
    }
    #[test]
    fn aggregate_leaf_expressions_remain_pinned() {
        let source: Source = syn::parse_quote! {
            by(d); n=count(); struct G;
            relation Sale(d:i32,id:i32) = (make()); rows(d,id) :- Sale(d,id);
        };
        let mut target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        target.leaves[0].expression = syn::parse_quote!(changed());
        assert!(check(&source, &target).is_err());
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
    #[test]
    fn checked_set_and_constant_fold_facts_reject_mutation() {
        let source: Source = syn::parse_quote! {
            by(group); revenue=sum(amount);
            struct Revenue; relation R(id:i32,value:i64);
            rows(group,id,amount) :- R(id,value), let group:i32 = 0, let amount:i64 = *value * 2;
        };
        let pipeline: cq_stage::Pipeline = syn::parse_quote!(pipeline { mini_linq::passes::print_pull });
        let context = compile_with_pipeline(&source, &pipeline).unwrap();
        check_with_pipeline(&source, &context, &pipeline).unwrap();
        let rendered = render(&context, &pipeline).unwrap().to_string();
        assert!(rendered.contains("SingleGroup"));
        assert!(!rendered.contains("HashMap"));
        assert!(!rendered.contains("HashSet"));
        assert!(!rendered.contains("collect"));
        let mut changed = context.clone();
        changed.constant_group = false;
        assert!(check_with_pipeline(&source, &changed, &pipeline).is_err());
        let mut changed = context.clone();
        changed.canonical_stream = false;
        assert!(check_with_pipeline(&source, &changed, &pipeline).is_err());
        let mut changed = context.clone();
        changed.hasher = syn::parse_quote!(::std::collections::hash_map::RandomState);
        assert!(check_with_pipeline(&source, &changed, &pipeline).is_err());
        let mut changed = context;
        changed.owned.clear();
        assert!(check_with_pipeline(&source, &changed, &pipeline).is_err());
        let unknown: cq_stage::Pipeline = syn::parse_quote!(pipeline { another::print });
        let context = compile_with_pipeline(&source, &unknown).unwrap();
        let rendered = render(&context, &unknown).unwrap().to_string();
        assert!(rendered.contains("HashSet"));
        assert!(rendered.contains("HashState"));
    }
}
