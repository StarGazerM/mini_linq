//! External test passes exercising only public core APIs.
use cq_lang_physical::{self as physical, Choice, Mode, Operator};
use cq_lang_structures::Structure;
use proc_macro::TokenStream;
use std::collections::{BTreeMap, BTreeSet};

fn planned(input: TokenStream, options: cq_order::Options, reductions: bool) -> TokenStream {
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(input.into(), |source, args| {
        if !args.is_empty() {
            return Err(syn::Error::new_spanned(
                args,
                "unexpected planner arguments",
            ));
        }
        let normal = cq_lang_cq::normal::normalize(source)?;
        let mut plan = cq_order::compile(&normal, &options)?;
        cq_order::check(&normal, &plan, &options)?;
        if reductions {
            let options = cq_reduce::Options {
                guards: cq_reduce::Pushdown::Atoms,
                semijoins: if cq_lang_cq::hypergraph::Analysis::of(&normal).acyclic {
                    cq_reduce::Reductions::Tree
                } else {
                    cq_reduce::Reductions::None
                },
            };
            let reduced = cq_reduce::compile(&plan, &options)?;
            cq_reduce::check(&plan, &reduced, &options)?;
            plan = reduced;
        }
        cq_elaborate::compile(&plan)
    })
    .into()
}
macro_rules! planner {
    ($name:ident, $heuristic:expr, $split:ident, $reduce:expr) => {
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            planned(
                input,
                cq_order::Options {
                    heuristic: $heuristic,
                    split: cq_order::Split::$split,
                },
                $reduce,
            )
        }
    };
}
planner!(
    plan_generic,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Fine,
    false
);
planner!(plan_hypergraph, cq_order::Heuristic::Written, Coarse, true);
planner!(
    plan_prefix_connected,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Coarse,
    false
);
planner!(
    plan_prefix_connected_fine,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Fine,
    false
);
planner!(
    plan_prefix_greedy,
    cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
    Coarse,
    false
);
planner!(
    plan_prefix_greedy_fine,
    cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
    Fine,
    false
);
planner!(
    plan_elimination,
    cq_order::Heuristic::Elimination,
    Coarse,
    false
);
planner!(
    plan_elimination_fine,
    cq_order::Heuristic::Elimination,
    Fine,
    false
);

#[proc_macro]
pub fn merged(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for definition in &mut target.program.definitions {
            if let Operator::Probe(_, k) = &definition.operator {
                if !k.columns.is_empty() {
                    let mut k = k.clone();
                    k.choice = None;
                    definition.operator = Operator::Merge(Default::default(), k);
                }
            }
        }
        retain(&mut target);
        Ok(target)
    })
    .into()
}
fn retain(target: &mut physical::Program) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut keyed = BTreeSet::new();
    let mut ordered: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for d in &target.program.definitions {
        if let Some(input) = d.operator.input() {
            *counts.entry(input.to_string()).or_default() += 1;
        }
        match &d.operator {
            Operator::Probe(_, k) | Operator::Semi(_, k) => {
                *counts.entry(k.right.to_string()).or_default() += 1;
                keyed.insert(k.right.to_string());
            }
            Operator::Merge(_, k) => {
                *counts.entry(k.right.to_string()).or_default() += 1;
                let cols: Vec<_> = k.columns.iter().map(ToString::to_string).collect();
                ordered
                    .entry(k.left.to_string())
                    .or_default()
                    .push(cols.clone());
                ordered.entry(k.right.to_string()).or_default().push(cols);
            }
            _ => {}
        }
    }
    *counts
        .entry(target.program.returned.to_string())
        .or_default() += 1;
    for d in &mut target.program.definitions {
        if matches!(d.operator, Operator::Rename(..)) {
            continue;
        }
        let name = d.name.to_string();
        let own = match &d.operator {
            Operator::Merge(_, k) => Some(
                k.columns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let retained = counts.get(&name).copied().unwrap_or(0) >= 2
            || keyed.contains(&name)
            || ordered
                .get(&name)
                .into_iter()
                .flatten()
                .any(|c| Some(c) != own.as_ref());
        d.mode = if retained {
            Mode::Let(Default::default())
        } else {
            Mode::Pipe(Default::default())
        };
    }
}

#[proc_macro]
pub fn canonical_projects(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        let facts = physical::contract::check(source)?;
        let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for d in &mut target.program.definitions {
            let name = d.name.to_string();
            let heading = facts.headings[&name].iter().cloned().collect();
            let inherited = match &d.operator {
                Operator::Unit(_) => Some(BTreeSet::new()),
                Operator::Rename(_) => None,
                Operator::Probe(_, k) | Operator::Merge(_, k) => keys
                    .get(&k.left.to_string())
                    .zip(keys.get(&k.right.to_string()))
                    .map(|(l, r)| l.union(r).cloned().collect()),
                Operator::Semi(_, k) => keys.get(&k.left.to_string()).cloned(),
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                    keys.get(&s.input.to_string()).cloned()
                }
                Operator::Bind(b) => keys.get(&b.input.to_string()).cloned(),
                Operator::Guard(g) => keys.get(&g.input.to_string()).cloned(),
                Operator::Project(p) => keys
                    .get(&p.input.to_string())
                    .filter(|k| k.is_subset(&heading))
                    .cloned(),
            };
            d.canon = (inherited.is_none()
                && (matches!(d.operator, Operator::Project(_))
                    || d.name == target.program.returned))
                .then(Default::default);
            if let Some(key) = inherited.or_else(|| d.canon.is_some().then_some(heading)) {
                keys.insert(name, key);
            }
        }
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn smallest(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        let facts = physical::contract::check(source)?;
        for i in 0..source.program.definitions.len().saturating_sub(1) {
            if physical::binary_intersection(
                &source.program.definitions[i],
                &source.program.definitions[i + 1],
                &facts,
            )
            .is_some()
            {
                if let Operator::Probe(_, k) = &mut target.program.definitions[i].operator {
                    k.choice = Some(Choice::Smallest(Default::default()));
                }
            }
        }
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn ordered_dedup(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            match &mut entry.structure {
                Structure::Dedup { ty, .. } | Structure::HeadDedup { ty, .. } => {
                    *ty = syn::parse_quote!(::std::collections::BTreeSet<_>)
                }
                _ => {}
            }
        }
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn ordered_indexes(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            if let Structure::Built { keyed, .. } = &mut entry.structure {
                keyed.ty = syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>);
            }
        }
        Ok(target)
    })
    .into()
}
/// Surface example: expands identity!{ R(a) } as a normal CQ clause.
#[proc_macro]
pub fn expand_identity(input: TokenStream) -> TokenStream {
    cq_stage::pass::<proc_macro2::TokenStream, proc_macro2::TokenStream>(
        input.into(),
        |source, args| {
            if args.to_string() != "expected" {
                return Err(syn::Error::new_spanned(
                    args,
                    "opaque arguments were not forwarded",
                ));
            }
            let tokens: Vec<_> = source.clone().into_iter().collect();
            let mut output = proc_macro2::TokenStream::new();
            let mut i = 0;
            while i < tokens.len() {
                if matches!(&tokens[i], proc_macro2::TokenTree::Ident(id) if id == "identity")
                    && tokens.get(i + 1).is_some_and(
                        |t| matches!(t, proc_macro2::TokenTree::Punct(p) if p.as_char() == '!'),
                    )
                {
                    if let Some(proc_macro2::TokenTree::Group(group)) = tokens.get(i + 2) {
                        output.extend(group.stream());
                        i += 3;
                        continue;
                    }
                }
                output.extend([tokens[i].clone()]);
                i += 1;
            }
            Ok(output)
        },
    )
    .into()
}
#[proc_macro]
pub fn custom_indexes(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            if let Structure::Built { keyed, .. } = &mut entry.structure {
                keyed.ty = syn::parse_quote!(SortedIndex<_, _>);
            }
        }
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn identity_structures(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| Ok(source.clone()))
        .into()
}
