use std::collections::BTreeMap;

use cq_ext_support::{Fresh, Leaf, bound, check_fresh, distinct, ensure, same_symbol, with_query};
use cq_lang_cq::{BodyItem, Program};
use cq_syntax::{Atom, CommaList, RelationDecl, rust, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Expr, Ident, Token, Type, parse::Parse};

mod kw {
    syn::custom_keyword!(select);
}
#[derive(Clone)]
struct Exists {
    body: CommaList<BodyItem>,
}
impl Parse for Exists {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            body: CommaList::parse_separated_nonempty(input)?,
        })
    }
}
#[derive(Clone)]
struct Scalar {
    name: Ident,
    ty: Type,
    default: Expr,
    select: Expr,
    body: CommaList<BodyItem>,
}
impl Parse for Scalar {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty = input.parse()?;
        input.parse::<Token![=]>()?;
        let default = input.parse()?;
        input.parse::<Token![;]>()?;
        input.parse::<kw::select>()?;
        let select;
        syn::parenthesized!(select in input);
        let select = select.parse()?;
        input.parse::<Token![;]>()?;
        let body = CommaList::parse_separated_nonempty(input)?;
        Ok(Self {
            name,
            ty,
            default,
            select,
            body,
        })
    }
}
#[derive(Clone)]
enum Spec {
    Exists(Exists),
    Scalar(Box<Scalar>),
}
impl Spec {
    fn body(&self) -> &CommaList<BodyItem> {
        match self {
            Self::Exists(e) => &e.body,
            Self::Scalar(s) => &s.body,
        }
    }
}
fn specs(source: &Program) -> syn::Result<Vec<(usize, Spec, Vec<Ident>)>> {
    let mut scope = BTreeMap::new();
    let mut specs = Vec::new();
    for (position, item) in source.query.body.iter().enumerate() {
        let spec = match item {
            BodyItem::Extension { name, tokens } if name == "exists" => {
                Some(Spec::Exists(syn::parse2(tokens.clone())?))
            }
            BodyItem::Extension { name, tokens } if name == "scalar" => {
                Some(Spec::Scalar(Box::new(syn::parse2(tokens.clone())?)))
            }
            _ => None,
        };
        if let Some(spec) = spec {
            ensure(
                spec.body()
                    .iter()
                    .any(|item| matches!(item, BodyItem::Positive { .. })),
                source,
                "correlated subquery needs a positive atom",
            )?;
            let outer = scope.values().cloned().collect();
            if let Spec::Scalar(scalar) = &spec {
                ensure(
                    !scope.contains_key(&symbol_name(&scalar.name)),
                    &scalar.name,
                    "scalar result shadows an outer binding",
                )?;
                scope.insert(symbol_name(&scalar.name), scalar.name.clone());
            }
            specs.push((position, spec, outer));
        }
        for id in bound(item) {
            scope.insert(symbol_name(&id), id);
        }
    }
    Ok(specs)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subquery {
    pub position: usize,
    pub hole: Program,
    pub links: Vec<(Ident, Ident)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub main: Program,
    pub leaves: Vec<Leaf>,
    pub subqueries: Vec<Subquery>,
}

fn close_inputs(source: &Program, fresh: &mut Fresh) -> (Vec<RelationDecl>, Vec<Leaf>) {
    let mut declarations = source.declarations.clone();
    let mut leaves = Vec::new();
    for declaration in &mut declarations {
        if same_symbol(&declaration.name, &source.query.head.relation) {
            continue;
        }
        let host = fresh.id("correlated_host");
        let expression = if let Some(value) = &declaration.value {
            let binding = fresh.id("leaf");
            leaves.push(Leaf {
                binding: binding.clone(),
                expression: value.expression.clone(),
            });
            syn::parse_quote!(&#binding)
        } else {
            let name = &declaration.name;
            syn::parse_quote!(&#name)
        };
        leaves.push(Leaf {
            binding: host.clone(),
            expression,
        });
        declaration.value = Some(syn::parse_quote!(= (#host)));
    }
    (declarations, leaves)
}
fn check_inputs(source: &Program, target: &Context) -> syn::Result<()> {
    ensure(
        source.declarations.len() == target.main.declarations.len(),
        source,
        "input declaration count changed",
    )?;
    let mut leaves = target.leaves.iter();
    let mut names = std::collections::BTreeSet::new();
    for (original, declaration) in source.declarations.iter().zip(&target.main.declarations) {
        let mut erased = declaration.clone();
        if !same_symbol(&original.name, &source.query.head.relation) {
            let expected = if let Some(value) = &original.value {
                let leaf = leaves
                    .next()
                    .ok_or_else(|| syn::Error::new_spanned(original, "missing leaf evaluation"))?;
                ensure(
                    leaf.expression == value.expression && names.insert(symbol_name(&leaf.binding)),
                    original,
                    "leaf evaluation changed",
                )?;
                let binding = &leaf.binding;
                syn::parse_quote!(&#binding)
            } else {
                let name = &original.name;
                syn::parse_quote!(&#name)
            };
            let host = leaves
                .next()
                .ok_or_else(|| syn::Error::new_spanned(original, "missing borrowed host"))?;
            ensure(
                host.expression == expected && names.insert(symbol_name(&host.binding)),
                original,
                "borrowed host changed",
            )?;
            let binding = &host.binding;
            ensure(
                declaration.value == Some(syn::parse_quote!(= (#binding))),
                declaration,
                "hole must use its borrowed host",
            )?;
            erased.value = original.value.clone();
        }
        ensure(erased == *original, declaration, "input schema changed")?;
    }
    ensure(leaves.next().is_none(), source, "extra context binding")
}

fn inner(
    source: &Program,
    declarations: &[RelationDecl],
    spec: &Spec,
    outer: &[Ident],
    fresh: &mut Fresh,
) -> (Program, Vec<(Ident, Ident)>) {
    let name = fresh.id("correlated_rows");
    let (head, output): (Atom, RelationDecl) = match spec {
        Spec::Exists(_) => (
            syn::parse_quote!(#name(())),
            syn::parse_quote!(relation #name(unit:());),
        ),
        Spec::Scalar(s) => {
            let expression = &s.select;
            let ty = &s.ty;
            (
                syn::parse_quote!(#name(#expression)),
                syn::parse_quote!(relation #name(value:#ty);),
            )
        }
    };
    let mut body = spec.body().clone();
    let mut links = Vec::<(Ident, Ident)>::new();
    for item in &mut body {
        if let BodyItem::Positive { atom } = item {
            for expression in &mut atom.args {
                if let Some(id) = rust::variable(expression)
                    .filter(|id| outer.iter().any(|bound| same_symbol(bound, id)))
                {
                    let local = if let Some((local, _)) =
                        links.iter().find(|(_, external)| same_symbol(external, id))
                    {
                        local.clone()
                    } else {
                        let local = fresh.id("correlation");
                        links.push((local.clone(), id.clone()));
                        local
                    };
                    *expression = syn::parse_quote!(#local);
                }
            }
        }
    }
    for (local, external) in &links {
        body.push(syn::parse_quote!(if #local == #external));
    }
    let query = syn::parse_quote!(#head :- #body;);
    (with_query(source, declarations, query, Some(output)), links)
}

fn clause(
    spec: &Spec,
    hole: &Program,
    fresh: &mut Fresh,
    pipeline: &cq_stage::Pipeline,
) -> syn::Result<BodyItem> {
    let rows = fresh.id("subquery_iter");
    let first = fresh.id("scalar_first");
    quoted_clause(spec, hole, &rows, &first, pipeline)
}
fn quoted_clause(
    spec: &Spec,
    hole: &Program,
    rows: &Ident,
    first: &Ident,
    pipeline: &cq_stage::Pipeline,
) -> syn::Result<BodyItem> {
    let invocation = pipeline.clone().start(hole.to_token_stream())?;
    Ok(match spec {
        Spec::Exists(_) => {
            syn::parse_quote!(if { let mut #rows = #invocation; #rows.next().is_some() })
        }
        Spec::Scalar(s) => {
            let Scalar {
                name, ty, default, ..
            } = s.as_ref();
            let mut default = default;
            while let Expr::Paren(paren) = default {
                default = &paren.expr;
            }
            syn::parse_quote!(let #name: #ty = {
                let mut #rows = ::cq_storage::Answers::collect_from(#invocation).into_iter();
                let #first = #rows.next();
                assert!(#rows.next().is_none(), "scalar subquery returned multiple distinct rows");
                match #first { Some((value,)) => value, None => #default }
            })
        }
    })
}

pub fn compile(source: &Program, pipeline: &cq_stage::Pipeline) -> syn::Result<Context> {
    let specs = specs(source)?;
    let mut fresh = Fresh::new(source);
    let (declarations, leaves) = close_inputs(source, &mut fresh);
    let mut main = source.clone();
    main.declarations = declarations.clone();
    let mut subqueries = Vec::new();
    for (position, spec, outer) in specs {
        let (hole, links) = inner(source, &declarations, &spec, &outer, &mut fresh);
        main.query.body[position] = clause(&spec, &hole, &mut fresh, pipeline)?;
        subqueries.push(Subquery {
            position,
            hole,
            links,
        });
    }
    Ok(Context {
        main,
        leaves,
        subqueries,
    })
}

/// Verify capture links by erasing fresh atom arguments and equality guards;
/// the per-row quotation itself is checked structurally, not by executing Rust.
pub fn check(source: &Program, target: &Context, pipeline: &cq_stage::Pipeline) -> syn::Result<()> {
    let specs = specs(source)?;
    check_fresh(source, target.leaves.iter().map(|leaf| &leaf.binding))?;
    check_inputs(source, target)?;
    ensure(
        specs.len() == target.subqueries.len(),
        source,
        "subquery count changed",
    )?;
    let mut erased = target.main.clone();
    erased.declarations = source.declarations.clone();
    for ((position, spec, outer), subquery) in specs.iter().zip(&target.subqueries) {
        check_fresh(
            source,
            subquery
                .links
                .iter()
                .map(|(local, _)| local)
                .chain(std::iter::once(&subquery.hole.query.head.relation)),
        )?;
        ensure(
            subquery.position == *position,
            source,
            "subquery position changed",
        )?;
        let query = &subquery.hole.query;
        let parsed: cq_lang_cq::Query = syn::parse2(quote!(#query))?;
        ensure(
            parsed == *query,
            query,
            "noncanonical recognized guard or capture",
        )?;
        let mut body = subquery.hole.query.body.clone();
        let mut locals = std::collections::BTreeSet::new();
        for (local, external) in subquery.links.iter().rev() {
            ensure(
                outer.iter().any(|bound| same_symbol(bound, external))
                    && locals.insert(symbol_name(local))
                    && !outer.iter().any(|bound| same_symbol(bound, local)),
                source,
                "invalid capture link",
            )?;
            let expected = BodyItem::Constraint(cq_syntax::Constraint::Compare {
                left: cq_syntax::constraint::Term::Variable(local.clone()),
                comparison: cq_syntax::constraint::Comparison::Eq(Default::default()),
                right: cq_syntax::constraint::Term::Parameter(external.clone().into()),
            });
            ensure(
                body.pop().map(|pair| pair.into_value()) == Some(expected),
                &subquery.hole,
                "missing correlation equality constraint",
            )?;
            let mut uses = 0;
            let mut original_atoms = spec.body().iter().filter_map(BodyItem::atom);
            for item in &mut body {
                if let BodyItem::Positive { atom } = item {
                    let original_atom = original_atoms.next().ok_or_else(|| {
                        syn::Error::new_spanned(&*atom, "capture position changed")
                    })?;
                    for (column, expression) in atom.args.iter_mut().enumerate() {
                        if rust::variable(expression) == Some(local) {
                            let original = original_atom.args.get(column).ok_or_else(|| {
                                syn::Error::new_spanned(&*expression, "capture position changed")
                            })?;
                            ensure(
                                rust::variable(original)
                                    .is_some_and(|id| same_symbol(id, external)),
                                original,
                                "capture source changed",
                            )?;
                            *expression = original.clone();
                            uses += 1;
                        }
                    }
                }
            }
            ensure(uses > 0, source, "unused capture link")?;
        }
        // Reparse the erased surface body: recognition can split guards and
        // resolve captures differently once original atom names are restored.
        // Atom occurrence order, rather than clause indices, preserves links.
        let head = &subquery.hole.query.head;
        body.pop_punct();
        let original = spec.body();
        let erased_query: cq_lang_cq::Query = syn::parse2(quote!(#head :- #body;))?;
        let expected: cq_lang_cq::Query = syn::parse2(quote!(#head :- #original;))?;
        ensure(
            erased_query == expected,
            &subquery.hole,
            "inner body changed",
        )?;
        for item in &subquery.hole.query.body {
            if let BodyItem::Positive { atom } = item {
                ensure(
                    !atom
                        .variables()
                        .any(|id| outer.iter().any(|bound| same_symbol(bound, id))),
                    atom,
                    "outer column was shadowed instead of correlated",
                )?;
            }
        }
        let name = &subquery.hole.query.head.relation;
        let (head, output): (Atom, RelationDecl) = match spec {
            Spec::Exists(_) => (
                syn::parse_quote!(#name(())),
                syn::parse_quote!(relation #name(unit:());),
            ),
            Spec::Scalar(s) => {
                let expr = &s.select;
                let ty = &s.ty;
                (
                    syn::parse_quote!(#name(#expr)),
                    syn::parse_quote!(relation #name(value:#ty);),
                )
            }
        };
        ensure(
            subquery.hole.query.head == head,
            &subquery.hole,
            "subquery head changed",
        )?;
        let expected = with_query(
            source,
            &target.main.declarations,
            subquery.hole.query.clone(),
            Some(output),
        );
        ensure(
            subquery.hole == expected,
            &subquery.hole,
            "subquery schema changed",
        )?;
        // Parse the generated expression and alpha-rename its local temporaries.
        let actual = target
            .main
            .query
            .body
            .get(*position)
            .ok_or_else(|| syn::Error::new_spanned(source, "missing subquery clause"))?;
        check_quote(actual, spec, &subquery.hole, source, pipeline)?;
        erased.query.body[*position] = source.query.body[*position].clone();
    }
    ensure(
        erased == *source,
        &target.main,
        "main query changed outside subqueries",
    )
}

fn check_quote(
    actual: &BodyItem,
    spec: &Spec,
    hole: &Program,
    source: &Program,
    pipeline: &cq_stage::Pipeline,
) -> syn::Result<()> {
    // Recognize the block's closing operation and quotation. Arbitrary Rust
    // semantics are outside correspondence, just as for the core printer.
    let expression = match (actual, spec) {
        (BodyItem::Filter(filter), Spec::Exists(_)) => &filter.condition,
        (BodyItem::Let(binding), Spec::Scalar(s)) => {
            ensure(
                rust::bindings(&binding.pattern) == vec![s.name.clone()],
                &binding.pattern,
                "scalar binder changed",
            )?;
            ensure(
                binding.annotation.as_ref().map(|a| &a.ty) == Some(&s.ty),
                &binding.pattern,
                "scalar type changed",
            )?;
            &binding.expression
        }
        _ => {
            return Err(syn::Error::new_spanned(
                actual,
                "wrong subquery clause kind",
            ));
        }
    };
    let Expr::Block(block) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "subquery must be evaluated in a block",
        ));
    };
    let expected_len = if matches!(spec, Spec::Exists(_)) {
        2
    } else {
        4
    };
    ensure(
        block.block.stmts.len() == expected_len,
        expression,
        "subquery block changed",
    )?;
    let Some(syn::Stmt::Local(local)) = block.block.stmts.first() else {
        return Err(syn::Error::new_spanned(
            expression,
            "missing subquery iterator",
        ));
    };
    let ids = rust::bindings(&local.pat);
    ensure(ids.len() == 1, &local.pat, "invalid iterator binding")?;
    let Some(init) = &local.init else {
        return Err(syn::Error::new_spanned(
            expression,
            "missing iterator quotation",
        ));
    };
    let quotation = match spec {
        Spec::Exists(_) => &*init.expr,
        Spec::Scalar(_) => {
            let Expr::MethodCall(method) = &*init.expr else {
                return Err(syn::Error::new_spanned(
                    expression,
                    "scalar must collect distinct answers",
                ));
            };
            ensure(
                method.method == "into_iter" && method.args.is_empty(),
                method,
                "scalar answer iterator changed",
            )?;
            let Expr::Call(call) = &*method.receiver else {
                return Err(syn::Error::new_spanned(
                    expression,
                    "missing scalar answer collection",
                ));
            };
            let Expr::Path(path) = &*call.func else {
                return Err(syn::Error::new_spanned(
                    expression,
                    "invalid scalar answer collector",
                ));
            };
            ensure(
                path.path == syn::parse_quote!(::cq_storage::Answers::collect_from)
                    && call.args.len() == 1,
                call,
                "scalar answer collector changed",
            )?;
            &call.args[0]
        }
    };
    let Expr::Macro(invocation) = quotation else {
        return Err(syn::Error::new_spanned(
            expression,
            "missing remaining-pipeline quotation",
        ));
    };
    let expected: syn::ExprMacro = syn::parse2(pipeline.clone().start(hole.to_token_stream())?)?;
    ensure(
        invocation.mac.path == expected.mac.path,
        invocation,
        "subquery bypassed the caller's remaining pipeline",
    )?;
    let packet: cq_ext_support::SurfaceInput = syn::parse2(invocation.mac.tokens.clone())?;
    let expected: cq_ext_support::SurfaceInput = syn::parse2(expected.mac.tokens)?;
    ensure(
        packet.arguments.to_string() == expected.arguments.to_string()
            && packet.pipeline.to_token_stream().to_string()
                == expected.pipeline.to_token_stream().to_string(),
        invocation,
        "subquery changed the remaining pipeline or pass arguments",
    )?;
    let parsed: Program = syn::parse2(packet.program)?;
    ensure(
        parsed == *hole,
        &parsed,
        "quoted subquery differs from its hole",
    )?;
    // Recognize the remaining fixed statements using the actual fresh names.
    let first_name = if matches!(spec, Spec::Scalar(_)) {
        let Some(syn::Stmt::Local(first)) = block.block.stmts.get(1) else {
            return Err(syn::Error::new_spanned(
                expression,
                "missing first scalar row",
            ));
        };
        rust::bindings(&first.pat).into_iter().next()
    } else {
        None
    };
    let first =
        first_name.unwrap_or_else(|| syn::Ident::new("unused", proc_macro2::Span::mixed_site()));
    check_fresh(
        source,
        ids.iter()
            .chain(matches!(spec, Spec::Scalar(_)).then_some(&first)),
    )?;
    let expected = quoted_clause(spec, hole, &ids[0], &first, pipeline)?;
    ensure(
        *actual == expected,
        actual,
        "subquery block/default changed",
    )
}

pub fn render(context: &Context, pipeline: &cq_stage::Pipeline) -> syn::Result<TokenStream> {
    let leaves = &context.leaves;
    let main = &context.main;
    let answers = distinct(
        pipeline.clone().start(main.to_token_stream())?,
        &mut Fresh::new(main),
        "correlated_answers",
    );
    Ok(if leaves.is_empty() {
        answers
    } else {
        quote!({ #(#leaves)* #answers.collect::<::std::vec::Vec<_>>().into_iter() })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pipeline() -> cq_stage::Pipeline {
        syn::parse_quote!(pipeline { next::plan(opaque arguments), last::print })
    }
    fn compile(source: &Program) -> syn::Result<Context> {
        super::compile(source, &pipeline())
    }
    fn check(source: &Program, target: &Context) -> syn::Result<()> {
        super::check(source, target, &pipeline())
    }
    #[test]
    fn testkit_checks_capture_correspondence() {
        cq_testkit::PassCase {
            source: syn::parse_quote! {
                struct C; relation A(x:i32); relation S(x:i32,y:i32);
                answer(x) :- A(x), exists!{S(x,y)};
            },
            expected_next: None,
            compile: Box::new(compile),
            check: Box::new(check),
            mutations: vec![Box::new(|context: &Context| {
                let mut changed = context.clone();
                changed.subqueries[0].links.clear();
                changed
            })],
            answers: None,
        }
        .run(None)
        .unwrap();
    }
    #[test]
    fn nested_quote_cannot_change_the_queue_or_its_arguments() {
        let source: Program = syn::parse_quote! {
            struct C; relation A(x:i32); relation S(x:i32,y:i32);
            answer(x) :- A(x), exists!{S(x,y)};
        };
        let target = compile(&source).unwrap();
        assert!(
            super::check(
                &source,
                &target,
                &syn::parse_quote!(pipeline { next::plan(different), last::print })
            )
            .is_err()
        );
        assert!(
            super::check(
                &source,
                &target,
                &syn::parse_quote!(pipeline { next::plan(opaque arguments), different::print })
            )
            .is_err()
        );
    }
    #[test]
    fn raw_and_bare_outer_occurrences_share_a_capture_without_changing_spelling() {
        let source: Program = syn::parse_quote! {
            struct Raw; relation A(x:i32); relation S(a:i32,b:i32);
            answer(x) :- A(x), exists!{S(r#x,x)};
        };
        let target = compile(&source).unwrap();
        assert_eq!(target.subqueries[0].links.len(), 1);
        check(&source, &target).unwrap();
    }
    #[test]
    fn correspondence_checks_capture_links_and_per_row_quotes() {
        let source: Program = syn::parse_quote! {
            struct C; relation R(x:i32); relation S(x:i32,a:i32); relation answer(x:i32,v:i32);
            answer(*x,v) :- R(x), exists!{S(x,a)}, scalar!{v:i32=(0); select (*a); S(x,a)};
        };
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        let mut changed = target.clone();
        changed.subqueries[0].links.clear();
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        let capture = changed.subqueries[0].hole.query.body.last_mut().unwrap();
        let BodyItem::Constraint(cq_syntax::Constraint::Compare { comparison, .. }) = capture
        else {
            panic!()
        };
        *comparison = cq_syntax::constraint::Comparison::Ne(Default::default());
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.main.query.body[1] = syn::parse_quote!(if true);
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.subqueries[1].hole.query.head = syn::parse_quote!(bad(0));
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.subqueries.pop();
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn scalar_scope_and_empty_inner_body_are_rejected() {
        for clause in ["scalar!{x:i32=(0); select (1); R(y)}", "exists!{if true}"] {
            let source = syn::parse_str(&format!(
                "struct C; relation R(x:i32); answer(x) :- R(x), {clause};"
            ))
            .unwrap();
            assert!(compile(&source).is_err());
        }
    }

    #[test]
    fn recognized_inner_guards_preserve_capture_occurrences_and_residual_order() {
        let source: Program = syn::parse_quote! {
            struct C; relation R(x:i32); relation S(x:i32,a:i32); relation T(a:i32);
            answer(x) :- R(x), exists! {
                S(x,a), if a > cutoff && f(a) && a != 0 && g(a), T(a)
            };
        };
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        let mut changed = target.clone();
        let BodyItem::Filter(f) = changed.subqueries[0]
            .hole
            .query
            .body
            .iter_mut()
            .find(|i| matches!(i, BodyItem::Filter(_)))
            .unwrap()
        else {
            panic!()
        };
        f.condition = syn::parse_quote!(g(a) && f(a));
        assert!(check(&source, &changed).is_err());
    }
}
