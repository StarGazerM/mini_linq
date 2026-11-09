//! Checked storage refinement of Boolean incremental Datalog.
//! Records concrete Rust providers, per-layout maintenance, each relation's
//! canonical layout, occurrence bindings, and negation-key projections.
//! Applicable provider types and initializers are opaque Rust syntax whose
//! collection laws are premises. Delta and next share a move type.
//! A relation's canonical layout is an ordinary member of its full layouts whose
//! key covers every column; admission and EDB loading deduplicate against it,
//! and full scans read it. There is no separate seen, all, or canonical set.
//! Optional per-host certificates record canonical admission roots, complete
//! maintained layout permutations, and set-valued negation projections.
//! Negative projections record first variable columns, repeated-column equality
//! pairs and constant filters; their key sets are prepared once per component.
//! Each projection also records its key-set provider token and hasher state
//! (WF-DL-Storage): a `set`-protocol token spelled over the inferred key row
//! `_` with no payload. Round copies these facts; it never chooses them.
//! Uncertified hosts remain valid storage but cannot discharge CQ host facts.
//! See doc/formal/datalog.tex for the storage refinement obligations.
use cq_datalog_support::{columns, ensure, equalities, filters, key_columns};
pub use cq_lang_datalog_incremental as incremental;
use cq_lang_structures::{Declaration, provider::InputSchema};
use cq_syntax::symbol_name;
pub use incremental::strata::source::{BodyItem, Query};
use std::collections::BTreeSet;
use syn::{Expr, Ident, Type};
mod layouts;
pub mod provider;
pub mod text;
pub use provider::{Hasher, Provider};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub incremental: incremental::Program,
    /// Declared provider tokens (WF-DL-Storage); Round embeds this table.
    pub tokens: Vec<provider::Token>,
    pub providers: Vec<Providers>,
    pub layouts: Vec<Layout>,
    pub delta_layouts: Vec<Layout>,
    pub reads: Vec<Read>,
    pub negations: Vec<Negative>,
    pub policy: Policy,
    /// Explicit device realization records, never recovered from provider types.
    pub device: Option<DeviceStorage>,
}
/// Device realization of the Storage records. The device refinement fills and
/// checks every entry; its printer consumes them instead of choosing providers,
/// initializers or scratch lifetimes. Expressions may read the device pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceStorage {
    /// GPU read facts keyed by the Storage provider token, never its spelling.
    pub providers: Vec<Declaration>,
    pub symbols: Vec<SymbolCodec>,
    /// Geometric run-maintenance advice consumed by every state initializer.
    pub ratio: usize,
    /// Per declaration: relation-order rows and candidate storage.
    pub relations: Vec<DeviceRelation>,
    /// Per `Program::layouts` entry, canonical layouts included.
    pub layouts: Vec<DeviceState>,
    /// Per `Program::delta_layouts` entry.
    pub delta_layouts: Vec<DeviceState>,
    /// Per `Program::negations` entry: the prepared negative key set.
    pub negations: Vec<DeviceState>,
    pub scratch: DeviceScratch,
}

/// Authored codec declaration; device storage copies its interned identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolCodec {
    pub logical: Type,
    pub logical_type_id: Option<cq_syntax::LogicalTypeId>,
    pub codec_id: cq_lang_structures::CodecId,
    pub codec: Type,
}
record!(SymbolCodec {
    logical: Type,
    logical_type_id: Option<cq_syntax::LogicalTypeId>,
    codec_id: cq_lang_structures::CodecId,
    codec: Type
});
/// Relation-order packed rows: EDB upload, rule candidates and output decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRelation {
    /// Relation-order logical/storage columns and authored codecs for delta views.
    pub input: InputSchema,
    pub word: Type,
    pub packing: Expr,
    /// Container of one round's candidate batches, before admission.
    pub candidates: Type,
    pub candidates_init: Expr,
    /// Scratch arena owning the candidate batches of this relation's holes.
    pub arena: Type,
    pub arena_init: Expr,
    /// How NEW is realized: `fresh_run` builds the `next` provider directly
    /// from admission's fresh rows, so `next_init` is only its declared empty
    /// value and is never printed.
    pub next: Ident,
    /// One certified admitted-row order per publication, shared by all views.
    pub admitted_orders: Vec<DeviceAdmittedOrder>,
    /// Relation-order delta references this certified order.
    pub relation_order: usize,
    /// Candidate relation order is `canonical` or requires `permutation`.
    pub candidate_order: Ident,
    /// Relation output field to canonical packed field, without device reordering.
    pub output_permutation: Vec<usize>,
}
/// A construction certificate rooted in canonical admission's sorted unique batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceAdmittedOrder {
    pub columns: Vec<usize>,
    /// Destination position to canonical input position, recorded once.
    pub permutation: Vec<usize>,
    /// `sorted_unique` adopts canonical order; `unique_permutation` sorts once.
    pub certificate: Ident,
}
/// The maintained backing state of one device provider and its empty view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceState {
    /// Complete-key physical row order; logical lookup prefixes remain in Layout.
    pub input: InputSchema,
    pub token: Ident,
    pub provider: Type,
    pub word: Type,
    pub offset: Type,
    /// Full-key set positions are unit offsets, not a materialized offset lane.
    pub offset_layout: Ident,
    /// Full-row keys carry no item columns and need no item-source map.
    pub item_sources: Ident,
    /// Ordinary full-key set construction and encoded key-prefix reads.
    pub construction: Ident,
    pub read: Ident,
    pub packing: Expr,
    /// Significant low-bit window, derived only from the declared codec schema.
    pub active_bits: Expr,
    /// Native scalar keys or stable lane pairs, never value-inspected selection.
    pub sort: Ident,
    /// Index into the relation's admitted_orders; negative projections have none.
    pub admitted_order: Option<usize>,
    pub state: Type,
    /// Resolved maintenance and auxiliary launch parameters, consumed verbatim.
    pub ratio: usize,
    pub radix_bits: u32,
    pub merge_tile_rows: u32,
    pub block_threads: u32,
    pub permutation_threads: u32,
    pub state_init: Expr,
    pub init: Expr,
}
/// Scratch lifetimes: `arenas` names the scope owning candidate arenas and
/// `release` the boundary after which cached scratch and unused pool storage
/// return to the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceScratch {
    pub arenas: Ident,
    pub release: Ident,
}
record!(DeviceStorage {
    providers: Vec<Declaration>,
    symbols: Vec<SymbolCodec>,
    ratio: usize,
    relations: Vec<DeviceRelation>,
    layouts: Vec<DeviceState>,
    delta_layouts: Vec<DeviceState>,
    negations: Vec<DeviceState>,
    scratch: DeviceScratch
});
record!(DeviceRelation {
    input: InputSchema,
    word: Type,
    packing: Expr,
    candidates: Type,
    candidates_init: Expr,
    arena: Type,
    arena_init: Expr,
    next: Ident,
    admitted_orders: Vec<DeviceAdmittedOrder>,
    relation_order: usize,
    candidate_order: Ident,
    output_permutation: Vec<usize>
});
record!(DeviceAdmittedOrder {
    columns: Vec<usize>,
    permutation: Vec<usize>,
    certificate: Ident
});
record!(DeviceState {
    input: InputSchema,
    token: Ident,
    provider: Type,
    word: Type,
    offset: Type,
    offset_layout: Ident,
    item_sources: Ident,
    construction: Ident,
    read: Ident,
    packing: Expr,
    active_bits: Expr,
    sort: Ident,
    admitted_order: Option<usize>,
    state: Type,
    ratio: usize,
    radix_bits: u32,
    merge_tile_rows: u32,
    block_threads: u32,
    permutation_threads: u32,
    state_init: Expr,
    init: Expr
});
record!(DeviceScratch {
    arenas: Ident,
    release: Ident
});
/// Concrete providers for one relation, indexed by source declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Providers {
    /// Index of this relation's canonical member of `Program::layouts`.
    pub canonical: usize,
    /// Delta/next row buffers (`rows_vec` unless a refinement declares another token).
    pub delta: Provider,
    pub next: Provider,
    pub delta_init: Expr,
    pub next_init: Expr,
}
/// Formal rule DL-LayoutProtocol: checked maintenance operations are independent of provider spelling.
/// `GroupedMap` is `entry(key).or_default().push(value)`; `Set` is a set of
/// all-column keys with no payload (`insert`, `contains`, `extend`);
/// `Bulk` is `push(key, value)` / `extend_from` with exact membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutProtocol {
    GroupedMap,
    Set,
    Bulk,
}
impl text::Text for LayoutProtocol {
    fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "grouped_map" => Ok(Self::GroupedMap),
            "set" => Ok(Self::Set),
            "bulk" => Ok(Self::Bulk),
            _ => Err(syn::Error::new_spanned(
                name,
                "unknown layout maintenance protocol",
            )),
        }
    }
    fn text(&self) -> proc_macro2::TokenStream {
        match self {
            Self::GroupedMap => quote::quote!(grouped_map),
            Self::Set => quote::quote!(set),
            Self::Bulk => quote::quote!(bulk),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub relation: Ident,
    pub columns: Vec<usize>,
    pub key: usize,
    pub provider: Provider,
    /// Recorded hasher state, `none` for unhashed tokens.
    pub hasher: Hasher,
    pub maintenance: Ident,
    pub protocol: LayoutProtocol,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub variant: usize,
    pub position: usize,
    pub kind: Ident,
    pub index: usize,
    pub columns: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negative {
    pub component: usize,
    pub relation: usize,
    pub columns: Vec<usize>,
    pub filters: Vec<(usize, Expr)>,
    pub equalities: Vec<(usize, usize)>,
    /// Key-set provider, spelled over [`negative_schema`].
    pub provider: Provider,
    /// Recorded hasher state of the key set.
    pub hasher: Hasher,
}
impl Negative {
    /// Whether both records describe the same logical projection, whatever
    /// provider realizes it.
    pub fn same_projection(&self, other: &Self) -> bool {
        self.component == other.component
            && self.relation == other.relation
            && self.columns == other.columns
            && self.filters == other.filters
            && self.equalities == other.equalities
    }
}
/// Key and payload schema of every negation key set: the inferred key row `_`
/// (the hole's projected row) and no payload.
pub fn negative_schema() -> (Type, Type) {
    (syn::parse_quote!(_), syn::parse_quote!(()))
}
/// The default negation key set: the base `hash_set` token with the Fx hasher.
pub fn default_key_set() -> (Provider, Hasher) {
    let token = provider::hash_set();
    let (key, payload) = negative_schema();
    let ty =
        provider::spell(&token, &key, &payload, Hasher::Fx).expect("hash_set spells the Fx hasher");
    (
        Provider {
            token: token.token,
            ty,
        },
        Hasher::Fx,
    )
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub insertion: Ident,
    pub negation_keys: Ident,
}
record!(Program { incremental: incremental::Program, tokens: Vec<provider::Token>, providers: Vec<Providers>, layouts: Vec<Layout>, delta_layouts: Vec<Layout>, reads: Vec<Read>, negations: Vec<Negative>, policy: Policy, device: Option<DeviceStorage> });
record!(Providers {
    canonical: usize,
    delta: Provider,
    next: Provider,
    delta_init: Expr,
    next_init: Expr
});
record!(Layout { relation: Ident, columns: Vec<usize>, key: usize, provider: Provider, hasher: Hasher, maintenance: Ident, protocol: LayoutProtocol });
/// Key and payload row types of a layout of `declaration` (its recorded schema).
pub fn layout_schema(
    declaration: &cq_syntax::RelationDecl,
    columns: &[usize],
    key: usize,
) -> (Type, Type) {
    let types: Vec<_> = columns
        .iter()
        .map(|&column| &declaration.columns[column].ty)
        .collect();
    let (keys, values) = types.split_at(key);
    (
        syn::parse_quote!((#(#keys,)*)),
        syn::parse_quote!((#(#values,)*)),
    )
}
/// The full row type of `declaration`, the schema of its row buffers.
pub fn row_type(declaration: &cq_syntax::RelationDecl) -> Type {
    let types = declaration.columns.iter().map(|column| &column.ty);
    syn::parse_quote!((#(#types,)*))
}
record!(Read { variant: usize, position: usize, kind: Ident, index: usize, columns: Vec<usize> });
record!(Negative { component: usize, relation: usize, columns: Vec<usize>, filters: Vec<(usize, Expr)>, equalities: Vec<(usize, usize)>, provider: Provider, hasher: Hasher });
record!(Policy {
    insertion: Ident,
    negation_keys: Ident
});

pub use incremental::Analysis;
/// Choose the default written-order occurrence bindings and default `hash_set`
/// Fx negation key sets; contracts do not call this.
pub fn read_plan(
    program: &Program,
    analysis: &Analysis,
) -> syn::Result<(Vec<Read>, Vec<Negative>)> {
    let mut reads = Vec::new();
    let mut negations = Vec::new();
    for (variant_index, variant) in program.incremental.variants.iter().enumerate() {
        let mut bound = BTreeSet::new();
        let head = analysis.positions[&symbol_name(&variant.query.head.relation)];
        let component = analysis.component[head].expect("checked IDB component");
        let order = incremental::variant_order(
            &program.incremental.strata.source.rules[variant.rule],
            &variant.order,
        )?;
        for (position, item) in variant.query.body.iter().enumerate() {
            match item {
                BodyItem::Positive { atom } => {
                    let source_position = order[position];
                    let is_delta = variant.delta == Some(source_position);
                    let relation = if is_delta {
                        let BodyItem::Positive { atom } = &program.incremental.strata.source.rules
                            [variant.rule]
                            .body[source_position]
                        else {
                            unreachable!("checked changing occurrence")
                        };
                        &atom.relation
                    } else {
                        &atom.relation
                    };
                    let (columns, key) = key_columns(atom, &bound);
                    let (kind, index) = if is_delta && position == 0 && key == 0 {
                        (
                            syn::parse_quote!(direct),
                            analysis.positions[&symbol_name(relation)],
                        )
                    } else {
                        let choices = if is_delta {
                            &program.delta_layouts
                        } else {
                            &program.layouts
                        };
                        let index = choices
                            .iter()
                            .position(|l| {
                                l.relation == *relation && l.columns == columns && l.key == key
                            })
                            .ok_or_else(|| {
                                syn::Error::new_spanned(atom, "storage omitted a read layout")
                            })?;
                        (
                            if is_delta {
                                syn::parse_quote!(delta)
                            } else {
                                syn::parse_quote!(full)
                            },
                            index,
                        )
                    };
                    reads.push(Read {
                        variant: variant_index,
                        position,
                        kind,
                        index,
                        columns,
                    });
                    bound.extend(atom.variables().map(symbol_name));
                }
                BodyItem::Negation(negative) => {
                    let atom = &negative.atom;
                    let (provider, hasher) = default_key_set();
                    let entry = Negative {
                        component,
                        relation: analysis.positions[&symbol_name(&atom.relation)],
                        columns: columns(atom),
                        filters: filters(atom),
                        equalities: equalities(atom),
                        provider,
                        hasher,
                    };
                    if !negations
                        .iter()
                        .any(|n: &Negative| n.same_projection(&entry))
                    {
                        negations.push(entry);
                    }
                }
                _ => {}
            }
        }
    }
    Ok((reads, negations))
}
pub mod contract {
    use super::*;
    /// Formal rule WF-DL-Storage: admissible bindings, layouts and total maintenance.
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let analysis = incremental::contract::check(&program.incremental)?;
        if let Some(device) = &program.device {
            ensure(
                device.ratio >= 2,
                program,
                "device maintenance ratio must be at least two",
            )?;
            for (index, symbol) in device.symbols.iter().enumerate() {
                ensure(
                    symbol.logical_type_id.is_some()
                        && !device.symbols[..index]
                            .iter()
                            .any(|previous| previous.logical_type_id == symbol.logical_type_id),
                    program,
                    "duplicate logical symbol type",
                )?;
                ensure(
                    program
                        .incremental
                        .strata
                        .source
                        .declarations
                        .iter()
                        .any(|declaration| {
                            declaration
                                .columns
                                .iter()
                                .any(|column| column.logical_type_id == symbol.logical_type_id)
                        }),
                    program,
                    "symbol type does not occur in a relation",
                )?;
            }
            // Device records cover their Storage vectors and realize the
            // recorded layout providers; the device refinement checks the rest.
            let realizes = |states: &[DeviceState], layouts: &[Layout]| {
                states.len() == layouts.len()
                    && states
                        .iter()
                        .zip(layouts)
                        .all(|(state, layout)| state.provider == layout.provider.ty)
            };
            ensure(
                device.relations.len() == program.incremental.strata.source.declarations.len()
                    && realizes(&device.layouts, &program.layouts)
                    && realizes(&device.delta_layouts, &program.delta_layouts)
                    && device.negations.len() == program.negations.len(),
                program,
                "device records must cover every relation, layout and negative key set",
            )?;
            for (index, declaration) in device.providers.iter().enumerate() {
                ensure(
                    !device.providers[..index]
                        .iter()
                        .any(|previous| previous.token == declaration.token)
                        && program
                            .tokens
                            .iter()
                            .any(|token| token.token == declaration.token)
                        && declaration.facts.abi.is_some(),
                    program,
                    "device provider declarations require unique declared Storage tokens and a native ABI",
                )?;
            }
            let declared = |token: &Ident| {
                device
                    .providers
                    .iter()
                    .any(|declaration| declaration.token == *token)
            };
            let schema_matches =
                |input: &InputSchema, columns: &[&cq_syntax::ColumnDecl], keys: usize| {
                    input.key_count == keys
                        && input.columns.len() == columns.len()
                        && input.columns.iter().zip(columns).all(|(input, logical)| {
                            input.logical_type_id.is_some()
                                && input.logical_type_id == logical.logical_type_id
                        })
                };
            for (index, (relation, declaration)) in device
                .relations
                .iter()
                .zip(&program.incremental.strata.source.declarations)
                .enumerate()
            {
                let columns = declaration.columns.iter().collect::<Vec<_>>();
                ensure(
                    schema_matches(&relation.input, &columns, columns.len()),
                    program,
                    "device relation input must record its complete logical schema",
                )?;
                if declaration.value.is_none() {
                    ensure(
                        program
                            .providers
                            .get(index)
                            .is_some_and(|provider| declared(&provider.delta.token)),
                        program,
                        "device delta provider must have declared GPU facts",
                    )?;
                }
            }
            for (state, layout) in device
                .layouts
                .iter()
                .zip(&program.layouts)
                .chain(device.delta_layouts.iter().zip(&program.delta_layouts))
            {
                let relation = analysis
                    .positions
                    .get(&symbol_name(&layout.relation))
                    .and_then(|&index| program.incremental.strata.source.declarations.get(index))
                    .ok_or_else(|| {
                        syn::Error::new_spanned(layout, "unknown device layout relation")
                    })?;
                let columns = layout
                    .columns
                    .iter()
                    .filter_map(|&column| relation.columns.get(column))
                    .collect::<Vec<_>>();
                ensure(
                    declared(&state.token)
                        && state.token == layout.provider.token
                        && columns.len() == layout.columns.len()
                        && schema_matches(&state.input, &columns, columns.len()),
                    program,
                    "device full-key input and token must match its recorded logical layout",
                )?;
            }
            for (state, negative) in device.negations.iter().zip(&program.negations) {
                let relation = program
                    .incremental
                    .strata
                    .source
                    .declarations
                    .get(negative.relation)
                    .ok_or_else(|| {
                        syn::Error::new_spanned(negative, "unknown device negative relation")
                    })?;
                let columns = negative
                    .columns
                    .iter()
                    .filter_map(|&column| relation.columns.get(column))
                    .collect::<Vec<_>>();
                ensure(
                    declared(&state.token)
                        && columns.len() == negative.columns.len()
                        && schema_matches(&state.input, &columns, columns.len()),
                    program,
                    "device negative input must match its recorded logical projection",
                )?;
            }
        }
        provider::check_table(&program.tokens)?;
        ensure(
            program.providers.len() == program.incremental.strata.source.declarations.len(),
            program,
            "storage providers must cover every declaration in declaration order",
        )?;
        for (index, (provider, declaration)) in program
            .providers
            .iter()
            .zip(&program.incremental.strata.source.declarations)
            .enumerate()
        {
            ensure(
                provider.delta == provider.next,
                provider,
                "delta and next providers must have the same type for move publication",
            )?;
            // Row buffers are unhashed; the payload schema is empty.
            let rows = row_type(declaration);
            let unit: Type = syn::parse_quote!(());
            provider::check_instance(&program.tokens, &provider.delta, &rows, &unit, Hasher::None)?;
            // Formal rule DL-Canonical: one complete, all-column-keyed member of
            // the relation's own full layouts; no separate membership store.
            let canonical = program.layouts.get(provider.canonical);
            ensure(
                canonical.is_some_and(|layout| {
                    analysis.positions.get(&symbol_name(&layout.relation)) == Some(&index)
                        && layout.key == declaration.arity()
                }),
                provider,
                "canonical must name one of the relation's full layouts keyed by every column",
            )?;
            if declaration.value.is_some() {
                let empty: Expr = syn::parse_quote!(::std::vec::Vec::new());
                ensure(
                    provider.delta.token == provider::ROWS_VEC
                        && provider.delta_init == empty
                        && provider.next_init == empty,
                    provider,
                    "EDB providers must retain defaults for unused IDB storage fields",
                )?;
            }
        }
        layouts::check(program, &analysis)?;
        let p = &program.policy;
        ensure(
            p.insertion == "all_columns" && p.negation_keys == "borrowed",
            program,
            "unsupported storage insertion or negation semantics",
        )?;
        Ok(analysis)
    }
}
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    check_rewrite_admitted(source, target)
}
// The stage runner has already checked both language contracts.
fn check_rewrite_admitted(source: &Program, target: &Program) -> syn::Result<()> {
    ensure(
        source.incremental == target.incremental && source.policy == target.policy,
        target,
        "storage rewrite changed its incremental base program",
    )
}
/// The rewrite callback assumes `cq_stage::run` checked both contracts.
/// Standalone callers must use the public free [`check_rewrite`] function.
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite_admitted(source, target)
    }
}
