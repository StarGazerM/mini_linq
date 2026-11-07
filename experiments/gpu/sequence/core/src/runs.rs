//! Checked bindings for declared full-key radix relation inputs.
use cq_lang_structures::provider::{CodecId, NativeAbi};
use cq_lang_structures::{Program, Structure, physical::symbol_name};
use quote::quote;
use std::collections::BTreeMap;
use syn::Type;

/// Compared columns share one checked encoding for each declared logical identity.
#[derive(Default)]
pub(crate) struct Codecs {
    declarations: BTreeMap<cq_syntax::LogicalTypeId, (Option<CodecId>, Option<Type>)>,
    columns: BTreeMap<String, cq_syntax::LogicalTypeId>,
}

impl Codecs {
    pub fn get(&self, logical: Option<cq_syntax::LogicalTypeId>) -> Option<&Type> {
        self.declarations
            .get(&logical?)
            .and_then(|(_, codec)| codec.as_ref())
    }

    pub fn codec_id(&self, logical: Option<cq_syntax::LogicalTypeId>) -> Option<CodecId> {
        self.declarations.get(&logical?).and_then(|(id, _)| *id)
    }

    pub fn logical_type_id(&self, column: &str) -> Option<cq_syntax::LogicalTypeId> {
        self.columns.get(column).copied()
    }

    pub fn column(&self, column: &str) -> Option<&Type> {
        self.get(self.logical_type_id(column))
    }

    /// Formal rule GPU-CodecIdentity: conflicting encodings of a logical symbol type are rejected before capture lowering.
    pub fn record(
        &mut self,
        logical: &Type,
        logical_type_id: cq_syntax::LogicalTypeId,
        codec_id: Option<CodecId>,
        codec: Option<&Type>,
    ) -> syn::Result<()> {
        if let Some((previous, _)) = self.declarations.get(&logical_type_id) {
            if *previous != codec_id {
                return Err(syn::Error::new_spanned(
                    logical,
                    "conflicting codecs for a logical symbol type",
                ));
            }
        } else {
            self.declarations
                .insert(logical_type_id, (codec_id, codec.cloned()));
        }
        Ok(())
    }

    pub fn check(
        physical: &cq_lang_structures::physical::Program,
        program: &Program,
        rows: &BTreeMap<String, cq_lang_structures::physical::Row>,
    ) -> syn::Result<Self> {
        let mut codecs = Self::default();
        for row in rows.values() {
            codecs.columns.extend(
                row.logical_type_ids
                    .iter()
                    .map(|(column, id)| (column.clone(), *id)),
            );
        }
        // Closed answer binders are not row columns. Their declared answer
        // slots still supply the logical identity needed by carrier and codec
        // records; a type spelling must never substitute for that identity.
        let physical = &physical.program;
        let captures: std::collections::BTreeSet<_> = physical.captures.items.iter()
            .flat_map(cq_lang_structures::syntax::Capture::binding_types)
            .map(|(name, _)| symbol_name(&name))
            .collect();
        if let Some(answer) = physical.declarations.iter()
            .find(|declaration| declaration.name == physical.head.relation)
        {
            for (argument, column) in physical.head.args.iter().zip(&answer.columns) {
                if let Some(name) = cq_lang_structures::syntax::rust::variable(argument)
                    && captures.contains(&symbol_name(name))
                    && let Some(id) = column.logical_type_id
                {
                    codecs.columns.insert(symbol_name(name), id);
                }
            }
        }
        for entry in &program.structures.entries {
            let keyed = match &entry.structure {
                Structure::Host { keyed, .. } | Structure::Built { keyed, .. } => keyed,
                _ => continue,
            };
            if let Some(input) = &keyed.provider.input {
                for column in &input.columns {
                    let id = column.logical_type_id.ok_or_else(|| {
                        syn::Error::new_spanned(
                            &column.logical,
                            "GPU-CodecIdentity: missing logical type identity",
                        )
                    })?;
                    codecs.record(&column.logical, id, column.codec_id, column.codec.as_ref())?;
                }
            }
        }
        Ok(codecs)
    }
}

pub(crate) fn scalar(ty: &Type) -> proc_macro2::TokenStream {
    quote!(<#ty as ::gpu_datalog_runtime::Scalar>::COLUMN)
}

// Formal rule GPU-Runs: declared radix relation handles bind full-key inputs; every
// lookup uses a leading packed prefix and every physical run remains visible.
pub fn bind(physical: &cq_lang_structures::physical::Program, source: &Program) -> syn::Result<Program> {
    let analysis = cq_lang_structures::contract::check(physical, source)?;
    let definitions = physical
        .program
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), definition.clone()))
        .collect();
    let mut hosts = BTreeMap::new();
    for entry in &source.structures.entries {
        let Structure::Host { reader, keyed, .. } = &entry.structure else {
            continue;
        };
        if !matches!(reader, cq_lang_structures::ReaderRef::Scan) {
            continue;
        }
        if analysis.facts(&keyed.provider).abi != Some(NativeAbi::RadixRelation) {
            continue;
        }
        let input = keyed.provider.input.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(entry, "GPU-RadixRelation: input schema is not recorded")
        })?;
        let types = &input.columns;
        let key_count = input.key_count;
        let host = &analysis.physical.hosts[&symbol_name(&entry.name)];
        let layout = &analysis.host_layouts[host];
        let row = &analysis.physical.rows[&symbol_name(&entry.name)];
        let origin =
            crate::rows::origin(&symbol_name(&entry.name), &definitions)?.ok_or_else(|| {
                syn::Error::new_spanned(entry, "GPU-RadixRelation: scan has no declared input origin")
            })?;
        if layout.len() != types.len() {
            return Err(syn::Error::new_spanned(
                entry,
                "GPU-RadixRelation: declared packed arity differs from input",
            ));
        }
        for column in row.types.keys() {
            if let Some(source) = origin.columns.get(column) {
                let position = layout.iter().position(|name| name == source).unwrap();
                let logical = types[position].logical_type_id;
                if logical.is_none() || row.logical_type_ids.get(column).copied() != logical {
                    return Err(syn::Error::new_spanned(
                        entry,
                        "GPU-RadixRelation: declared packed logical type id differs from input",
                    ));
                }
            }
        }
        hosts.insert(host.clone(), (keyed.provider.clone(), key_count));
    }
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        let Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        let Some(origin) = crate::rows::origin(&name, &definitions)? else {
            continue;
        };
        let Some((provider, _key_count)) = hosts.get(&origin.host) else {
            continue;
        };
        // A selected source is not the raw run: keep the computed Built index.
        if origin.selection.is_some() || origin.predicate {
            continue;
        }
        let keys = keyed.level_names().concat();
        let layout = &analysis.host_layouts[&origin.host];
        if keys.len() > layout.len()
            || keys.iter().any(|key| !origin.columns.get(key).is_some_and(|column| layout.contains(column)))
        {
            return Err(syn::Error::new_spanned(
                &entry.name,
                "GPU-RadixRelation: lookup key is absent from its full-key source",
            ));
        }
        keyed.provider = provider.clone();
    }
    target.structures.declare(&[])?;
    cq_lang_structures::contract::check(physical, &target)?;
    Ok(target)
}

/// Formal rule GPU-Representation: provider and codec declarations, never
/// logical Rust type spellings, supply the facts checked by BE-RustRepresentation.
pub fn record_representation(
    physical: &cq_lang_structures::physical::Program,
    structures: &cq_lang_structures::Program,
    source: &cq_lang_sequence::Program,
) -> syn::Result<cq_lang_sequence::Program> {
    let analysis = cq_lang_structures::contract::check(physical, structures)?;
    let mut hosts = BTreeMap::new();
    for entry in &structures.structures.entries {
        let Structure::Host {
            reader: cq_lang_structures::ReaderRef::Scan,
            keyed,
            ..
        } = &entry.structure
        else {
            continue;
        };
        let name = symbol_name(&entry.name);
        let Some(host) = analysis.physical.hosts.get(&name) else {
            continue;
        };
        if let Some(input) = &keyed.provider.input {
            let columns = &input.columns;
            let layout = &analysis.host_layouts[host];
            if columns.len() != layout.len() {
                return Err(syn::Error::new_spanned(
                    entry,
                    "GPU-RadixRelation: declared packed arity differs from input",
                ));
            }
            for (column, ty) in layout.iter().zip(columns) {
                hosts.insert((host.clone(), column.clone()), ty.codec.is_some());
            }
        }
    }
    let mut target = source.clone();
    // An authored binding annotation declares the identity of its Rust result.
    // Keep the type tokens opaque: rustc checks its Scalar ABI and supplies the
    // layout when the generated boundary or callable uses that result.
    for definition in &physical.program.definitions {
        if let cq_lang_structures::physical::Operator::Bind(bind) = &definition.operator
            && bind.annotation.is_some()
        {
            for binding in cq_lang_structures::syntax::rust::bindings(&bind.pattern) {
                let column = symbol_name(&binding);
                if !target.representation.encoded.contains(&column) {
                    target.representation.identity.insert(column);
                }
            }
        }
    }
    for definition in &physical.program.definitions {
        let cq_lang_structures::physical::Operator::Rename(rename) = &definition.operator else {
            continue;
        };
        let host = symbol_name(&rename.relation);
        // Rename establishes the column identity once. Selections preserve it;
        // recording a representation must not demand a single-column interval.
        for mapping in &rename.mapping {
            let column = symbol_name(&mapping.target);
            match hosts.get(&(host.clone(), symbol_name(&mapping.source))) {
                Some(true) => {
                    target.representation.identity.remove(&column);
                    target.representation.encoded.insert(column.clone());
                }
                Some(false) if !target.representation.encoded.contains(&column) => {
                    target.representation.identity.insert(column.clone());
                }
                _ => {}
            }
        }
    }
    for entry in &structures.structures.entries {
        let Structure::Built { keyed, .. } = &entry.structure else {
            continue;
        };
        if analysis.facts(&keyed.provider).abi != Some(NativeAbi::Range) {
            continue;
        }
        // The native range provider publishes Scalar-ABI rows, including
        // computed columns. Explicit codec facts still take precedence.
        let row = &analysis.physical.rows[&symbol_name(&entry.name)];
        for column in row.types.keys() {
            if !target.representation.encoded.contains(column) {
                target.representation.identity.insert(column.clone());
            }
        }
    }
    let codecs = Codecs::check(physical, structures, &analysis.physical.rows)?;
    for row in analysis.physical.rows.values() {
        for column in row.types.keys() {
            if codecs
                .get(row.logical_type_ids.get(column).copied())
                .is_some()
            {
                target.representation.identity.remove(column);
                target.representation.encoded.insert(column.clone());
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../tests/support/stack.rs");

    #[test]
    fn authored_binding_annotations_record_identity_without_type_inspection() {
        let Stack { physical, structures, program } = syn::parse_quote! {
            physical {
                relation R(x:i64) = (&data.r);
                r = pipe rename R {x -> x};
                bound = pipe bind r with value:UserDefinedScalar = (convert(x));
                result = let canon project bound keep {value};
                return result as answer(value);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
            }
            sequence {
                input = store rows r;
                output_rows = store canon map result map bound flatten tile 256 input;
                output count;
            }
        };
        let recorded = record_representation(&physical, &structures, &program).unwrap();
        assert!(recorded.representation.identity.contains("value"));
        let facts = cq_lang_sequence::contract::check(&physical, &structures, &recorded).unwrap();
        let physical_facts = cq_lang_structures::physical::contract::check(&physical).unwrap();
        let mut operator = physical.program.definitions[1]
            .operator
            .clone();
        assert!(!facts.rust_requires_host(&physical_facts, &operator));
        let cq_lang_structures::physical::Operator::Bind(bind) = &mut operator else {
            unreachable!()
        };
        bind.annotation = None;
        assert!(facts.rust_requires_host(&physical_facts, &operator));
    }

    #[test]
    fn codec_identity_uses_ids_not_type_payloads() {
        let logical = syn::parse_quote!(Logical);
        let codec = syn::parse_quote!(Codec);
        let alias = syn::parse_quote!(AnotherSpelling);
        let id = cq_syntax::LogicalTypeId(3);
        let mut codecs = Codecs::default();
        codecs
            .record(&logical, id, Some(CodecId(7)), Some(&codec))
            .unwrap();
        codecs
            .record(
                &syn::parse_quote!(RenamedLogical),
                id,
                Some(CodecId(7)),
                Some(&alias),
            )
            .unwrap();
        assert_eq!(codecs.get(Some(id)), Some(&codec));
        assert!(
            codecs
                .record(&logical, id, Some(CodecId(8)), Some(&codec))
                .is_err()
        );
        assert!(codecs.record(&logical, id, None, None).is_err());
        let distinct = cq_syntax::LogicalTypeId(4);
        codecs
            .record(&logical, distinct, Some(CodecId(8)), Some(&alias))
            .unwrap();
        assert_eq!(codecs.get(Some(distinct)), Some(&alias));
        assert_eq!(codecs.get(Some(id)), Some(&codec));

        let mut identity = Codecs::default();
        identity.record(&logical, id, None, None).unwrap();
        identity.record(&logical, id, None, None).unwrap();
        assert_eq!(identity.get(Some(id)), None);
        assert!(
            identity
                .record(&logical, id, Some(CodecId(7)), Some(&codec))
                .is_err()
        );
    }

    #[test]
    fn declared_logical_ids_drive_representation_and_reject_mismatches() {
        let Stack { physical, structures, program } = syn::parse_quote! {
            physical {
                relation R(a: Logical [5], b: Logical [6]) = (&data.r);
                r = pipe rename R {a -> x, b -> y};
                result = let canon project r keep {x,y};
                return result as answer(x,y);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                    packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 };
                }
                r host scan = @device_columns input { keys 0; columns [
                    [5] Logical => u32 codec [1] First,
                    [6] Logical => u32 codec [2] Second
                ] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32)> [];
                result rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                result canon = rows;
            }
            sequence {
                input = store rows r;
                result_rows = store canon map result flatten tile 256 input;
                output count;
            }
        };
        let recorded = record_representation(&physical, &structures, &program).unwrap();
        assert!(recorded.representation.encoded.contains("x"));
        assert!(recorded.representation.encoded.contains("y"));
        let analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let codecs = Codecs::check(&physical, &structures, &analysis.physical.rows).unwrap();
        assert_eq!(codecs.column("x"), Some(&syn::parse_quote!(First)));
        assert_eq!(codecs.column("y"), Some(&syn::parse_quote!(Second)));
        let mut changed = structures.clone();
        let Structure::Host { keyed, .. } = &mut changed.structures.entries[0].structure
        else {
            unreachable!()
        };
        keyed.provider.input.as_mut().unwrap().columns[0].logical_type_id =
            Some(cq_syntax::LogicalTypeId(6));
        assert!(record_representation(&physical, &changed, &program).is_err());
    }

    #[test]
    fn representation_provenance_does_not_require_single_column_intervals() {
        let Stack { physical, structures, program } = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64) = (&data.r);
                r = pipe rename R {a -> x,b -> y};
                selected = pipe select r if (x > 1,y > 2);
                result = let canon project selected keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
            }
            sequence {
                input = store rows r;
                result_rows = store canon map result compact selected flatten tile 256 input;
                output count;
            }
            graded gpu { input at host; result_rows at host.device; }
        };
        let recorded = record_representation(&physical, &structures, &program).unwrap();
        assert!(recorded.representation.identity.contains("x"));
        assert!(recorded.representation.identity.contains("y"));
        let analysis = cq_lang_sequence::check_grading(&physical, &structures, &recorded, &crate::machine()).unwrap();
        assert!(analysis.gpu_bindings.native.is_empty());
    }

    #[test]
    fn schema_and_codecs_do_not_depend_on_provider_spelling() {
        let provider: cq_lang_structures::Provider = syn::parse_quote! {
            @custom input {
                keys 1;
                columns [MyScalar => OpaqueScalar, Logical => Stored codec Codec]
            } as CompletelyUnrelatedAlias
        };
        let input = provider.input.as_ref().unwrap();
        assert_eq!((input.columns.len(), input.key_count), (2, 1));
        assert_eq!(input.columns[0].logical, syn::parse_quote!(MyScalar));
        assert_eq!(input.columns[1].codec, Some(syn::parse_quote!(Codec)));
        let printed = quote!(#provider);
        let roundtrip: cq_lang_structures::Provider = syn::parse2(printed).unwrap();
        assert_eq!(provider, roundtrip);
    }

    #[test]
    fn native_binding_reads_declared_abi_and_copies_the_input_instance() {
        let (physical, source) = syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
            let physical = input.parse()?;
            let structures = Program::parse(input, &physical)?;
            Ok((physical, structures))
        }, quote! {
            physical {
                relation R(x:i64) = (&data.r);
                relation D(x:i64) = (&data.d);
                d = pipe rename D {x -> x};
                r = let rename R {x -> x};
                joined = pipe probe first [x] d,r;
                result = let canon project joined keep {x};
                return result as answer(x);
            }
            structures {
                providers {
                    native = AnyDefaultSpelling { reads [exact, range, membership, resident], abi radix_relation };
                    index = OtherDefaultSpelling { reads [exact] };
                    host_scan = Host { reads [] };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> {
                        reads [distinct], build inferred
                    };
                }
                r host scan = @native input { keys 1; columns [i64 => StoredScalar] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as Alias [];
                r = @index as Index [x];
                d host scan = @host_scan as Host [];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
            }
        }).unwrap();
        let target = bind(&physical, &source).unwrap();
        let Structure::Host { keyed: host, .. } = &source.structures.entries[0].structure else {
            unreachable!()
        };
        let Structure::Built { keyed: built, .. } = &target.structures.entries[1].structure else {
            unreachable!()
        };
        assert_eq!(built.provider, host.provider);
        assert_eq!(built.provider.ty, syn::parse_quote!(Alias));
        for omit_advice in [false, true] {
            let mut stale = source.clone();
            let Structure::Host { keyed, .. } = &mut stale.structures.entries[0].structure else {
                unreachable!()
            };
            if omit_advice {
                keyed.provider.radix = None;
            } else {
                keyed.provider.input.as_mut().unwrap().key_count = 0;
            }
            assert!(cq_lang_structures::contract::check(&physical, &stale).is_err());
        }
    }
}
