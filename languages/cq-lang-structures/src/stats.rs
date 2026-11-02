//! Formal rule STR-BuildStats: selected, immutable build publications. Declarations are
//! provider token laws, not capabilities inferred from Rust spellings. Merely
//! declaring a statistic neither selects it nor causes computation or emission.
//!
//! ```text
//! Publications ::= ["publications" "{" Publication* "}"]
//! Publication ::= Ident "=" "build" Ident "provider" Ident "layout" Level+
//!                 "columns" Level "domains" "[" Domain,* "]"
//!                 "stat" StatDeclaration ";"
//! Domain ::= Ident ":" EncodingId DomainId EpochInteger
//! Reference ::= Ident "build" Ident "provider" Ident "layout" Level+
//!               "columns" Level "domains" "[" Domain,* "]"
//!               "field" Field "unit" Unit "precision" Precision
//! ```
use super::*;
use syn::parse::{Parse, ParseStream};

mod kw {
    syn::custom_keyword!(publications);
}

macro_rules! words {
    ($name:ident { $($variant:ident => $word:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl Parse for $name {
            fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
                let word: Ident = input.parse()?;
                match word.to_string().as_str() {
                    $($word => Ok(Self::$variant),)+
                    _ => error(&word, concat!("STR-BuildStats: unknown ", stringify!($name))),
                }
            }
        }
        impl ToTokens for $name {
            fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
                let word = match self { $(Self::$variant => $word,)+ };
                Ident::new(word, proc_macro2::Span::call_site()).to_tokens(tokens);
            }
        }
    };
}

words!(Field {
    Rows => "rows", Keys => "keys", MaxGroup => "max_group", Bounds => "bounds",
    Blocks => "blocks", Runs => "runs", InputRows => "input_rows", Groups => "groups"
});
words!(Unit {
    Occurrences => "occurrences", DistinctKeys => "distinct_keys",
    MaxBucketOccurrences => "max_bucket_occurrences", CodeBounds => "code_bounds",
    Blocks => "blocks", Runs => "runs", Groups => "groups"
});
words!(Precision {
    Exact => "exact", Estimate => "estimate", UpperBound => "upper_bound", LowerBound => "lower_bound"
});
words!(Producer {
    RunFinalize => "run_finalize", HashFinalize => "hash_finalize",
    ScanEncode => "scan_encode", MergeFinalize => "merge_finalize", GroupFinalize => "group_finalize"
});
words!(Placement { Host => "host", Cuda => "cuda", Metal => "metal" });
words!(Availability { Required => "required", Optional => "optional" });
words!(Computation { Build => "build", Additional => "additional" });
// Disjoint means disjoint grouping keys, not merely disjoint row partitions.
words!(Aggregation {
    Single => "single", Disjoint => "disjoint", Overlapping => "overlapping", KeyMerge => "key_merge"
});

/// Numeric ABI: `accessor(&self) -> Option<u64>`. `None` means unavailable,
/// never empty or zero. Reads/availability are O(1), immutable and never force
/// lazy finalization. A bounds accessor instead returns an optional typed
/// empty-or-endpoints value in the recorded code domain, not a numeric scalar.
/// Rust obligations belong to consumers; this language checks token laws only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatDeclaration {
    pub name: Ident,
    pub field: Field,
    pub unit: Unit,
    pub precision: Precision,
    pub producer: Producer,
    pub accessor: Ident,
    pub placement: Placement,
    pub availability: Availability,
    pub computation: Computation,
    pub aggregation: Aggregation,
}

impl Parse for StatDeclaration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        Ok(Self {
            name,
            field: input.parse()?,
            unit: input.parse()?,
            precision: input.parse()?,
            producer: input.parse()?,
            accessor: input.parse()?,
            placement: input.parse()?,
            availability: input.parse()?,
            computation: input.parse()?,
            aggregation: input.parse()?,
        })
    }
}

impl ToTokens for StatDeclaration {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            name,
            field,
            unit,
            precision,
            producer,
            accessor,
            placement,
            availability,
            computation,
            aggregation,
        } = self;
        tokens.extend(
            quote::quote!(#name: #field #unit #precision #producer #accessor
            #placement #availability #computation #aggregation),
        );
    }
}

impl StatDeclaration {
    pub fn is_scalar(&self) -> bool {
        self.unit != Unit::CodeBounds
    }

    fn check(&self) -> syn::Result<()> {
        let expected = match self.field {
            Field::Rows | Field::InputRows => Unit::Occurrences,
            Field::Keys => Unit::DistinctKeys,
            Field::MaxGroup => Unit::MaxBucketOccurrences,
            Field::Bounds => Unit::CodeBounds,
            Field::Blocks => Unit::Blocks,
            Field::Runs => Unit::Runs,
            Field::Groups => Unit::Groups,
        };
        if self.unit != expected {
            return error(self, "STR-BuildStats: statistic field and unit differ");
        }
        if matches!(self.field, Field::Keys | Field::MaxGroup)
            && self.precision == Precision::Exact
            && self.aggregation == Aggregation::Overlapping
            && self.computation != Computation::Additional
        {
            return error(
                self,
                "STR-BuildStats: overlapping segments cannot publish exact global keys or max_group without a selected statistics pass or required key merge",
            );
        }
        if self.aggregation == Aggregation::KeyMerge && self.producer != Producer::MergeFinalize {
            return error(self, "STR-BuildStats: key_merge requires merge_finalize");
        }
        Ok(())
    }
}

/// One ordered projection component, including a mandatory dictionary epoch.
/// Identity codecs also record their domain and epoch; absence is not epoch 0.
/// Encoding/domain/epoch identities are authored declaration premises in this
/// slice, not an independent proof against a loader's codec-source records.
/// Checks enforce publication consistency and exact consumer identity; they
/// never infer a domain or lexical ordering from a Rust codec spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain {
    pub column: Ident,
    pub encoding: Ident,
    pub domain: Ident,
    pub epoch: u64,
}

impl Parse for Domain {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let column = input.parse()?;
        input.parse::<Token![:]>()?;
        Ok(Self {
            column,
            encoding: input.parse()?,
            domain: input.parse()?,
            epoch: input.parse::<syn::LitInt>()?.base10_parse()?,
        })
    }
}

impl ToTokens for Domain {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            column,
            encoding,
            domain,
            epoch,
        } = self;
        let epoch = proc_macro2::Literal::u64_unsuffixed(*epoch);
        tokens.extend(quote::quote!(#column: #encoding #domain #epoch));
    }
}

fn word(input: ParseStream<'_>, expected: &str) -> syn::Result<()> {
    let actual: Ident = input.parse()?;
    if actual != expected {
        return error(&actual, format!("expected `{expected}`"));
    }
    Ok(())
}

fn domains(input: ParseStream<'_>) -> syn::Result<Vec<Domain>> {
    word(input, "domains")?;
    let body;
    syn::bracketed!(body in input);
    Ok(CommaList::<Domain>::parse_terminated(&body)?
        .into_iter()
        .collect())
}

/// Selected immutable Physical result plus complete ordered index layout.
/// The build identifier is the version, not a relation-name cache key. Distinct
/// rounds/full/delta publications must use their distinct Physical results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub name: Ident,
    pub build: Ident,
    pub provider: Ident,
    pub layout: Vec<Level>,
    pub columns: Level,
    pub domains: Vec<Domain>,
    pub statistic: StatDeclaration,
}

impl Parse for Publication {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        word(input, "build")?;
        let build = input.parse()?;
        word(input, "provider")?;
        let provider = input.parse()?;
        word(input, "layout")?;
        let layout = parse_levels(input)?;
        word(input, "columns")?;
        let columns = input.parse()?;
        let domains = domains(input)?;
        word(input, "stat")?;
        let statistic = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self {
            name,
            build,
            provider,
            layout,
            columns,
            domains,
            statistic,
        })
    }
}

impl ToTokens for Publication {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            name,
            build,
            provider,
            layout,
            columns,
            domains,
            statistic,
        } = self;
        tokens.extend(quote::quote!(#name = build #build provider #provider
            layout #(#layout)* columns #columns domains [#(#domains),*] stat #statistic;));
    }
}

/// A consumer repeats identity rather than trusting a name alone. In
/// particular it cannot silently keep a reference across a new build, layout,
/// dictionary epoch or precision. Producer/ABI choices stay with publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub publication: Ident,
    pub build: Ident,
    pub provider: Ident,
    pub layout: Vec<Level>,
    pub columns: Level,
    pub domains: Vec<Domain>,
    pub field: Field,
    pub unit: Unit,
    pub precision: Precision,
}

impl Publication {
    pub fn reference(&self) -> Reference {
        Reference {
            publication: self.name.clone(),
            build: self.build.clone(),
            provider: self.provider.clone(),
            layout: self.layout.clone(),
            columns: self.columns.clone(),
            domains: self.domains.clone(),
            field: self.statistic.field,
            unit: self.statistic.unit,
            precision: self.statistic.precision,
        }
    }
}

impl Parse for Reference {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let publication = input.parse()?;
        word(input, "build")?;
        let build = input.parse()?;
        word(input, "provider")?;
        let provider = input.parse()?;
        word(input, "layout")?;
        let layout = parse_levels(input)?;
        word(input, "columns")?;
        let columns = input.parse()?;
        let domains = domains(input)?;
        word(input, "field")?;
        let field = input.parse()?;
        word(input, "unit")?;
        let unit = input.parse()?;
        word(input, "precision")?;
        let precision = input.parse()?;
        Ok(Self {
            publication,
            build,
            provider,
            layout,
            columns,
            domains,
            field,
            unit,
            precision,
        })
    }
}

impl ToTokens for Reference {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            publication,
            build,
            provider,
            layout,
            columns,
            domains,
            field,
            unit,
            precision,
        } = self;
        tokens.extend(quote::quote!(#publication build #build provider #provider
            layout #(#layout)* columns #columns domains [#(#domains),*]
            field #field unit #unit precision #precision));
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Publications(pub Vec<Publication>);

impl Parse for Publications {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::publications) {
            return Ok(Self::default());
        }
        input.parse::<kw::publications>()?;
        let body;
        syn::braced!(body in input);
        let mut publications = Vec::new();
        while !body.is_empty() {
            publications.push(body.parse()?);
        }
        Ok(Self(publications))
    }
}

impl ToTokens for Publications {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        if !self.0.is_empty() {
            let publications = &self.0;
            tokens.extend(quote::quote!(publications { #(#publications)* }));
        }
    }
}

pub(crate) fn check_declarations(statistics: &[StatDeclaration]) -> syn::Result<()> {
    let mut seen = BTreeSet::new();
    for statistic in statistics {
        statistic.check()?;
        if !seen.insert(symbol_name(&statistic.name)) {
            return error(statistic, "STR-BuildStats: duplicate statistic declaration");
        }
    }
    Ok(())
}

/// Check selected records, never discover or select statistics. Layout order
/// is compared against authored Built entries, not sorted read-demand keys.
pub(crate) fn check(program: &Program, analysis: &mut Analysis) -> syn::Result<()> {
    let mut domains_by_build = BTreeMap::new();
    let mut selected_fields = BTreeSet::new();
    for publication in &program.structures.publications.0 {
        let build = symbol_name(&publication.build);
        let selected = program.structures.entries.iter().find_map(|entry| {
            if symbol_name(&entry.name) != build {
                return None;
            }
            let Structure::Built { keyed, .. } = &entry.structure else {
                return None;
            };
            (keyed.levels == publication.layout && keyed.provider.token == publication.provider)
                .then_some(keyed)
        });
        let Some(selected) = selected else {
            return error(
                publication,
                "STR-BuildStats: publication requires its selected immutable build, provider and complete ordered layout",
            );
        };
        if analysis.physical.modes.get(&build) != Some(&ModeKind::Let) {
            return error(
                publication,
                "STR-BuildStats: publication requires a completed immutable let build",
            );
        }
        let facts = analysis.facts(&selected.provider);
        if !facts.statistics.contains(&publication.statistic) {
            return error(
                publication,
                "STR-BuildStats: publication differs from its provider statistic declaration",
            );
        }
        let layout: Vec<_> = selected
            .levels
            .iter()
            .flat_map(|l| l.columns.iter().map(symbol_name))
            .collect();
        let columns: Vec<_> = publication
            .columns
            .columns
            .iter()
            .map(symbol_name)
            .collect();
        let field = publication.statistic.field;
        if columns.iter().any(|c| !layout.contains(c))
            || columns.iter().collect::<BTreeSet<_>>().len() != columns.len()
        {
            return error(
                publication,
                "STR-BuildStats: statistic projection must contain distinct selected key columns",
            );
        }
        if matches!(field, Field::Keys | Field::MaxGroup) && columns != layout {
            return error(
                publication,
                "STR-BuildStats: keys and max_group describe the complete ordered grouping key",
            );
        }
        let keyed = matches!(field, Field::Keys | Field::MaxGroup | Field::Bounds);
        if keyed && columns.is_empty() || !keyed && !columns.is_empty() {
            return error(
                publication,
                "STR-BuildStats: only key, group and bounds statistics carry a key projection",
            );
        }
        if publication
            .domains
            .iter()
            .map(|d| symbol_name(&d.column))
            .collect::<Vec<_>>()
            != columns
        {
            return error(
                publication,
                "STR-BuildStats: every ordered projection column requires its encoding, domain and epoch",
            );
        }
        for domain in &publication.domains {
            let identity = (build.clone(), symbol_name(&domain.column));
            if let Some(previous) = domains_by_build.insert(identity, domain.clone()) {
                if previous != *domain {
                    return error(
                        publication,
                        "STR-BuildStats: one immutable build column has conflicting encoding/domain epochs",
                    );
                }
            }
        }
        let ordered_layout: Vec<Vec<_>> = publication
            .layout
            .iter()
            .map(|level| level.columns.iter().map(symbol_name).collect())
            .collect();
        let identity = (
            build,
            ordered_layout,
            symbol_name(&publication.provider),
            symbol_name(&publication.statistic.name),
            columns,
        );
        if !selected_fields.insert(identity) {
            return error(
                publication,
                "STR-BuildStats: duplicate selected statistic field",
            );
        }
        if analysis
            .publications
            .insert(symbol_name(&publication.name), publication.clone())
            .is_some()
        {
            return error(publication, "STR-BuildStats: duplicate publication name");
        }
    }
    Ok(())
}
