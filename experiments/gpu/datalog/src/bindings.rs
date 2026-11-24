//! Checked construction of complete GPU Round invocations before translation.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, Hole, HoleInterface, InputSource, Program, storage};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, Type};

/// Produce input records with the shared Structures declaration constructor.
/// Statistics demand, when selected, belongs to Structures refinement; native
/// input bindings do not invent publication demands.
pub(crate) fn input_providers(
    program: &Program,
) -> syn::Result<BTreeMap<String, cq_lang_structures::Provider>> {
    let context = &program.context;
    let device = program.storage.device.as_ref()
        .ok_or_else(|| syn::Error::new_spanned(program, "missing device storage records"))?;
    let provider = |source: &storage::Provider,
                    input: &cq_lang_structures::provider::InputSchema,
                    word: &Type,
                    state: &storage::DeviceState| -> syn::Result<cq_lang_structures::Provider> {
        let declaration = device.providers.iter()
            .find(|declaration| declaration.token == source.token)
            .ok_or_else(|| syn::Error::new_spanned(&source.token, "undeclared device input provider"))?;
        let mut instance = declaration.instance();
        instance.input = Some(input.clone());
        instance.radix = Some(cq_lang_structures::provider::RadixAdvice {
            word: word.clone(),
            offset: state.offset.clone(),
            ratio: state.ratio,
            radix_bits: state.radix_bits,
            merge_tile: state.merge_tile_rows,
        });
        instance.ty = source.ty.clone();
        Ok(instance)
    };
    let mut inputs = BTreeMap::new();
    for (relation, state) in context.relations.iter().zip(&device.relations) {
        if let Some(delta) = &relation.delta {
            inputs.insert(
                delta.to_string(),
                provider(&relation.providers.delta, &state.input, &state.word,
                    &device.layouts[relation.providers.canonical])?,
            );
        }
    }
    for (layout, state) in context
        .layouts
        .iter()
        .zip(&device.layouts)
        .chain(context.delta_layouts.iter().zip(&device.delta_layouts))
    {
        inputs.insert(
            layout.name.to_string(),
            provider(&layout.provider, &state.input, &state.word, state)?,
        );
    }
    for (component, stratum) in context.strata.iter().enumerate() {
        for key in &stratum.keys {
            ensure(
                !key.columns.is_empty(),
                program,
                "GPU Datalog does not support unit-valued existential negation keys",
            )?;
            let state = &device.negations[crate::print::negation(program, component, key)?];
            inputs.insert(key.name.to_string(), provider(&key.provider, &state.input, &state.word, state)?);
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            if let Some(scan) = &rule.scan {
                let input = match scan.source {
                    InputSource::Delta(index) => provider(
                        &context.relations[index].providers.delta,
                        &device.relations[index].input,
                        &device.relations[index].word,
                        &device.layouts[context.relations[index].providers.canonical],
                    ),
                    InputSource::Layout(index) => provider(
                        &context.layouts[index].provider,
                        &device.layouts[index].input,
                        &device.layouts[index].word,
                        &device.layouts[index],
                    ),
                }?;
                inputs.insert(scan.binding.to_string(), input);
            }
        }
    }
    Ok(inputs)
}

/// The recorded device provider of each component's negative key set, as a
/// `radix_relation` token instance (declared by device storage).
fn key_providers(program: &Program) -> syn::Result<Vec<Vec<round::storage::Provider>>> {
    let device = program
        .storage
        .device
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(program, "missing device storage records"))?;
    program
        .context
        .strata
        .iter()
        .enumerate()
        .map(|(component, stratum)| {
            stratum
                .keys
                .iter()
                .map(|key| {
                    let state = &device.negations[crate::print::negation(program, component, key)?];
                    Ok(round::storage::Provider {
                        token: state.token.clone(),
                        ty: state.provider.clone(),
                    })
                })
                .collect()
        })
        .collect()
}

/// Each input carries its Storage token, declared GPU facts and physical schema.
fn packet(
    hole: &Hole,
    inputs: &BTreeMap<String, cq_lang_structures::Provider>,
    providers: &[cq_lang_structures::Declaration],
) -> TokenStream {
    let declared = hole
        .cq
        .declarations
        .iter()
        .map(|declaration| declaration.name.to_string())
        .collect::<BTreeSet<_>>();
    let inputs = inputs
        .iter()
        .filter(|(name, _)| declared.contains(*name))
        .collect::<Vec<_>>();
    let providers = providers.iter().filter(|declaration| {
        inputs
            .iter()
            .any(|(_, provider)| provider.token == declaration.token)
    });
    let bindings = inputs.iter().map(|(name, provider)| {
        let name = Ident::new(name, proc_macro2::Span::call_site());
        quote!(#name => #provider;)
    });
    quote!(providers { #(#providers)* } #(#bindings)*)
}

/// A publication is a barrier even when it does not publish the current head:
/// a replay may still read one of the full or delta hosts it replaces. Keep
/// the same cut of the checked schedule for both planning and printing.
pub(crate) struct Epoch<'a> {
    pub rules: Vec<usize>,
    pub publications: Vec<&'a round::Publication>,
}

pub(crate) fn epochs<'a>(
    stratum: &'a round::Stratum,
    schedule: &round::schedule::StratumSchedule,
    phase: &str,
) -> Vec<Epoch<'a>> {
    let after: BTreeMap<_, _> = schedule.publication.iter()
        .filter(|placement| placement.phase == phase)
        .map(|placement| (placement.relation, placement.wave))
        .collect();
    let mut epochs = Vec::new();
    let mut rules = Vec::new();
    for (global, wave) in schedule.waves.iter().enumerate().filter(|(_, wave)| wave.phase == phase) {
        rules.extend_from_slice(&wave.holes);
        let publications: Vec<_> = stratum.publication.iter()
            .filter(|p| after.get(&p.relation) == Some(&global))
            .collect();
        if !publications.is_empty() {
            epochs.push(Epoch { rules: std::mem::take(&mut rules), publications });
        }
    }
    let publications: Vec<_> = stratum.publication.iter()
        .filter(|p| !after.contains_key(&p.relation)).collect();
    if !rules.is_empty() || !publications.is_empty() {
        epochs.push(Epoch { rules, publications });
    }
    epochs
}

/// A nested function may call an enclosing item, but that does not imply the
/// call is safe to evaluate twice. Replay only ordinary relational expressions
/// over the checked snapshot, not arbitrary Rust captures, calls or blocks.
fn replay_safe(cq: &round::cq::Program) -> bool {
    fn scalar(expr: &syn::Expr, bound: &BTreeSet<String>) -> bool {
        use syn::Expr;
        match expr {
            Expr::Path(path) => path.path.get_ident()
                .is_some_and(|name| bound.contains(&cq_syntax::symbol_name(name))),
            Expr::Lit(_) | Expr::Infer(_) => true,
            Expr::Paren(expr) => scalar(&expr.expr, bound),
            Expr::Group(expr) => scalar(&expr.expr, bound),
            Expr::Cast(expr) => scalar(&expr.expr, bound),
            Expr::Unary(expr) => scalar(&expr.expr, bound),
            Expr::Binary(expr) => {
                use syn::BinOp::*;
                matches!(expr.op, Add(_) | Sub(_) | Mul(_) | Div(_) | Rem(_) | And(_) | Or(_) |
                    BitXor(_) | BitAnd(_) | BitOr(_) | Shl(_) | Shr(_) | Eq(_) | Lt(_) | Le(_) |
                    Ne(_) | Ge(_) | Gt(_)) && scalar(&expr.left, bound) && scalar(&expr.right, bound)
            }
            _ => false,
        }
    }
    use round::cq::BodyItem;
    if !cq.query.captures.items.is_empty() || crate::print::reads_environment(cq) {
        return false;
    }
    let mut bound = BTreeSet::new();
    for item in &cq.query.body {
        match item {
            BodyItem::Positive { atom } => {
                if !atom.args.iter().all(|arg| cq_syntax::rust::variable(arg).is_some() || scalar(arg, &bound)) {
                    return false;
                }
                bound.extend(atom.args.iter().filter_map(cq_syntax::rust::variable).map(cq_syntax::symbol_name));
            }
            BodyItem::Negation(n) if n.atom.args.iter().all(|arg| scalar(arg, &bound)) => {}
            BodyItem::Filter(f) if scalar(&f.condition, &bound) => {}
            _ => return false,
        }
    }
    cq.query.head.args.iter().all(|arg| scalar(arg, &bound))
}

/// Membership is all-or-none for a head in a phase. In particular an earlier
/// epoch cannot contribute even a legacy batch to a replayed head.
fn eligible_heads(program: &Program) -> Vec<Vec<usize>> {
    let mut eligible = Vec::new();
    for (stratum, schedule) in program.context.strata.iter().zip(&program.schedule.strata) {
        for (phase, rules) in [(round::schedule::SEED, &stratum.seeds), (round::schedule::STEP, &stratum.steps)] {
            let mut members = BTreeMap::<usize, Vec<usize>>::new();
            for rule in rules {
                members.entry(rule.head).or_default().push(rule.hole);
            }
            for epoch in epochs(stratum, schedule, phase) {
                for publication in epoch.publications {
                    let Some(holes) = members.remove(&publication.relation) else { continue };
                    let local = epoch.rules.iter().filter(|&&i| rules[i].head == publication.relation).count();
                    if local == holes.len() && holes.iter().all(|&i| replay_safe(&program.holes[i].cq)) {
                        eligible.push(holes);
                    }
                }
            }
        }
    }
    eligible
}

fn arena_argument(head: bool) -> TokenStream {
    if head { quote!(head; &mut *__cq_gpu_arena) } else { quote!(&mut *__cq_gpu_arena) }
}

pub(crate) fn head_argument(hole: &Hole) -> bool {
    hole.pipeline.passes.last().is_some_and(|terminal| {
        crate::shared::unpack(&terminal.arguments).and_then(|(_, args)| crate::union_project::split(&args))
            .is_ok_and(|(_, arena)| arena.to_string() == arena_argument(true).to_string())
    })
}

/// Record the invocation owner by ordinal, independent of the invoked macro's name.
/// With `head_cursor`, `legacy("rule@delta", ...)` excludes the entire head
/// in that version's phase before a terminal ABI is recorded.
pub fn compile_with(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    struct Owner {
        pass: usize,
        head_cursor: bool,
        legacy: BTreeMap<String, (proc_macro2::Span, bool)>,
    }
    impl syn::parse::Parse for Owner {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let name: Ident = input.parse()?;
            if name != "binding" {
                return Err(syn::Error::new_spanned(
                    name,
                    "expected binding = invocation_index",
                ));
            }
            input.parse::<syn::Token![=]>()?;
            let owner = input.parse::<syn::LitInt>()?.base10_parse()?;
            let head_cursor = !input.is_empty();
            let mut legacy = BTreeMap::new();
            if head_cursor {
                input.parse::<syn::Token![,]>()?;
                let flag: Ident = input.parse()?;
                ensure(flag == "head_cursor", &flag, "expected head_cursor")?;
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
                let flag: Ident = input.parse()?;
                ensure(flag == "legacy", &flag, "expected legacy(\"rule@delta\", ...)")?;
                let labels;
                syn::parenthesized!(labels in input);
                for label in labels.parse_terminated(|input| input.parse::<syn::LitStr>(), syn::Token![,])? {
                    let previous = legacy.insert(label.value(), (label.span(), false));
                    ensure(previous.is_none(), &label, "duplicate legacy rule version")?;
                }
            }
            Ok(Self { pass: owner, head_cursor, legacy })
        }
    }
    let Owner { pass: owner, head_cursor, mut legacy } = syn::parse2(arguments)?;
    round::contract::check(source)?;
    crate::storage::check_device(&source.storage)?;
    let mut target = source.clone();
    let inputs = input_providers(&target)?;
    let union_projects = crate::union_project::plans(&target, &inputs);
    let mut excluded = BTreeSet::new();
    if !legacy.is_empty() {
        for stratum in &source.context.strata {
            for rule in stratum.seeds.iter().chain(&stratum.steps) {
                let label = crate::print::version_label(source, rule.variant, rule.head);
                if let Some((_, matched)) = legacy.get_mut(&label) {
                    *matched = true;
                    excluded.insert(rule.hole);
                }
            }
        }
        if let Some((label, (span, _))) = legacy.iter().find(|(_, (_, matched))| !*matched) {
            return Err(syn::Error::new(*span, format!("legacy exclusion `{label}` names no rule version")));
        }
    }
    let selected: BTreeSet<_> = if head_cursor && std::env::var_os("CQ_EXPANSION_COUNTERS").is_none() {
        eligible_heads(source).into_iter()
            .filter(|head| !head.iter().any(|hole| excluded.contains(hole)))
            .flatten()
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut shared = BTreeMap::new();
    let symbols = crate::storage::Symbols::from_program(&source.storage)?;
    let mut shared_excluded = excluded.clone();
    shared_excluded.extend(union_projects.keys().copied());
    for groups in crate::shared::plan(source,&shared_excluded,&symbols).into_values() {
        for group in groups {
            let first = group.holes().next().expect("nonempty RoundShared branches");
            let cut = group.cut();
            shared.extend(group.holes().map(|i| (i,quote!(round_shared(#first,#cut);))));
        }
    }
    for (i, hole) in target.holes.iter_mut().enumerate() {
        ensure(
            hole.interface == HoleInterface::Batch,
            &*hole,
            "GPU Datalog requires batch holes",
        )?;
        ensure(
            owner > hole.lower && owner < hole.pipeline.passes.len().saturating_sub(1),
            &*hole,
            "binding owner must follow lowering and precede the terminal",
        )?;
        ensure(
            hole.binding.is_none() && hole.pipeline.passes[owner].arguments.is_empty(),
            &*hole,
            "binding invocation already has arguments or an owner",
        )?;
        let bindings = packet(
            hole,
            &inputs,
            &target
                .storage
                .device
                .as_ref()
                .expect("checked device records")
                .providers,
        );
        hole.binding = Some(owner);
        hole.pipeline.passes[owner].arguments = bindings;
        let terminal = hole.pipeline.passes.last_mut().expect("checked terminal");
        ensure(
            terminal.arguments.is_empty(),
            &terminal.path,
            "GPU batch terminal already has arguments",
        )?;
        let marker = shared.get(&i);
        let choice = union_projects.get(&i);
        let arena = arena_argument(selected.contains(&i));
        terminal.arguments = quote!(#marker #choice #arena);
    }
    round::check_rewrite(source, &target)?;
    check(&target)?;
    Ok(target)
}

/// Formal rule DL-DeviceBindings: negative key sets use their recorded device
/// providers, the declared owner contains exactly the input-layout packet and
/// the terminal contains its complete arena argument.
pub fn check(program: &Program) -> syn::Result<()> {
    checked(program).map(drop)
}

pub(crate) fn checked(program: &Program) -> syn::Result<crate::shared::Phases> {
    for (stratum, providers) in program.context.strata.iter().zip(key_providers(program)?) {
        for (key, provider) in stratum.keys.iter().zip(providers) {
            ensure(
                key.provider == provider && key.hasher == round::storage::Hasher::None,
                key,
                "GPU key set differs from its recorded device provider",
            )?;
        }
    }
    let inputs = input_providers(program)?;
    let union_projects = crate::union_project::plans(program, &inputs);
    let marked: BTreeSet<_> = program.holes.iter().enumerate()
        .filter(|(_, hole)| head_argument(hole)).map(|(i, _)| i).collect();
    if !marked.is_empty() {
        let mut valid = BTreeSet::new();
        if std::env::var_os("CQ_EXPANSION_COUNTERS").is_none() {
            for head in eligible_heads(program) {
                if head.iter().all(|hole| marked.contains(hole)) {
                    valid.extend(head);
                }
            }
        }
        ensure(valid == marked, program, "head arena argument requires a complete replay-safe head in one publication epoch")?;
    }
    for (i, hole) in program.holes.iter().enumerate() {
        let owner = hole
            .binding
            .ok_or_else(|| syn::Error::new_spanned(hole, "missing GPU binding invocation owner"))?;
        let invocation = hole.pipeline.passes.get(owner).ok_or_else(|| {
            syn::Error::new_spanned(hole, "GPU binding invocation owner is out of bounds")
        })?;
        ensure(
            owner > hole.lower && owner < hole.pipeline.passes.len().saturating_sub(1),
            hole,
            "GPU binding invocation owner is outside the Structures pipeline",
        )?;
        ensure(
            invocation.arguments.to_string()
                == packet(
                    hole,
                    &inputs,
                    &program
                        .storage
                        .device
                        .as_ref()
                        .expect("checked device records")
                        .providers,
                )
                .to_string(),
            hole,
            "GPU binding invocation differs from its declared inputs",
        )?;
        ensure(
            hole.pipeline.passes.last().is_some_and(|terminal| {
                crate::shared::unpack(&terminal.arguments).is_ok_and(|(marker,args)| {
                    crate::union_project::split(&args).is_ok_and(|(plan,arena)| {
                        (marker.is_none() || plan.is_none()) && plan.as_ref() == union_projects.get(&i) &&
                            [false,true].into_iter().any(|head| arena.to_string() == arena_argument(head).to_string())
                    })
                })
            }),
            hole,
            "GPU batch invocation differs from its recorded arena argument or incompatible RoundShared/UnionProject certificate",
        )?;
    }
    crate::shared::check(program)
}
