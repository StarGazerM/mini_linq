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
    if qualified(ty, "cq_gpu_runtime", "DeviceSet") && types.len() == 3 {
        dedup_policy(ty)?;
        types.pop();
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum DedupPolicy {
    Staged,
    Compacting,
    Fused,
}

fn dedup_policy(ty: &Type) -> syn::Result<DedupPolicy> {
    let Type::Path(path) = ty else {
        return fail(
            ty,
            "CQ_GPU_DEDUP_POLICY",
            "expected DeviceSet representation",
        );
    };
    let syn::PathArguments::AngleBracketed(args) = &path.path.segments.last().unwrap().arguments
    else {
        return fail(ty, "CQ_GPU_DEDUP_POLICY", "expected DeviceSet row type");
    };
    let types: Vec<_> = args
        .args
        .iter()
        .filter_map(|arg| {
            if let syn::GenericArgument::Type(ty) = arg {
                Some(ty)
            } else {
                None
            }
        })
        .collect();
    match types.as_slice() {
        [_] | [_, _] => Ok(DedupPolicy::Staged),
        [_, _, policy] if qualified(policy, "cq_gpu_runtime", "Staged") => Ok(DedupPolicy::Staged),
        [_, _, policy] if qualified(policy, "cq_gpu_runtime", "Compacting") => {
            Ok(DedupPolicy::Compacting)
        }
        [_, _, policy] if qualified(policy, "cq_gpu_runtime", "Fused") => Ok(DedupPolicy::Fused),
        _ => fail(
            ty,
            "CQ_GPU_DEDUP_POLICY",
            "record cq_gpu_runtime::{Staged,Compacting,Fused} as DeviceSet's third type parameter",
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

fn ungroup(expression: &syn::Expr) -> &syn::Expr {
    match expression {
        syn::Expr::Paren(p) => ungroup(&p.expr),
        syn::Expr::Group(g) => ungroup(&g.expr),
        _ => expression,
    }
}

fn copied_column(expression: &syn::Expr) -> Option<String> {
    let syn::Expr::Unary(unary) = ungroup(expression) else {
        return None;
    };
    if !matches!(unary.op, syn::UnOp::Deref(_)) {
        return None;
    }
    syntax::rust::variable(ungroup(&unary.expr)).map(symbol_name)
}

fn closed_bind(bind: &syntax::algebra::Bind, types: &BTreeMap<String, Type>) -> bool {
    let environment = bind.environment.as_ref().map_or_else(
        || types.keys().cloned().collect(),
        |environment| environment.variables.iter().map(symbol_name).collect(),
    );
    syntax::rust::free_variables(&bind.expression)
        .dependencies(&environment)
        .is_empty()
}

// Query-independent typed scalars are captured on the host. Query-dependent
// expressions remain restricted to copied columns, never arbitrary device Rust.
fn bind_value(
    bind: &syntax::algebra::Bind,
    types: &BTreeMap<String, Type>,
    columns: &BTreeMap<String, Tokens>,
    constants: &mut Constants,
) -> syn::Result<(String, Tokens)> {
    let syn::Pat::Ident(pattern) = &bind.pattern else {
        return fail(
            bind,
            "CQ_GPU_OPAQUE_BIND",
            "bind requires one typed scalar identifier",
        );
    };
    if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some() {
        return fail(
            bind,
            "CQ_GPU_OPAQUE_BIND",
            "bind requires an immutable scalar identifier",
        );
    }
    let bindings = bind.binding_types();
    let Some((_, ty)) = bindings.first().filter(|_| bindings.len() == 1) else {
        return fail(
            bind,
            "CQ_GPU_OPAQUE_BIND",
            "bind requires an explicit supported scalar type",
        );
    };
    if !supported(ty) || type_name(ty).as_deref() == Some("String") {
        return fail(
            bind,
            "CQ_GPU_OPAQUE_BIND",
            "bind requires a supported Copy scalar type",
        );
    }
    let value = if closed_bind(bind, types)
        && (integral(ty) || type_name(ty).as_deref() == Some("bool"))
    {
        let expression = ungroup(&bind.expression);
        constants.add(quote!({ let __cq_gpu_literal: #ty = #expression; __cq_gpu_literal }))
    } else if let Some(source) = copied_column(&bind.expression) {
        if bind.environment.as_ref().is_some_and(|environment| {
            !environment
                .variables
                .iter()
                .any(|v| symbol_name(v) == source)
        }) || types.get(&source) != Some(ty)
        {
            return fail(
                bind,
                "CQ_GPU_BIND_TYPE",
                "copied column must be in scope and have the exact declared bind type",
            );
        }
        columns.get(&source).cloned().ok_or_else(|| {
            syn::Error::new_spanned(bind, "CQ_GPU_BIND_COLUMN: copied column is not carried")
        })?
    } else {
        let expression = ungroup(&bind.expression);
        let valid = match expression {
            syn::Expr::Lit(lit) => match &lit.lit {
                syn::Lit::Int(_) => integral(ty),
                syn::Lit::Bool(_) => type_name(ty).as_deref() == Some("bool"),
                syn::Lit::Str(_) => matches!(ty, Type::Reference(r) if r.mutability.is_none()
                    && type_name(&r.elem).as_deref() == Some("str")),
                _ => false,
            },
            syn::Expr::Unary(unary) => {
                matches!(unary.op, syn::UnOp::Neg(_))
                    && integral(ty)
                    && matches!(&*unary.expr, syn::Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Int(_)))
            }
            _ => false,
        };
        if !valid {
            return fail(
                bind,
                "CQ_GPU_OPAQUE_BIND",
                "only copied dereferences, typed literals, and closed integral or bool captures are supported",
            );
        }
        constants.add(quote!({ let __cq_gpu_literal: #ty = #expression; __cq_gpu_literal }))
    };
    Ok((symbol_name(&pattern.ident), value))
}

fn leveled_type(ty: &Type) -> bool {
    qualified(ty, "cq_gpu_runtime", "LeveledDeviceIndex")
}
fn resident_type(ty: &Type) -> bool {
    qualified(ty, "cq_gpu_runtime", "FrozenDeviceIndex") || leveled_type(ty)
}
fn leveled_provider(facts: &ir::Analysis, name: &str, provider: usize) -> bool {
    leveled_type(&facts.built[name][provider].1)
}
fn kernel_run_id(facts: &ir::Analysis, name: &str, provider: usize) -> Ident {
    provider_id(
        if leveled_provider(facts, name, provider) {
            "runs_view"
        } else {
            "run_view"
        },
        name,
        provider,
    )
}

// Keep the descriptor's static capacity aligned with the runtime ABI. The
// generated array assertion catches an ABI change before compiling a kernel.
const LEVELED_MAX_RUNS: usize = 16;

fn leveled_ranges(snapshot: &Ident, run: &Ident, lookup: Tokens) -> Tokens {
    let search = leveled_ranges_into(snapshot, run, lookup, quote!(__leveled_ranges));
    quote! {
        let mut __leveled_ranges = [0u64; ::cq_gpu_runtime::MAX_LEVELS];
        #search
    }
}

fn leveled_ranges_into(snapshot: &Ident, run: &Ident, lookup: Tokens, ranges: Tokens) -> Tokens {
    quote! {
        let _: [(); #LEVELED_MAX_RUNS] = [(); ::cq_gpu_runtime::MAX_LEVELS];
        let mut __leveled_total=0u64;
        let mut __leveled_run_id=0usize;
        while __leveled_run_id < ::cq_gpu_runtime::leveled_count(#snapshot) as usize {
            let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
            let (__run_begin,__run_end)=if __valid {#lookup}else{(0,0)};
            (#ranges)[__leveled_run_id]=__run_begin as u64 | ((__run_end as u64)<<32);
            __leveled_total+=(__run_end-__run_begin) as u64;
            __leveled_run_id+=1;
        }
        let __leveled_len=if __leveled_total<=u32::MAX as u64 {
            __leveled_total as u32
        } else {
            ::cq_gpu_runtime::flag_error(__output,2);
            0
        };
    }
}

// A descriptor contains one (begin,end) range for each run and input row.
// Its scan is over the sum of those ranges, not over a single run. Restoring
// a candidate resolves its logical position to the original run and row.
fn leveled_restore(snapshot: &Ident, run: &Ident, range: Tokens) -> Tokens {
    quote! {
        let mut __leveled_position=__build_row;
        let mut __leveled_run_id=0usize;
        let mut __leveled_build_row=0u32;
        if __valid {
            while __leveled_run_id < ::cq_gpu_runtime::leveled_count(#snapshot) as usize {
                let (__run_begin,__run_end)=#range;
                if __leveled_position<__run_end-__run_begin {
                    __leveled_build_row=__run_begin+__leveled_position;
                    break;
                }
                __leveled_position-=__run_end-__run_begin;
                __leveled_run_id+=1;
            }
        }
        let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
        let __build_row=__leveled_build_row;
    }
}

/// A full resident run certificate stored in the optional Layout type argument.
/// Key columns within one level are unordered; schema slots give their stable
/// physical order across differently named reads of the same relation.
fn frozen_recorded_layout(ty: &Type) -> syn::Result<Option<Vec<Vec<usize>>>> {
    let Type::Path(path) = ty else {
        return Ok(None);
    };
    let syn::PathArguments::AngleBracketed(arguments) =
        &path.path.segments.last().unwrap().arguments
    else {
        return Ok(None);
    };
    let types = arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect::<Vec<_>>();
    if types.len() < 3 {
        return Ok(None);
    }
    if types.len() != 3 {
        return fail(
            ty,
            "CQ_GPU_FROZEN_LAYOUT",
            "expected Row, Codec, and one nested tuple Layout",
        );
    }
    let Type::Tuple(levels) = types[2] else {
        return fail(
            ty,
            "CQ_GPU_FROZEN_LAYOUT",
            "Layout must be a tuple of FrozenKey tuples",
        );
    };
    let mut result = Vec::new();
    for level in &levels.elems {
        let Type::Tuple(columns) = level else {
            return fail(
                level,
                "CQ_GPU_FROZEN_LAYOUT",
                "each layout level must be a FrozenKey tuple",
            );
        };
        let mut slots = Vec::new();
        for column in &columns.elems {
            if !qualified(column, "cq_gpu_runtime", "FrozenKey") {
                return fail(
                    column,
                    "CQ_GPU_FROZEN_LAYOUT",
                    "layout columns must be cq_gpu_runtime::FrozenKey<SLOT>",
                );
            }
            let Type::Path(path) = column else {
                unreachable!()
            };
            let syn::PathArguments::AngleBracketed(arguments) =
                &path.path.segments.last().unwrap().arguments
            else {
                return fail(
                    column,
                    "CQ_GPU_FROZEN_LAYOUT",
                    "FrozenKey requires one literal schema slot",
                );
            };
            let Some(syn::GenericArgument::Const(syn::Expr::Lit(literal))) =
                arguments.args.first().filter(|_| arguments.args.len() == 1)
            else {
                return fail(
                    column,
                    "CQ_GPU_FROZEN_LAYOUT",
                    "FrozenKey requires one literal schema slot",
                );
            };
            let syn::Lit::Int(slot) = &literal.lit else {
                return fail(
                    column,
                    "CQ_GPU_FROZEN_LAYOUT",
                    "FrozenKey requires an integer schema slot",
                );
            };
            slots.push(slot.base10_parse()?);
        }
        result.push(slots);
    }
    Ok(Some(result))
}

fn frozen_columns(
    program: &ir::Program,
    facts: &ir::Analysis,
    name: &str,
) -> syn::Result<Vec<String>> {
    let definition = program
        .physical
        .program
        .definitions
        .iter()
        .find(|definition| symbol_name(&definition.name) == name)
        .expect("checked host");
    let Operator::Rename(rename) = &definition.operator else {
        return fail(
            definition,
            "CQ_GPU_FROZEN_HOST",
            "resident provider must be a host rename",
        );
    };
    Ok(facts.host_layouts[&symbol_name(&rename.relation)]
        .iter()
        .map(|column| {
            rename
                .mapping
                .iter()
                .find(|mapping| symbol_name(&mapping.source) == *column)
                .map(|mapping| symbol_name(&mapping.target))
                .expect("checked host mapping")
        })
        .collect())
}

fn check_frozen_slots(at: &impl ToTokens, levels: &[Vec<usize>], width: usize) -> syn::Result<()> {
    let mut seen = BTreeSet::new();
    if levels.is_empty()
        || levels.iter().any(Vec::is_empty)
        || levels
            .iter()
            .flatten()
            .any(|&slot| slot >= width || !seen.insert(slot))
        || levels
            .iter()
            .any(|level| level.windows(2).any(|pair| pair[0] >= pair[1]))
    {
        return fail(
            at,
            "CQ_GPU_FROZEN_LAYOUT",
            "resident layouts require nonempty levels of distinct in-schema slots, ascending within each level",
        );
    }
    Ok(())
}

/// Record only existing keyed host reads; no Physical modes or read keys change.
/// `Input => [[1,2],[0]]` records one full run shared by reads of [1,2] and
/// [0,1,2]. Repeated Input entries record incomparable chains.
pub(crate) fn frozen_hosts(program: &ir::Program, arguments: Tokens) -> syn::Result<ir::Program> {
    maintained_hosts(program, arguments, false)
}

/// Record maintained changing-relation hosts using the same exact chain checks.
pub(crate) fn leveled_hosts(program: &ir::Program, arguments: Tokens) -> syn::Result<ir::Program> {
    maintained_hosts(program, arguments, true)
}

fn maintained_hosts(
    program: &ir::Program,
    arguments: Tokens,
    leveled: bool,
) -> syn::Result<ir::Program> {
    let provider = if leveled {
        quote!(::cq_gpu_runtime::LeveledDeviceIndex)
    } else {
        quote!(::cq_gpu_runtime::FrozenDeviceIndex)
    };
    use syn::parse::Parser;
    let parse = |input: syn::parse::ParseStream<'_>| {
        let mut requests = Vec::new();
        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            let levels = if input.peek(syn::Token![=>]) {
                input.parse::<syn::Token![=>]>()?;
                let outer: syn::ExprArray = input.parse()?;
                let mut levels = Vec::new();
                for expression in outer.elems {
                    let syn::Expr::Array(level) = expression else {
                        return fail(
                            &expression,
                            "CQ_GPU_FROZEN_LAYOUT",
                            "expected a list of key-level lists",
                        );
                    };
                    let mut slots = Vec::new();
                    for expression in level.elems {
                        let syn::Expr::Lit(literal) = &expression else {
                            return fail(
                                &expression,
                                "CQ_GPU_FROZEN_LAYOUT",
                                "schema slots must be integer literals",
                            );
                        };
                        let syn::Lit::Int(slot) = &literal.lit else {
                            return fail(
                                &expression,
                                "CQ_GPU_FROZEN_LAYOUT",
                                "schema slots must be integer literals",
                            );
                        };
                        slots.push(slot.base10_parse::<usize>()?);
                    }
                    slots.sort_unstable();
                    levels.push(slots);
                }
                Some(levels)
            } else {
                None
            };
            requests.push((name, levels));
            if input.is_empty() {
                break;
            }
            input.parse::<syn::Token![,]>()?;
        }
        Ok(requests)
    };
    let requests = parse.parse2(arguments)?;
    let facts = ir::contract::check(program)?;
    for (name, levels) in &requests {
        let Some(input) = program.physical.inputs().find(|input| input.name == *name) else {
            return fail(name, "CQ_GPU_FROZEN_HOST", "unknown frozen input relation");
        };
        if let Some(levels) = levels {
            check_frozen_slots(name, levels, input.columns.len())?;
        }
        if leveled
            && !program.structures.entries.iter().any(|entry| {
                matches!(&entry.structure, ir::Structure::Host { reader, .. } if reader != "scan")
                    && facts
                        .physical
                        .hosts
                        .get(&symbol_name(&entry.name))
                        .is_some_and(|host| *host == symbol_name(name))
            })
        {
            return fail(
                name,
                "CQ_GPU_LEVELED_HOST",
                "maintained input requires an existing keyed Host read",
            );
        }
    }
    let mut target = program.clone();
    for entry in &mut target.structures.entries {
        let ir::Structure::Host { keyed, reader, .. } = &mut entry.structure else {
            continue;
        };
        if reader == "scan" {
            continue;
        }
        let definition = program
            .physical
            .program
            .definitions
            .iter()
            .find(|definition| definition.name == entry.name)
            .expect("checked host");
        let Operator::Rename(rename) = &definition.operator else {
            continue;
        };
        let selected = requests
            .iter()
            .filter(|(name, _)| *name == rename.relation)
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        let columns = frozen_columns(program, &facts, &symbol_name(&entry.name))?;
        let mut keys = keyed
            .columns()
            .map(|key| {
                columns
                    .iter()
                    .position(|column| *column == symbol_name(key))
                    .expect("checked key")
            })
            .collect::<Vec<_>>();
        keys.sort_unstable();
        let chain = selected.iter().find_map(|(_, levels)| {
            levels.as_ref().filter(|levels| {
                let mut prefix = Vec::new();
                levels.iter().any(|level| {
                    prefix.extend(level.iter().copied());
                    prefix.sort_unstable();
                    prefix == keys
                })
            })
        });
        if let Some(chain) = chain {
            let levels = chain.iter().map(|level| {
                let columns = level
                    .iter()
                    .map(|slot| quote!(::cq_gpu_runtime::FrozenKey<#slot>));
                quote!((#(#columns,)*))
            });
            keyed.ty = syn::parse2(quote!(#provider<_,_,(#(#levels,)*)>))?;
        } else if selected.iter().any(|(_, levels)| levels.is_none()) {
            keyed.ty = syn::parse2(quote!(#provider<_>))?;
        } else {
            return fail(
                keyed,
                "CQ_GPU_FROZEN_READ_PREFIX",
                format!(
                    "host read slots {keys:?} are not an exact prefix of any recorded resident chain"
                ),
            );
        }
    }
    let mut checked = ir::contract::check(&target)?;
    if leveled {
        record_frozen_providers(&target, &mut checked)?;
        validate_leveled_reads(&target, &checked)?;
    }
    Ok(target)
}

fn record_frozen_providers(program: &ir::Program, facts: &mut ir::Analysis) -> syn::Result<()> {
    for entry in &program.structures.entries {
        let ir::Structure::Host { keyed, reader, .. } = &entry.structure else {
            continue;
        };
        if !resident_type(&keyed.ty) {
            continue;
        }
        let name = symbol_name(&entry.name);
        let columns = frozen_columns(program, facts, &name)?;
        let mut levels = if let Some(slots) = frozen_recorded_layout(&keyed.ty)? {
            check_frozen_slots(keyed, &slots, columns.len())?;
            slots
                .iter()
                .map(|level| level.iter().map(|&slot| columns[slot].clone()).collect())
                .collect::<Vec<Vec<String>>>()
        } else {
            keyed.level_names()
        };
        if reader == "scan" || levels.is_empty() || levels.iter().any(Vec::is_empty) {
            return fail(
                entry,
                "CQ_GPU_FROZEN_LEVELS",
                "resident runs require nonempty key levels",
            );
        }
        for level in &mut levels {
            level.sort_by_key(|key| {
                columns
                    .iter()
                    .position(|column| column == key)
                    .expect("checked key")
            });
        }
        let mut requested = keyed.columns().map(symbol_name).collect::<Vec<_>>();
        requested.sort();
        if !serves(&levels, &requested) {
            return fail(
                entry,
                "CQ_GPU_FROZEN_READ_PREFIX",
                "host read is not an exact prefix of its full resident layout",
            );
        }
        for column in levels.iter().flatten() {
            let ty = &facts.physical.rows[&name].types[column];
            if !integral(ty) && !string(ty) && type_name(ty).as_deref() != Some("bool") {
                return fail(
                    entry,
                    "CQ_GPU_NON_INTEGRAL_KEY",
                    "resident sorted keys require supported exact scalar encodings",
                );
            }
        }
        let providers = facts.built.entry(name).or_default();
        if !providers.iter().any(|provider| provider.0 == levels) {
            providers.push((levels, keyed.ty.clone()));
        }
    }
    Ok(())
}

fn frozen_provider(facts: &ir::Analysis, name: &str, provider: usize) -> bool {
    resident_type(&facts.built[name][provider].1)
}
fn membership(facts: &ir::Analysis, name: &str) -> bool {
    facts.membership.iter().any(|(build, _)| build == name)
}
// A fully bound join produces no new values. In a pure stream ending in
// recorded canon, duplicate matching witnesses are unobservable; membership
// preserves the set without assuming a unique key in the provider.
fn boolean_probe(facts: &ir::Analysis, endpoint: &str, keyed: &physical::Keyed) -> bool {
    facts.dedup.contains_key(endpoint)
        && facts.physical.headings[&symbol_name(&keyed.right)]
            .iter()
            .all(|column| keyed.columns.iter().any(|key| symbol_name(key) == *column))
}
fn exact_run_key(facts: &ir::Analysis, keyed: &physical::Keyed, column: &str) -> bool {
    let right = symbol_name(&keyed.right);
    keyed.columns.iter().any(|key| symbol_name(key) == column)
        && !residual_key(facts, &right, read_provider(facts, keyed), column)
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
fn hashed(facts: &ir::Analysis, name: &str, provider: usize) -> bool {
    let build = &facts.built[name][provider];
    qualified(&build.1, "cq_gpu_runtime", "HashedRun")
}
fn prefix_run(facts: &ir::Analysis, name: &str, provider: usize) -> bool {
    let build = &facts.built[name][provider];
    qualified(&build.1, "cq_gpu_runtime", "PrefixRun")
        || (resident_type(&build.1) && build.0.iter().map(Vec::len).sum::<usize>() > 1)
}
fn residual_key(facts: &ir::Analysis, name: &str, provider: usize, key: &str) -> bool {
    if prefix_run(facts, name, provider) {
        !facts.built[name][provider]
            .0
            .iter()
            .flatten()
            .any(|column| column == key)
    } else {
        hashed(facts, name, provider) || key != directory_column(facts, name, provider)
    }
}
fn lookup_key(
    facts: &ir::Analysis,
    name: &str,
    provider: usize,
    columns: &BTreeMap<String, Tokens>,
) -> Tokens {
    if !hashed(facts, name, provider) {
        return columns[directory_column(facts, name, provider)].clone();
    }
    let run = provider_id("run_view", name, provider);
    let values = facts.built[name][provider].0[0]
        .iter()
        .map(|key| &columns[key]);
    quote!({
        let mut __hash = ::cq_gpu_runtime::HASHED_RUN_SEED;
        #(__hash = ::cq_gpu_runtime::hashed_run_step(__hash, #values);)*
        (__hash & #run.bucket_mask) as i64
    })
}
fn read_provider(facts: &ir::Analysis, keyed: &physical::Keyed) -> usize {
    let mut keys = keyed.columns.iter().map(symbol_name).collect::<Vec<_>>();
    keys.sort();
    facts.built[&symbol_name(&keyed.right)]
        .iter()
        .position(|build| serves(&build.0, &keys))
        .expect("validated keyed provider")
}

struct Smallest {
    reader: String,
    column: String,
    second: String,
    sides: [(String, usize, Vec<String>); 2],
}

fn smallest_pair(
    facts: &ir::Analysis,
    first: &physical::Definition,
    second: Option<&physical::Definition>,
) -> syn::Result<Option<Smallest>> {
    if !matches!(&first.operator, Operator::Probe(_, key)
        if matches!(key.choice, Some(physical::Choice::Smallest(_))))
    {
        return Ok(None);
    }
    let Some(second) = second else {
        return fail(
            first,
            "CQ_GPU_SMALLEST_PAIR",
            "smallest requires its checking read in the same stream",
        );
    };
    let Some((a, b, column)) = physical::binary_intersection(first, second, &facts.physical) else {
        return fail(
            first,
            "CQ_GPU_SMALLEST_PAIR",
            "smallest requires an adjacent binary intersection",
        );
    };
    let mut sides = Vec::new();
    for (ordinal, key) in [a, b].into_iter().enumerate() {
        let name = symbol_name(&key.right);
        let mut prefix: Vec<_> = key
            .columns
            .iter()
            .map(symbol_name)
            .filter(|c| c != &column)
            .collect();
        prefix.sort();
        let provider = facts.built.get(&name).and_then(|builds| {
            builds.iter().enumerate().find_map(|(provider, (levels, _))| {
                let Some((last, leading)) = levels.split_last() else { return None };
                let mut columns: Vec<_> = leading.iter().flatten().cloned().collect();
                columns.sort();
                (prefix_run(facts, &name, provider)
                    && last == &[column.clone()]
                    && columns == prefix
                    && !leading.is_empty()).then_some(provider)
            })
        }).ok_or_else(|| syn::Error::new_spanned(first, format!(
            "CQ_GPU_SMALLEST_TRIE: side {} provider {name} needs a recorded sorted PrefixRun with prefix [{}] followed by singleton [{}]; structure selection must record the candidate trie and cq_gpu::prefix_runs must realize it",
            ordinal + 1, prefix.join(","), column
        )))?;
        sides.push((name, provider, prefix));
    }
    Ok(Some(Smallest {
        reader: symbol_name(&first.name),
        column,
        second: symbol_name(&second.name),
        sides: sides.try_into().unwrap(),
    }))
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

fn leveled_hosts_by_name(program: &ir::Program, facts: &ir::Analysis) -> BTreeSet<String> {
    program
        .physical
        .program
        .definitions
        .iter()
        .filter_map(|definition| {
            let name = symbol_name(&definition.name);
            if facts
                .built
                .get(&name)
                .is_some_and(|providers| providers.iter().any(|p| leveled_type(&p.1)))
                && let Operator::Rename(rename) = &definition.operator
            {
                Some(symbol_name(&rename.relation))
            } else {
                None
            }
        })
        .collect()
}

fn validate_leveled_reads(program: &ir::Program, facts: &ir::Analysis) -> syn::Result<()> {
    let hosts = leveled_hosts_by_name(program, facts);
    let names: BTreeSet<_> = program
        .physical
        .program
        .definitions
        .iter()
        .filter_map(|definition| {
            if let Operator::Rename(rename) = &definition.operator
                && hosts.contains(&symbol_name(&rename.relation))
            {
                Some(symbol_name(&definition.name))
            } else {
                None
            }
        })
        .collect();
    for (position, definition) in program.physical.program.definitions.iter().enumerate() {
        if matches!(&definition.operator,Operator::Probe(_,key)|Operator::Semi(_,key)
            if matches!(key.choice,Some(physical::Choice::Smallest(_))))
        {
            if let Some(next) = program.physical.program.definitions.get(position + 1)
                && let Some((a, b, _)) =
                    physical::binary_intersection(definition, next, &facts.physical)
                && [a, b]
                    .iter()
                    .any(|key| names.contains(&symbol_name(&key.right)))
            {
                return fail(
                    definition,
                    "CQ_GPU_LEVELED_SMALLEST",
                    "leveled hosts do not support recorded single-run smallest intersections",
                );
            }
        }
        if definition
            .operator
            .input()
            .is_some_and(|input| names.contains(&symbol_name(input)))
            || (definition.name == program.physical.program.returned
                && names.contains(&symbol_name(&definition.name)))
        {
            return fail(
                definition,
                "CQ_GPU_LEVELED_SCAN",
                "leveled hosts supply keyed reads only; bind a contiguous DeviceColumns host for scans",
            );
        }
    }
    Ok(())
}

/// Reject unsupported *written* choices, including choices on unused results.
/// The output is a bijective ledger of all physical keyed reads, not just probes.
pub(crate) fn validate(program: &ir::Program, facts: &ir::Analysis) -> syn::Result<Vec<Read>> {
    validate_leveled_reads(program, facts)?;
    for (name, &size) in &facts.stages {
        if size == 0 || size > 1024 {
            return fail(
                &program.structures,
                "CQ_GPU_STAGE_SIZE",
                format!("stage {name} records batch {size}; supported scan batches are 1..=1024"),
            );
        }
    }
    let mut smallest_reads = BTreeMap::new();
    for (position, d) in program.physical.program.definitions.iter().enumerate() {
        let n = &d.name;
        match &d.operator {
            Operator::Merge(..) => return fail(n, "CQ_GPU_MERGE", "merge is unsupported"),
            Operator::Union(..) => return fail(n, "CQ_GPU_UNION", "union blocks are unsupported"),
            Operator::Anti(_, keyed) => {
                for operand in [&keyed.right] {
                    let name = symbol_name(operand);
                    if !facts.retained.contains_key(&name)
                        && !facts.built.get(&name).is_some_and(|providers| {
                            providers.iter().any(|provider| resident_type(&provider.1))
                        })
                    {
                        return fail(
                            n,
                            "CQ_GPU_ANTI_ROWS",
                            "anti right operand requires recorded retained rows or a resident host index",
                        );
                    }
                }
                if !matches!(keyed.choice, Some(physical::Choice::Eager(_))) {
                    return fail(
                        n,
                        "CQ_GPU_ANTI_CHOICE",
                        "anti requires recorded eager evaluation",
                    );
                }
            }
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
            Operator::Bind(bind) => {
                let input = symbol_name(&bind.input);
                let types = &facts.physical.rows[&input].types;
                let columns = types
                    .keys()
                    .map(|name| (name.clone(), quote!(0u64)))
                    .collect();
                bind_value(bind, types, &columns, &mut Constants::default())?;
            }
            Operator::Probe(_, k) | Operator::Semi(_, k)
                if matches!(k.choice, Some(physical::Choice::Smallest(_))) =>
            {
                let Some(pair) = smallest_pair(
                    facts,
                    d,
                    program.physical.program.definitions.get(position + 1),
                )?
                else {
                    return fail(
                        n,
                        "CQ_GPU_SMALLEST_PAIR",
                        "smallest must be recorded on the introducing probe",
                    );
                };
                smallest_reads.insert(symbol_name(n), "smallest:shorter-prefix".to_owned());
                smallest_reads.insert(pair.second, "smallest:binary-membership".to_owned());
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
                let hashed = qualified(&keyed.ty, "cq_gpu_runtime", "HashedRun");
                if keyed.levels.len() != 1 && !prefix {
                    return fail(
                        entry,
                        "CQ_GPU_MULTILEVEL_BUILD",
                        "multilevel builds require explicitly recorded cq_gpu_runtime::PrefixRun",
                    );
                }
                if !qualified(&keyed.ty, "cq_storage", "SortedRun") && !prefix && !bitmap && !hashed
                {
                    return fail(
                        entry,
                        "CQ_GPU_BUILD_TYPE",
                        format!(
                            "record SortedRun, HashedRun, PrefixRun, or certified AdaptiveKeySet membership; unsupported provider: {}",
                            entry.to_token_stream()
                        ),
                    );
                }
                if hashed && keyed.levels[0].columns.len() < 2 {
                    return fail(
                        entry,
                        "CQ_GPU_COMPOSITE_KEY",
                        "HashedRun requires at least two recorded key columns",
                    );
                }
                if keyed.levels[0].columns.len() != 1 && !hashed {
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
                    let ty = &facts.physical.rows[&symbol_name(&entry.name)].types[&key];
                    if !integral(ty)
                        && !(hashed && (string(ty) || type_name(ty).as_deref() == Some("bool")))
                    {
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
                dedup_policy(ty)?;
            }
            ir::Structure::Host { reader, keyed, .. }
                if reader != "scan" && !resident_type(&keyed.ty) =>
            {
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
            let source = if let Some(source) = smallest_reads.get(reader) {
                source.clone()
            } else if matches!(
                supply,
                syntax::SupplyKind::Const | syntax::SupplyKind::Range
            ) {
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
                    let provider = builds
                        .iter()
                        .position(|build| serves(&build.0, keys))
                        .unwrap();
                    if prefix_run(facts, build, provider) {
                        "rank-point+binary-search".to_owned()
                    } else {
                        "rank-point+residual".to_owned()
                    }
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
                let slot = context.hosts[&name]
                    .layout
                    .iter()
                    .position(|column| *column == symbol_name(&mapping.source))
                    .unwrap();
                bounds.insert(
                    symbol_name(&mapping.target),
                    quote!(__execution.column_bounds(#table,#slot)?),
                );
            }
        }
        Operator::Unit(_) => {}
        _ => {
            let input = symbol_name(first.operator.input().expect("checked stream root"));
            let table = id("table", &input);
            for (slot, column) in context.carried_columns[&input].iter().enumerate() {
                bounds.insert(
                    column.clone(),
                    quote!(__execution.column_bounds(#table,#slot)?),
                );
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
                    && let Some(slot) = context.carried_columns[&right]
                        .iter()
                        .position(|c| c == column)
                {
                    if leveled_provider(facts, &right, read_provider(facts, keyed)) {
                        let Operator::Rename(rename) = &context.physical[&right].operator else {
                            unreachable!()
                        };
                        let value = &context.hosts[&symbol_name(&rename.relation)].value;
                        quote!((#value).column_bounds(#slot)?)
                    } else {
                        quote!(__execution.column_bounds(#table,#slot)?)
                    }
                } else {
                    let binding = facts.filtered.get(&right)
                        .and_then(|f| f.constants.iter().find(|b| symbol_name(&b.column) == *column))
                        .ok_or_else(|| syn::Error::new_spanned(keyed,
                            "CQ_GPU_PAYLOAD_COLUMN: output has no source column or certified constant"))?;
                    let value = constants.term(
                        &binding.value,
                        &BTreeMap::new(),
                        facts.physical.rows[&right].types.get(column),
                    )?;
                    quote!(::cq_gpu_runtime::Bounds::singleton(#value))
                };
                bounds.insert(column.clone(), bound);
            }
        }
        if let Operator::Bind(bind) = &context.physical[name].operator {
            let types = &facts.physical.rows[&symbol_name(&bind.input)].types;
            let placeholders = types
                .keys()
                .map(|column| (column.clone(), quote!(0u64)))
                .collect();
            let (column, value) = bind_value(bind, types, &placeholders, constants)?;
            let bound = if let Some(source) =
                copied_column(&bind.expression).filter(|_| !closed_bind(bind, types))
            {
                bounds.get(&source).cloned().ok_or_else(|| {
                    syn::Error::new_spanned(
                        bind,
                        "CQ_GPU_COLUMN_BOUNDS: copied bind source has no bounds",
                    )
                })?
            } else {
                quote!(::cq_gpu_runtime::Bounds::singleton(#value))
            };
            bounds.insert(column, bound);
        }
        bounds.retain(|column, _| context.carried_columns[name].contains(column));
    }
    context.carried_columns[endpoint]
        .iter()
        .map(|column| {
            bounds.get(column).cloned().ok_or_else(|| {
                syn::Error::new_spanned(
                    first,
                    format!("CQ_GPU_COLUMN_BOUNDS: missing source for {column}"),
                )
            })
        })
        .collect()
}

pub(crate) fn compile(
    program: &ir::Program,
    options: crate::options::Options,
) -> syn::Result<syn::Expr> {
    let mut facts = ir::contract::check(program)?;
    record_frozen_providers(program, &mut facts)?;
    let provenance = validate(program, &facts)?;
    let retry = options.output == crate::options::OutputPolicy::Retry;
    let recorded_output = if retry { "retry" } else { "default" };
    let retry_default_capacity = crate::options::DEFAULT_RETRY_CAPACITY;
    // GPU join filters read bound device columns, not host PositionalRows handles.
    // Keep the checked filter facts below; suppress only the CPU source binder.
    let mut source_facts = facts.clone();
    source_facts.join_filters.clear();
    let mut context = Context::new(program, &source_facts);
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
            if leveled_hosts_by_name(program, &facts).contains(&name) {
                let value = &context.hosts[&name].value;
                Ok(quote!((#value).rows() as usize))
            } else {
                let table = id("host", &name);
                Ok(quote!(__execution.table_view(#table)?.len as usize))
            }
        })
        .transpose()?;
    let recorded_capacity = match &options.capacity {
        Some(relation) => {
            let name = symbol_name(relation);
            quote!(Some(#name))
        }
        None => quote!(None),
    };
    let frozen_names: BTreeSet<_> = facts
        .built
        .iter()
        .filter(|(_, providers)| providers.iter().any(|provider| resident_type(&provider.1)))
        .map(|(name, _)| name.clone())
        .collect();
    // Resident run payloads retain original host schema positions, not the
    // pruned/sorted layout used by freshly materialized producers.
    for name in &frozen_names {
        let Operator::Rename(rename) = &context.physical[name].operator else {
            return fail(
                context.physical[name],
                "CQ_GPU_FROZEN_HOST",
                "resident provider must be a host rename",
            );
        };
        context.carried_columns.insert(
            name.clone(),
            context.hosts[&symbol_name(&rename.relation)]
                .layout
                .iter()
                .map(|column| {
                    rename
                        .mapping
                        .iter()
                        .find(|mapping| symbol_name(&mapping.source) == *column)
                        .map(|mapping| symbol_name(&mapping.target))
                        .expect("checked host rename covers schema")
                })
                .collect(),
        );
    }
    let returned = symbol_name(&program.physical.program.returned);
    // Retained producers are build boundaries, including Rename row sources.
    // Their stream consumers start from the materialized table just as they do
    // for a retained Select; source predicates must not be fused into readers.
    let endpoints: BTreeSet<_> = program
        .physical
        .program
        .definitions
        .iter()
        .filter(|d| {
            frozen_names.contains(&symbol_name(&d.name))
                || !matches!(d.mode, physical::Mode::Pipe(_))
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
    let leveled_hosts = leveled_hosts_by_name(program, &facts);
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
        if !leveled_hosts.contains(&name) {
            host.push(quote!(let #table=__execution.bind_host(#value,#schema,#width)?;));
            if program.physical.program.host_is_set(&input.name) {
                host.push(quote!(__execution.certify_host_set(#table, #name)?;));
            }
        }
    }
    for name in &frozen_names {
        let Operator::Rename(rename) = &context.physical[name].operator else {
            unreachable!()
        };
        let input = symbol_name(&rename.relation);
        let value = &context.hosts[&input].value;
        let source = id("host", &input);
        let table = id("table", name);
        if !leveled_hosts.contains(&input) {
            host.push(quote!(let #table = #source;));
        }
        for (provider, build) in facts.built[name].iter().enumerate() {
            if !resident_type(&build.1) {
                return fail(
                    context.physical[name],
                    "CQ_GPU_FROZEN_PROVIDER",
                    "resident hosts cannot also request per-hole built providers",
                );
            }
            let levels = build.0.iter().map(|level| {
                let slots = level.iter().map(|column| {
                    context.carried_columns[name]
                        .iter()
                        .position(|carried| carried == column)
                        .expect("checked resident key slot")
                });
                quote!(&[#(#slots),*] as &[usize])
            });
            let run = provider_id("run", name, provider);
            if leveled_type(&build.1) {
                host.push(quote!(let #run = (#value).snapshot_ptr(&[#(#levels),*])?;));
            } else {
                host.push(quote!(let #run = (#value).resident_run(&[#(#levels),*])?;));
            }
        }
    }
    for (name, types) in &options.datalog_keys {
        if types.iter().any(|ty| !supported(ty)) {
            return fail(
                name,
                "CQ_GPU_DATALOG_KEY_TYPE",
                "unsupported frozen key column type",
            );
        }
        let table = id("datalog_table", &symbol_name(name));
        let membership = id("datalog_membership", &symbol_name(name));
        // Empty projected keys use a canonical u8 witness row; the index has
        // no key columns and therefore tests existence.
        let row_types = if types.is_empty() {
            vec![syn::parse_quote!(u8)]
        } else {
            types.clone()
        };
        let width = row_types.len();
        let key_width = types.len();
        host_pools.push(quote!(::cq_gpu_runtime::HostColumns::pool(&#name)));
        host_dictionaries.push(quote!(
            ::cq_gpu_runtime::HostColumns::seed_dictionary(&#name,__execution.dictionary_mut())?;
        ));
        host.push(quote! {
            {
                fn __cq_gpu_check_key_schema<'data, H: ::cq_gpu_runtime::HostColumns<'data, Row=(#(#row_types,)*)>>(_: H) {}
                __cq_gpu_check_key_schema(&#name);
            }
            let #table=__execution.bind_host(&#name,stringify!(#name),#width)?;
            let #membership=(#name).resident_membership(#key_width)?;
        });
    }
    let mut kernels = Vec::new();
    let mut kernel_reads = Vec::new();
    let mut covered_reads = BTreeMap::<String, usize>::new();
    let mut filter_reads = Vec::new();
    let mut covered_filters = BTreeSet::new();
    let mut all_constants = Constants::default();
    all_constants.datalog_keys = options
        .datalog_keys
        .iter()
        .map(|(name, types)| (symbol_name(name), types.clone()))
        .collect();
    for definition in &program.physical.program.definitions {
        let endpoint = symbol_name(&definition.name);
        if !endpoints.contains(&endpoint) || frozen_names.contains(&endpoint) {
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
        let fused = facts.dedup.get(&endpoint).is_some_and(|ty| {
            dedup_policy(ty).expect("validated dedup policy") == DedupPolicy::Fused
        });
        let fused_retry = retry && fused;
        let selection = if let Operator::Rename(rename) = &first.operator {
            if !fused
                && !chain
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
                let projection = rename.constants.is_none()
                    && !facts.join_filters.contains_key(&chain[0])
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
        // Pure projection aliases already own exact resident rows. Other
        // producers obey the selected output allocation/replay policy.
        let selection = selection.filter(|(_, _, projection)| !retry || *projection);
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
                    state.driver_columns.insert(column.clone(), index);
                    if selection.is_none()
                        && !fused_retry
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
                if let Some(constants) = &rename.constants {
                    let input = program
                        .physical
                        .inputs()
                        .find(|input| input.name == rename.relation)
                        .expect("checked occurrence source");
                    for equality in &constants.equalities {
                        let (index, column) = input
                            .columns
                            .iter()
                            .enumerate()
                            .find(|(_, column)| column.name == equality.left)
                            .expect("checked occurrence constant column");
                        let syntax::EqualityValue::Literal(expression) = &equality.right else {
                            return fail(
                                equality,
                                "CQ_GPU_OCCURRENCE_CONSTANT",
                                "occurrence constants require literal values",
                            );
                        };
                        let value = all_constants.literal(expression, Some(&column.ty))?;
                        loads.push(quote! {
                            __valid = __valid
                                && ::cq_gpu_runtime::column(__driver,#index,__row) == #value;
                        });
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
                    state.driver_columns.insert(column.clone(), index);
                }
                (
                    quote!(__execution.table_view(#table)?),
                    quote!(#(#loads)*),
                    state,
                    Some(table),
                )
            }
        };
        state.driver_row = driver_table.as_ref().map(|_| quote!(__row));
        let start = usize::from(matches!(
            first.operator,
            Operator::Rename(_) | Operator::Unit(_)
        ));
        let mut runs: BTreeSet<_> = chain
            .iter()
            .filter_map(|n| match &context.physical[n].operator {
                Operator::Probe(_, k) | Operator::Semi(_, k) | Operator::Anti(_, k) => {
                    Some((symbol_name(&k.right), read_provider(&facts, k)))
                }
                _ => None,
            })
            .collect();
        for (index, name) in chain.iter().enumerate() {
            if let Some(pair) = smallest_pair(
                &facts,
                context.physical[name],
                chain.get(index + 1).map(|next| context.physical[next]),
            )? {
                runs.extend(
                    pair.sides
                        .into_iter()
                        .map(|(name, provider, _)| (name, provider)),
                );
            }
        }
        let mut run_ids: Vec<_> = runs
            .iter()
            .map(|(n, p)| kernel_run_id(&facts, n, *p))
            .collect();
        let mut run_types = runs
            .iter()
            .map(|(name, provider)| {
                if leveled_provider(&facts, name, *provider) {
                    quote!(*const ::cq_gpu_runtime::LeveledRunsView)
                } else if membership(&facts, name) {
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
                if frozen_provider(&facts, n, *p) {
                    quote!(let #view = #run;)
                } else if membership(&facts, n) {
                    quote!(let #view=__execution.membership_view(#run)?;)
                } else {
                    quote!(let #view=__execution.run_view(#run)?;)
                }
            })
            .collect::<Vec<_>>();
        for (name, _) in &options.datalog_keys {
            let view = id("datalog_view", &symbol_name(name));
            let membership = id("datalog_membership", &symbol_name(name));
            run_ids.push(view.clone());
            run_types.push(quote!(::cq_gpu_runtime::ExactMembershipView));
            run_bindings.push(quote!(let #view=#membership;));
        }
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
        let recorded_batches: BTreeSet<_> = chain
            .iter()
            .filter_map(|n| facts.stages.get(n))
            .copied()
            .collect();
        let expanding = chain
            .iter()
            .any(|name| match &context.physical[name].operator {
                Operator::Probe(_, key) => {
                    !membership(&facts, &symbol_name(&key.right))
                        && !boolean_probe(&facts, &endpoint, key)
                }
                _ => false,
            });
        let maximum = if expanding { 256 } else { 1024 };
        if recorded_batches.len() > 1
            || recorded_batches
                .iter()
                .any(|&size| size == 0 || size > maximum)
        {
            return fail(
                definition,
                "CQ_GPU_STAGE_SIZE",
                format!(
                    "fused streams require one exact batch size in 1..={maximum}; split incompatible stages into materialized streams"
                ),
            );
        }
        let items = if (expanding && recorded_batches.contains(&256))
            || (fused_retry && (recorded_batches.is_empty() || recorded_batches.contains(&256)))
        {
            options.items
        } else {
            1
        };
        let batch = if items > 1 {
            256 * items
        } else {
            recorded_batches.first().copied().unwrap_or(1024)
        };
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
            serial: !fused_retry
                && !chain
                    .iter()
                    .any(|n| facts.stages.contains_key(n) || facts.expands.contains(n)),
            selection: selection.is_some(),
            batch: expansion_batch,
            items,
            fused_retry,
            frontier_threshold: options.frontier,
            descriptors: Vec::new(),
        };
        let resume = if !retry && (fused || (capacity.is_none() && selection.is_none())) {
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
        let root_descriptor_words = emitter.descriptors.last().copied().unwrap_or(0);
        // Publication copies a maximum-sized descriptor, including zeroed
        // padding. Reusing the root claim area must not lose the tail room
        // formerly supplied by the separate claim allocation.
        let shared_words = if frontier && root_descriptor_words == descriptor_words {
            shared_words + descriptor_words
                - emitter
                    .descriptors
                    .first()
                    .copied()
                    .unwrap_or(descriptor_words)
        } else {
            shared_words
        };
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
            root_descriptor_words,
            continuations,
            frontier,
            batch,
            items,
            selection,
            bounds,
            endpoint,
        });
    }
    for (name, _) in &options.datalog_keys {
        if !all_constants.used_datalog_keys.contains(&symbol_name(name)) {
            return fail(
                name,
                "CQ_GPU_DATALOG_KEYS",
                "certified key host is not used by an exact membership intrinsic",
            );
        }
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
            retry,
        );
        device.push(item);
        launches.push(launch);
    }
    let output = id("table", &returned);
    let mut output_types = Vec::new();
    let mut output_columns = Vec::new();
    for expression in &program.physical.program.head.args {
        let Some(column) = syntax::rust::variable(expression)
            .map(symbol_name)
            .or_else(|| copied_column(expression))
        else {
            return fail(
                expression,
                "CQ_GPU_HEAD_EXPR",
                "heads require columns or copied dereferences; normalize typed literals to Bind",
            );
        };
        if copied_column(expression).is_some()
            && type_name(&facts.physical.rows[&returned].types[&column]).as_deref()
                == Some("String")
        {
            return fail(
                expression,
                "CQ_GPU_HEAD_EXPR",
                "copied head requires a Copy scalar type",
            );
        }
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
    let final_dedup = facts
        .head_dedup
        .get(&returned)
        .map(|ty| {
            let codec = provider_codec(ty)?.value();
            if retry && dedup_policy(ty)? == DedupPolicy::Fused {
                let kernel = format_ident!("cq_gpu_head_{}", module_number);
                let prepare = format_ident!("prepare_{}", kernel);
                let columns: Vec<_> = (0..output_columns.len()).collect();
                let values: Vec<_> = columns.iter().map(|column| {
                    let value = format_ident!("__head_value_{column}");
                    quote!(#value)
                }).collect();
                let loads = columns.iter().zip(&values).map(|(column,value)| quote! {
                    let #value=if __valid {::cq_gpu_runtime::column(__head_input,#column,__row)}else{0};
                });
                let candidate = tile_candidate(&values, true);
                let (buffers, output_tile) = tile_output(columns.len(), options.items, true);
                let items = options.items;
                let tile_rows = 256 * items;
                let hint = capacity.as_ref().map(|capacity| quote!(Some(#capacity))).unwrap_or_else(|| quote!(None));
                device.push(quote! {
                    #[kernel]
                    #[launch_bounds(256)]
                    #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(64,64))]
                    pub fn #kernel(
                        __head_input: ::cq_gpu_runtime::TableView,
                        __output: ::cq_gpu_runtime::OutputView,
                        __fused: ::cq_gpu_runtime::FusedRetryView,
                    ) {
                        let __tid=::cuda_device::thread::threadIdx_x() as usize;
                        let __scratch=::cuda_device::DynamicSharedArray::<u8>::get_raw() as *mut u32;
                        unsafe {
                            let __len=::cq_gpu_runtime::table_len(__head_input);
                            let mut __tile=::cuda_device::thread::blockIdx_x() as u64;
                            while __tile<__len.div_ceil(#tile_rows as u64) {
                                #buffers
                                let mut __item=0usize;
                                while __item<#items {
                                    let __row=__tile * #tile_rows as u64+(__item*256+__tid) as u64;
                                    let mut __valid=__row<__len;
                                    #(#loads)*
                                    #candidate
                                    __item+=1;
                                }
                                #output_tile
                                __tile+=::cuda_device::thread::gridDim_x() as u64;
                            }
                        }
                    }
                });
                return Ok(quote! {
                    let #output=__execution.project_table(#output,&[#(#output_columns),*])?;
                    let __head_input=__execution.table_view(#output)?;
                    let __head_bounds=[#(__execution.column_bounds(#output,#columns)?),*];
                    let __head_grid=__head_input.len.div_ceil(#tile_rows as u64).clamp(1,568) as u32;
                    let __head_launch=__module.#prepare(::cuda_core::LaunchConfig1D::new(__head_grid,256,64))?;
                    let #output=__execution.execute_retry_fused(concat!(module_path!(),"::",stringify!(#kernel)),&__head_bounds,#hint,
                        |__stream,__output,__fused| {
                            unsafe {__module.#kernel(__stream,&__head_launch,__head_input,__output,__fused)?;}
                            Ok(())
                        },
                    )?;
                    let #output=__execution.reencode_table(#output,#codec)?;
                });
            }
            if !retry && dedup_policy(ty)? == DedupPolicy::Fused {
                // Head canon is after the ordered projection, not canon over
                // the producer's wider heading. Keep this separate boundary
                // so other consumers of the producer retain their rows.
                let kernel = format_ident!("cq_gpu_head_{}", module_number);
                let prepare = format_ident!("prepare_{}", kernel);
                let columns: Vec<_> = (0..output_columns.len()).collect();
                device.push(quote! {
                    #[kernel]
                    #[launch_bounds(256)]
                    #[launch_contract(domain=1,block=(256,1,1))]
                    pub fn #kernel(
                        input: ::cq_gpu_runtime::TableView,
                        output: ::cq_gpu_runtime::OutputView,
                        resume: ::cq_gpu_runtime::ResumeView,
                        set: ::cq_gpu_runtime::FusedSetView,
                    ) {
                        unsafe {
                            let lane=::cuda_device::thread::index_1d().get() as usize;
                            let checkpoint=resume.state.add(lane);
                            let saved=*checkpoint;
                            if saved==u64::MAX { return; }
                            let mut row=if saved==0 {lane as u64}else{saved-1};
                            let len=::cq_gpu_runtime::table_len(input);
                            while row<len && !::cq_gpu_runtime::resume_requested(resume) {
                                match ::cq_gpu_runtime::fused_emit(output,set,[
                                    #(::cq_gpu_runtime::column(input,#columns,row)),*
                                ]) {
                                    0 => { row+=::cuda_device::thread::gridDim_x() as u64*256; }
                                    2 => { ::cq_gpu_runtime::resume_suspend(resume); }
                                    _ => {}
                                }
                            }
                            *checkpoint=if row>=len {u64::MAX}else{row+1};
                        }
                    }
                });
                return Ok(quote! {
                    let #output=__execution.project_table(#output,&[#(#output_columns),*])?;
                    let __head_input=__execution.table_view(#output)?;
                    let __head_bounds=[#(__execution.column_bounds(#output,#columns)?),*];
                    let __head_grid=__head_input.len.div_ceil(256).clamp(1,568) as u32;
                    let __head_launch=__module.#prepare(::cuda_core::LaunchConfig1D::new(__head_grid,256,0))?;
                    let #output=__execution.execute_resumable_fused(
                        &__head_bounds,__head_input.len as usize,__head_grid as usize*256,
                        |stream,output,resume,set| {
                            unsafe {__module.#kernel(stream,&__head_launch,__head_input,output,resume,set)?;}
                            Ok(())
                        },
                    )?;
                    let #output=__execution.reencode_table(#output,#codec)?;
                });
            }
            Ok::<_, syn::Error>(quote! {
                let #output=__execution.project_table(#output,&[#(#output_columns),*])?;
                let #output=__execution.dedup(#output)?;
                let #output=__execution.reencode_table(#output,#codec)?;
            })
        })
        .transpose()?;
    let output_codec = facts
        .head_dedup
        .get(&returned)
        .map(provider_codec)
        .transpose()?
        .map(RecordedCodec::marker)
        .unwrap_or(output_codec);
    let final_columns: Vec<_> = if final_dedup.is_some() {
        (0..output_columns.len()).collect()
    } else {
        output_columns
    };
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
    let recorded_physical = program.physical.to_token_stream().to_string();
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
    let shared_dictionary = options.datalog_dictionary.as_ref().map(|dictionary| {
        quote! {
            {
                let __cq_gpu_shared_dictionary:
                    &::std::sync::Arc<::cq_gpu_runtime::Dictionary<'_>> = &#dictionary;
                __execution.dictionary_mut().merge(__cq_gpu_shared_dictionary.as_ref())?;
            }
        }
    });
    let shared_pool = options.datalog_pool.as_ref().map_or_else(
        || quote!(None),
        |pool| {
            quote!({
                let __cq_gpu_shared_pool: &::std::sync::Arc<::cq_gpu_runtime::Pool> = &#pool;
                Some(__cq_gpu_shared_pool.clone())
            })
        },
    );
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
    let completion = if options.defer_completion {
        quote!(
            const __CQ_GPU_COMPLETION: &str = "round";
        )
    } else {
        quote! {
            const __CQ_GPU_COMPLETION: &str = "query";
            __execution.finish()?;
        }
    };
    syn::parse2(quote!({
        #module
        const __CQ_GPU_KEYED_READS: &[&str]=&[#(#reads),*];
        const __CQ_GPU_KERNEL_READS: &[(&str,&str)]=&[#(#kernel_reads),*];
        const __CQ_GPU_JOIN_FILTER_READS: &[(&str,&str)]=&[#(#filter_reads),*];
        const __CQ_GPU_RECORDED_PHYSICAL: &str=#recorded_physical;
        const __CQ_GPU_RECORDED_STRUCTURES: &[&str]=&[#(#recorded_structures),*];
        const __CQ_GPU_DERIVED_CODECS: &[(&str,&str)]=&[#(#derived_codecs),*];
        const __CQ_GPU_CAPACITY: ::core::option::Option<&str>=#recorded_capacity;
        const __CQ_GPU_OUTPUT_POLICY: &str=#recorded_output;
        const __CQ_GPU_RETRY_DEFAULT_CAPACITY: usize=#retry_default_capacity;
        #(#source_bindings)*
        let __pool=#shared_pool #(.or_else(|| #host_pools))*;
        ::cq_gpu_runtime::with_runtime_for(__pool,|__runtime| {
            let mut __execution=__runtime.execution();
            #load
            #shared_dictionary
            #(#host_dictionaries)*
            #(#host)*
            #(#constants)*
            #(#launches)*
            #final_dedup
            let __answer=__execution.into_columns::<(#(#output_types,)*),#output_codec>(
                #output,&[#(#final_columns),*]
            )?;
            #completion
            Ok(__answer)
        }).unwrap_or_else(|error|panic!("CQ_GPU_EXECUTION: {error}"))
    }))
}

#[derive(Clone, Default)]
struct State {
    columns: BTreeMap<String, Tokens>,
    witnesses: BTreeMap<String, (Tokens, Tokens)>,
    leveled_witnesses: BTreeMap<String, LeveledWitness>,
    item: Option<Tokens>,
    driver_columns: BTreeMap<String, usize>,
    driver_row: Option<Tokens>,
}
impl State {
    fn bind_driver(&mut self, column: &str, expression: &syn::Expr) {
        let slot =
            copied_column(expression).and_then(|source| self.driver_columns.get(&source).copied());
        if let Some(slot) = slot {
            self.driver_columns.insert(column.to_owned(), slot);
        } else {
            self.driver_columns.remove(column);
        }
    }
}

#[derive(Clone)]
struct LeveledWitness {
    ranges: Tokens,
    // Low word: total matching rows. High word: snapshot's live run count.
    meta: Tokens,
}
impl LeveledWitness {
    fn ranges(&self) -> Tokens {
        let ranges = &self.ranges;
        let meta = &self.meta;
        quote! {
            let __leveled_ranges=&#ranges;
            let __leveled_len=if __valid {(#meta) as u32}else{0u32};
        }
    }
    fn point(&self) -> Tokens {
        let ranges = &self.ranges;
        quote!({
            let __range=(#ranges)[__leveled_run_id];
            (__range as u32,(__range>>32) as u32)
        })
    }
    fn stage(&self, target: &Self, rows: usize, words: &mut usize) -> (Tokens, Tokens) {
        let slot = *words;
        *words += (2 + 2 * LEVELED_MAX_RUNS) * rows;
        let ranges = &self.ranges;
        let meta = &self.meta;
        let target_ranges = &target.ranges;
        let target_meta = &target.meta;
        let save = quote! {
            *__descriptor.add(#slot+__input_slot)=(#meta) as u32;
            *__descriptor.add(#slot+#rows+__input_slot)=((#meta)>>32) as u32;
            let mut __witness_run=0usize;
            while __witness_run<((#meta)>>32) as usize {
                let __range=(#ranges)[__witness_run];
                *__descriptor.add(#slot+(2+2*__witness_run)*#rows+__input_slot)=__range as u32;
                *__descriptor.add(#slot+(3+2*__witness_run)*#rows+__input_slot)=(__range>>32) as u32;
                __witness_run+=1;
            }
        };
        let restore = quote! {
            #target_meta=if __valid {
                *__descriptor.add(#slot+__parent) as u64 |
                    ((*__descriptor.add(#slot+#rows+__parent) as u64)<<32)
            } else {0};
            let mut __witness_run=0usize;
            while __witness_run<((#target_meta)>>32) as usize {
                (#target_ranges)[__witness_run]=
                    *__descriptor.add(#slot+(2+2*__witness_run)*#rows+__parent) as u64 |
                    ((*__descriptor.add(#slot+(3+2*__witness_run)*#rows+__parent) as u64)<<32);
                __witness_run+=1;
            }
        };
        (save, restore)
    }
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
    root_descriptor_words: usize,
}

struct ResumeEmitter<'a, 'b, 'c> {
    source: &'c mut Emitter<'a, 'b>,
    program: ResumeProgram,
    dispatch: Vec<(u32, usize, Tokens, Tokens)>,
}

impl ResumeEmitter<'_, '_, '_> {
    fn register(&mut self) -> Tokens {
        let slot = self.program.registers;
        self.program.registers += 1;
        quote!(__resume_registers[#slot])
    }

    fn witness_registers(&mut self) -> LeveledWitness {
        let begin = self.program.registers;
        self.program.registers += LEVELED_MAX_RUNS;
        let end = self.program.registers;
        let meta = self.register();
        LeveledWitness {
            ranges: quote!(__resume_registers[#begin..#end]),
            meta,
        }
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
            if facts.dedup.get(self.source.endpoint).is_some_and(|ty| {
                dedup_policy(ty).expect("validated dedup policy") == DedupPolicy::Fused
            }) {
                let insert = quote! {
                    if __valid {
                        match ::cq_gpu_runtime::fused_emit(__output,__fused,[#(#values),*]) {
                            0 => { __valid=false; }
                            2 => { ::cq_gpu_runtime::resume_suspend(__resume); }
                            _ => {}
                        }
                    }
                };
                return Ok(if self.source.serial {
                    self.block(quote! {
                        #insert
                        if !__valid { __resume_pc=#next; }
                    })
                } else {
                    self.block(quote! {
                        #insert
                        // Rejoin before retrying busy slots: no lane waits on
                        // an owner that has not yet executed its publication.
                        let (_,__pending)=::cq_gpu_runtime::tiles::scan(__valid as u32,__scratch);
                        if __pending==0 { __resume_pc=#next; }
                    })
                });
            }
            if self.source.items > 1 && !self.source.serial {
                let items = self.source.items;
                let item = state.item.as_ref().expect("cooperative item cursor");
                let hits = self.program.registers;
                let payload = hits + items;
                self.program.registers += items * (values.len() + 1);
                let stores = values.iter().enumerate().map(|(column, value)| {
                    let base = payload + column * items;
                    quote!(__resume_registers[#base+__item]=if __valid {(#value) as u64}else{0};)
                });
                let writes = values.iter().enumerate().map(|(column, _)| {
                    let base = payload + column * items;
                    quote!(::cq_gpu_runtime::write_output(__output,#column,__destination,__resume_registers[#base+__tile_item] as i64);)
                });
                return Ok(self.block(quote! {
                    let __item=#item;
                    __resume_registers[#hits+__item]=__valid as u64;
                    #(#stores)*
                    if __item+1<#items {
                        __resume_pc=#next;
                    } else {
                        let mut __tile_count=0u32;
                        let mut __tile_item=0usize;
                        while __tile_item<#items {
                            __tile_count+=__resume_registers[#hits+__tile_item] as u32;
                            __tile_item+=1;
                        }
                        let (__offset,__total)=::cq_gpu_runtime::tiles::scan(__tile_count,__scratch);
                        if __tid==0 {
                            *(__scratch.add(8) as *mut u64)=if __total!=0 {::cq_gpu_runtime::resume_reserve(__output,__total as u64)}else{0};
                        }
                        ::cuda_device::thread::sync_threads();
                        let __base=*(__scratch.add(8) as *const u64);
                        if __base==u64::MAX {
                            if __tid==0 {::cq_gpu_runtime::resume_suspend(__resume);}
                            __resume_pause=true;
                        } else {
                            let mut __destination=__base+__offset as u64;
                            __tile_item=0;
                            while __tile_item<#items {
                                if __resume_registers[#hits+__tile_item]!=0 {
                                    #(#writes)*
                                    __destination+=1;
                                }
                                __tile_item+=1;
                            }
                            __resume_pc=#next;
                        }
                    }
                }));
            }
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
            Operator::Bind(bind) => {
                let (column, value) = bind_value(
                    bind,
                    &facts.physical.rows[&symbol_name(&bind.input)].types,
                    &state.columns,
                    self.source.constants,
                )?;
                state.bind_driver(&column, &bind.expression);
                state.columns.insert(column, value);
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                self.steps(index + 1, state, next)
            }
            Operator::Project(_) => {
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                self.steps(index + 1, state, next)
            }
            Operator::Semi(_, k) | Operator::Probe(_, k) | Operator::Anti(_, k)
                if membership(facts, &symbol_name(&k.right)) =>
            {
                let negative = matches!(definition.operator, Operator::Anti(..));
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
                        let value = self.source.constants.term(
                            &binding.value,
                            &state.columns,
                            facts.physical.rows[&right].types.get(column),
                        )?;
                        state.columns.insert(column.clone(), value);
                        state.driver_columns.remove(column);
                    }
                }
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                let rest = self.steps(index + 1, state, next)?;
                Ok(self.block(quote! {
                    __valid=__valid && (::cq_gpu_runtime::membership_contains(#view,#key) != #negative);
                    __resume_pc=#rest;
                }))
            }
            Operator::Probe(_, k) if boolean_probe(facts, self.source.endpoint, k) => {
                let found =
                    if let Some(witness) = self.source.leveled_witness(index, k, &mut state)? {
                        let meta = witness.meta;
                        quote!((#meta) as u32!=0)
                    } else {
                        self.source.membership_lookup(k, &state)?
                    };
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                let rest = self.steps(index + 1, state, next)?;
                Ok(self.block(quote! {
                    __valid=__valid && (#found);
                    __resume_pc=#rest;
                }))
            }
            Operator::Semi(_, k) | Operator::Anti(_, k) => {
                let negative = matches!(definition.operator, Operator::Anti(..));
                if leveled_provider(facts, &symbol_name(&k.right), read_provider(facts, k)) {
                    let mut save = Tokens::new();
                    let found = if facts.witnesses.contains_key(&name) {
                        let snapshot =
                            kernel_run_id(facts, &symbol_name(&k.right), read_provider(facts, k));
                        let run = provider_id(
                            "run_view",
                            &symbol_name(&k.right),
                            read_provider(facts, k),
                        );
                        let lookup = self.source.lookup(k, &state)?;
                        let witness = self.witness_registers();
                        let ranges = &witness.ranges;
                        let search = leveled_ranges_into(&snapshot, &run, lookup, ranges.clone());
                        let meta = &witness.meta;
                        save = quote! {
                            #search
                            #meta=__leveled_len as u64 | ((::cq_gpu_runtime::leveled_count(#snapshot) as u64)<<32);
                        };
                        let found = quote!((#meta) as u32!=0);
                        state.leveled_witnesses.insert(name.clone(), witness);
                        found
                    } else {
                        self.source.membership_lookup(k, &state)?
                    };
                    state
                        .columns
                        .retain(|column, _| context.carried_columns[&name].contains(column));
                    let rest = self.steps(index + 1, state, next)?;
                    return Ok(self.block(quote! {
                        #save
                        __valid=__valid && ((#found)!=#negative);
                        __resume_pc=#rest;
                    }));
                }
                let lookup = self.source.lookup(k, &state)?;
                let begin = self.register();
                let end = self.register();
                if facts.witnesses.contains_key(&name) {
                    state
                        .witnesses
                        .insert(name.clone(), (quote!(#begin as u32), quote!(#end as u32)));
                }
                let contains = self
                    .source
                    .contains(k, &state, quote!(__begin), quote!(__end))?;
                state
                    .columns
                    .retain(|column, _| context.carried_columns[&name].contains(column));
                let rest = self.steps(index + 1, state, next)?;
                Ok(self.block(quote! {
                    let (__begin,__end)=if __valid { #lookup } else {(0,0)};
                    #begin=__begin as u64;
                    #end=__end as u64;
                    #contains
                    __valid=__valid && (__found != #negative);
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
        let local = self.source.serial || self.source.lane_local_probe(&name);
        let right = symbol_name(&k.right);
        let provider = read_provider(facts, k);
        let run = provider_id("run_view", &right, provider);
        let pair = self.source.smallest(index)?;
        let witness = self.source.leveled_witness(index, k, &mut state)?;
        let first = self
            .source
            .probe_lookup(index, k, pair.as_ref(), &mut state)?;
        let leveled = leveled_provider(facts, &right, provider);
        let snapshot = kernel_run_id(facts, &right, provider);
        let mut lookup = if leveled {
            let ranges = witness
                .as_ref()
                .map(LeveledWitness::ranges)
                .unwrap_or_else(|| leveled_ranges(&snapshot, &run, first.clone()));
            quote! {#ranges let (__begin,__end)=(0u32,__leveled_len);}
        } else if let Some(pair) = &pair {
            let choose = self
                .source
                .smallest_lookup(pair, &mut state, first.clone())?;
            quote! {
                let (__begin,__end,__smallest_right,__smallest_other_begin,__smallest_other_end) =
                    if __valid {#choose}else{(0,0,false,0,0)};
            }
        } else {
            quote!(let (__begin,__end)=if __valid {#first}else{(0,0)};)
        };
        let mut child = State::default();
        let mut save = Vec::new();
        let mut restore = Vec::new();
        let offset = (self.program.shared_words + 1) & !1;
        let items = if leveled && !local {
            1
        } else {
            self.source.items
        };
        let rows = 256 * items;
        let offsets = 2 + rows;
        let total_slot = 2 + 2 * rows;
        let mut words = total_slot + 2;
        let caller_next = next;
        let mut next = next;
        let mut run_loop = None;
        if leveled {
            let (range_save, range_restore) = if local && witness.is_some() {
                (
                    Tokens::new(),
                    leveled_restore(&snapshot, &run, witness.as_ref().unwrap().point()),
                )
            } else if local {
                let slot = self.program.registers;
                self.program.registers += LEVELED_MAX_RUNS;
                let search = leveled_ranges_into(
                    &snapshot,
                    &run,
                    first,
                    quote!(__resume_registers[#slot..#slot+#LEVELED_MAX_RUNS]),
                );
                lookup = quote! {#search let (__begin,__end)=(0u32,__leveled_len);};
                let save = Tokens::new();
                let restore = leveled_restore(
                    &snapshot,
                    &run,
                    quote!((
                        __resume_registers[#slot+__leveled_run_id] as u32,
                        (__resume_registers[#slot+__leveled_run_id]>>32) as u32
                    )),
                );
                (save, restore)
            } else {
                let cursor = self.register();
                let valid = self.register();
                let run_guard = self.block(Tokens::new());
                let advance_run = self.block(quote! {#cursor+=1;__resume_pc=#run_guard;});
                next = advance_run;
                lookup = if let Some(witness) = &witness {
                    let ranges = &witness.ranges;
                    quote! {
                        let __range=(#ranges)[#cursor as usize];
                        let (__begin,__end)=if __valid {(__range as u32,(__range>>32) as u32)}else{(0u32,0u32)};
                    }
                } else {
                    quote! {
                        let #run=::cq_gpu_runtime::leveled_run(#snapshot,#cursor as usize);
                        let (__begin,__end)=if __valid {#first}else{(0u32,0u32)};
                    }
                };
                run_loop = Some((cursor, valid, run_guard, Tokens::new()));
                (
                    Tokens::new(),
                    quote! {
                        let __leveled_run_id=*__descriptor.add(1) as usize;
                        let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
                    },
                )
            };
            save.push(range_save);
            restore.push(range_restore);
        }
        let item = if leveled && !local {
            quote!(0usize)
        } else {
            state.item.clone().unwrap_or(quote!(0usize))
        };
        let compact = if local {
            None
        } else {
            self.source
                .compact_root(&name, k, pair.as_ref(), &mut state, rows, &mut words)
        };
        let root_guard = compact.as_ref().map(|(guard, _, _)| guard);
        // The range loop resumes in a different PC from its checked entry.
        // Rebind the already-checked parent identity in the descriptor-save PC.
        let stage_root_identity = if leveled && !local && compact.is_some() {
            state
                .driver_row
                .as_ref()
                .map(|row| quote!(let __driver_row_id=#row;))
        } else {
            None
        };
        if let Some((_, root_save, root_restore)) = &compact {
            save.push(root_save.clone());
            restore.push(root_restore.clone());
            let register = self.register();
            restore.push(quote!(#register=__driver_row;));
            child.driver_row = Some(quote!(#register));
            child.driver_columns = state.driver_columns.clone();
        }
        if pair.is_some() && compact.is_none() {
            if local {
                let side = self.register();
                let other_begin = self.register();
                let other_end = self.register();
                save.push(quote! {
                    #side=__smallest_right as u64;
                    #other_begin=__smallest_other_begin as u64;
                    #other_end=__smallest_other_end as u64;
                });
                restore.push(quote! {
                    let __smallest_right=#side!=0;
                    let __smallest_other_begin=#other_begin as u32;
                    let __smallest_other_end=#other_end as u32;
                });
            } else {
                let slot = words;
                words += 3 * rows;
                save.push(quote! {
                    *__descriptor.add(#slot+__input_slot)=__smallest_right as u32;
                    *__descriptor.add(#slot+#rows+__input_slot)=__smallest_other_begin;
                    *__descriptor.add(#slot+2*#rows+__input_slot)=__smallest_other_end;
                });
                restore.push(quote! {
                    let __smallest_right=__valid && *__descriptor.add(#slot+__parent)!=0;
                    let __smallest_other_begin=if __valid {*__descriptor.add(#slot+#rows+__parent)}else{0};
                    let __smallest_other_end=if __valid {*__descriptor.add(#slot+2*#rows+__parent)}else{0};
                });
            }
        }
        if local {
            child = state;
        } else {
            for (column, value) in &state.columns {
                let register = self.register();
                if let Some(slot) = state
                    .driver_columns
                    .get(column)
                    .filter(|_| compact.is_some())
                {
                    restore.push(quote! {
                        #register=if __valid {::cq_gpu_runtime::column(__driver,#slot,__driver_row) as u64}else{0};
                    });
                } else {
                    let slot = words;
                    words += 2 * rows;
                    save.push(
                        quote! {*(__descriptor.add(#slot) as *mut i64).add(__input_slot)=#value;},
                    );
                    restore.push(quote! {
                        #register=if __valid {*(__descriptor.add(#slot) as *const i64).add(__parent) as u64}else{0};
                    });
                }
                child
                    .columns
                    .insert(column.clone(), quote!(#register as i64));
            }
            for (origin, (a, b)) in &state.witnesses {
                let slot = words;
                words += 2 * rows;
                let begin = self.register();
                let end = self.register();
                save.push(quote! {*__descriptor.add(#slot+__input_slot)=#a;*__descriptor.add(#slot+#rows+__input_slot)=#b;});
                restore.push(quote! {
                    #begin=if __valid {*__descriptor.add(#slot+__parent) as u64}else{0};
                    #end=if __valid {*__descriptor.add(#slot+#rows+__parent) as u64}else{0};
                });
                child
                    .witnesses
                    .insert(origin.clone(), (quote!(#begin as u32), quote!(#end as u32)));
            }
            for (origin, witness) in &state.leveled_witnesses {
                let target = self.witness_registers();
                let (range_save, range_restore) = witness.stage(&target, rows, &mut words);
                save.push(range_save);
                restore.push(range_restore);
                child.leveled_witnesses.insert(origin.clone(), target);
            }
            self.program.shared_words = offset + words;
            self.program.descriptor_words = self.program.descriptor_words.max(words);
            if offset == 16 {
                self.program.root_descriptor_words = words;
            }
        }
        let row = self.register();
        let end = self.register();
        if !local {
            child.item = Some(quote!((#row % #items as u64) as usize));
        }
        let external = (!local).then(|| self.register());
        let guard = self.block(Tokens::new());
        let advance = self.block(quote! {#row+=1;__resume_pc=#guard;});
        let mut loads = Vec::new();
        for column in &facts.physical.headings[&right] {
            if let Some(pair) = &pair {
                if column == &pair.column {
                    let value = self.source.smallest_value(pair, quote!(__build_row));
                    let register = self.register();
                    loads.push(quote!(#register=if __valid {(#value) as u64}else{0};));
                    child
                        .columns
                        .insert(column.clone(), quote!(#register as i64));
                    child.driver_columns.remove(column);
                }
                continue;
            }
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
                self.source.constants.term(
                    &constant.value,
                    &child.columns,
                    facts.physical.rows[&right].types.get(column),
                )?
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
            if !exact_run_key(facts, k, column) {
                child.driver_columns.remove(column);
            }
        }
        let other_range = if let Some(pair) = pair.as_ref().filter(|_| compact.is_some()) {
            self.source.smallest_other_range(pair, &child)?
        } else {
            Tokens::new()
        };
        let (residual, after, next_index) = if let Some(pair) = &pair {
            (
                self.source
                    .smallest_contains(pair, child.columns[&pair.column].clone()),
                &pair.second,
                index + 2,
            )
        } else {
            (
                self.source.residual(k, &child, quote!(__build_row))?,
                &name,
                index + 1,
            )
        };
        child
            .columns
            .retain(|column, _| context.carried_columns[after].contains(column));
        let previous_items = self.source.items;
        if local || leveled {
            self.source.items = 1;
        }
        let rest = self.steps(next_index, child, advance)?;
        self.source.items = previous_items;
        if local {
            let load = self.block(quote! {
                let __build_row=#row as u32;
                __valid=#row<#end;
                #(#restore)* #(#loads)*
                __valid=__valid && (#residual);
                __resume_pc=#rest;
            });
            self.program.blocks[guard as usize] = if self.source.serial {
                quote! {__resume_pc=if #row<#end {#load}else{#next};}
            } else {
                quote! {
                    let (_,__remaining)=::cq_gpu_runtime::tiles::scan((#row<#end) as u32,__scratch);
                    __resume_pc=if __remaining!=0 {#load}else{#next};
                }
            };
            return Ok(self.block(quote! {
                #lookup
                #(#save)*
                #row=__begin as u64;
                #end=__end as u64;
                __resume_pc=#guard;
            }));
        }
        let external = external.expect("cooperative expansion has an external-task flag");
        let batch = if items > 1 {
            256u32
        } else if leveled {
            self.source.batch.min(256) as u32
        } else {
            self.source.batch as u32
        };
        let chunk_rows = batch as usize * items;
        let load = self.block(quote! {
            let __descriptor=__scratch.add(#offset);
            let __position=#row * #batch as u64 + __tid as u64;
            let __total=*__descriptor.add(#total_slot);
            __valid=__tid<#batch as usize && __position<__total as u64;
            let __parent=if __valid {
                ::cq_gpu_runtime::tiles::owner(__descriptor.add(#offsets),#rows as u32,__position as u32) as usize
            }else{0};
            let __build_row=if __valid {
                *__descriptor.add(2+__parent)+__position as u32-*__descriptor.add(#offsets+__parent)
            }else{0};
            #(#restore)* #(#loads)*
            #other_range
            __valid=__valid && (#residual);
            __resume_pc=#rest;
        });
        // PC 3 retires an externally claimed frontier task. An inline child
        // instead returns to its enclosing frame, whose cursors remain live.
        self.program.blocks[guard as usize] = quote! {
            __resume_pc=if #row<#end {#load}else if #external!=0 {3}else{#next};
        };
        let stage = self.dispatch.len() as u32 + 1;
        self.dispatch.push((
            stage,
            offset,
            quote! {
                let mut __word=__tid;
                while __word<#words {
                    *__scratch.add(#offset+__word)=*__descriptor.add(__word);
                    __word+=256;
                }
            },
            quote! {
                #row=__chunk as u64 * #items as u64;
                #end=(__chunk as u64+1) * #items as u64;
                #external=1;
                __resume_pc=#guard;
            },
        ));
        let publish = self.source.frontier && facts.expands.contains(&name);
        let threshold = self.source.frontier_threshold as u64 * 1024;
        let sum = (items > 1).then(|| self.register());
        let prepare_sum = sum
            .as_ref()
            .map(|sum| quote!(let mut __expansion_sum:u64=if __item==0 {0u64}else{#sum};));
        let save_sum = sum.as_ref().map(|sum| quote!(#sum=__expansion_sum;));
        let scan = self
            .source
            .fill_expansion(quote!(__end - __begin), rows, items);
        let header_run = run_loop
            .as_ref()
            .map(|(cursor, _, _, _)| quote!(#cursor as u32))
            .unwrap_or_else(|| quote!(0));
        let body = quote! {
            #stage_root_identity
            let __item=#item;
            let __input_slot=__item*256+__tid;
            let __descriptor=__scratch.add(#offset);
            if __tid==0 && __item==0 {*__descriptor=#stage;*__descriptor.add(1)=#header_run;*__descriptor.add(#total_slot+1)=0;}
            *__descriptor.add(2+__input_slot)=__begin;
            #(#save)*
            #prepare_sum
            #scan
            #save_sum
            if __item+1<#items { __resume_pc=#next; } else {
            ::cuda_device::thread::sync_threads();
            let __chunks=(__total as u64).div_ceil(#chunk_rows as u64) as u32;
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
            #end=__chunks as u64 * #items as u64;
            #external=0;
            __resume_pc=if __published {#next}else{#guard};
            }
        };
        if let Some((cursor, valid, run_guard, prepare)) = run_loop {
            let entry = self.block(quote! {
                #lookup
                if ::cq_gpu_runtime::leveled_any(__begin<__end) {#body}
                else {__resume_pc=#next;}
            });
            self.program.blocks[run_guard as usize] = quote! {
                __valid=#valid!=0;
                __resume_pc=if #cursor<::cq_gpu_runtime::leveled_count(#snapshot) as u64 {#entry}else{#caller_next};
            };
            Ok(self.block(quote! {
                #root_guard
                #prepare
                #valid=__valid as u64;
                #cursor=0;
                __resume_pc=#run_guard;
            }))
        } else {
            Ok(self.block(quote! {#root_guard #lookup #body}))
        }
    }
}
struct Continuation {
    name: Ident,
    stage: u32,
    body: Tokens,
}
// Pack the two per-lane counts into one scan word. At most 1024 candidates
// reach a chunk, so 11 low bits hold every unique offset without carrying into
// the candidate upper-bound count. Neither admission nor BUSY retry is a block
// scan or a resumable query step.
fn tile_candidate(values: &[Tokens], fused_retry: bool) -> Tokens {
    let admission = fused_retry.then(|| quote! {
        __tile_candidates+=__valid as u32;
        let __fused_participants=::cuda_device::warp::ballot_sync(u32::MAX,__valid && !__fused_full);
        if __valid && !__fused_full {
            let __admission=::cq_gpu_runtime::fused_retry_insert(__fused,[#(#values),*],__fused_participants);
            __fused_full=__admission==2;
            __valid=__admission==1;
        } else {
            __valid=false;
        }
    });
    let stores = values
        .iter()
        .enumerate()
        .map(|(column, value)| quote!(__tile_values[__item][#column]=#value;));
    quote! {
        #admission
        __tile_hits[__item]=__valid;
        if __valid { #(#stores)* }
        __tile_count+=__valid as u32;
    }
}

fn tile_output(columns: usize, items: usize, fused_retry: bool) -> (Tokens, Tokens) {
    let writes = (0..columns).map(|column| quote! {
        ::cq_gpu_runtime::write_output(__output,#column,__destination,__tile_values[__tile_item][#column]);
    });
    let counters = fused_retry.then(|| {
        quote! {
            let mut __tile_candidates=0u32;
            let mut __fused_full=::cq_gpu_runtime::fused_retry_full(__fused);
        }
    });
    let count = if fused_retry {
        quote!(__tile_count | (__tile_candidates << 11))
    } else {
        quote!(__tile_count)
    };
    let unpack = fused_retry.then(|| {
        quote! {
            let __offset=__offset & 2047u32;
            let __candidates=__total >> 11;
            let __total=__total & 2047u32;
        }
    });
    let count_candidates = fused_retry.then(|| quote! {
        if __candidates!=0 { ::cq_gpu_runtime::fused_retry_count(__output,__fused,__candidates as u64); }
    });
    (
        quote! {
            let mut __tile_hits=[false;#items];
            let mut __tile_values=[[0i64;#columns];#items];
            let mut __tile_count=0u32;
            #counters
        },
        quote! {
            let (__offset,__total)=::cq_gpu_runtime::tiles::scan(#count,__scratch);
            #unpack
            if __tid==0 {
                #count_candidates
                *(__scratch.add(8) as *mut u64)=if __total!=0 {::cq_gpu_runtime::reserve(__output,__total as u64)}else{0};
            }
            ::cuda_device::thread::sync_threads();
            let mut __destination=*(__scratch.add(8) as *const u64)+__offset as u64;
            let mut __tile_item=0usize;
            while __tile_item<#items {
                if __tile_hits[__tile_item] {
                    #(#writes)*
                    __destination+=1;
                }
                __tile_item+=1;
            }
            ::cuda_device::thread::sync_threads();
        },
    )
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
    items: usize,
    fused_retry: bool,
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
        let batch = if self.items > 1 {
            256 * self.items
        } else {
            self.chain
                .iter()
                .filter_map(|n| self.facts.stages.get(n))
                .copied()
                .min()
                .unwrap_or(1024)
        };
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
                root_descriptor_words: 0,
            },
            dispatch: Vec::new(),
        };
        let tile = emitter.register();
        let stripe = emitter.register();
        let mut state = State::default();
        if emitter.source.items > 1 {
            state.item = Some(quote!((#stripe / 256) as usize));
        }
        let mut save = Vec::new();
        state.driver_columns = initial.driver_columns.clone();
        if let Some(row) = initial
            .driver_row
            .as_ref()
            .filter(|_| emitter.source.items > 1)
        {
            let register = emitter.register();
            save.push(quote!(#register=#row;));
            state.driver_row = Some(quote!(#register));
        }
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
        if frontier && emitter.program.root_descriptor_words == emitter.program.descriptor_words {
            let tail = emitter
                .dispatch
                .iter()
                .map(|(_, offset, _, _)| *offset)
                .max()
                .unwrap_or(16);
            emitter.program.shared_words = emitter
                .program
                .shared_words
                .max(tail + emitter.program.descriptor_words);
        }
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
            let reuse = emitter.program.root_descriptor_words == emitter.program.descriptor_words;
            let claim_offset = if reuse {
                quote!(16usize)
            } else {
                quote!(__resume_shared_words)
            };
            let dispatch = emitter
                .dispatch
                .into_iter()
                .map(|(stage, offset, copy, body)| {
                    let copy = (!reuse || offset != 16).then_some(copy);
                    quote!(#stage=>{#copy #body})
                });
            emitter.program.blocks[1] = quote! {
                if __tid==0 {*(__scratch.add(8) as *mut u64)=::cq_gpu_runtime::frontier_next_root(__frontier);}
                ::cuda_device::thread::sync_threads();
                #tile=*(__scratch.add(8) as *const u64);
                if #tile<__driver.len.div_ceil(#batch as u64) {
                    #stripe=0;
                    __resume_pc=4;
                } else {
                    let __descriptor=__scratch.add(#claim_offset);
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
            if self.items > 1 {
                return Ok(tile_candidate(&values, self.fused_retry));
            }
            if self.fused_retry {
                let (buffers, output) = tile_output(values.len(), 1, true);
                let candidate = tile_candidate(&values, true);
                return Ok(quote! { #buffers let __item=0usize; #candidate #output });
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
            Operator::Bind(bind) => {
                let (column, value) = bind_value(
                    bind,
                    &self.facts.physical.rows[&symbol_name(&bind.input)].types,
                    &state.columns,
                    self.constants,
                )?;
                state.bind_driver(&column, &bind.expression);
                state.columns.insert(column, value);
                state
                    .columns
                    .retain(|column, _| self.context.carried_columns[name].contains(column));
                self.steps(index + 1, state)
            }
            Operator::Project(_) => {
                state
                    .columns
                    .retain(|c, _| self.context.carried_columns[name].contains(c));
                self.steps(index + 1, state)
            }
            Operator::Semi(_, k) | Operator::Anti(_, k)
                if membership(self.facts, &symbol_name(&k.right)) =>
            {
                self.member(index, state, k, false)
            }
            Operator::Probe(_, k) if membership(self.facts, &symbol_name(&k.right)) => {
                self.member(index, state, k, true)
            }
            Operator::Probe(_, k) if boolean_probe(self.facts, self.endpoint, k) => {
                let found = if let Some(witness) = self.leveled_witness(index, k, state)? {
                    let meta = witness.meta;
                    quote!((#meta) as u32!=0)
                } else {
                    self.membership_lookup(k, state)?
                };
                state
                    .columns
                    .retain(|column, _| self.context.carried_columns[name].contains(column));
                let rest = self.steps(index + 1, state)?;
                Ok(quote! { __valid=__valid && (#found); #rest })
            }
            Operator::Semi(_, k) | Operator::Anti(_, k) => {
                let negative = matches!(definition.operator, Operator::Anti(..));
                if leveled_provider(
                    self.facts,
                    &symbol_name(&k.right),
                    read_provider(self.facts, k),
                ) {
                    let mut save = Tokens::new();
                    let found = if self.facts.witnesses.contains_key(name) {
                        let snapshot = kernel_run_id(
                            self.facts,
                            &symbol_name(&k.right),
                            read_provider(self.facts, k),
                        );
                        let run = provider_id(
                            "run_view",
                            &symbol_name(&k.right),
                            read_provider(self.facts, k),
                        );
                        let lookup = self.lookup(k, state)?;
                        let search = leveled_ranges(&snapshot, &run, lookup);
                        let ranges = id("witness_ranges", name);
                        let meta = id("witness_meta", name);
                        save = quote! {
                            #search
                            let #ranges=__leveled_ranges;
                            let #meta=__leveled_len as u64 | ((::cq_gpu_runtime::leveled_count(#snapshot) as u64)<<32);
                        };
                        state.leveled_witnesses.insert(
                            name.clone(),
                            LeveledWitness {
                                ranges: quote!(#ranges),
                                meta: quote!(#meta),
                            },
                        );
                        quote!((#meta) as u32!=0)
                    } else {
                        self.membership_lookup(k, state)?
                    };
                    state
                        .columns
                        .retain(|column, _| self.context.carried_columns[name].contains(column));
                    let rest = self.steps(index + 1, state)?;
                    return Ok(quote! {#save __valid=__valid && ((#found)!=#negative);#rest});
                }
                let lookup = self.lookup(k, state)?;
                let begin = id("begin", name);
                let end = id("end", name);
                if self.facts.witnesses.contains_key(name) {
                    state
                        .witnesses
                        .insert(name.clone(), (quote!(#begin), quote!(#end)));
                }
                let contains = self.contains(k, state, quote!(#begin), quote!(#end))?;
                state
                    .columns
                    .retain(|column, _| self.context.carried_columns[name].contains(column));
                let rest = self.steps(index + 1, state)?;
                Ok(quote! {
                    let (#begin,#end)=if __valid {#lookup}else{(0,0)};
                    #contains
                    __valid=__valid && (__found != #negative);
                    #rest
                })
            }
            Operator::Probe(_, k) if self.serial || self.lane_local_probe(name) => {
                self.serial_expand(index, state, k)
            }
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
                let value = self.constants.term(
                    &binding.value,
                    &state.columns,
                    self.facts.physical.rows[&right].types.get(column),
                )?;
                state.columns.insert(column.clone(), value);
                state.driver_columns.remove(column);
            }
        }
        state
            .columns
            .retain(|column, _| self.context.carried_columns[&self.chain[index]].contains(column));
        let rest = self.steps(index + 1, state)?;
        let negative = matches!(
            self.context.physical[&self.chain[index]].operator,
            Operator::Anti(..)
        );
        Ok(
            quote! {__valid=__valid && (::cq_gpu_runtime::membership_contains(#view,#key) != #negative);#rest},
        )
    }
    fn lane_local_probe(&self, name: &str) -> bool {
        !self.facts.expands.contains(name)
            && self
                .chain
                .iter()
                .any(|name| self.facts.expands.contains(name))
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
        let cooperative = !self.serial;
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let run = provider_id("run_view", &right, provider);
        let begin = id("begin", name);
        let end = id("end", name);
        let row = id("row", name);
        let pair = self.smallest(index)?;
        let witness = self.leveled_witness(index, k, state)?;
        let lookup = if let Some(witness) = witness {
            witness.point()
        } else {
            self.probe_lookup(index, k, pair.as_ref(), state)?
        };
        if let Some(pair) = pair {
            let choose = self.smallest_lookup(&pair, state, lookup)?;
            let local = id("value", &pair.column);
            let value = self.smallest_value(&pair, quote!(#row));
            let contains = self.smallest_contains(&pair, quote!(#local));
            state.columns.insert(pair.column.clone(), quote!(#local));
            state.driver_columns.remove(&pair.column);
            state
                .columns
                .retain(|column, _| self.context.carried_columns[&pair.second].contains(column));
            let previous_items = self.items;
            if cooperative {
                self.items = 1;
            }
            let rest = self.steps(index + 2, state)?;
            self.items = previous_items;
            return Ok(if cooperative {
                quote! {
                    let (#begin,#end,__smallest_right,__smallest_other_begin,__smallest_other_end) =
                        if __valid {#choose}else{(0,0,false,0,0)};
                    let mut #row=#begin;
                    loop {
                        let (_,__remaining)=::cq_gpu_runtime::tiles::scan((#row<#end) as u32,__scratch);
                        if __remaining==0 {break;}
                        let mut __valid=#row<#end;
                        let #local=if __valid {#value}else{0};
                        __valid=__valid && (#contains);
                        #rest
                        #row=#row.saturating_add(1);
                    }
                }
            } else {
                quote! {
                    if __valid {
                        let (#begin,#end,__smallest_right,__smallest_other_begin,__smallest_other_end) = #choose;
                        let mut #row = #begin;
                        while #row < #end {
                            let #local = #value;
                            let mut __valid = #contains;
                            #rest
                            #row += 1;
                        }
                    }
                }
            });
        }
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
                if cooperative {
                    quote!(::cq_gpu_runtime::run_column(#run,#slot,#row))
                } else {
                    let payload = id("payload", &format!("{name}_{column}"));
                    payloads.push(quote!(let #payload=#run.columns.add(#slot).read();));
                    quote!(#payload.add(#row as usize).read())
                }
            } else if let Some(constant) = self.facts.filtered.get(&right).and_then(|f| {
                f.constants
                    .iter()
                    .find(|b| symbol_name(&b.column) == *column)
            }) {
                self.constants.term(
                    &constant.value,
                    &state.columns,
                    self.facts.physical.rows[&right].types.get(column),
                )?
            } else {
                return fail(
                    k,
                    "CQ_GPU_PAYLOAD_COLUMN",
                    format!("recorded run {right} does not carry {column}"),
                );
            };
            loads.push(quote!(let #local=if __valid {#value}else{0};));
            state.columns.insert(column.clone(), quote!(#local));
            state.driver_columns.remove(column);
        }
        let residual = self.residual(k, state, quote!(#row))?;
        state
            .columns
            .retain(|c, _| self.context.carried_columns[name].contains(c));
        let previous_items = self.items;
        if cooperative {
            self.items = 1;
        }
        let rest = self.steps(index + 1, state)?;
        self.items = previous_items;
        let body = if cooperative {
            quote! {
                let (#begin,#end)=if __valid {#lookup}else{(0,0)};
                let mut #row=#begin;
                loop {
                    let (_,__remaining)=::cq_gpu_runtime::tiles::scan((#row<#end) as u32,__scratch);
                    if __remaining==0 {break;}
                    let mut __valid=#row<#end;
                    #(#loads)*
                    __valid=__valid && (#residual);
                    #rest
                    #row=#row.saturating_add(1);
                }
            }
        } else {
            quote! {
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
            }
        };
        if leveled_provider(self.facts, &right, provider) {
            let snapshot = kernel_run_id(self.facts, &right, provider);
            Ok(quote! {
                let mut __leveled_run_id=0usize;
                while __leveled_run_id<::cq_gpu_runtime::leveled_count(#snapshot) as usize {
                    let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
                    #body
                    __leveled_run_id+=1;
                }
            })
        } else {
            Ok(body)
        }
    }
    /// Keep root identity rather than duplicate driver values. Exact range keys
    /// are reconstructed from the selected run row; only residual keys retain
    /// their original binding. Smallest adds its packed parent/side half-word.
    fn compact_root(
        &self,
        name: &str,
        key: &physical::Keyed,
        pair: Option<&Smallest>,
        state: &mut State,
        rows: usize,
        words: &mut usize,
    ) -> Option<(Tokens, Tokens, Tokens)> {
        if self.serial {
            return None;
        }
        let after = pair.map_or(name, |pair| pair.second.as_str());
        let mut needed: BTreeSet<_> = self.context.carried_columns[after]
            .iter()
            .cloned()
            .collect();
        if let Some(pair) = pair {
            for (_, _, prefix) in &pair.sides {
                needed.extend(prefix.iter().cloned());
            }
        } else {
            needed.extend(
                key.columns
                    .iter()
                    .map(symbol_name)
                    .filter(|column| !exact_run_key(self.facts, key, column)),
            );
        }
        state.columns.retain(|column, _| {
            needed.contains(column) && (pair.is_some() || !exact_run_key(self.facts, key, column))
        });
        state
            .witnesses
            .retain(|origin, _| self.context.carried_witnesses[after].contains(origin));
        state
            .leveled_witnesses
            .retain(|origin, _| self.context.carried_witnesses[after].contains(origin));
        let row = state.driver_row.clone()?;
        let roots = *words;
        *words += rows;
        let (parent_save, parent_restore, parent) = if pair.is_some() {
            let parents = *words;
            *words += rows.div_ceil(2);
            (
                quote! {
                    *(__descriptor.add(#parents) as *mut u16).add(__input_slot)=
                        __input_slot as u16 | ((__smallest_right as u16) << 15);
                },
                quote! {
                    let __root_parent=if __valid {
                        *(__descriptor.add(#parents) as *const u16).add(__parent)
                    }else{0};
                    let __smallest_right=__root_parent & 0x8000u16 != 0;
                },
                quote!((__root_parent & 0x7fffu16) as usize),
            )
        } else {
            (Tokens::new(), Tokens::new(), quote!(__parent))
        };
        Some((
            quote! {
                let __driver_row_id=#row;
                if __valid && __driver_row_id > u32::MAX as u64 {
                    ::cq_gpu_runtime::flag_error(__output,2);
                    __valid=false;
                }
            },
            quote! {
                *__descriptor.add(#roots+__input_slot)=if __valid {__driver_row_id as u32}else{0};
                #parent_save
            },
            quote! {
                #parent_restore
                let __driver_row=if __valid {*__descriptor.add(#roots+#parent) as u64}else{0};
            },
        ))
    }

    fn smallest_other_range(&self, pair: &Smallest, state: &State) -> syn::Result<Tokens> {
        let (left, lp, lk) = &pair.sides[0];
        let (right, rp, rk) = &pair.sides[1];
        let left = self.lookup_columns(left, *lp, lk, state)?;
        let right = self.lookup_columns(right, *rp, rk, state)?;
        Ok(quote! {
            let (__smallest_other_begin,__smallest_other_end)=if __valid {
                if __smallest_right {#left}else{#right}
            }else{(0,0)};
        })
    }

    fn fill_expansion(&self, length: Tokens, rows: usize, items: usize) -> Tokens {
        let offsets = 2 + rows;
        let total_slot = 2 + 2 * rows;
        if items == 1 {
            return quote! {
                let (__offset,mut __total)=::cq_gpu_runtime::tiles::scan(#length,__scratch);
                let __wrapped=__offset.checked_add(#length).is_none();
                let (_,__overflows)=::cq_gpu_runtime::tiles::scan(__wrapped as u32,__scratch);
                if __overflows!=0 {
                    if __tid==0 {::cq_gpu_runtime::flag_error(__output,2);}
                    __total=0;
                }
                *__descriptor.add(#offsets+__input_slot)=__offset;
                if __tid==0 {*__descriptor.add(#total_slot)=__total;}
            };
        }
        quote! {
            let mut __count=#length;
            if __count > u32::MAX / 256 {
                ::cq_gpu_runtime::flag_error(__output,2);
                __count=0;
            }
            let (__offset,__stripe_total)=::cq_gpu_runtime::tiles::scan(__count,__scratch);
            *__descriptor.add(#offsets+__input_slot)=(__expansion_sum+__offset as u64) as u32;
            __expansion_sum+=__stripe_total as u64;
            let mut __total=0u32;
            if __item+1==#items {
                if __expansion_sum > u32::MAX as u64 {
                    if __tid==0 {::cq_gpu_runtime::flag_error(__output,2);}
                } else {
                    __total=__expansion_sum as u32;
                }
                if __tid==0 {*__descriptor.add(#total_slot)=__total;}
            }
        }
    }

    fn tile_output(&self) -> (Tokens, Tokens) {
        tile_output(
            self.context.carried_columns[self.endpoint].len(),
            self.items,
            self.fused_retry,
        )
    }

    fn probe_lookup(
        &self,
        index: usize,
        k: &physical::Keyed,
        pair: Option<&Smallest>,
        state: &mut State,
    ) -> syn::Result<Tokens> {
        let name = symbol_name(&k.right);
        if leveled_provider(self.facts, &name, read_provider(self.facts, k)) {
            return self.lookup(k, state);
        }
        let mut keys: Vec<_> = k.columns.iter().map(symbol_name).collect();
        keys.sort();
        if let Some(witness) = self.witness_lookup(&self.chain[index], &name, &keys, state)? {
            return Ok(witness);
        }
        if let Some(pair) = pair {
            let (name, provider, keys) = &pair.sides[0];
            self.lookup_columns(name, *provider, keys, state)
        } else {
            self.lookup(k, state)
        }
    }

    fn leveled_witness(
        &self,
        index: usize,
        key: &physical::Keyed,
        state: &mut State,
    ) -> syn::Result<Option<LeveledWitness>> {
        let right = symbol_name(&key.right);
        if !leveled_provider(self.facts, &right, read_provider(self.facts, key)) {
            return Ok(None);
        }
        let mut keys = key.columns.iter().map(symbol_name).collect::<Vec<_>>();
        keys.sort();
        let origin = self.facts.witnesses.keys().find(|origin| {
            let share = &self.facts.shares[*origin];
            share.force == self.chain[index] && share.build == right && share.keys == keys
        });
        origin
            .map(|origin| {
                state.leveled_witnesses.remove(origin).ok_or_else(|| {
                    syn::Error::new_spanned(
                        key,
                        "CQ_GPU_WITNESS: forcing read lost its recorded leveled range set",
                    )
                })
            })
            .transpose()
    }

    fn witness_lookup(
        &self,
        force: &str,
        build: &str,
        keys: &[String],
        state: &mut State,
    ) -> syn::Result<Option<Tokens>> {
        let origin = self.facts.witnesses.keys().find(|origin| {
            let share = &self.facts.shares[*origin];
            share.force == force && share.build == build && share.keys == keys
        });
        let Some(origin) = origin else {
            return Ok(None);
        };
        let (begin, end) = state.witnesses.remove(origin).ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "CQ_GPU_WITNESS: forcing read lost its recorded witness",
            )
        })?;
        Ok(Some(quote!((#begin,#end))))
    }

    fn smallest(&self, index: usize) -> syn::Result<Option<Smallest>> {
        smallest_pair(
            self.facts,
            self.context.physical[&self.chain[index]],
            self.chain
                .get(index + 1)
                .map(|name| self.context.physical[name]),
        )
    }

    fn smallest_lookup(
        &self,
        pair: &Smallest,
        state: &mut State,
        first: Tokens,
    ) -> syn::Result<Tokens> {
        let (name, provider, prefix) = &pair.sides[1];
        let second =
            if let Some(witness) = self.witness_lookup(&pair.reader, name, prefix, state)? {
                witness
            } else {
                self.lookup_columns(name, *provider, prefix, state)?
            };
        Ok(quote!({
            let (__smallest_a_begin, __smallest_a_end) = #first;
            let (__smallest_b_begin, __smallest_b_end) = #second;
            let __smallest_right = __smallest_b_end - __smallest_b_begin < __smallest_a_end - __smallest_a_begin;
            if __smallest_right {
                (__smallest_b_begin, __smallest_b_end, true, __smallest_a_begin, __smallest_a_end)
            } else {
                (__smallest_a_begin, __smallest_a_end, false, __smallest_b_begin, __smallest_b_end)
            }
        }))
    }

    fn smallest_columns(&self, pair: &Smallest) -> [Tokens; 2] {
        pair.sides.each_ref().map(|(name, provider, _)| {
            let view = provider_id("run_view", name, *provider);
            let slot = self.context.carried_columns[name]
                .iter()
                .position(|c| c == &pair.column)
                .expect("checked retained smallest candidate column");
            quote!(#view.columns.add(#slot).read())
        })
    }

    fn smallest_value(&self, pair: &Smallest, row: Tokens) -> Tokens {
        let [left, right] = self.smallest_columns(pair);
        quote!({
            let __smallest_values = if __smallest_right { #right } else { #left };
            __smallest_values.add(#row as usize).read()
        })
    }

    fn smallest_contains(&self, pair: &Smallest, value: Tokens) -> Tokens {
        let [left, right] = self.smallest_columns(pair);
        quote!({
            let __smallest_other = if __smallest_right { #left } else { #right };
            let __contains_key = #value;
            let mut __contains_lo = __smallest_other_begin;
            let mut __contains_hi = __smallest_other_end;
            while __contains_lo < __contains_hi {
                let __contains_mid = __contains_lo + (__contains_hi - __contains_lo) / 2;
                if __smallest_other.add(__contains_mid as usize).read() < __contains_key {
                    __contains_lo = __contains_mid + 1;
                } else {
                    __contains_hi = __contains_mid;
                }
            }
            __contains_lo < __smallest_other_end
                && __smallest_other.add(__contains_lo as usize).read() == __contains_key
        })
    }

    /// The rank directory resolves level one. Lexicographic construction makes
    /// each subsequent recorded column sorted inside the preceding prefix.
    fn lookup(&self, k: &physical::Keyed, state: &State) -> syn::Result<Tokens> {
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let keys: Vec<_> = k.columns.iter().map(symbol_name).collect();
        self.lookup_columns(&right, provider, &keys, state)
    }

    fn lookup_columns(
        &self,
        right: &str,
        provider: usize,
        keys: &[String],
        state: &State,
    ) -> syn::Result<Tokens> {
        let view = provider_id("run_view", right, provider);
        let key = lookup_key(self.facts, right, provider, &state.columns);
        let point = quote!(::cq_gpu_runtime::blocked_run::point(#view.rank,#key).unwrap_or((0,0)));
        if !prefix_run(self.facts, right, provider) || keys.len() == 1 {
            return Ok(point);
        }
        let mut searches = Vec::new();
        for column in self.facts.built[right][provider].0.iter().flatten().skip(1) {
            if !keys.contains(column) {
                break;
            }
            let slot = self.context.carried_columns[right]
                .iter()
                .position(|c| c == column)
                .ok_or_else(|| {
                    syn::Error::new(
                        Span::call_site(),
                        "CQ_GPU_PREFIX_KEY: missing retained key column",
                    )
                })?;
            let value = &state.columns[column];
            searches.push(quote! {
                let __prefix_key = #value;
                let mut __lo = __prefix_begin;
                let mut __hi = __prefix_end;
                while __lo < __hi {
                    let __mid = __lo + (__hi - __lo) / 2;
                    if ::cq_gpu_runtime::run_column(#view,#slot,__mid) < __prefix_key {
                        __lo = __mid + 1;
                    } else {
                        __hi = __mid;
                    }
                }
                __prefix_begin = __lo;
                __hi = __prefix_end;
                while __lo < __hi {
                    let __mid = __lo + (__hi - __lo) / 2;
                    if ::cq_gpu_runtime::run_column(#view,#slot,__mid) <= __prefix_key {
                        __lo = __mid + 1;
                    } else {
                        __hi = __mid;
                    }
                }
                __prefix_end = __lo;
            });
        }
        Ok(quote!({
            let (mut __prefix_begin, mut __prefix_end) = #point;
            #(#searches)*
            (__prefix_begin, __prefix_end)
        }))
    }
    fn membership_lookup(&self, key: &physical::Keyed, state: &State) -> syn::Result<Tokens> {
        let right = symbol_name(&key.right);
        let provider = read_provider(self.facts, key);
        let found = self.single_membership_lookup(key, state)?;
        if !leveled_provider(self.facts, &right, provider) {
            return Ok(found);
        }
        let snapshot = kernel_run_id(self.facts, &right, provider);
        let run = provider_id("run_view", &right, provider);
        Ok(quote!({
            let mut __leveled_found=false;
            let mut __leveled_run_id=0usize;
            while __leveled_run_id<::cq_gpu_runtime::leveled_count(#snapshot) as usize && !__leveled_found {
                let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
                __leveled_found=#found;
                __leveled_run_id+=1;
            }
            __leveled_found
        }))
    }
    fn single_membership_lookup(
        &self,
        key: &physical::Keyed,
        state: &State,
    ) -> syn::Result<Tokens> {
        let right = symbol_name(&key.right);
        let provider = read_provider(self.facts, key);
        if prefix_run(self.facts, &right, provider) && key.columns.len() > 1 {
            let mut columns: Vec<_> = self.facts.built[&right][provider]
                .0
                .iter()
                .flatten()
                .filter(|column| key.columns.iter().any(|key| symbol_name(key) == **column))
                .cloned()
                .collect();
            let last = columns.pop().expect("validated prefix key");
            let lookup = self.lookup_columns(&right, provider, &columns, state)?;
            let view = provider_id("run_view", &right, provider);
            let slot = self.context.carried_columns[&right]
                .iter()
                .position(|column| *column == last)
                .expect("retained prefix key");
            let value = &state.columns[&last];
            return Ok(quote!({
                let (__begin,__end)=#lookup;
                let __contains_key=#value;
                let mut __contains_lo=__begin;
                let mut __contains_hi=__end;
                while __contains_lo<__contains_hi {
                    let __mid=__contains_lo+(__contains_hi-__contains_lo)/2;
                    if ::cq_gpu_runtime::run_column(#view,#slot,__mid)<__contains_key {
                        __contains_lo=__mid+1;
                    } else { __contains_hi=__mid; }
                }
                __contains_lo<__end && ::cq_gpu_runtime::run_column(#view,#slot,__contains_lo)==__contains_key
            }));
        }
        let lookup = self.lookup(key, state)?;
        let contains = self.contains(key, state, quote!(__begin), quote!(__end))?;
        Ok(quote!({ let (__begin,__end)=#lookup; #contains __found }))
    }

    fn contains(
        &self,
        k: &physical::Keyed,
        state: &State,
        begin: Tokens,
        end: Tokens,
    ) -> syn::Result<Tokens> {
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        if !k
            .columns
            .iter()
            .any(|key| residual_key(self.facts, &right, provider, &symbol_name(key)))
        {
            return Ok(quote!(let __found = #begin < #end;));
        }
        let residual = self.residual(k, state, quote!(__candidate))?;
        Ok(quote! {
            let mut __candidate = #begin;
            let mut __found = false;
            while __candidate < #end && !__found {
                __found = #residual;
                __candidate += 1;
            }
        })
    }
    fn residual(&self, k: &physical::Keyed, state: &State, row: Tokens) -> syn::Result<Tokens> {
        let right = symbol_name(&k.right);
        let provider = read_provider(self.facts, k);
        let view = provider_id("run_view", &right, provider);
        let mut tests = Vec::new();
        for key in k
            .columns
            .iter()
            .map(symbol_name)
            .filter(|key| residual_key(self.facts, &right, provider, key))
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
        let pair = self.smallest(index)?;
        let witness = self.leveled_witness(index, k, state)?;
        let first = self.probe_lookup(index, k, pair.as_ref(), state)?;
        let leveled = leveled_provider(self.facts, &right, provider);
        let snapshot = kernel_run_id(self.facts, &right, provider);
        let lookup = if leveled {
            if let Some(witness) = &witness {
                let range = witness.point();
                quote! {let (#begin,#end)=if __valid {#range}else{(0u32,0u32)};}
            } else {
                quote! {
                    let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
                    let (#begin,#end)=if __valid {#first}else{(0u32,0u32)};
                }
            }
        } else if let Some(pair) = &pair {
            let choose = self.smallest_lookup(pair, state, first)?;
            quote! {
                let (#begin,#end,__smallest_right,__smallest_other_begin,__smallest_other_end) =
                    if __valid {#choose}else{(0,0,false,0,0)};
            }
        } else {
            quote!(let (#begin,#end)=if __valid {#first}else{(0,0)};)
        };
        let offset = (self.shared_words + 1) & !1;
        // A leveled range set is streamed one run and input stripe at a time;
        // the recorded root tile still visits every one of its original items.
        let items = if leveled { 1 } else { self.items };
        let rows = 256 * items;
        let offsets = 2 + rows;
        let total_slot = 2 + 2 * rows;
        let mut words = total_slot + 2;
        let mut save = Vec::new();
        let mut restore = Vec::new();
        if leveled {
            restore.push(quote! {
                let __leveled_run_id=*__descriptor.add(1) as usize;
                let #run=::cq_gpu_runtime::leveled_run(#snapshot,__leveled_run_id);
            });
        }
        let mut child = State::default();
        let compact = self.compact_root(name, k, pair.as_ref(), state, rows, &mut words);
        let root_guard = compact.as_ref().map(|(guard, _, _)| guard);
        if let Some((_, root_save, root_restore)) = &compact {
            save.push(root_save.clone());
            restore.push(root_restore.clone());
            child.driver_row = Some(quote!(__driver_row));
            child.driver_columns = state.driver_columns.clone();
        }
        if pair.is_some() && compact.is_none() {
            let slot = words;
            words += 3 * rows;
            save.push(quote! {
                *__descriptor.add(#slot+__input_slot)=__smallest_right as u32;
                *__descriptor.add(#slot+#rows+__input_slot)=__smallest_other_begin;
                *__descriptor.add(#slot+2*#rows+__input_slot)=__smallest_other_end;
            });
            restore.push(quote! {
                let __smallest_right=__valid && *__descriptor.add(#slot+__parent)!=0;
                let __smallest_other_begin=if __valid {*__descriptor.add(#slot+#rows+__parent)}else{0};
                let __smallest_other_end=if __valid {*__descriptor.add(#slot+2*#rows+__parent)}else{0};
            });
        }
        for (column, value) in &state.columns {
            let local = id("value", column);
            if let Some(slot) = state
                .driver_columns
                .get(column)
                .filter(|_| compact.is_some())
            {
                restore.push(quote! {
                    let #local=if __valid {::cq_gpu_runtime::column(__driver,#slot,__driver_row)}else{0};
                });
            } else {
                let slot = words;
                words += 2 * rows;
                save.push(
                    quote! {*(__descriptor.add(#slot) as *mut i64).add(__input_slot)=#value;},
                );
                restore.push(quote!{let #local=if __valid {*(__descriptor.add(#slot) as *const i64).add(__parent)}else{0};});
            }
            child.columns.insert(column.clone(), quote!(#local));
        }
        for (origin, (a, b)) in &state.witnesses {
            let slot = words;
            words += 2 * rows;
            let begin = id("begin", origin);
            let end = id("end", origin);
            save.push(
                quote! {*__descriptor.add(#slot+__input_slot)=#a;*__descriptor.add(#slot+#rows+__input_slot)=#b;},
            );
            restore.push(quote!{let #begin=if __valid {*__descriptor.add(#slot+__parent)}else{0};let #end=if __valid {*__descriptor.add(#slot+#rows+__parent)}else{0};});
            child
                .witnesses
                .insert(origin.clone(), (quote!(#begin), quote!(#end)));
        }
        for (origin, witness) in &state.leveled_witnesses {
            let ranges = id("witness_ranges", origin);
            let meta = id("witness_meta", origin);
            let target = LeveledWitness {
                ranges: quote!(#ranges),
                meta: quote!(#meta),
            };
            let (range_save, range_restore) = witness.stage(&target, rows, &mut words);
            save.push(range_save);
            restore.push(quote! {
                let mut #ranges=[0u64;::cq_gpu_runtime::MAX_LEVELS];
                let mut #meta:u64;
                #range_restore
            });
            child.leveled_witnesses.insert(origin.clone(), target);
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
            if let Some(pair) = &pair {
                if column == &pair.column {
                    let local = id("value", column);
                    let value = self.smallest_value(pair, quote!(__build_row));
                    loads.push(quote!(let #local=if __valid {#value}else{0};));
                    child.columns.insert(column.clone(), quote!(#local));
                    child.driver_columns.remove(column);
                }
                continue;
            }
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
                self.constants.term(
                    &constant.value,
                    &child.columns,
                    self.facts.physical.rows[&right].types.get(column),
                )?
            } else {
                return fail(
                    k,
                    "CQ_GPU_PAYLOAD_COLUMN",
                    format!("recorded run {right} does not carry {column}"),
                );
            };
            loads.push(quote!(let #local=if __valid {#value}else{0};));
            child.columns.insert(column.clone(), quote!(#local));
            if !exact_run_key(self.facts, k, column) {
                child.driver_columns.remove(column);
            }
        }
        let other_range = if let Some(pair) = pair.as_ref().filter(|_| compact.is_some()) {
            self.smallest_other_range(pair, &child)?
        } else {
            Tokens::new()
        };
        let (residual, after, next) = if let Some(pair) = &pair {
            (
                self.smallest_contains(pair, child.columns[&pair.column].clone()),
                &pair.second,
                index + 2,
            )
        } else {
            (
                self.residual(k, &child, quote!(__build_row))?,
                name,
                index + 1,
            )
        };
        child
            .columns
            .retain(|c, _| self.context.carried_columns[after].contains(c));
        let continuation_count = self.continuations.len();
        let parent_items = self.items;
        if leveled {
            self.items = 1;
        }
        let rest = self.steps(next, &mut child)?;
        self.items = parent_items;
        let batch = if items > 1 {
            rows as u32
        } else if leveled {
            self.batch.min(256) as u32
        } else {
            self.batch as u32
        };
        let continuation_body = quote! {
            let __position=__chunk as u64 * #batch as u64 + (__item*256+__tid) as u64;
            let __total=*__descriptor.add(#total_slot);
            let mut __valid=__tid<#batch as usize && __position<__total as u64;
            let __parent=if __valid {::cq_gpu_runtime::tiles::owner(__descriptor.add(#offsets),#rows as u32,__position as u32) as usize}else{0};
            let __build_row=if __valid {*__descriptor.add(2+__parent)+__position as u32-*__descriptor.add(#offsets+__parent)}else{0};
            #(#restore)* #(#loads)*
            #other_range
            __valid=__valid && (#residual);
            #rest
        };
        let continuation_body = if items > 1 {
            let terminal = self.continuations.len() == continuation_count;
            let (buffers, output) = if terminal {
                self.tile_output()
            } else {
                (quote!(let mut __expansion_sum=0u64;), Tokens::new())
            };
            quote! {
                #buffers
                let mut __item=0usize;
                while __item<#items {
                    #continuation_body
                    __item+=1;
                }
                #output
            }
        } else {
            quote! { let __item=0usize; #continuation_body }
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
        let scan = self.fill_expansion(quote!(#end-#begin), rows, items);
        // Wide consumers finish with their output barrier; publication also
        // finishes convergently. The scheduler supplies the root-retirement
        // barrier, so another barrier here would duplicate the manual shape.
        let retire_barrier = (items == 1).then(|| quote!(::cuda_device::thread::sync_threads();));
        let header_run = if leveled {
            quote!(__leveled_run_id as u32)
        } else {
            quote!(0)
        };
        let stage_lookup = if leveled {
            Tokens::new()
        } else {
            lookup.clone()
        };
        let body = quote! {
            #root_guard
            #stage_lookup
            let __input_slot=__item*256+__tid;
            let __descriptor=__scratch.add(#offset);
            if __tid==0 && __item==0 {*__descriptor=#stage;*__descriptor.add(1)=#header_run;*__descriptor.add(#total_slot+1)=0;}
            *__descriptor.add(2+__input_slot)=#begin;
            #(#save)*
            #scan
            if __item+1==#items {
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
            #retire_barrier
            }
        };
        if leveled {
            Ok(quote! {{
                let __item=0usize;
                let mut __leveled_run_id=0usize;
                while __leveled_run_id<::cq_gpu_runtime::leveled_count(#snapshot) as usize {
                    #lookup
                    if ::cq_gpu_runtime::leveled_any(#begin<#end) {
                        #body
                    }
                    __leveled_run_id+=1;
                }
            }})
        } else {
            Ok(body)
        }
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
    root_descriptor_words: usize,
    continuations: Vec<Continuation>,
    frontier: bool,
    batch: usize,
    items: usize,
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
        retry: bool,
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
            root_descriptor_words,
            continuations,
            frontier,
            batch,
            items,
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
        // A claim happens only with no live root frame. If the root frame is
        // the largest, it holds a stolen descriptor without overlapping any
        // later frame; resumable dispatch copies only to disjoint frames.
        let reuse_claim = resume
            .as_ref()
            .map_or(root_descriptor_words == descriptor_words, |program| {
                program.root_descriptor_words == program.descriptor_words
            });
        let claim_offset = if reuse_claim { 16usize } else { shared_words };
        let shared_bytes = ((shared_words
            + if frontier && !reuse_claim {
                descriptor_words + 2
            } else {
                0
            })
            * 4) as u32;
        let common =
            quote!(__driver,#(#root_columns,)*#(#run_ids,)*#(#constants,)*__output,__frontier);
        let signature = quote!(__driver: ::cq_gpu_runtime::TableView,#(#root_columns: ::cq_gpu_runtime::ColumnView,)*#(#run_ids: #run_types,)*#(#constants:i64,)*__output: ::cq_gpu_runtime::OutputView,__frontier: ::cq_gpu_runtime::FrontierView);
        let fused = facts.dedup.get(&endpoint).is_some_and(|ty| {
            dedup_policy(ty).expect("validated dedup policy") == DedupPolicy::Fused
        });
        let fused_retry = retry && fused;
        let fused_type = if fused_retry {
            quote!(::cq_gpu_runtime::FusedRetryView)
        } else {
            quote!(::cq_gpu_runtime::FusedSetView)
        };
        let fused_args = fused.then(|| quote!(__fused: #fused_type,));
        let continuation_extra = fused_retry.then(|| quote!(,__fused));
        let continuation_args = fused_retry.then(|| quote!(__fused: #fused_type,));
        let continuation_common = quote!(#common #continuation_extra);
        let fused_call = fused.then(|| quote!(__fused,));
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
        let body = substitute(body, &continuation_common);
        let functions=continuations.iter().filter(|_|resume.is_none()).map(|c| {
            let name=&c.name;let body=substitute(c.body.clone(),&continuation_common);
            quote!{#[device] #[inline(always)] unsafe fn #name(#signature,#continuation_args __descriptor:*mut u32,__chunk:u32) {
                let __tid=::cuda_device::thread::threadIdx_x() as usize;
                let __scratch=::cuda_device::DynamicSharedArray::<u8>::get_raw() as *mut u32;
                #body
            }}
        });
        let dispatch = continuations.iter().map(|c| {
            let stage = c.stage;
            let name = &c.name;
            quote!(#stage=>#name(#continuation_common,__descriptor,__chunk),)
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
            let __item=__stripe/256;
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
        let root_rows = if items > 1 {
            let (buffers, output) = if continuations.is_empty() {
                tile_output(context.carried_columns[&endpoint].len(), items, fused_retry)
            } else {
                (quote!(let mut __expansion_sum=0u64;), Tokens::new())
            };
            quote! { #buffers #root_rows #output }
        } else {
            root_rows
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
                        let __descriptor=__scratch.add(#claim_offset);
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
            pub fn #kernel(#signature,#resume_args #fused_args #selection_args) {
                let __tid=::cuda_device::thread::threadIdx_x() as usize;
                let __scratch=::cuda_device::DynamicSharedArray::<u8>::get_raw() as *mut u32;
                unsafe {#scheduler}
            }
        };
        let canon = facts.dedup.contains_key(&endpoint);
        let dedup = (canon && !fused).then(|| quote!(let #table=__execution.dedup(#table)?;));
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
                } else if hashed(facts, &endpoint, provider) {
                    let slots = build.0[0].iter().map(|key| {
                        context.carried_columns[&endpoint]
                            .iter()
                            .position(|c| c == key)
                            .expect("checked carried hashed run key")
                    });
                    quote!(let #run=__execution.prepare_hashed_run(#table,&[#(#slots),*])?;)
                } else if prefix_run(facts, &endpoint, provider) {
                    let slots = build.0.iter().flatten().map(|key| {
                        context.carried_columns[&endpoint]
                            .iter()
                            .position(|c| c == key)
                            .expect("checked carried prefix run key")
                    });
                    quote!(let #run=__execution.prepare_prefix_run(#table,&[#(#slots),*])?;)
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
        let resume_policy = facts
            .dedup
            .get(&endpoint)
            .map(|ty| dedup_policy(ty).expect("validated dedup policy"));
        let resume_method = match resume_policy {
            Some(DedupPolicy::Fused) => format_ident!("execute_resumable_fused"),
            Some(DedupPolicy::Compacting) => format_ident!("execute_resumable_compacting"),
            _ => format_ident!("execute_resumable"),
        };
        let initial_capacity = capacity
            .cloned()
            .unwrap_or_else(|| quote!(__driver.len as usize));
        let execute = if retry {
            let initial_capacity = capacity
                .cloned()
                .unwrap_or_else(|| quote!(__CQ_GPU_RETRY_DEFAULT_CAPACITY));
            let method = if fused {
                format_ident!("execute_retry_fused")
            } else {
                format_ident!("execute_retry")
            };
            let arguments = if fused {
                let hint = capacity
                    .map(|capacity| quote!(Some(#capacity)))
                    .unwrap_or_else(|| quote!(None));
                quote!(concat!(module_path!(),"::",stringify!(#kernel)),&[#(#bounds),*],#hint)
            } else {
                quote!(concat!(module_path!(),"::",stringify!(#kernel)),&[#(#bounds),*],#initial_capacity)
            };
            let extra = fused.then(|| quote!(,__fused));
            quote! {
                __execution.#method(#arguments,|__stream,__output #extra| {
                    __pool.reset(__driver.len.div_ceil(#batch as u64))?;
                    unsafe {__module.#kernel(__stream,&__launch,#common #extra)?;}
                    Ok(())
                })?
            }
        } else if let Some(program) = &resume {
            let reservation = (resume_policy == Some(DedupPolicy::Compacting)).then(|| {
                let maximum = if program.serial { 1usize } else { 256 * items };
                quote!(#maximum,)
            });
            quote! {
                __execution.#resume_method(
                    &[#(#bounds),*],#initial_capacity,__grid as usize * #checkpoint_words,
                    #reservation
                    |__stream,__output,__resume,#fused_call| {
                        unsafe {__module.#kernel(__stream,&__launch,#common,__resume,#fused_call)?;}
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
        let reset = (!retry).then(|| quote!(__pool.reset(__driver.len.div_ceil(#batch as u64))?;));
        let launch = quote! {
            let #table={
                let __driver=#driver;
                #(#root_bindings)*
                #(#run_bindings)*
                let __pool=__execution.frontier(#slots,#descriptor_words as u32)?;
                let __frontier=__pool.view();
                let __grid=__driver.len.div_ceil(#batch as u64).clamp(1,568) as u32;
                let __launch=__module.#prepare(::cuda_core::LaunchConfig1D::new(__grid,256,#shared_bytes))?;
                #reset
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
