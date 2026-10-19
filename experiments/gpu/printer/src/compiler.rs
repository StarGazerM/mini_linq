use crate::predicate::Constants;
use cq_lang_structures::{
    self as ir,
    physical::{self, Operator},
    syntax::{self, symbol_name},
};
use cq_print::Context;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote, ToTokens};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordedCodec {
    Plain,
    For,
    Delta,
    Dictionary,
}
impl RecordedCodec {
    fn marker(self) -> Tokens {
        match self {
            Self::Plain => quote!(::cq_gpu_runtime::Plain),
            Self::For => quote!(::cq_gpu_runtime::For),
            Self::Delta => quote!(::cq_gpu_runtime::Delta),
            Self::Dictionary => quote!(::cq_gpu_runtime::DictionaryCodec),
        }
    }
    fn value(self) -> Tokens {
        match self {
            Self::Plain => quote!(::cq_gpu_runtime::Codec::Plain),
            Self::For => quote!(::cq_gpu_runtime::Codec::For),
            Self::Delta => quote!(::cq_gpu_runtime::Codec::Delta),
            Self::Dictionary => quote!(::cq_gpu_runtime::Codec::Dictionary),
        }
    }
}

// Host codecs belong to the bound value. Only derived representations read
// codec choices from a recorded provider type; no data/property inference occurs.
fn provider_codec(ty: &Type) -> syn::Result<RecordedCodec> {
    let Type::Path(path) = ty else {
        return fail(ty, "CQ_GPU_CODEC_TYPE", "expected a recorded provider type");
    };
    let syn::PathArguments::AngleBracketed(arguments) =
        &path.path.segments.last().unwrap().arguments
    else {
        return fail(ty, "CQ_GPU_CODEC_TYPE", "provider must name its row type");
    };
    let mut types = Vec::new();
    for argument in &arguments.args {
        match argument {
            syn::GenericArgument::Type(ty) => types.push(ty),
            syn::GenericArgument::Lifetime(_) => {}
            _ => return fail(ty, "CQ_GPU_CODEC_TYPE", "expected row and codec types"),
        }
    }
    match types.as_slice() {
        [_] => Ok(RecordedCodec::Plain),
        [_, codec] if qualified(codec, "cq_gpu_runtime", "Plain") => Ok(RecordedCodec::Plain),
        [_, codec] if qualified(codec, "cq_gpu_runtime", "For") => Ok(RecordedCodec::For),
        [_, codec] if qualified(codec, "cq_gpu_runtime", "Delta") => Ok(RecordedCodec::Delta),
        [_, codec] if qualified(codec, "cq_gpu_runtime", "DictionaryCodec") => {
            Ok(RecordedCodec::Dictionary)
        }
        _ => fail(
            ty,
            "CQ_GPU_CODEC_TYPE",
            "record one row type and optional cq_gpu_runtime::{Plain,For,Delta,DictionaryCodec}",
        ),
    }
}

fn materialized_codec(facts: &ir::Analysis, name: &str) -> syn::Result<RecordedCodec> {
    let rows = facts
        .retained
        .get(name)
        .map(|ty| {
            if known_vec(ty) {
                Ok(RecordedCodec::Plain)
            } else {
                provider_codec(ty)
            }
        })
        .transpose()?;
    let dedup = facts.dedup.get(name).map(provider_codec).transpose()?;
    if let (Some(rows), Some(dedup)) = (rows, dedup) {
        if rows != dedup {
            return fail(
                &facts.retained[name],
                "CQ_GPU_CODEC_CONFLICT",
                "retained rows and canon must record the same materialized codec",
            );
        }
    }
    Ok(rows.or(dedup).unwrap_or(RecordedCodec::Plain))
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
    for (name, &size) in &facts.stages {
        if size == 0 || size > 1024 {
            return fail(&program.structures, "CQ_GPU_STAGE_SIZE",
                format!("stage {name} records batch {size}; supported scan batches are 1..=1024"));
        }
    }
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
            Operator::Bind(bind) if bind.for_token.is_some() => {
                return fail(n, "CQ_GPU_GENERATOR", "iterator generators are unsupported");
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
                provider_codec(ty)?;
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
            ir::Structure::Rows { ty, .. } => {
                if qualified(ty, "cq_gpu_runtime", "DeviceColumns") {
                    provider_codec(ty)?;
                } else if !known_vec(ty) {
                    return fail(
                        entry,
                        "CQ_GPU_ROWS_TYPE",
                        "retained rows must record Vec or cq_gpu_runtime::DeviceColumns",
                    );
                }
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
    for definition in &program.physical.program.definitions {
        materialized_codec(facts, &symbol_name(&definition.name))?;
    }
    let mut reads = Vec::new();
    for (build, items) in &facts.physical.keyed_reads {
        for (reader, keys, supply) in items {
            let source = if matches!(supply, syntax::SupplyKind::Const | syntax::SupplyKind::Range) {
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

#[derive(Clone, Copy)]
enum RootDecode {
    Tile,
    Stripe,
}

// Rewrite root columns structurally, including demand-driven expressions and
// scan filters. Decode convergently before filtering: Delta needs every warp
// lane even when that row is invalid or fails an earlier predicate.
fn decode_root(
    root: &mut Tokens,
    columns: &mut BTreeMap<String, Tokens>,
    layout: RootDecode,
) -> syn::Result<(Tokens, Vec<usize>)> {
    use syn::visit_mut::VisitMut;

    #[derive(Default)]
    struct Decode {
        columns: BTreeMap<usize, Ident>,
    }
    impl VisitMut for Decode {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            let index = if let syn::Expr::Call(call) = expression {
                let runtime_column = matches!(&*call.func, syn::Expr::Path(path)
                    if path.qself.is_none() && path.path.segments.len() == 2
                        && path.path.segments[0].ident == "cq_gpu_runtime"
                        && path.path.segments[1].ident == "column");
                if runtime_column
                    && call.args.len() == 3
                    && matches!(&call.args[0], syn::Expr::Path(path)
                        if path.path.is_ident("__driver"))
                    && matches!(&call.args[2], syn::Expr::Path(path)
                        if path.path.is_ident("__row"))
                {
                    match &call.args[1] {
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Int(index),
                            ..
                        }) => index.base10_parse::<usize>().ok(),
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(index) = index {
                let values = self
                    .columns
                    .entry(index)
                    .or_insert_with(|| id("tile_column", &index.to_string()));
                *expression = syn::parse_quote!(#values);
            } else {
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
    }

    let mut decode = Decode::default();
    let mut block: syn::Block = syn::parse2(quote!({#root}))?;
    decode.visit_block_mut(&mut block);
    let statements = block.stmts;
    *root = quote!(#(#statements)*);
    for tokens in columns.values_mut() {
        let mut expression: syn::Expr = syn::parse2(tokens.clone())?;
        decode.visit_expr_mut(&mut expression);
        *tokens = expression.to_token_stream();
    }
    let mut loads = Tokens::new();
    let mut rows = Tokens::new();
    for (index, value) in &decode.columns {
        let view = id("root_column", &index.to_string());
        match layout {
            RootDecode::Tile => {
                let [a, b] = [0, 1].map(|stripe| id("tile_register", &format!("{index}_{stripe}")));
                loads.extend(quote! {
                    let [#a,#b] = ::cq_gpu_runtime::decode_tile_pair(
                        #view,__tile,__tid as u32,__pair
                    );
                });
                // Keep only one decoded pair live across the join body. Named
                // scalar selection avoids dynamically indexed PTX local arrays.
                rows.extend(quote! {
                    let #value = if __stripe % 512 == 0 { #a } else { #b };
                });
            }
            RootDecode::Stripe => loads.extend(quote! {
                let #value = ::cq_gpu_runtime::decode_stripe(
                    #view,__row / 1024,__tid as u32,
                    ((__row % 1024) / 256) as usize
                );
            }),
        }
    }
    *root = quote!(#rows #root);
    Ok((loads, decode.columns.into_keys().collect()))
}

// Track host-side metadata separately from device registers: every supported
// output is a copied input/run column or a certified membership constant.
fn output_bounds(
    context: &Context<'_>,
    facts: &ir::Analysis,
    chain: &[String],
    endpoint: &str,
    constants: &mut Constants,
) -> syn::Result<Vec<Tokens>> {
    let mut bounds = BTreeMap::new();
    let first = context.physical[&chain[0]];
    match &first.operator {
        Operator::Rename(rename) => {
            let name = symbol_name(&rename.relation);
            let table = id("host", &name);
            for mapping in &rename.mapping {
                let slot = context.hosts[&name].layout.iter()
                    .position(|column| *column == symbol_name(&mapping.source)).unwrap();
                bounds.insert(symbol_name(&mapping.target),
                    quote!(__execution.column_bounds(#table,#slot)?));
            }
        }
        Operator::Unit(_) => {}
        _ => {
            let input = symbol_name(first.operator.input().expect("checked stream root"));
            let table = id("table", &input);
            for (slot, column) in context.carried_columns[&input].iter().enumerate() {
                bounds.insert(column.clone(), quote!(__execution.column_bounds(#table,#slot)?));
            }
        }
    }
    for name in chain {
        if let Operator::Probe(_, keyed) = &context.physical[name].operator {
            let right = symbol_name(&keyed.right);
            let table = id("table", &right);
            for column in &facts.physical.headings[&right] {
                if bounds.contains_key(column) || !context.carried_columns[name].contains(column) {
                    continue;
                }
                let bound = if !membership(facts, &right)
                    && let Some(slot) = context.carried_columns[&right].iter().position(|c| c == column)
                {
                    quote!(__execution.column_bounds(#table,#slot)?)
                } else {
                    let binding = facts.filtered.get(&right)
                        .and_then(|f| f.constants.iter().find(|b| symbol_name(&b.column) == *column))
                        .ok_or_else(|| syn::Error::new_spanned(keyed,
                            "CQ_GPU_PAYLOAD_COLUMN: output has no source column or certified constant"))?;
                    let value = constants.term(&binding.value, &BTreeMap::new(),
                        facts.physical.rows[&right].types.get(column))?;
                    quote!(::cq_gpu_runtime::Bounds::singleton(#value))
                };
                bounds.insert(column.clone(), bound);
            }
        }
        bounds.retain(|column, _| context.carried_columns[name].contains(column));
    }
    context.carried_columns[endpoint].iter().map(|column| {
        bounds.get(column).cloned().ok_or_else(|| syn::Error::new_spanned(first,
            format!("CQ_GPU_COLUMN_BOUNDS: missing source for {column}")))
    }).collect()
}

pub(crate) fn compile(
    program: &ir::Program,
    options: crate::options::Options,
) -> syn::Result<syn::Expr> {
    let facts = ir::contract::check(program)?;
    let provenance = validate(program, &facts)?;
    let capacity = options
        .capacity
        .as_ref()
        .map(|relation| {
            let name = symbol_name(relation);
            if !program
                .physical
                .inputs()
                .any(|input| symbol_name(&input.name) == name)
            {
                return fail(
                    relation,
                    "CQ_GPU_CAPACITY",
                    "capacity must name a declared input relation",
                );
            }
            let table = id("host", &name);
            Ok(quote!(__execution.table_view(#table)?.len as usize))
        })
        .transpose()?;
    let recorded_capacity = match &options.capacity {
        Some(relation) => {
            let name = symbol_name(relation);
            quote!(Some(#name))
        }
        None => quote!(None),
    };
    // GPU join filters read bound device columns, not host PositionalRows handles.
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
    let mut host_pools = Vec::new();
    let mut host_dictionaries = Vec::new();
    for input in program.physical.inputs() {
        let name = symbol_name(&input.name);
        let value = &context.hosts[&name].value;
        let table = id("host", &name);
        let width = input.columns.len();
        let schema = input.to_token_stream().to_string();
        let row_types = input.columns.iter().map(|column| &column.ty);
        host.push(quote! {
            {
                fn __cq_gpu_check_schema<'data, Row, H: ::cq_gpu_runtime::HostColumns<'data, Row=Row>>(_: H) {}
                __cq_gpu_check_schema::<(#(#row_types,)*),_>(#value);
            }
        });
        host_pools.push(quote!(::cq_gpu_runtime::HostColumns::pool(#value)));
        host_dictionaries.push(quote!(
            ::cq_gpu_runtime::HostColumns::seed_dictionary(
                #value,__execution.dictionary_mut()
            )?;
        ));
        host.push(quote!(let #table=__execution.bind_host(#value,#schema,#width)?;));
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
        let (driver, mut root, mut state, driver_table) = match &first.operator {
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
                    Some(table),
                )
            }
            Operator::Unit(_) => (
                quote!(::cq_gpu_runtime::TableView {
                    columns: ::core::ptr::null(),
                    len: 1,
                    count: ::core::ptr::null(),
                    error: ::core::ptr::null_mut()
                }),
                quote!(),
                State::default(),
                None,
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
                    Some(table),
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
        let recorded_batches: BTreeSet<_> = chain.iter()
            .filter_map(|n| facts.stages.get(n)).copied().collect();
        let expanding = chain.iter().any(|n| matches!(
            context.physical[n].operator, Operator::Probe(..)
        ));
        let maximum = if expanding { 256 } else { 1024 };
        if recorded_batches.len() > 1
            || recorded_batches.iter().any(|&size| size == 0 || size > maximum)
        {
            return fail(definition, "CQ_GPU_STAGE_SIZE", format!(
                "fused streams require one exact batch size in 1..={maximum}; split incompatible stages into materialized streams"
            ));
        }
        let batch = recorded_batches.first().copied().unwrap_or(1024);
        let expansion_batch = recorded_batches.first().copied().unwrap_or(256);
        let bounds = output_bounds(&context, &facts, &chain, &endpoint, &mut all_constants)?;
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
            batch: expansion_batch,
            frontier_threshold: options.frontier,
            descriptors: Vec::new(),
        };
        let resume = if capacity.is_none() && selection.is_none() {
            Some(emitter.resumable(start, &state, root.clone())?)
        } else {
            None
        };
        let (root_tile, root_indices) = if selection.is_some() {
            decode_root(&mut root, &mut state.columns, RootDecode::Stripe)?
        } else if resume.is_none() && batch == 1024 {
            decode_root(&mut root, &mut state.columns, RootDecode::Tile)?
        } else {
            (Tokens::new(), Vec::new())
        };
        let root_columns: Vec<_> = root_indices
            .iter()
            .map(|index| id("root_column", &index.to_string()))
            .collect();
        let root_bindings = root_indices
            .iter()
            .zip(&root_columns)
            .map(|(index, view)| {
                let table = driver_table
                    .as_ref()
                    .expect("column reads have a table root");
                quote!(let #view=unsafe {__execution.column_view(#table,#index)?};)
            })
            .collect();
        let body = if resume.is_some() {
            Tokens::new()
        } else {
            emitter.steps(start, &mut state)?
        };
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
            root_tile,
            root_columns,
            root_bindings,
            body,
            resume,
            run_ids,
            run_types,
            run_bindings,
            shared_words,
            descriptor_words,
            continuations,
            frontier,
            batch,
            selection,
            bounds,
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
    let mut launches = Vec::new();
    for kernel in kernels {
        let (item, launch) = kernel.emit(
            &module_id,
            &constant_ids,
            &context,
            &facts,
            capacity.as_ref(),
        );
        device.push(item);
        launches.push(launch);
    }
    let output = id("table", &returned);
    let mut output_types = Vec::new();
    let mut output_columns = Vec::new();
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
        output_types.push(&facts.physical.rows[&returned].types[&column]);
        output_columns.push(index);
    }
    let output_codec = materialized_codec(&facts, &returned)?.marker();
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
    let derived_codecs = program
        .physical
        .program
        .definitions
        .iter()
        .filter(|definition| endpoints.contains(&symbol_name(&definition.name)))
        .map(|definition| {
            let name = symbol_name(&definition.name);
            let codec = format!(
                "{:?}",
                materialized_codec(&facts, &name).expect("validated materialized codec")
            );
            quote!((#name,#codec))
        });
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
    let load = has_kernels.then(|| {
        quote! {
            ::std::thread_local! {
                static __CQ_GPU_MODULE: ::std::cell::RefCell<
                    Option<(
                        ::std::sync::Arc<::cuda_core::CudaContext>,
                        ::std::rc::Rc<#module_id::LoadedModule>
                    )>
                > = const { ::std::cell::RefCell::new(None) };
            }
            let __module = __CQ_GPU_MODULE.with(|__cache| -> ::cq_gpu_runtime::Result<_> {
                if let Some((__context,__module)) = __cache.borrow().as_ref() {
                    if ::std::sync::Arc::ptr_eq(__context,__runtime.context()) {
                        return Ok(__module.clone());
                    }
                }
                let __module = ::std::rc::Rc::new(unsafe {
                    #module_id::load(__runtime.context())?
                });
                *__cache.borrow_mut() = Some((__runtime.context().clone(),__module.clone()));
                Ok(__module)
            })?;
        }
    });
    syn::parse2(quote!({
        #module
        const __CQ_GPU_KEYED_READS: &[&str]=&[#(#reads),*];
        const __CQ_GPU_KERNEL_READS: &[(&str,&str)]=&[#(#kernel_reads),*];
        const __CQ_GPU_JOIN_FILTER_READS: &[(&str,&str)]=&[#(#filter_reads),*];
        const __CQ_GPU_RECORDED_STRUCTURES: &[&str]=&[#(#recorded_structures),*];
        const __CQ_GPU_DERIVED_CODECS: &[(&str,&str)]=&[#(#derived_codecs),*];
        const __CQ_GPU_CAPACITY: ::core::option::Option<&str>=#recorded_capacity;
        #(#source_bindings)*
        let __pool=None #(.or_else(|| #host_pools))*;
        ::cq_gpu_runtime::with_runtime_for(__pool,|__runtime| {
            let mut __execution=__runtime.execution();
            #load
            #(#host_dictionaries)*
            #(#host)*
            #(#constants)*
            #(#launches)*
            let __answer=__execution.into_columns::<(#(#output_types,)*),#output_codec>(
                #output,&[#(#output_columns),*]
            )?;
            __execution.finish()?;
            Ok(__answer)
        }).unwrap_or_else(|error|panic!("CQ_GPU_EXECUTION: {error}"))
    }))
}

#[derive(Clone, Default)]
struct State {
    columns: BTreeMap<String, Tokens>,
    witnesses: BTreeMap<String, (Tokens, Tokens)>,
}

// The growing path is a defunctionalized traversal, not a second query pass.
// Each PC denotes the next operation; reservations advance it only after all
// output writes complete. Registers and every live descriptor are spilled only
// when a launch suspends, including the cursors of all enclosing expansions.
struct ResumeProgram {
    blocks: Vec<Tokens>,
    registers: usize,
    shared_words: usize,
    descriptor_words: usize,
    serial: bool,
}

struct ResumeEmitter<'a, 'b, 'c> {
    source: &'c mut Emitter<'a, 'b>,
    program: ResumeProgram,
    dispatch: Vec<Tokens>,
}

impl ResumeEmitter<'_, '_, '_> {
    fn register(&mut self) -> Tokens {
        let slot = self.program.registers;
        self.program.registers += 1;
        quote!(__resume_registers[#slot])
    }

    fn block(&mut self, body: Tokens) -> u32 {
        let pc = self.program.blocks.len() as u32;
        self.program.blocks.push(body);
        pc
    }

    fn steps(&mut self, index: usize, mut state: State, next: u32) -> syn::Result<u32> {
        let context = self.source.context;
        let facts = self.source.facts;
        if index == self.source.chain.len() {
            let values = context.carried_columns[self.source.endpoint]
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
            let writes = values.iter().enumerate().map(|(column, value)| {
                quote!(::cq_gpu_runtime::write_output(__output,#column,__destination,#value);)
            });
            return Ok(if self.source.serial {
                self.block(quote! {
                    if __valid {
                        let __destination=::cq_gpu_runtime::resume_reserve(__output,1);
                        if __destination==u64::MAX {
                            ::cq_gpu_runtime::resume_suspend(__resume);
                            __resume_pause=true;
                        } else {
                            #(#writes)*
                            __resume_pc=#next;
                        }
                    } else { __resume_pc=#next; }
                })
            } else {
                self.block(quote! {
                    let (__offset,__total)=::cq_gpu_runtime::tiles::scan(__valid as u32,__scratch);
                    if __tid==0 {
                        *(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::resume_reserve(__output,__total as u64);
                    }
                    ::cuda_device::thread::sync_threads();
                    let __base=*(__scratch.add(8) as *const u64);
                    if __base==u64::MAX {
                        if __tid==0 {::cq_gpu_runtime::resume_suspend(__resume);}
                        __resume_pause=true;
                    } else {
                        let __destination=__base+__offset as u64;
                        if __valid {#(#writes)*}
                        __resume_pc=#next;
                    }
                })
            });
        }
        let name = self.source.chain[index].clone();
        let definition = context.physical[&name];
        match &definition.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                let condition = self.source.constants.theta(
                    &s.condition,
                    &state.columns,
                    &facts.physical.rows[&symbol_name(&s.input)].types,
                )?;
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                let rest = self.steps(index + 1, state, next)?;
                Ok(self.block(quote! {__valid=__valid && (#condition);__resume_pc=#rest;}))
            }
            Operator::Project(_) => {
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                self.steps(index + 1, state, next)
            }
            Operator::Semi(_, k) | Operator::Probe(_, k)
                if membership(facts, &symbol_name(&k.right)) =>
            {
                let right = symbol_name(&k.right);
                let provider = read_provider(facts, k);
                let view = provider_id("run_view", &right, provider);
                let key = state.columns[directory_column(facts, &right, provider)].clone();
                if matches!(&definition.operator, Operator::Probe(_, _)) {
                    for column in &facts.physical.headings[&right] {
                        if !context.carried_columns[&name].contains(column)
                            || state.columns.contains_key(column)
                        {
                            continue;
                        }
                        let binding = facts.filtered.get(&right).and_then(|f| {
                            f.constants.iter().find(|b| symbol_name(&b.column) == *column)
                        }).ok_or_else(|| syn::Error::new_spanned(
                            k, "CQ_GPU_MEMBERSHIP_PAYLOAD: membership probe cannot expose unrecorded payload",
                        ))?;
                        let value = self.source.constants.term(&binding.value, &state.columns, facts.physical.rows[&right].types.get(column))?;
                        state.columns.insert(column.clone(), value);
                    }
                }
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                let rest = self.steps(index + 1, state, next)?;
                Ok(self.block(quote! {
                    __valid=__valid && ::cq_gpu_runtime::membership_contains(#view,#key);
                    __resume_pc=#rest;
                }))
            }
            Operator::Semi(_, k) => {
                let right = symbol_name(&k.right);
                let provider = read_provider(facts, k);
                let run = provider_id("run_view", &right, provider);
                let key = state.columns[directory_column(facts, &right, provider)].clone();
                let begin = self.register();
                let end = self.register();
                if facts.witnesses.contains_key(&name) {
                    state
                        .witnesses
                        .insert(name.clone(), (quote!(#begin as u32), quote!(#end as u32)));
                }
                let residual = self.source.residual(k, &state, quote!(__candidate))?;
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                let rest = self.steps(index + 1, state, next)?;
                Ok(self.block(quote! {
                    let (__begin,__end)=if __valid {
                        ::cq_gpu_runtime::blocked_run::point(#run.rank,#key).unwrap_or((0,0))
                    } else {(0,0)};
                    #begin=__begin as u64;
                    #end=__end as u64;
                    let mut __candidate=__begin;
                    let mut __found=false;
                    while __candidate<__end && !__found {
                        __found=#residual;
                        __candidate+=1;
                    }
                    __valid=__valid && __found;
                    __resume_pc=#rest;
                }))
            }
            Operator::Probe(_, k) => self.expand(index, state, k, next),
            _ => fail(
                &definition.name,
                "CQ_GPU_STREAM_OPERATOR",
                "unsupported stream operator",
            ),
        }
    }

    fn expand(
        &mut self,
        index: usize,
        mut state: State,
        k: &physical::Keyed,
        next: u32,
    ) -> syn::Result<u32> {
        let context = self.source.context;
        let facts = self.source.facts;
        let name = self.source.chain[index].clone();
        let right = symbol_name(&k.right);
        let provider = read_provider(facts, k);
        let run = provider_id("run_view", &right, provider);
        let witness = facts
            .witnesses
            .keys()
            .find(|origin| facts.shares[*origin].force == name)
            .cloned();
        let lookup = if let Some(origin) = witness {
            let (a, b) = state.witnesses.remove(&origin).ok_or_else(|| {
                syn::Error::new_spanned(k, "CQ_GPU_WITNESS: forcing read lost its recorded witness")
            })?;
            quote!((#a,#b))
        } else {
            let key = &state.columns[directory_column(facts, &right, provider)];
            quote!(::cq_gpu_runtime::blocked_run::point(#run.rank,#key).unwrap_or((0,0)))
        };
        let mut child = State::default();
        let mut save = Vec::new();
        let mut restore = Vec::new();
        let offset = (self.program.shared_words + 1) & !1;
        let mut words = 516usize;
        if self.source.serial {
            child = state;
        } else {
            for (column, value) in &state.columns {
                let slot = words;
                words += 512;
                let register = self.register();
                save.push(quote! {*(__descriptor.add(#slot) as *mut i64).add(__tid)=#value;});
                restore.push(quote! {
                    #register=if __valid {*(__descriptor.add(#slot) as *const i64).add(__parent) as u64}else{0};
                });
                child
                    .columns
                    .insert(column.clone(), quote!(#register as i64));
            }
            for (origin, (a, b)) in &state.witnesses {
                let slot = words;
                words += 512;
                let begin = self.register();
                let end = self.register();
                save.push(quote! {*__descriptor.add(#slot+__tid)=#a;*__descriptor.add(#slot+256+__tid)=#b;});
                restore.push(quote! {
                    #begin=if __valid {*__descriptor.add(#slot+__parent) as u64}else{0};
                    #end=if __valid {*__descriptor.add(#slot+256+__parent) as u64}else{0};
                });
                child
                    .witnesses
                    .insert(origin.clone(), (quote!(#begin as u32), quote!(#end as u32)));
            }
            self.program.shared_words = offset + words;
            self.program.descriptor_words = self.program.descriptor_words.max(words);
        }
        let row = self.register();
        let end = self.register();
        let external = (!self.source.serial).then(|| self.register());
        let guard = self.block(Tokens::new());
        let advance = self.block(quote! {#row+=1;__resume_pc=#guard;});
        let mut loads = Vec::new();
        for column in &facts.physical.headings[&right] {
            if !context.carried_columns[&name].contains(column)
                || child.columns.contains_key(column)
            {
                continue;
            }
            let value = if let Some(slot) = context.carried_columns[&right]
                .iter()
                .position(|c| c == column)
            {
                quote!(::cq_gpu_runtime::run_column(#run,#slot,__build_row))
            } else if let Some(constant) = facts.filtered.get(&right).and_then(|f| {
                f.constants
                    .iter()
                    .find(|b| symbol_name(&b.column) == *column)
            }) {
                self.source
                    .constants
                    .term(&constant.value, &child.columns, facts.physical.rows[&right].types.get(column))?
            } else {
                return fail(
                    k,
                    "CQ_GPU_PAYLOAD_COLUMN",
                    format!("recorded run {right} does not carry {column}"),
                );
            };
            let register = self.register();
            loads.push(quote! {#register=if __valid {(#value) as u64}else{0};});
            child
                .columns
                .insert(column.clone(), quote!(#register as i64));
        }
        let residual = self.source.residual(k, &child, quote!(__build_row))?;
        child
            .columns
            .retain(|column, _| context.carried_columns[&name].contains(column));
        let rest = self.steps(index + 1, child, advance)?;
        if self.source.serial {
            let load = self.block(quote! {
                let __build_row=#row as u32;
                __valid=true;
                #(#loads)*
                __valid=#residual;
                __resume_pc=#rest;
            });
            self.program.blocks[guard as usize] = quote! {
                __resume_pc=if #row<#end {#load}else{#next};
            };
            return Ok(self.block(quote! {
                let (__begin,__end)=if __valid {#lookup}else{(0,0)};
                #row=__begin as u64;
                #end=__end as u64;
                __resume_pc=#guard;
            }));
        }
        let external = external.expect("cooperative expansion has an external-task flag");
        let batch = self.source.batch as u32;
        let load = self.block(quote! {
            let __descriptor=__scratch.add(#offset);
            let __position=#row * #batch as u64 + __tid as u64;
            let __total=*__descriptor.add(514);
            __valid=__tid<#batch as usize && __position<__total as u64;
            let __parent=if __valid {
                ::cq_gpu_runtime::tiles::owner(__descriptor.add(258),256,__position as u32) as usize
            }else{0};
            let __build_row=if __valid {
                *__descriptor.add(2+__parent)+__position as u32-*__descriptor.add(258+__parent)
            }else{0};
            #(#restore)* #(#loads)*
            __valid=__valid && (#residual);
            __resume_pc=#rest;
        });
        // PC 3 retires an externally claimed frontier task. An inline child
        // instead returns to its enclosing frame, whose cursors remain live.
        self.program.blocks[guard as usize] = quote! {
            __resume_pc=if #row<#end {#load}else if #external!=0 {3}else{#next};
        };
        let stage = self.dispatch.len() as u32 + 1;
        self.dispatch.push(quote! {
            #stage=>{
                let mut __word=__tid;
                while __word<#words {
                    *__scratch.add(#offset+__word)=*__descriptor.add(__word);
                    __word+=256;
                }
                #row=__chunk as u64;
                #end=__chunk as u64+1;
                #external=1;
                __resume_pc=#guard;
            }
        });
        let publish = self.source.frontier && facts.expands.contains(&name);
        let threshold = self.source.frontier_threshold as u64 * 1024;
        Ok(self.block(quote! {
            let (__begin,__end)=if __valid {#lookup}else{(0,0)};
            let __descriptor=__scratch.add(#offset);
            if __tid==0 {*__descriptor=#stage;*__descriptor.add(1)=0;*__descriptor.add(515)=0;}
            *__descriptor.add(2+__tid)=__begin;
            #(#save)*
            let (__offset,mut __total)=::cq_gpu_runtime::tiles::scan(__end-__begin,__scratch);
            let __wrapped=__offset.checked_add(__end-__begin).is_none();
            let (_,__overflows)=::cq_gpu_runtime::tiles::scan(__wrapped as u32,__scratch);
            if __overflows!=0 {
                if __tid==0 {::cq_gpu_runtime::flag_error(__output,2);}
                __total=0;
            }
            *__descriptor.add(258+__tid)=__offset;
            if __tid==0 {*__descriptor.add(514)=__total;}
            ::cuda_device::thread::sync_threads();
            let __chunks=(__total as u64).div_ceil(#batch as u64) as u32;
            let __published=if #publish && __total as u64>#threshold {
                let mut __padding=#words+__tid;
                while __padding<__frontier.descriptor_words as usize {
                    *__descriptor.add(__padding)=0;
                    __padding+=256;
                }
                ::cuda_device::thread::sync_threads();
                ::cq_gpu_runtime::frontier_publish(__frontier,__descriptor,__chunks,__scratch.add(12))
            }else{false};
            #row=0;
            #end=__chunks as u64;
            #external=0;
            __resume_pc=if __published {#next}else{#guard};
        }))
    }
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
    fn resumable(
        &mut self,
        start: usize,
        initial: &State,
        root: Tokens,
    ) -> syn::Result<ResumeProgram> {
        let batch = self
            .chain
            .iter()
            .filter_map(|n| self.facts.stages.get(n))
            .copied()
            .min()
            .unwrap_or(1024);
        let serial = self.serial;
        let frontier = self.frontier;
        let mut emitter = ResumeEmitter {
            source: self,
            program: ResumeProgram {
                // 0 initializes a worker; 1 schedules a root/task; 2 advances a
                // stripe; 3 retires a claimed frontier task; 4 loads a root.
                blocks: vec![Tokens::new(); 5],
                registers: 1,
                shared_words: 16,
                descriptor_words: 1,
                serial,
            },
            dispatch: Vec::new(),
        };
        let tile = emitter.register();
        let stripe = emitter.register();
        let mut state = State::default();
        let mut save = Vec::new();
        for (column, value) in &initial.columns {
            let register = emitter.register();
            save.push(quote!(#register=if __valid {(#value) as u64}else{0};));
            state
                .columns
                .insert(column.clone(), quote!(#register as i64));
        }
        for (origin, (a, b)) in &initial.witnesses {
            let begin = emitter.register();
            let end = emitter.register();
            save.push(quote!(#begin=(#a) as u64;#end=(#b) as u64;));
            state
                .witnesses
                .insert(origin.clone(), (quote!(#begin as u32), quote!(#end as u32)));
        }
        let entry = emitter.steps(start, state, 2)?;
        emitter.program.blocks[0] = quote! {
            #tile=::cuda_device::thread::blockIdx_x() as u64;
            #stripe=0;
            __resume_pc=1;
        };
        emitter.program.blocks[4] = quote! {
            let __slot=#stripe as usize+__tid;
            let __row=#tile * #batch as u64+__slot as u64;
            __valid=__slot<#batch && __row<::cq_gpu_runtime::table_len(__driver);
            #root #(#save)*
            __resume_pc=#entry;
        };
        if frontier {
            let dispatch = emitter.dispatch;
            emitter.program.blocks[1] = quote! {
                if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::frontier_next_root(__frontier);}
                ::cuda_device::thread::sync_threads();
                #tile=*(__scratch.add(8) as *const u64);
                if #tile<__driver.len.div_ceil(#batch as u64) {
                    #stripe=0;
                    __resume_pc=4;
                } else {
                    let __descriptor=__scratch.add(__resume_shared_words);
                    if let Some(__chunk)=::cq_gpu_runtime::frontier_claim(__frontier,__descriptor,__scratch.add(12)) {
                        match *__descriptor {#(#dispatch)* _=>{
                            if __tid==0 {::cq_gpu_runtime::flag_error(__output,2);}
                            __resume_pc=3;
                        }}
                    } else {
                        if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::frontier_pending(__frontier);}
                        ::cuda_device::thread::sync_threads();
                        if *(__scratch.add(8) as *const u64)==0 {__resume_pc=u32::MAX;}
                    }
                }
            };
            emitter.program.blocks[2] = quote! {
                #stripe+=256;
                __resume_pc=if #stripe<#batch as u64 {4}else{3};
            };
            emitter.program.blocks[3] = quote! {
                if __tid==0 {::cq_gpu_runtime::frontier_done(__frontier);}
                __resume_pc=1;
            };
        } else {
            emitter.program.blocks[1] = quote! {
                __resume_pc=if #tile<::cq_gpu_runtime::table_len(__driver).div_ceil(#batch as u64) {4}else{u32::MAX};
            };
            emitter.program.blocks[2] = quote! {
                #stripe+=256;
                if #stripe<#batch as u64 {__resume_pc=4;}else{
                    #stripe=0;
                    #tile+=::cuda_device::thread::gridDim_x() as u64;
                    __resume_pc=1;
                }
            };
            emitter.program.blocks[3] = quote! {__resume_pc=u32::MAX;};
        }
        Ok(emitter.program)
    }

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
                let value = self.constants.term(&binding.value, &state.columns, self.facts.physical.rows[&right].types.get(column))?;
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
        let mut payloads = Vec::new();
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
                let payload = id("payload", &format!("{name}_{column}"));
                payloads.push(quote!(let #payload=#run.columns.add(#slot).read();));
                quote!(#payload.add(#row as usize).read())
            } else if let Some(constant) = self.facts.filtered.get(&right).and_then(|f| {
                f.constants
                    .iter()
                    .find(|b| symbol_name(&b.column) == *column)
            }) {
                self.constants.term(&constant.value, &state.columns, self.facts.physical.rows[&right].types.get(column))?
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
                if #begin<#end {
                    // Run payload addresses are immutable while this expansion
                    // writes its distinct output; avoid a pointer-table load
                    // for every candidate without touching an empty run.
                    #(#payloads)*
                    let mut #row=#begin;
                    while #row<#end {
                        #(#loads)*
                        let mut __valid=#residual;
                        #rest
                        #row+=1;
                    }
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
                self.constants.term(&constant.value, &child.columns, self.facts.physical.rows[&right].types.get(column))?
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
    root_tile: Tokens,
    root_columns: Vec<Ident>,
    root_bindings: Vec<Tokens>,
    body: Tokens,
    resume: Option<ResumeProgram>,
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
    bounds: Vec<Tokens>,
}
impl Kernel {
    fn emit(
        self,
        _module: &Ident,
        constants: &[Ident],
        context: &Context<'_>,
        facts: &ir::Analysis,
        capacity: Option<&Tokens>,
    ) -> (Tokens, Tokens) {
        let Self {
            kernel,
            driver,
            root,
            root_tile,
            root_columns,
            root_bindings,
            body,
            resume,
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
            bounds,
        } = self;
        let prepare = format_ident!("prepare_{}", kernel);
        let table = id("table", &endpoint);
        let (shared_words, descriptor_words) = resume
            .as_ref()
            .map(|program| (program.shared_words, program.descriptor_words))
            .unwrap_or((shared_words, descriptor_words));
        let shared_bytes =
            ((shared_words + if frontier { descriptor_words + 2 } else { 0 }) * 4) as u32;
        let common =
            quote!(__driver,#(#root_columns,)*#(#run_ids,)*#(#constants,)*__output,__frontier);
        let signature = quote!(__driver: ::cq_gpu_runtime::TableView,#(#root_columns: ::cq_gpu_runtime::ColumnView,)*#(#run_ids: #run_types,)*#(#constants:i64,)*__output: ::cq_gpu_runtime::OutputView,__frontier: ::cq_gpu_runtime::FrontierView);
        let resume_args = resume
            .as_ref()
            .map(|_| quote!(__resume: ::cq_gpu_runtime::ResumeView,));
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
        let functions=continuations.iter().filter(|_|resume.is_none()).map(|c| {
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
        // The full-vector decoder assigns each warp two independent 64-row
        // sequences, returning adjacent pairs at offsets 0, 1, 64 and 65.
        // Partial batches and selection kernels retain their striped ABI.
        let root_slot = if !root_tile.is_empty() && selection.is_none() {
            quote!(
                (__tid / 32) * 128
                    + (__tid % 32) * 2
                    + (__stripe / 512) * 64
                    + (__stripe / 256) % 2
            )
        } else {
            quote!(__stripe + __tid)
        };
        let root_len = if frontier {
            quote!(::cq_gpu_runtime::table_len(__driver))
        } else {
            quote!(__driver_len)
        };
        let row = quote! {
            let __slot=#root_slot;
            let __row=__tile*#batch as u64+__slot as u64;
            let mut __valid=__slot<#batch && __row<#root_len;
            #root #body
        };
        let root_rows = if !root_tile.is_empty() && selection.is_none() {
            quote! {
                let mut __pair=0usize;
                while __pair<2 {
                    #root_tile
                    let mut __stripe=__pair*512;
                    while __stripe<(__pair+1)*512 {
                        #row
                        __stripe+=256;
                    }
                    __pair+=1;
                }
            }
        } else {
            quote! {
                let mut __stripe=0usize;
                while __stripe<#batch {
                    #row
                    __stripe+=256;
                }
            }
        };
        let scheduler = if frontier {
            quote! {
                loop {
                    if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::frontier_next_root(__frontier);}
                    ::cuda_device::thread::sync_threads();
                    let __tile=*(__scratch.add(8) as *const u64);
                    if __tile<__driver.len.div_ceil(#batch as u64) {
                        #root_rows
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
                // Upstream extent and error are immutable during this launch.
                let __driver_len=::cq_gpu_runtime::table_len(__driver);
                let mut __tile=::cuda_device::thread::blockIdx_x() as u64;
                while __tile<__driver_len.div_ceil(#batch as u64) {
                    #root_rows
                    __tile+=::cuda_device::thread::gridDim_x() as u64;
                }
            }
        };
        let scheduler = if selection.is_some() {
            quote! {
                let __row=::cuda_device::thread::blockIdx_x() as u64*256+__tid as u64;
                #root_tile
                let mut __valid=__row<::cq_gpu_runtime::table_len(__driver);
                #root #body
            }
        } else {
            scheduler
        };
        let checkpoint_words = resume
            .as_ref()
            .map(|program| {
                256 * (program.registers + 1) + (shared_bytes as usize / 4 - 16).div_ceil(2)
            })
            .unwrap_or(0);
        let scheduler = if let Some(program) = &resume {
            let registers = program.registers;
            let thread_words = registers + 1;
            let shared_checkpoint = 256 * thread_words;
            let total_shared_words = shared_bytes as usize / 4;
            let blocks = program.blocks.iter().enumerate().map(|(pc, body)| {
                let pc = pc as u32;
                quote!(#pc=>{#body})
            });
            let restore_shared = (!program.serial).then(|| {
                quote! {
                    let __saved_shared=__checkpoint.add(#shared_checkpoint) as *const u32;
                    let mut __word=16+__tid;
                    while __word<#total_shared_words {
                        *__scratch.add(__word)=*__saved_shared.add(__word-16);
                        __word+=256;
                    }
                    ::cuda_device::thread::sync_threads();
                }
            });
            let save_shared = (!program.serial).then(|| {
                quote! {
                    let __saved_shared=__checkpoint.add(#shared_checkpoint) as *mut u32;
                    let mut __word=16+__tid;
                    while __word<#total_shared_words {
                        *__saved_shared.add(__word-16)=*__scratch.add(__word);
                        __word+=256;
                    }
                }
            });
            let stop = if program.serial {
                quote!(if ::cq_gpu_runtime::resume_requested(__resume) {
                    break;
                })
            } else {
                quote! {
                    if __tid==0 {*__scratch.add(15)=::cq_gpu_runtime::resume_requested(__resume) as u32;}
                    ::cuda_device::thread::sync_threads();
                    if *__scratch.add(15)!=0 {break;}
                }
            };
            quote! {
                let __resume_shared_words=#shared_words;
                let __checkpoint=__resume.state.add(
                    ::cuda_device::thread::blockIdx_x() as usize * #checkpoint_words
                );
                let __lane_checkpoint=__checkpoint.add(__tid * #thread_words);
                let mut __resume_pc=*__lane_checkpoint as u32;
                if __resume_pc==u32::MAX {return;}
                let mut __resume_registers=[0u64;#registers];
                let mut __register=0usize;
                while __register<#registers {
                    __resume_registers[__register]=*__lane_checkpoint.add(1+__register);
                    __register+=1;
                }
                let mut __valid=__resume_registers[0]!=0;
                let mut __resume_pause=false;
                #restore_shared
                loop {
                    if __resume_pc==u32::MAX || __resume_pause {break;}
                    #stop
                    match __resume_pc {
                        #(#blocks,)*
                        _=>{
                            ::cq_gpu_runtime::flag_error(__output,2);
                            __resume_pc=u32::MAX;
                        }
                    }
                }
                if __resume_pc!=u32::MAX {
                    __resume_registers[0]=__valid as u64;
                    let mut __register=0usize;
                    while __register<#registers {
                        *__lane_checkpoint.add(1+__register)=__resume_registers[__register];
                        __register+=1;
                    }
                    #save_shared
                }
                *__lane_checkpoint=__resume_pc as u64;
            }
        } else {
            scheduler
        };
        // Four resident 256-thread blocks bound decoder register pressure on
        // the serial scan path. Frontier/resume kernels retain their own needs.
        let min_blocks = if !frontier && resume.is_none() && selection.is_none() {
            4u32
        } else {
            0
        };
        let device = quote! {
            #(#functions)*
            #[kernel]
            #[launch_bounds(256, #min_blocks)]
            #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_bytes,#shared_bytes))]
            pub fn #kernel(#signature,#resume_args #selection_args) {
                let __tid=::cuda_device::thread::threadIdx_x() as usize;
                let __scratch=::cuda_device::DynamicSharedArray::<u8>::get_raw() as *mut u32;
                unsafe {#scheduler}
            }
        };
        let canon = facts.dedup.contains_key(&endpoint);
        let dedup = canon.then(|| quote!(let #table=__execution.dedup(#table)?;));
        let codec = materialized_codec(facts, &endpoint)
            .expect("validated materialized codec")
            .value();
        let encode = quote!(let #table=__execution.reencode_table(#table,#codec)?;);
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
        let execute = if resume.is_some() {
            quote! {
                __execution.execute_resumable(
                    &[#(#bounds),*],__driver.len as usize,__grid as usize * #checkpoint_words,
                    |__stream,__output,__resume| {
                        unsafe {__module.#kernel(__stream,&__launch,#common,__resume)?;}
                        Ok(())
                    },
                )?
            }
        } else {
            let capacity = capacity
                .cloned()
                .unwrap_or_else(|| quote!(__driver.len as usize));
            quote! {
                __execution.execute_bounded(&[#(#bounds),*],#capacity,|__stream,__output| {
                    unsafe {__module.#kernel(__stream,&__launch,#common)?;}
                    Ok(())
                })?
            }
        };
        let launch = quote! {
            let #table={
                let __driver=#driver;
                #(#root_bindings)*
                #(#run_bindings)*
                let __pool=__execution.frontier(#slots,#descriptor_words as u32)?;
                let __frontier=__pool.view();
                let __grid=__driver.len.div_ceil(#batch as u64).clamp(1,568) as u32;
                let __launch=__module.#prepare(::cuda_core::LaunchConfig1D::new(__grid,256,#shared_bytes))?;
                __pool.reset(__driver.len.div_ceil(#batch as u64))?;
                #execute
            };
            #dedup #encode #build #(#filter_builds)*
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
                    #dedup #encode #build #(#filter_builds)*
                }
            } else {
                quote! {
                    let #table={
                        let __driver=#driver;
                        #(#root_bindings)*
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
                    #dedup #encode #build #(#filter_builds)*
                }
            }
        } else {
            launch
        };
        (device, launch)
    }
}
