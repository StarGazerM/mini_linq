use crate::predicate::Constants;
use cq_lang_structures::{
    self as ir,
    physical::{self, Operator},
    syntax::{self, symbol_name},
};
use cq_print::Context;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::{Ident, Type};

static MODULE: AtomicUsize = AtomicUsize::new(0);
fn id(role: &str, name: &str) -> Ident {
    format_ident!("__cq_gpu_{}_{}", role, name, span = Span::mixed_site())
}
fn fail<T>(at: &impl ToTokens, code: &str, detail: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(at, format!("{code}: {detail}")))
}
fn type_name(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}
fn qualified(ty: &Type, module: &str, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.qself.is_none()
        && path.path.segments.len() == 2
        && path.path.segments[0].ident == module
        && path.path.segments[1].ident == name)
}
fn known_vec(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.qself.is_none()
        && ((path.path.segments.len() == 1 && path.path.segments[0].ident == "Vec")
            || (path.path.segments.len() == 3
                && (path.path.segments[0].ident == "std" || path.path.segments[0].ident == "alloc")
                && path.path.segments[1].ident == "vec"
                && path.path.segments[2].ident == "Vec")))
}
fn integral(ty: &Type) -> bool {
    matches!(
        type_name(ty).as_deref(),
        Some("i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize")
    )
}
fn string(ty: &Type) -> bool {
    type_name(ty).as_deref() == Some("String")
        || matches!(ty,Type::Reference(r) if type_name(&r.elem).as_deref()==Some("str"))
}
fn supported(ty: &Type) -> bool {
    integral(ty) || string(ty) || type_name(ty).as_deref() == Some("bool")
}

fn membership(facts: &ir::Analysis, name: &str) -> bool {
    facts.membership.iter().any(|(build, _)| build == name)
}
fn directory_column<'a>(facts: &'a ir::Analysis, name: &str, provider: usize) -> &'a str {
    let build = &facts.built[name][provider];
    if qualified(&build.1, "cq_gpu_runtime", "PrefixRun") {
        crate::prefix::recorded_column(&build.1, &facts.layouts[name])
            .expect("validated recorded prefix directory")
    } else {
        &build.0[0][0]
    }
}
fn read_provider(facts: &ir::Analysis, keyed: &physical::Keyed) -> usize {
    let mut keys = keyed.columns.iter().map(symbol_name).collect::<Vec<_>>();
    keys.sort();
    facts.built[&symbol_name(&keyed.right)]
        .iter()
        .position(|build| serves(&build.0, &keys))
        .expect("validated keyed provider")
}
fn provider_id(role: &str, name: &str, provider: usize) -> Ident {
    if provider == 0 {
        id(role, name)
    } else {
        id(role, &format!("{name}_{provider}"))
    }
}
fn membership_budget(ty: &Type) -> syn::Result<usize> {
    if let Type::Path(path) = ty {
        if let syn::PathArguments::AngleBracketed(args) =
            &path.path.segments.last().unwrap().arguments
        {
            if let Some(syn::GenericArgument::Const(syn::Expr::Lit(lit))) = args.args.last() {
                if let syn::Lit::Int(value) = &lit.lit {
                    return value.base10_parse();
                }
            }
        }
    }
    fail(
        ty,
        "CQ_GPU_MEMBERSHIP_BUDGET",
        "AdaptiveKeySet must record a literal byte budget",
    )
}
fn serves(levels: &[Vec<String>], keys: &[String]) -> bool {
    let mut prefix = Vec::new();
    levels.iter().any(|level| {
        prefix.extend(level.iter().cloned());
        prefix.sort();
        prefix == keys
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Read {
    pub reader: String,
    pub build: String,
    pub keys: Vec<String>,
    pub source: String,
}

/// Reject unsupported *written* choices, including choices on unused results.
/// The output is a bijective ledger of all physical keyed reads, not just probes.
pub(crate) fn validate(program: &ir::Program, facts: &ir::Analysis) -> syn::Result<Vec<Read>> {
    for d in &program.physical.program.definitions {
        let n = &d.name;
        match &d.operator {
            Operator::Merge(..) => return fail(n, "CQ_GPU_MERGE", "merge is unsupported"),
            Operator::Union(..) => return fail(n, "CQ_GPU_UNION", "union blocks are unsupported"),
            Operator::Anti(..) => return fail(n, "CQ_GPU_ANTI", "anti is unsupported"),
            Operator::Guard(..) => {
                return fail(
                    n,
                    "CQ_GPU_OPAQUE_GUARD",
                    "opaque guard is unsupported; use a supported relational predicate",
                );
            }
            Operator::Bind(..) => {
                return fail(n, "CQ_GPU_OPAQUE_BIND", "opaque bind is unsupported");
            }
            Operator::Probe(_, k) | Operator::Semi(_, k)
                if matches!(k.choice, Some(physical::Choice::Smallest(_))) =>
            {
                return fail(n, "CQ_GPU_SMALLEST", "smallest traversal is unsupported");
            }
            _ => {}
        }
        if matches!(d.mode, physical::Mode::Exists(_)) {
            return fail(n, "CQ_GPU_EXISTS", "exists retention is unsupported");
        }
    }
    for entry in &program.structures.entries {
        match &entry.structure {
            ir::Structure::Built { keyed, .. } => {
                let bitmap = keyed.membership.is_some()
                    && qualified(&keyed.ty, "cq_storage", "AdaptiveKeySet");
                let prefix = qualified(&keyed.ty, "cq_gpu_runtime", "PrefixRun");
                if keyed.levels.len() != 1 && !prefix {
                    return fail(
                        entry,
                        "CQ_GPU_MULTILEVEL_BUILD",
                        "multilevel builds require explicitly recorded cq_gpu_runtime::PrefixRun",
                    );
                }
                if !qualified(&keyed.ty, "cq_storage", "SortedRun") && !prefix && !bitmap {
                    return fail(
                        entry,
                        "CQ_GPU_BUILD_TYPE",
                        "record SortedRun, PrefixRun, or certified AdaptiveKeySet membership",
                    );
                }
                if keyed.levels[0].columns.len() != 1 {
                    return fail(
                        entry,
                        "CQ_GPU_COMPOSITE_KEY",
                        "the run prefix must be one integral key column",
                    );
                }
                if prefix {
                    let name = symbol_name(&entry.name);
                    let recorded =
                        crate::prefix::recorded_column(&keyed.ty, &facts.layouts[&name])?;
                    if recorded != symbol_name(&keyed.levels[0].columns[0]) {
                        return fail(
                            entry,
                            "CQ_GPU_PREFIX_LEVEL",
                            "recorded prefix must be the first directory level",
                        );
                    }
                }
                for key in keyed.columns().map(symbol_name) {
                    if !integral(&facts.physical.rows[&symbol_name(&entry.name)].types[&key]) {
                        return fail(
                            entry,
                            "CQ_GPU_NON_INTEGRAL_KEY",
                            "key columns must have integral declared types",
                        );
                    }
                }
                if bitmap {
                    membership_budget(&keyed.ty)?;
                }
                if keyed.membership.is_some() && !bitmap {
                    return fail(
                        entry,
                        "CQ_GPU_MEMBERSHIP_TYPE",
                        "membership requires recorded AdaptiveKeySet",
                    );
                }
                let name = symbol_name(&entry.name);
                if membership(facts, &name) && facts.built[&name].len() != 1 {
                    return fail(
                        entry,
                        "CQ_GPU_MULTIPLE_MEMBERSHIP",
                        "membership-only results require one recorded provider",
                    );
                }
            }
            ir::Structure::Dedup { ty, .. } | ir::Structure::HeadDedup { ty, .. } => {
                let recorded = qualified(ty, "cq_gpu_runtime", "DeviceSet");
                if !recorded {
                    return fail(
                        entry,
                        "CQ_GPU_NON_DEVICE_CANON",
                        "canon must explicitly record ::cq_gpu_runtime::DeviceSet<_>",
                    );
                }
            }
            ir::Structure::Host { reader, .. } if reader != "scan" => {
                let name = symbol_name(reader);
                let constant = program.physical.program.definitions.iter().any(|d| {
                    symbol_name(&d.name) == name
                        && matches!(d.operator, Operator::Point(..) | Operator::Range(..))
                });
                if !constant {
                    return fail(
                        entry,
                        "CQ_GPU_HOST_KEYED_FALLBACK",
                        "host keyed fallback is unsupported; record a SortedRun let",
                    );
                }
            }
            ir::Structure::Rows { ty, .. } if !known_vec(ty) => {
                return fail(entry, "CQ_GPU_ROWS_TYPE", "retained rows must record Vec");
            }
            ir::Structure::Repr {
                repr: ir::Repr::Positions { .. },
                ..
            } => {
                return fail(
                    entry,
                    "CQ_GPU_POSITIONS",
                    "positions representations are unsupported",
                );
            }
            ir::Structure::JoinFilter { filter, .. } => {
                if !qualified(&filter.ty, "cq_storage", "AdaptiveKeySet") {
                    return fail(
                        entry,
                        "CQ_GPU_JOIN_FILTER_TYPE",
                        "join filters require recorded ::cq_storage::AdaptiveKeySet; Bloom and opaque providers are unsupported",
                    );
                }
                membership_budget(&filter.ty)?;
            }
            ir::Structure::Selection { .. }
            | ir::Structure::Read { .. }
            | ir::Structure::KeySet { .. } => {
                return fail(
                    entry,
                    "CQ_GPU_STRUCTURE",
                    "positional/keyset providers are unsupported",
                );
            }
            ir::Structure::Repr {
                witness: Some(w), ..
            } if !qualified(&w.ty, "cq_storage", "RunCursor") => {
                return fail(
                    entry,
                    "CQ_GPU_WITNESS_TYPE",
                    "witness must record RunCursor",
                );
            }
            _ => {}
        }
    }
    for input in program.physical.inputs() {
        for c in &input.columns {
            if !supported(&c.ty) {
                return fail(
                    c,
                    "CQ_GPU_COLUMN_TYPE",
                    "only integral, bool, String and &str columns are supported",
                );
            }
        }
    }
    let mut reads = Vec::new();
    for (build, items) in &facts.physical.keyed_reads {
        for (reader, keys, supply) in items {
            let source = if *supply == syntax::SupplyKind::Const {
                "scan-filter".to_owned()
            } else {
                let Some(builds) = facts.built.get(build) else {
                    return fail(
                        &program.structures,
                        "CQ_GPU_HOST_KEYED_FALLBACK",
                        format!("read {reader} of {build} has no recorded run"),
                    );
                };
                if !builds.iter().any(|build| serves(&build.0, keys)) {
                    return fail(
                        &program.structures,
                        "CQ_GPU_READ_KEY",
                        format!("read {reader} key does not match a recorded prefix"),
                    );
                }
                if let Some((origin, _)) = facts
                    .witnesses
                    .iter()
                    .find(|(origin, _)| facts.shares[*origin].force == *reader)
                {
                    format!("witness:{origin}")
                } else if membership(facts, build) {
                    "bitmap-membership".to_owned()
                } else if keys.len() > 1 {
                    "rank-point+residual".to_owned()
                } else {
                    "rank-point".to_owned()
                }
            };
            reads.push(Read {
                reader: reader.clone(),
                build: build.clone(),
                keys: keys.clone(),
                source,
            });
        }
    }
    Ok(reads)
}

pub(crate) fn compile(
    program: &ir::Program,
    options: crate::options::Options,
) -> syn::Result<syn::Expr> {
    let facts = ir::contract::check(program)?;
    let provenance = validate(program, &facts)?;
    // GPU join filters read uploaded columns, not host PositionalRows handles.
    // Keep the checked filter facts below; suppress only the CPU source binder.
    let mut source_facts = facts.clone();
    source_facts.join_filters.clear();
    let context = Context::new(program, &source_facts);
    let returned = symbol_name(&program.physical.program.returned);
    let endpoints: BTreeSet<_> = program
        .physical
        .program
        .definitions
        .iter()
        .filter(|d| {
            !matches!(d.mode, physical::Mode::Pipe(_))
                || d.canon.is_some()
                || symbol_name(&d.name) == returned
        })
        .map(|d| symbol_name(&d.name))
        .collect();
    let module_number = MODULE.fetch_add(1, Ordering::Relaxed);
    let module_id = format_ident!("__cq_gpu_module_{module_number}");
    let mut host = Vec::new();
    for input in program.physical.inputs() {
        let name = symbol_name(&input.name);
        let value = &context.hosts[&name].value;
        let table = id("host", &name);
        let width = input.columns.len();
        let schema = input.to_token_stream().to_string();
        let encoders = (0..width).map(|i| {
            let field = syn::Index::from(i);
            quote!(__encoded[#i]=::cq_gpu_runtime::encode_value(&__row.#field,__dictionary)?;)
        });
        host.push(quote!(let #table=__execution.encode_slice(::core::convert::AsRef::<[_]>::as_ref(#value),#schema,#width,|__row,__dictionary,__encoded| {#(#encoders)* Ok(())})?;));
    }
    let mut kernels = Vec::new();
    let mut kernel_reads = Vec::new();
    let mut covered_reads = BTreeMap::<String, usize>::new();
    let mut filter_reads = Vec::new();
    let mut covered_filters = BTreeSet::new();
    let mut all_constants = Constants::default();
    for definition in &program.physical.program.definitions {
        let endpoint = symbol_name(&definition.name);
        if !endpoints.contains(&endpoint) {
            continue;
        }
        let mut chain = vec![endpoint.clone()];
        loop {
            let d = context.physical[chain.last().unwrap()];
            let Some(input) = d.operator.input() else {
                break;
            };
            let input = symbol_name(input);
            if endpoints.contains(&input) {
                break;
            }
            chain.push(input);
        }
        chain.reverse();
        let first = context.physical[&chain[0]];
        let selection = if let Operator::Rename(rename) = &first.operator {
            if !chain
                .iter()
                .any(|name| facts.stages.contains_key(name) || facts.expands.contains(name))
                && chain.iter().all(|name| {
                    matches!(
                        &context.physical[name].operator,
                        Operator::Rename(_)
                            | Operator::Project(_)
                            | Operator::Select(_)
                            | Operator::Point(..)
                            | Operator::Range(..)
                            | Operator::Semi(..)
                    )
                })
            {
                let host_name = symbol_name(&rename.relation);
                let source = id("host", &host_name);
                let columns = context.carried_columns[&endpoint]
                    .iter()
                    .map(|column| {
                        let mapping = rename
                            .mapping
                            .iter()
                            .find(|m| symbol_name(&m.target) == *column)
                            .expect("selection column originates in rename");
                        context.hosts[&host_name]
                            .layout
                            .iter()
                            .position(|c| *c == symbol_name(&mapping.source))
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                let projection = !facts.join_filters.contains_key(&chain[0])
                    && chain.iter().all(|name| {
                        matches!(
                            &context.physical[name].operator,
                            Operator::Rename(_) | Operator::Project(_)
                        )
                    });
                Some((quote!(#source), columns, projection))
            } else {
                None
            }
        } else {
            None
        };
        let (driver, mut root, mut state) = match &first.operator {
            Operator::Rename(rename) => {
                let host_name = symbol_name(&rename.relation);
                let table = id("host", &host_name);
                let mut state = State::default();
                let mut loads = Vec::new();
                for mapping in &rename.mapping {
                    let column = symbol_name(&mapping.target);
                    if !context.carried_columns[&chain[0]].contains(&column) {
                        continue;
                    }
                    let index = context.hosts[&host_name]
                        .layout
                        .iter()
                        .position(|c| *c == symbol_name(&mapping.source))
                        .unwrap();
                    if selection.is_none()
                        && !chain
                            .iter()
                            .any(|n| facts.stages.contains_key(n) || facts.expands.contains(n))
                    {
                        state.columns.insert(
                            column,
                            quote!(::cq_gpu_runtime::column(__driver,#index,__row)),
                        );
                    } else {
                        let variable = id("value", &column);
                        loads.push(quote!(let #variable=if __valid {::cq_gpu_runtime::column(__driver,#index,__row)} else {0};));
                        state.columns.insert(column, quote!(#variable));
                    }
                }
                (
                    quote!(__execution.table_view(#table)?),
                    quote!(#(#loads)*),
                    state,
                )
            }
            Operator::Unit(_) => (
                quote!(::cq_gpu_runtime::TableView {
                    columns: ::core::ptr::null(),
                    len: 1
                }),
                quote!(),
                State::default(),
            ),
            _ => {
                let input = symbol_name(first.operator.input().ok_or_else(|| {
                    syn::Error::new_spanned(
                        first,
                        "CQ_GPU_STREAM_ROOT: expected an indexable producer",
                    )
                })?);
                let table = id("table", &input);
                let mut state = State::default();
                let mut loads = Vec::new();
                for (index, column) in context.carried_columns[&input].iter().enumerate() {
                    let variable = id("value", column);
                    loads.push(quote!(let #variable=if __valid {::cq_gpu_runtime::column(__driver,#index,__row)} else {0};));
                    state.columns.insert(column.clone(), quote!(#variable));
                }
                (
                    quote!(__execution.table_view(#table)?),
                    quote!(#(#loads)*),
                    state,
                )
            }
        };
        let start = usize::from(matches!(
            first.operator,
            Operator::Rename(_) | Operator::Unit(_)
        ));
        let runs: BTreeSet<_> = chain
            .iter()
            .filter_map(|n| match &context.physical[n].operator {
                Operator::Probe(_, k) | Operator::Semi(_, k) => {
                    Some((symbol_name(&k.right), read_provider(&facts, k)))
                }
                _ => None,
            })
            .collect();
        let mut run_ids: Vec<_> = runs
            .iter()
            .map(|(n, p)| provider_id("run_view", n, *p))
            .collect();
        let mut run_types = runs
            .iter()
            .map(|(name, _)| {
                if membership(&facts, name) {
                    quote!(::cq_gpu_runtime::MembershipView)
                } else {
                    quote!(::cq_gpu_runtime::RunView)
                }
            })
            .collect::<Vec<_>>();
        let mut run_bindings = runs
            .iter()
            .zip(&run_ids)
            .map(|((n, p), view)| {
                let run = provider_id("run", n, *p);
                if membership(&facts, n) {
                    quote!(let #view=__execution.membership_view(#run)?;)
                } else {
                    quote!(let #view=__execution.run_view(#run)?;)
                }
            })
            .collect::<Vec<_>>();
        let mut stream_filters = Vec::new();
        if let Some(filters) = facts.join_filters.get(&chain[0]) {
            for (index, filter) in filters.iter().enumerate() {
                let key = symbol_name(&filter.key.columns[0]);
                let value = state.columns.get(&key).ok_or_else(|| {
                    syn::Error::new_spanned(
                        filter,
                        "CQ_GPU_JOIN_FILTER_COLUMN: scan must carry its recorded filter key",
                    )
                })?;
                let filter_name = format!("{}_{}", chain[0], index);
                let handle = id("join_filter", &filter_name);
                let view = id("join_filter_view", &filter_name);
                run_ids.push(view.clone());
                run_types.push(quote!(::cq_gpu_runtime::MembershipView));
                run_bindings.push(quote!(let #view=__execution.membership_view(#handle)?;));
                root = quote!(#root __valid=__valid && ::cq_gpu_runtime::membership_contains(#view,#value););
                covered_filters.insert((chain[0].clone(), index));
                stream_filters.push(format!(
                    "{}:join-filter:{}:{}[{}]:covering={}:provider={}",
                    chain[0],
                    index,
                    filter.build,
                    key,
                    filter.covering,
                    filter.ty.to_token_stream()
                ));
            }
        }
        let batch = chain
            .iter()
            .filter_map(|n| facts.stages.get(n))
            .copied()
            .min()
            .unwrap_or(1024)
            .min(1024);
        let frontier = options.frontier != 0 && chain.iter().any(|n| facts.expands.contains(n));
        let mut emitter = Emitter {
            context: &context,
            facts: &facts,
            chain: &chain,
            endpoint: &endpoint,
            constants: &mut all_constants,
            shared_words: 16,
            continuations: Vec::new(),
            module_number,
            frontier,
            serial: !chain
                .iter()
                .any(|n| facts.stages.contains_key(n) || facts.expands.contains(n)),
            selection: selection.is_some(),
            batch: batch.min(256),
            frontier_threshold: options.frontier,
            descriptors: Vec::new(),
        };
        let body = emitter.steps(start, &mut state)?;
        let shared_words = emitter.shared_words;
        let descriptor_words = emitter.descriptors.iter().copied().max().unwrap_or(1);
        let continuations = emitter.continuations;
        let kernel = format_ident!("cq_gpu_stream_{}_{}", module_number, endpoint);
        for read in stream_filters {
            filter_reads.push((kernel.to_string(), read));
        }
        for read in provenance
            .iter()
            .filter(|read| chain.contains(&read.reader))
        {
            *covered_reads.entry(read.reader.clone()).or_default() += 1;
            kernel_reads.push((
                kernel.to_string(),
                format!(
                    "{}:{}[{}]:{}",
                    read.reader,
                    read.build,
                    read.keys.join(","),
                    read.source
                ),
            ));
        }
        kernels.push(Kernel {
            kernel,
            driver,
            root,
            body,
            run_ids,
            run_types,
            run_bindings,
            shared_words,
            descriptor_words,
            continuations,
            frontier,
            batch,
            selection,
            endpoint,
        });
    }
    for read in &provenance {
        if covered_reads.get(&read.reader) != Some(&1) {
            return fail(
                &program.structures,
                "CQ_GPU_READ_CORRESPONDENCE",
                format!(
                    "keyed read {} must occur in exactly one generated stream",
                    read.reader
                ),
            );
        }
    }
    for (scan, filters) in &facts.join_filters {
        for index in 0..filters.len() {
            if !covered_filters.contains(&(scan.clone(), index)) {
                return fail(
                    &program.structures,
                    "CQ_GPU_JOIN_FILTER_CORRESPONDENCE",
                    format!("recorded filter {scan}:{index} must occur at its scan"),
                );
            }
        }
    }
    let constant_ids: Vec<_> = all_constants
        .values
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    let constants=all_constants.values.iter().map(|(id,value)|quote!(let #id=::cq_gpu_runtime::encode_value(&(#value),__execution.dictionary_mut())?;));
    let mut device = Vec::new();
    for kernel in kernels {
        let (item, launch) = kernel.emit(&module_id, &constant_ids, &context, &facts);
        device.push(item);
        host.push(launch);
    }
    let output = id("table", &returned);
    let mut decoded = Vec::new();
    for expression in &program.physical.program.head.args {
        let Some(variable) = syntax::rust::variable(expression) else {
            return fail(
                expression,
                "CQ_GPU_HEAD_EXPR",
                "heads must contain column variables; compute relational columns before printing",
            );
        };
        let column = symbol_name(variable);
        let Some(index) = context.carried_columns[&returned]
            .iter()
            .position(|c| c == &column)
        else {
            return fail(
                expression,
                "CQ_GPU_HEAD_COLUMN",
                "head column is not carried",
            );
        };
        let ty = &facts.physical.rows[&returned].types[&column];
        let ty: Type = if string(ty) {
            syn::parse_quote!(::std::string::String)
        } else {
            ty.clone()
        };
        decoded.push(quote!(::cq_gpu_runtime::decode_value::<#ty>(__columns[#index][__row],__execution.dictionary())?));
    }
    let final_dedup = facts.head_dedup.contains_key(&returned);
    if final_dedup {
        return fail(
            &program.physical.program.head,
            "CQ_GPU_HEAD_CANON",
            "computed head canonicalization is unsupported",
        );
    }
    let reads = provenance.iter().map(|r| {
        format!(
            "{}:{}[{}]:{}",
            r.reader,
            r.build,
            r.keys.join(","),
            r.source
        )
    });
    let kernel_reads = kernel_reads
        .iter()
        .map(|(kernel, read)| quote!((#kernel,#read)));
    let filter_reads = filter_reads
        .iter()
        .map(|(kernel, read)| quote!((#kernel,#read)));
    let recorded_structures = program
        .structures
        .entries
        .iter()
        .map(|entry| entry.to_token_stream().to_string());
    let source_bindings = &context.statements;
    let has_kernels = device.iter().any(|item| !item.is_empty());
    let module = has_kernels.then(|| {
        quote! {
            #[::cuda_device::cuda_module]
            mod #module_id {
                use ::cuda_device::{device,kernel,launch_bounds,launch_contract};
                #(#device)*
            }
        }
    });
    let load =
        has_kernels.then(|| quote!(let __module=unsafe {#module_id::load(__runtime.context())?};));
    syn::parse2(quote!({
        #module
        const __CQ_GPU_KEYED_READS: &[&str]=&[#(#reads),*];
        const __CQ_GPU_KERNEL_READS: &[(&str,&str)]=&[#(#kernel_reads),*];
        const __CQ_GPU_JOIN_FILTER_READS: &[(&str,&str)]=&[#(#filter_reads),*];
        const __CQ_GPU_RECORDED_STRUCTURES: &[&str]=&[#(#recorded_structures),*];
        #(#source_bindings)*
        ::cq_gpu_runtime::with_runtime(|__runtime| {
            let mut __execution=__runtime.execution();
            #load
            #(#constants)*
            #(#host)*
            let __count=__execution.table_view(#output)?.len as usize;
            let __columns=__execution.download(#output)?;
            let mut __answer=::std::vec::Vec::with_capacity(__count);
            for __row in 0..__count { __answer.push((#(#decoded,)*)); }
            __execution.finish()?;
            Ok(__answer.into_iter())
        }).unwrap_or_else(|error|panic!("CQ_GPU_EXECUTION: {error}"))
    }))
}

#[derive(Clone, Default)]
struct State {
    columns: BTreeMap<String, Tokens>,
    witnesses: BTreeMap<String, (Tokens, Tokens)>,
}
struct Continuation {
    name: Ident,
    stage: u32,
    body: Tokens,
}
struct Emitter<'a, 'b> {
    context: &'a Context<'a>,
    facts: &'a ir::Analysis,
    chain: &'a [String],
    endpoint: &'a str,
    constants: &'b mut Constants,
    shared_words: usize,
    continuations: Vec<Continuation>,
    module_number: usize,
    frontier: bool,
    serial: bool,
    selection: bool,
    batch: usize,
    frontier_threshold: u32,
    descriptors: Vec<usize>,
}
impl Emitter<'_, '_> {
    fn steps(&mut self, index: usize, state: &mut State) -> syn::Result<Tokens> {
        if index == self.chain.len() {
            let values = self.context.carried_columns[self.endpoint]
                .iter()
                .map(|column| {
                    state.columns.get(column).cloned().ok_or_else(|| {
                        syn::Error::new(
                            Span::call_site(),
                            format!("CQ_GPU_COLUMN: missing output {column}"),
                        )
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;
            if self.selection {
                let writes = values.iter().enumerate().map(|(column,value)|quote!(::cq_gpu_runtime::write_output(__output,#column,__destination,#value);));
                return Ok(quote! {
                    let __destination=::cq_gpu_runtime::stable_select(__output,__counts,__scatter,__row,__valid,__scratch);
                    if __destination!=u64::MAX { #(#writes)* }
                });
            }
            if self.serial {
                return Ok(quote! {
                    if __valid { ::cq_gpu_runtime::emit(__output,[#(#values),*]); }
                });
            }
            let writes=values.iter().enumerate().map(|(column,value)|quote!(::cq_gpu_runtime::write_output(__output,#column,__base+__offset as u64,#value);));
            return Ok(quote! {
                let (__offset,__total)=::cq_gpu_runtime::tiles::scan(__valid as u32,__scratch);
                if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::reserve(__output,__total as u64);}
                ::cuda_device::thread::sync_threads();
                let __base=*(__scratch.add(8) as *const u64);
                if __valid {#(#writes)*}
                ::cuda_device::thread::sync_threads();
            });
        }
        let name = &self.chain[index];
        let definition = self.context.physical[name];
        match &definition.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                let condition = self.constants.theta(
                    &s.condition,
                    &state.columns,
                    &self.facts.physical.rows[&symbol_name(&s.input)].types,
                )?;
                state
                    .columns
                    .retain(|column, _| self.context.carried_columns[name].contains(column));
                let rest = self.steps(index + 1, state)?;
                Ok(quote! {__valid=__valid && (#condition);#rest})
            }
            Operator::Project(_) => {
                state
                    .columns
                    .retain(|c, _| self.context.carried_columns[name].contains(c));
                self.steps(index + 1, state)
            }
            Operator::Semi(_, k) if membership(self.facts, &symbol_name(&k.right)) => {
                self.member(index, state, k, false)
            }
            Operator::Probe(_, k) if membership(self.facts, &symbol_name(&k.right)) => {
                self.member(index, state, k, true)
            }
            Operator::Semi(_, k) => {
                let right = symbol_name(&k.right);
                let provider = read_provider(self.facts, k);
                let run = provider_id("run_view", &right, provider);
                let prefix = directory_column(self.facts, &right, provider);
                let key = state.columns[prefix].clone();
                let begin = id("begin", name);
                let end = id("end", name);
                if self.facts.witnesses.contains_key(name) {
                    state
                        .witnesses
                        .insert(name.clone(), (quote!(#begin), quote!(#end)));
                }
                let residual = self.residual(k, state, quote!(__candidate))?;
                state
                    .columns
                    .retain(|column, _| self.context.carried_columns[name].contains(column));
                let rest = self.steps(index + 1, state)?;
                Ok(quote! {
                    let (#begin,#end)=if __valid {::cq_gpu_runtime::blocked_run::point(#run.rank,#key).unwrap_or((0,0))}else{(0,0)};
                    let mut __candidate=#begin;
                    let mut __found=false;
                    while __candidate<#end && !__found {
                        __found=#residual;
                        __candidate+=1;
                    }
                    __valid=__valid && __found;
                    #rest
                })
            }
            Operator::Probe(_, k) if self.serial => self.serial_expand(index, state, k),
            Operator::Probe(_, k) => self.expand(index, state, k),
            _ => fail(
                &definition.name,
                "CQ_GPU_STREAM_OPERATOR",
                "unsupported stream operator",
            ),
        }
    }
    fn member(
        &mut self,
        index: usize,
        state: &mut State,
        k: &physical::Keyed,
        payload: bool,
    ) -> syn::Result<Tokens> {
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let view = provider_id("run_view", &right, provider);
        let key = state.columns[directory_column(self.facts, &right, provider)].clone();
        if payload {
            for column in &self.facts.physical.headings[&right] {
                if !self.context.carried_columns[&self.chain[index]].contains(column) {
                    continue;
                }
                if state.columns.contains_key(column) {
                    continue;
                }
                let Some(binding) = self.facts.filtered.get(&right).and_then(|f| {
                    f.constants
                        .iter()
                        .find(|b| symbol_name(&b.column) == *column)
                }) else {
                    return fail(
                        k,
                        "CQ_GPU_MEMBERSHIP_PAYLOAD",
                        "membership probe cannot expose unrecorded payload",
                    );
                };
                let value = self.constants.term(&binding.value, &state.columns)?;
                state.columns.insert(column.clone(), value);
            }
        }
        state
            .columns
            .retain(|column, _| self.context.carried_columns[&self.chain[index]].contains(column));
        let rest = self.steps(index + 1, state)?;
        Ok(quote! {__valid=__valid && ::cq_gpu_runtime::membership_contains(#view,#key);#rest})
    }
    // Exhaustive thread-local traversal needs neither a uniqueness promise nor
    // a shared-memory materialization. Explicit batch/expand stages retain the
    // cooperative traversal below, including their publication contract.
    fn serial_expand(
        &mut self,
        index: usize,
        state: &mut State,
        k: &physical::Keyed,
    ) -> syn::Result<Tokens> {
        let name = &self.chain[index];
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let run = provider_id("run_view", &right, provider);
        let begin = id("begin", name);
        let end = id("end", name);
        let row = id("row", name);
        let witness = self
            .facts
            .witnesses
            .keys()
            .find(|origin| self.facts.shares[*origin].force == *name)
            .cloned();
        let lookup = if let Some(origin) = witness {
            let (a, b) = state.witnesses.remove(&origin).ok_or_else(|| {
                syn::Error::new_spanned(k, "CQ_GPU_WITNESS: forcing read lost its recorded witness")
            })?;
            quote!((#a,#b))
        } else {
            let prefix = directory_column(self.facts, &right, provider);
            let key = &state.columns[prefix];
            quote!(::cq_gpu_runtime::blocked_run::point(#run.rank,#key).unwrap_or((0,0)))
        };
        let mut loads = Vec::new();
        for column in &self.facts.physical.headings[&right] {
            if !self.context.carried_columns[name].contains(column)
                || state.columns.contains_key(column)
            {
                continue;
            }
            let local = id("value", column);
            let value = if let Some(slot) = self.context.carried_columns[&right]
                .iter()
                .position(|c| c == column)
            {
                quote!(::cq_gpu_runtime::run_column(#run,#slot,#row))
            } else if let Some(constant) = self.facts.filtered.get(&right).and_then(|f| {
                f.constants
                    .iter()
                    .find(|b| symbol_name(&b.column) == *column)
            }) {
                self.constants.term(&constant.value, &state.columns)?
            } else {
                return fail(
                    k,
                    "CQ_GPU_PAYLOAD_COLUMN",
                    format!("recorded run {right} does not carry {column}"),
                );
            };
            loads.push(quote!(let #local=#value;));
            state.columns.insert(column.clone(), quote!(#local));
        }
        let residual = self.residual(k, state, quote!(#row))?;
        state
            .columns
            .retain(|c, _| self.context.carried_columns[name].contains(c));
        let rest = self.steps(index + 1, state)?;
        Ok(quote! {
            if __valid {
                let (#begin,#end)=#lookup;
                let mut #row=#begin;
                while #row<#end {
                    #(#loads)*
                    let mut __valid=#residual;
                    #rest
                    #row+=1;
                }
            }
        })
    }
    fn residual(&self, k: &physical::Keyed, state: &State, row: Tokens) -> syn::Result<Tokens> {
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let view = provider_id("run_view", &right, provider);
        let prefix = directory_column(self.facts, &right, provider);
        let mut tests = Vec::new();
        for key in k
            .columns
            .iter()
            .map(symbol_name)
            .filter(|key| key != prefix)
        {
            let slot = self.context.carried_columns[&right]
                .iter()
                .position(|c| c == &key)
                .ok_or_else(|| {
                    syn::Error::new_spanned(k, "CQ_GPU_RESIDUAL_KEY: missing retained key column")
                })?;
            let value = &state.columns[&key];
            tests.push(quote!(::cq_gpu_runtime::run_column(#view,#slot,#row)==#value));
        }
        Ok(quote!(true #(&& (#tests))*))
    }
    fn expand(
        &mut self,
        index: usize,
        state: &mut State,
        k: &physical::Keyed,
    ) -> syn::Result<Tokens> {
        let name = &self.chain[index];
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let run = provider_id("run_view", &right, provider);
        let begin = id("begin", name);
        let end = id("end", name);
        let witness = self
            .facts
            .witnesses
            .keys()
            .find(|origin| self.facts.shares[*origin].force == *name)
            .cloned();
        let lookup = if let Some(origin) = &witness {
            let (a, b) = state.witnesses.get(origin).ok_or_else(|| {
                syn::Error::new_spanned(k, "CQ_GPU_WITNESS: forcing read lost its recorded witness")
            })?;
            quote!(if __valid {(#a,#b)}else{(0,0)})
        } else {
            let prefix = directory_column(self.facts, &right, provider);
            let key = state.columns[prefix].clone();
            quote!(if __valid {::cq_gpu_runtime::blocked_run::point(#run.rank,#key).unwrap_or((0,0))}else{(0,0)})
        };
        if let Some(origin) = witness {
            state.witnesses.remove(&origin);
        }
        let offset = (self.shared_words + 1) & !1;
        let mut words = 516usize;
        let mut save = Vec::new();
        let mut restore = Vec::new();
        let mut child = State::default();
        for (column, value) in &state.columns {
            let slot = words;
            words += 512;
            let local = id("value", column);
            save.push(quote! {*(__descriptor.add(#slot) as *mut i64).add(__tid)=#value;});
            restore.push(quote!{let #local=if __valid {*(__descriptor.add(#slot) as *const i64).add(__parent)}else{0};});
            child.columns.insert(column.clone(), quote!(#local));
        }
        for (origin, (a, b)) in &state.witnesses {
            let slot = words;
            words += 512;
            let begin = id("begin", origin);
            let end = id("end", origin);
            save.push(
                quote! {*__descriptor.add(#slot+__tid)=#a;*__descriptor.add(#slot+256+__tid)=#b;},
            );
            restore.push(quote!{let #begin=if __valid {*__descriptor.add(#slot+__parent)}else{0};let #end=if __valid {*__descriptor.add(#slot+256+__parent)}else{0};});
            child
                .witnesses
                .insert(origin.clone(), (quote!(#begin), quote!(#end)));
        }
        self.shared_words = offset + words;
        let stage = self.continuations.len() as u32 + 1;
        let continuation = format_ident!(
            "cq_gpu_expand_{}_{}_{}",
            self.module_number,
            self.endpoint,
            name
        );
        // Reserve the tag before recursively assigning descendant tags.
        self.continuations.push(Continuation {
            name: continuation.clone(),
            stage,
            body: Tokens::new(),
        });
        let mut loads = Vec::new();
        for column in &self.facts.physical.headings[&right] {
            if !self.context.carried_columns[name].contains(column) {
                continue;
            }
            if child.columns.contains_key(column) {
                continue;
            }
            let local = id("value", column);
            let value = if let Some(slot) = self.context.carried_columns[&right]
                .iter()
                .position(|c| c == column)
            {
                quote!(::cq_gpu_runtime::run_column(#run,#slot,__build_row))
            } else if let Some(constant) = self.facts.filtered.get(&right).and_then(|f| {
                f.constants
                    .iter()
                    .find(|b| symbol_name(&b.column) == *column)
            }) {
                self.constants.term(&constant.value, &child.columns)?
            } else {
                return fail(
                    k,
                    "CQ_GPU_PAYLOAD_COLUMN",
                    format!("recorded run {right} does not carry {column}"),
                );
            };
            loads.push(quote!(let #local=if __valid {#value}else{0};));
            child.columns.insert(column.clone(), quote!(#local));
        }
        let residual = self.residual(k, &child, quote!(__build_row))?;
        child
            .columns
            .retain(|c, _| self.context.carried_columns[name].contains(c));
        let rest = self.steps(index + 1, &mut child)?;
        let batch = self.batch as u32;
        let continuation_body = quote! {
            let __position=__chunk as u64 * #batch as u64 + __tid as u64;
            let __total=*__descriptor.add(514);
            let mut __valid=__tid<#batch as usize && __position<__total as u64;
            let __parent=if __valid {::cq_gpu_runtime::tiles::owner(__descriptor.add(258),256,__position as u32) as usize}else{0};
            let __build_row=if __valid {*__descriptor.add(2+__parent)+__position as u32-*__descriptor.add(258+__parent)}else{0};
            #(#restore)* #(#loads)*
            __valid=__valid && (#residual);
            #rest
        };
        self.continuations
            .iter_mut()
            .find(|c| c.stage == stage)
            .unwrap()
            .body = continuation_body;
        self.descriptors.push(words);
        let publish = self.frontier && self.facts.expands.contains(name);
        let frontier_threshold = self.frontier_threshold as u64 * 1024;
        let call = quote!(__CQ_CALL_CONTINUATION);
        // Call placeholders are replaced structurally when the full constant ABI is known.
        Ok(quote! {
            let (#begin,#end)=#lookup;
            let __descriptor=__scratch.add(#offset);
            if __tid==0 {*__descriptor=#stage;*__descriptor.add(1)=0;*__descriptor.add(515)=0;}
            *__descriptor.add(2+__tid)=#begin;
            #(#save)*
            let (__offset,mut __total)=::cq_gpu_runtime::tiles::scan(#end-#begin,__scratch);
            let __wrapped=__offset.checked_add(#end-#begin).is_none();
            let (_,__overflows)=::cq_gpu_runtime::tiles::scan(__wrapped as u32,__scratch);
            if __overflows!=0 {
                if __tid==0 {::cq_gpu_runtime::flag_error(__output,2);}
                __total=0;
            }
            *__descriptor.add(258+__tid)=__offset;
            if __tid==0 {*__descriptor.add(514)=__total;}
            ::cuda_device::thread::sync_threads();
            let __chunks=(__total as u64).div_ceil(#batch as u64) as u32;
            let __published=if #publish && __total as u64>#frontier_threshold {
                let mut __padding=#words+__tid;
                while __padding<__frontier.descriptor_words as usize {
                    *__descriptor.add(__padding)=0;
                    __padding+=256;
                }
                ::cuda_device::thread::sync_threads();
                ::cq_gpu_runtime::frontier_publish(__frontier,__descriptor,__chunks,__scratch.add(12))
            } else {false};
            if !__published {
                let mut __chunk=0u32;
                while __chunk<__chunks {
                    #continuation(#call,__descriptor,__chunk);
                    __chunk+=1;
                }
            }
            ::cuda_device::thread::sync_threads();
        })
    }
}

struct Kernel {
    kernel: Ident,
    driver: Tokens,
    root: Tokens,
    body: Tokens,
    run_ids: Vec<Ident>,
    run_types: Vec<Tokens>,
    run_bindings: Vec<Tokens>,
    shared_words: usize,
    descriptor_words: usize,
    continuations: Vec<Continuation>,
    frontier: bool,
    batch: usize,
    endpoint: String,
    selection: Option<(Tokens, Vec<usize>, bool)>,
}
impl Kernel {
    fn emit(
        self,
        _module: &Ident,
        constants: &[Ident],
        context: &Context<'_>,
        facts: &ir::Analysis,
    ) -> (Tokens, Tokens) {
        let Self {
            kernel,
            driver,
            root,
            body,
            run_ids,
            run_types,
            run_bindings,
            shared_words,
            descriptor_words,
            continuations,
            frontier,
            batch,
            endpoint,
            selection,
        } = self;
        let prepare = format_ident!("prepare_{}", kernel);
        let table = id("table", &endpoint);
        let width = context.carried_columns[&endpoint].len();
        let shared_bytes =
            ((shared_words + if frontier { descriptor_words + 2 } else { 0 }) * 4) as u32;
        let common = quote!(__driver,#(#run_ids,)*#(#constants,)*__output,__frontier);
        let signature = quote!(__driver: ::cq_gpu_runtime::TableView,#(#run_ids: #run_types,)*#(#constants:i64,)*__output: ::cq_gpu_runtime::OutputView,__frontier: ::cq_gpu_runtime::FrontierView);
        let selection_args = selection
            .as_ref()
            .map(|_| quote!(__counts:*mut u32,__scatter:bool,));
        fn substitute(tokens: Tokens, replacement: &Tokens) -> Tokens {
            tokens
                .into_iter()
                .map(|token| match &token {
                    proc_macro2::TokenTree::Ident(i) if i == "__CQ_CALL_CONTINUATION" => {
                        replacement.clone()
                    }
                    proc_macro2::TokenTree::Group(g) => {
                        let mut group = proc_macro2::Group::new(
                            g.delimiter(),
                            substitute(g.stream(), replacement),
                        );
                        group.set_span(g.span());
                        Tokens::from(proc_macro2::TokenTree::Group(group))
                    }
                    _ => Tokens::from(token),
                })
                .collect()
        }
        let body = substitute(body, &common);
        let functions=continuations.iter().map(|c| {
            let name=&c.name;let body=substitute(c.body.clone(),&common);
            quote!{#[device] #[inline(always)] unsafe fn #name(#signature,__descriptor:*mut u32,__chunk:u32) {
                let __tid=::cuda_device::thread::threadIdx_x() as usize;
                let __scratch=::cuda_device::DynamicSharedArray::<u8>::get_raw() as *mut u32;
                #body
            }}
        });
        let dispatch = continuations.iter().map(|c| {
            let stage = c.stage;
            let name = &c.name;
            quote!(#stage=>#name(#common,__descriptor,__chunk),)
        });
        let scheduler = if frontier {
            quote! {
                loop {
                    if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::frontier_next_root(__frontier);}
                    ::cuda_device::thread::sync_threads();
                    let __tile=*(__scratch.add(8) as *const u64);
                    if __tile<__driver.len.div_ceil(#batch as u64) {
                        let mut __stripe=0usize;
                        while __stripe<#batch {
                            let __slot=__stripe+__tid;
                            let __row=__tile*#batch as u64+__slot as u64;
                            let mut __valid=__slot<#batch && __row<__driver.len;
                            #root #body
                            __stripe+=256;
                        }
                        if __tid==0 {::cq_gpu_runtime::frontier_done(__frontier);}
                    } else {
                        let __descriptor=__scratch.add(#shared_words);
                        if let Some(__chunk)=::cq_gpu_runtime::frontier_claim(__frontier,__descriptor,__scratch.add(12)) {
                            match *__descriptor {#(#dispatch)* _=>{}}
                            if __tid==0 {::cq_gpu_runtime::frontier_done(__frontier);}
                        } else {
                            if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::frontier_pending(__frontier);}
                            ::cuda_device::thread::sync_threads();
                            if *(__scratch.add(8) as *const u64)==0 {break;}
                        }
                    }
                    ::cuda_device::thread::sync_threads();
                }
            }
        } else {
            quote! {
                let mut __tile=::cuda_device::thread::blockIdx_x() as u64;
                while __tile<__driver.len.div_ceil(#batch as u64) {
                    let mut __stripe=0usize;
                    while __stripe<#batch {
                        let __slot=__stripe+__tid;
                        let __row=__tile*#batch as u64+__slot as u64;
                        let mut __valid=__slot<#batch && __row<__driver.len;
                        #root #body
                        __stripe+=256;
                    }
                    __tile+=::cuda_device::thread::gridDim_x() as u64;
                }
            }
        };
        let scheduler = if selection.is_some() {
            quote! {
                let __row=::cuda_device::thread::blockIdx_x() as u64*256+__tid as u64;
                let mut __valid=__row<__driver.len;
                #root #body
            }
        } else {
            scheduler
        };
        let device = quote! {
            #(#functions)*
            #[kernel]
            #[launch_bounds(256)]
            #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_bytes,#shared_bytes))]
            pub fn #kernel(#signature,#selection_args) {
                let __tid=::cuda_device::thread::threadIdx_x() as usize;
                let __scratch=::cuda_device::DynamicSharedArray::<u8>::get_raw() as *mut u32;
                unsafe {#scheduler}
            }
        };
        let canon = facts.dedup.contains_key(&endpoint);
        let dedup = canon.then(|| quote!(let #table=__execution.dedup(#table)?;));
        let build = facts.built.get(&endpoint).map(|builds| {
            let providers = builds.iter().enumerate().map(|(provider, build)| {
                let run = provider_id("run", &endpoint, provider);
                let key = directory_column(facts, &endpoint, provider);
                let slot = context.carried_columns[&endpoint]
                    .iter()
                    .position(|c| c == key)
                    .expect("checked carried run key");
                if membership(facts, &endpoint) {
                    let budget = membership_budget(&build.1).expect("validated membership budget");
                    quote!(let #run=__execution.prepare_membership(#table,#slot,#budget)?;)
                } else {
                    quote!(let #run=__execution.prepare_run(#table,#slot)?;)
                }
            });
            quote!(#(#providers)*)
        });
        let mut filter_builds = Vec::new();
        for (scan, filters) in &facts.join_filters {
            for (index, filter) in filters.iter().enumerate() {
                if symbol_name(&filter.build) != endpoint {
                    continue;
                }
                let handle = id("join_filter", &format!("{scan}_{index}"));
                let key = symbol_name(&filter.key.columns[0]);
                let slot = context.carried_columns[&endpoint]
                    .iter()
                    .position(|column| column == &key)
                    .expect("checked covering key is carried by its build");
                let budget = membership_budget(&filter.ty).expect("validated filter budget");
                filter_builds.push(
                    quote!(let #handle=__execution.prepare_membership(#table,#slot,#budget)?;),
                );
            }
        }
        let slots = if frontier { 64u32 } else { 0 };
        let launch = quote! {
            let #table={
                let __driver=#driver;
                #(#run_bindings)*
                let __pool=__execution.frontier(#slots,#descriptor_words as u32)?;
                let __frontier=__pool.view();
                let __grid=__driver.len.div_ceil(#batch as u64).clamp(1,568) as u32;
                let __launch=__module.#prepare(::cuda_core::LaunchConfig1D::new(__grid,256,#shared_bytes))?;
                __execution.execute(#width,__driver.len as usize,|__stream,__output| {
                    __pool.reset(__driver.len.div_ceil(#batch as u64))?;
                    unsafe {__module.#kernel(__stream,&__launch,#common)?;}
                    Ok(())
                })?
            };
            #dedup #build #(#filter_builds)*
        };
        let device = if selection
            .as_ref()
            .is_some_and(|(_, _, projection)| *projection)
        {
            Tokens::new()
        } else {
            device
        };
        let launch = if let Some((source, columns, projection)) = selection {
            if projection {
                quote! {
                    let #table=__execution.project_table(#source,&[#(#columns),*])?;
                    #dedup #build #(#filter_builds)*
                }
            } else {
                quote! {
                    let #table={
                        let __driver=#driver;
                        #(#run_bindings)*
                        let __pool=__execution.frontier(0,#descriptor_words as u32)?;
                        let __frontier=__pool.view();
                        let __grid=__driver.len.div_ceil(256).max(1) as u32;
                        let __launch=__module.#prepare(::cuda_core::LaunchConfig1D::new(__grid,256,#shared_bytes))?;
                        __execution.execute_selection(#source,&[#(#columns),*],|__stream,__output,__counts,__scatter| {
                            unsafe { __module.#kernel(__stream,&__launch,#common,__counts,__scatter)?; }
                            Ok(())
                        })?
                    };
                    #dedup #build #(#filter_builds)*
                }
            }
        } else {
            launch
        };
        (device, launch)
    }
}
