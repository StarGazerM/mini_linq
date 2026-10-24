use cq_datalog_print::*;
use incremental::strata::source;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use round::storage::incremental;
struct Expanded {
    program: Program,
    context: RoundContext,
    holes: Vec<TokenStream>,
}
fn expand(
    source: &incremental::Program,
    mut callback: impl FnMut(round::cq::Program) -> syn::Result<TokenStream>,
) -> syn::Result<Expanded> {
    let storage = cq_datalog_storage::compile(source)?;
    let program = cq_datalog_fork::compile(&storage)?;
    let holes = splice_with(&program, |hole| callback(hole.cq.clone()))?;
    Ok(Expanded {
        context: program.context.clone(),
        program,
        holes,
    })
}
fn render(expanded: Expanded) -> TokenStream {
    let mut expression = render_with(
        &expanded.program,
        &expanded.holes,
        |rules, context, holes, _| {
            Ok(evaluate(
                rules,
                context,
                holes,
                &expanded.program.holes,
                false,
            ))
        },
    )
    .unwrap();
    erase_partitions(&mut expression);
    expression.to_token_stream()
}

// Compare operation order independently of the codegen calling convention.
// Only a complete dispatch envelope is erased; all query, admission,
// publication, and loop syntax remains subject to the original fixture checks.
fn erase_partitions(expression: &mut syn::Expr) {
    use syn::visit_mut::{self, VisitMut};
    struct Substitute(std::collections::BTreeMap<String, syn::Expr>);
    impl VisitMut for Substitute {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if let syn::Expr::Paren(paren) = expression
                && let syn::Expr::Unary(unary) = &*paren.expr
                && matches!(unary.op, syn::UnOp::Deref(_))
                && let syn::Expr::Path(path) = &*unary.expr
                && let Some(name) = path.path.get_ident()
                && let Some(value) = self.0.get(&name.to_string())
            {
                *expression = value.clone();
                return;
            }
            visit_mut::visit_expr_mut(self, expression);
        }
    }
    struct Erase;
    impl VisitMut for Erase {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if let syn::Expr::Block(block) = expression
                && let [syn::Stmt::Expr(syn::Expr::Call(call), None)] = &block.block.stmts[..]
                && let syn::Expr::Path(path) = &*call.func
                && path
                    .path
                    .segments
                    .iter()
                    .any(|part| part.ident == "__codegen")
            {
                let [syn::Expr::Tuple(arguments), syn::Expr::Closure(closure)] =
                    &call.args.iter().cloned().collect::<Vec<_>>()[..]
                else {
                    panic!("dispatch arguments")
                };
                let syn::Pat::Tuple(parameters) = &closure.inputs[0] else {
                    panic!("dispatch parameters")
                };
                assert_eq!(arguments.elems.len(), parameters.elems.len());
                let substitutions = parameters
                    .elems
                    .iter()
                    .zip(&arguments.elems)
                    .map(|(parameter, argument)| {
                        let syn::Pat::Ident(parameter) = parameter else {
                            panic!("parameter name")
                        };
                        let syn::Expr::Reference(argument) = argument else {
                            panic!("borrowed storage")
                        };
                        (parameter.ident.to_string(), *argument.expr.clone())
                    })
                    .collect();
                let mut body = *closure.body.clone();
                Substitute(substitutions).visit_expr_mut(&mut body);
                let syn::Expr::Block(body) = body else {
                    panic!("dispatch body")
                };
                let [syn::Stmt::Expr(body, None)] = &body.block.stmts[..] else {
                    panic!("body expression")
                };
                *expression = body.clone();
                self.visit_expr_mut(expression);
                return;
            }
            visit_mut::visit_expr_mut(self, expression);
        }
    }
    Erase.visit_expr_mut(expression);
}
fn source() -> TokenStream {
    quote! {struct Q;relation E(x:i32)=(input);relation R(x:i32);output R;R(*x):-E(x);}
}

#[test]
fn authored_round_expansion_matches_independent_fixture() {
    let source = source();
    let program: incremental::Program = syn::parse2(quote! {
        #source
        R(*x) :- E(x), R(x);
        strata { component [R]; }
        incremental { scheme seminaive_covering;


            delta R = Recent;


            variant 0 seed { R(*x) :- E(x); }
            variant 1 delta 1 { R(*x) :- E(x), Recent(x); }
        }
    })
    .unwrap();
    let fork = expand(&program, |_| Ok(quote!(authored_rows()))).unwrap();
    let partitioned = render_with(&fork.program, &fork.holes, |rules, context, holes, _| {
        Ok(evaluate(rules, context, holes, &fork.program.holes, false))
    })
    .unwrap()
    .to_token_stream()
    .to_string();
    for slot in 0..3 {
        assert!(partitioned.contains(&format!("partition_{slot} :: run")));
    }
    assert_eq!(partitioned.matches("inline (always)").count(), 3);
    assert!(partitioned.contains("& mut __cq_datalog_seen_1"));
    assert!(partitioned.contains("& __cq_datalog_host_0"));
    let actual: syn::Expr = syn::parse2(render(fork)).unwrap();
    let expected: syn::Expr = syn::parse_str(include_str!("fixtures/round.rs")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn lower_stratum_negation_builds_one_key_set_outside_rounds() {
    let program: incremental::Program = syn::parse_quote! {
        observe = (observer);
        struct Layered;
        relation Domain(x:String) = (domain);
        relation Blocked(x:String);
        relation Out(x:String);
        output Out;
        Blocked(x.clone()) :- Domain(x);
        Out(x.clone()) :- Domain(x), !Blocked(x), !Blocked(x);
        strata { component [Blocked]; component [Out]; }
        incremental { scheme seminaive_covering;


            delta Blocked = DB;
            delta Out = DO;

            variant 0 seed { Blocked(x.clone()) :- Domain(x); }
            variant 1 seed { Out(x.clone()) :- Domain(x), !Blocked(x), !Blocked(x); }
        }
    };
    let fork = expand(&program, |hole| {
        if hole.name.to_string().contains("key_hole") {
            Ok(quote!(completed_keys()))
        } else {
            Ok(quote!(candidate_rows()))
        }
    })
    .unwrap();
    assert_eq!(fork.context.strata[1].keys.len(), 1);
    let expanded = render(fork).to_string();
    assert_eq!(expanded.matches("completed_keys ()").count(), 1);
    assert_eq!(expanded.matches("Rows :: rows").count(), 2);
    assert!(expanded.contains("observer"));
}

#[test]
fn full_idb_layouts_publish_only_admitted_rows_after_all_holes() {
    let program: incremental::Program = syn::parse_quote! {
        struct Closure;
        relation E(x:String,y:String) = (edges);
        relation R(x:String,y:String);
        output R;
        R((*x).clone(),(*y).clone()) :- E(x,y);
        R((*x).clone(),(*z).clone()) :- R(x,y), R(y,z);
        strata { component [R]; }
        incremental { scheme seminaive_covering;


            delta R = D;




            variant 0 seed { R((*x).clone(),(*y).clone()) :- E(x,y); }
            variant 1 delta 0 { R((*x).clone(),(*z).clone()) :- D(x,y), R(y,z); }
            variant 1 delta 1 { R((*x).clone(),(*z).clone()) :- R(x,y), D(y,z); }
        }
    };
    let expanded = expand(&program, |hole| {
        let name = hole.name;
        Ok(quote!(#name()))
    })
    .unwrap();
    let next = &expanded.context.relations[1].next;
    let first_host = &expanded.context.layouts[1].host;
    let second_host = &expanded.context.layouts[2].host;
    let expected_updates = quote! {
        for row in &#next {
            #first_host.entry((::core::clone::Clone::clone(&row.0),))
                .or_default().push((::core::clone::Clone::clone(&row.1),));
            #second_host.entry(())
                .or_default().push((::core::clone::Clone::clone(&row.0),
                    ::core::clone::Clone::clone(&row.1),));
        }
    }
    .to_string();
    let rendered = render(expanded).to_string();
    // One publication after seeds and one after all recursive variants, not
    // one insertion into the full maps per rule or per rejected candidate.
    assert_eq!(rendered.matches(&expected_updates).count(), 2);
    assert_eq!(rendered.matches("Rows :: rows").count(), 2);
    let last_step = rendered.find("__cq_datalog_hole_2 ()").unwrap();
    assert!(rendered.rfind(&expected_updates).unwrap() > last_step);
}

#[test]
fn delta_layout_runs_are_built_once_per_publication_outside_all_holes() {
    let source: source::Program = syn::parse_quote! {
        struct SharedRuns;
        relation E(x:String,y:String) = (edges);
        relation R(x:String,y:String);
        output R;
        R((*x).clone(),(*y).clone()) :- E(x,y);
        R((*x).clone(),(*z).clone()) :- R(x,y), R(y,z);
        R((*x).clone(),(*z).clone()) :- E(x,y), R(y,z);
        R((*x).clone(),(*z).clone()) :- E(x,y), R(z,y);
        R((*x).clone(),(*z).clone()) :- E(x,y), R(z,w);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let program = cq_datalog_incremental::compile(&strata).unwrap();
    let fork = expand(&program, |hole| {
        let name = hole.name;
        Ok(quote!(#name()))
    })
    .unwrap();
    assert_eq!(fork.context.delta_layouts.len(), 3);
    let builds: Vec<_> = fork
        .context
        .delta_layouts
        .iter()
        .map(|layout| {
            let host = &layout.host;
            let delta = fork.context.relations[layout.relation]
                .delta_host
                .as_ref()
                .unwrap();
            let columns: Vec<_> = layout
                .columns
                .iter()
                .map(|&column| {
                    let column = syn::Index::from(column);
                    quote!(::core::clone::Clone::clone(&row.#column))
                })
                .collect();
            let (keys, values) = columns.split_at(layout.key);
            let build: syn::Expr = syn::parse_quote!({
                #host = ::core::default::Default::default();
                for row in &#delta {
                    #host.entry((#(#keys,)*))
                        .or_default().push((#(#values,)*));
                }
            });
            build
        })
        .collect();
    let seed = fork.holes[fork.context.strata[0].seeds[0].hole].to_string();
    let steps: Vec<_> = fork.context.strata[0]
        .steps
        .iter()
        .map(|rule| fork.holes[rule.hole].to_string())
        .collect();
    let rendered: syn::ExprBlock = syn::parse2(render(fork)).unwrap();
    let text = rendered.to_token_stream().to_string();
    for build in &builds {
        assert_eq!(
            text.matches(&build.to_token_stream().to_string()).count(),
            2,
            "one seed build and one round build, with no per-hole copies"
        );
    }
    let stratum = rendered
        .block
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            syn::Stmt::Expr(syn::Expr::Block(block), _) => Some(&block.block),
            _ => None,
        })
        .unwrap();
    let round = stratum
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            syn::Stmt::Expr(syn::Expr::While(loop_), _) => Some(&loop_.body),
            _ => None,
        })
        .unwrap();
    for (block, holes) in [(stratum, vec![seed]), (round, steps)] {
        let positions: Vec<_> = builds
            .iter()
            .map(|build| {
                let positions: Vec<_> = block
                    .stmts
                    .iter()
                    .enumerate()
                    .filter_map(|(i, stmt)| match stmt {
                        syn::Stmt::Expr(expr, _) if expr == build => Some(i),
                        _ => None,
                    })
                    .collect();
                // The complete replacement/build is a direct sibling after
                // the holes, never nested in their candidate-row loops.
                assert_eq!(positions.len(), 1);
                positions[0]
            })
            .collect();
        for hole in holes {
            let position = block
                .stmts
                .iter()
                .position(|stmt| stmt.to_token_stream().to_string().contains(&hole))
                .unwrap();
            assert!(positions.iter().all(|&build| build > position));
        }
    }
}

#[test]
fn multioutput_render_moves_each_relation_in_output_order() {
    let source: source::Program = syn::parse_quote! {
        struct Ordered;
        relation E(x:i32) = (input);
        relation A(x:i32); relation B(x:String);
        output B, A;
        A(*x) :- E(x);
        B(x.to_string()) :- A(x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let program = cq_datalog_incremental::compile(&strata).unwrap();
    let fork = expand(&program, |_| Ok(quote!(rows()))).unwrap();
    let a = &fork.context.relations[1].all;
    let b = &fork.context.relations[2].all;
    let expected: syn::Expr = syn::parse_quote!((#b.into_iter(), #a.into_iter(),));
    let actual: syn::ExprBlock = syn::parse2(render(fork)).unwrap();
    assert_eq!(
        actual.block.stmts.last(),
        Some(&syn::Stmt::Expr(expected, None))
    );
}

#[test]
fn constant_key_projection_is_built_once_before_recursive_rounds() {
    let source: source::Program = syn::parse_quote! {
        struct Filtered;
        relation E(x:i32) = (input);
        relation M(kind:String,x:i32) = (modifiers);
        relation A(x:i32);
        output A;
        A(*x) :- E(x), !M("abstract",x);
        A(*x) :- A(x), !M("abstract",x), !M("abstract",x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let program = cq_datalog_incremental::compile(&strata).unwrap();
    let fork = expand(&program, |hole| {
        if hole.name.to_string().contains("key_hole") {
            Ok(quote!(filtered_keys()))
        } else {
            Ok(quote!(candidate_rows()))
        }
    })
    .unwrap();
    assert_eq!(fork.context.strata[0].keys.len(), 1);
    let rendered = render(fork).to_string();
    assert_eq!(rendered.matches("filtered_keys ()").count(), 1);
    assert!(rendered.find("filtered_keys ()").unwrap() < rendered.find("while").unwrap());
}

#[test]
fn edb_canonicalization_precedes_and_is_shared_by_layouts_keys_and_outputs() {
    let source: source::Program = syn::parse_quote! {
        struct CanonicalInputs;
        relation E(x:String,y:String) = (input());
        relation R(x:String,y:String);
        output E, R;
        R((*x).clone(),(*z).clone()) :- E(x,y), E(y,z), !E(z,x);
        R((*x).clone(),(*z).clone()) :- R(x,y), E(z,y), !E(z,x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let program = cq_datalog_incremental::compile(&strata).unwrap();
    let fork = expand(&program, |hole| {
        if hole.name.to_string().contains("key_hole") {
            Ok(quote!(canonical_keys()))
        } else {
            Ok(quote!(candidate_rows()))
        }
    })
    .unwrap();
    assert!(
        fork.context
            .layouts
            .iter()
            .filter(|layout| layout.relation == 0)
            .count()
            >= 3
    );
    let all = &fork.context.relations[0].all;
    let canonicalize =
        quote!(Canonical(::cq_storage::rows::Rows::rows(#all).collect())).to_string();
    let output = quote!(::cq_storage::rows::Rows::rows(#all).map).to_string();
    let rendered = render(fork).to_string();
    assert_eq!(rendered.matches("input ()").count(), 1);
    assert_eq!(rendered.matches(&canonicalize).count(), 1);
    assert_eq!(rendered.matches("struct Canonical").count(), 1);
    assert!(rendered.contains(&output));
    let canonicalized = rendered.find(&canonicalize).unwrap();
    assert!(canonicalized < rendered.find("HashMap").unwrap());
    assert!(canonicalized < rendered.find("canonical_keys ()").unwrap());
    assert!(canonicalized < rendered.find("while").unwrap());
    // Output collection must not introduce a second all-column hash set.
    assert_eq!(
        rendered
            .matches("collect :: < :: std :: collections :: HashSet")
            .count(),
        1
    );
}

fn push_program() -> Program {
    let source: source::Program = syn::parse_quote! {
        struct Push;
        relation E(x:i32) = (input);
        relation B(x:i32) = (blocked);
        relation R(x:i32);
        output R;
        R(*x) :- E(x), !B(x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let mut program = cq_datalog_fork::compile(&storage).unwrap();
    for hole in &mut program.holes {
        hole.interface = HoleInterface::Push;
        hole.pipeline.passes.last_mut().unwrap().path =
            syn::parse_quote!(::cq_datalog::passes::print_push);
    }
    program
}

#[test]
fn push_rule_and_key_holes_receive_consumers_without_iterator_adapters() {
    let program = push_program();
    let holes = splice(&program).unwrap();
    assert_eq!(holes.len(), 2);
    for invocation in &holes {
        assert!(
            invocation
                .to_string()
                .contains("consumer { __cq_datalog_consumer }")
        );
    }
    let expression = compile(&program).unwrap().to_token_stream().to_string();
    assert_eq!(expression.matches("print_push").count(), 2);
    assert!(expression.contains("__cq_collected_rows . extend"));
    assert!(!expression.contains("__cq_candidates_"));
    assert!(!expression.contains("IntoIterator"));
}

#[test]
fn renderer_rejects_incomplete_push_splices_before_evaluation() {
    let program = push_program();
    let holes = splice(&program).unwrap();
    let error = render_with(&program, &holes[..1], |_, _, _, _| {
        panic!("incomplete splice reached the evaluator")
    })
    .unwrap_err();
    assert!(error.to_string().contains("hole splice coverage changed"));
}

#[test]
fn pull_collection_tokens_remain_unchanged() {
    let provider = quote!(::std::vec::Vec<(i32,)>);
    let actual = collect_hole(
        HoleInterface::Pull,
        &quote!(authored_rows()),
        provider.clone(),
    );
    assert_eq!(
        actual.to_string(),
        quote!((authored_rows()).collect::<#provider>()).to_string(),
    );
}

#[test]
fn rule_and_key_lower_continuations_carry_exact_recorded_context() {
    use cq_stage::Language;

    let program = push_program();
    let invocations = splice(&program).unwrap();
    assert_eq!(program.context.strata[0].keys.len(), 1);
    for (hole, invocation) in program.holes.iter().zip(invocations) {
        // CQ itself has no restricted facts. Lower is the first authorized
        // boundary; putting the eventual facts on this entry would be invalid.
        assert!(hole.cq.context().is_empty());
        let initial: syn::Macro = syn::parse2(invocation).unwrap();
        assert!(!initial.tokens.to_string().starts_with("context"));

        let lower = &hole.pipeline.passes[hole.lower];
        let arguments = &lower.arguments;
        assert_eq!(
            cq_physical_lowering::context(arguments.clone()).unwrap(),
            hole.facts,
        );
        assert!(hole.facts.consumer_canonical);
        assert_eq!(hole.facts.discharged_host_sets.len(), 1);

        // Author an independent IX projection of the actual certified input.
        // The non-injective head still needs no per-hole result canonicalizer:
        // the rule admission/key-set consumer supplies that boundary.
        let declaration = hole
            .cq
            .declarations
            .iter()
            .find(|declaration| {
                hole.facts
                    .discharged_host_sets
                    .contains(&declaration.name.to_string())
            })
            .unwrap();
        let host = &declaration.name;
        let column = &declaration.columns[0].name;
        let ix: cq_lang_ix::Program = syn::parse_quote! {
            relational {
                #declaration
                r = rename #host@0 {#column -> x};
                b = bind r with y:i32 = (*x % 2);
                p = project b keep {y};
                output p as answer(y);
            }
            indexes {}
        };
        let physical = cq_physical_lowering::compile_with(&ix, arguments.clone()).unwrap();
        assert_eq!(physical.context(), hole.facts);
        assert!(physical.program.return_canon.is_none());
        assert!(
            physical
                .program
                .definitions
                .iter()
                .all(|definition| definition.canon.is_none())
        );
        let continuation =
            cq_stage::pass_with_context::<cq_lang_ix::Program, cq_lang_physical::Program>(
                quote!(arguments { #arguments }; pipeline { next }; #ix),
                cq_physical_lowering::context,
                cq_physical_lowering::compile_with,
            );
        let context = &hole.facts;
        assert_eq!(
            continuation.to_string(),
            quote!(next! { context { #context }; pipeline {}; #physical }).to_string(),
        );
        let continuation: syn::Macro = syn::parse2(continuation).unwrap();
        let accepted = cq_stage::pass::<cq_lang_physical::Program, syn::Expr>(
            continuation.tokens,
            |source, _| {
                assert_eq!(source.context(), hole.facts);
                Ok(syn::parse_quote!(42))
            },
        );
        assert_eq!(accepted.to_string(), "42");
        let rejected = cq_stage::pass::<cq_lang_physical::Program, syn::Expr>(
            quote!(pipeline {}; #physical),
            |_, _| panic!("factful hole re-entry without an envelope reached its printer"),
        );
        assert!(rejected.to_string().contains("CTX-Entry"));
    }
}
