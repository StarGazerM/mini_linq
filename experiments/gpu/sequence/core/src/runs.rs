//! Checked bindings for declared packed-run inputs.
use cq_lang_structures::provider::{CodecId, NativeAbi};
use cq_lang_structures::{Program, Structure, physical::symbol_name};
use quote::quote;
use std::collections::BTreeMap;
use syn::Type;

/// Compared columns share one checked encoding for each logical Rust type.
#[derive(Default)]
pub(crate) struct Codecs(Vec<(Type, Option<CodecId>, Option<Type>)>);

impl Codecs {
    pub fn get(&self, logical: &Type) -> Option<&Type> {
        self.0
            .iter()
            .find(|(ty, _, _)| ty == logical)
            .and_then(|(_, _, codec)| codec.as_ref())
    }

    /// Formal rule GPU-CodecIdentity: conflicting encodings of a logical symbol type are rejected before capture lowering.
    pub fn record(
        &mut self,
        logical: &Type,
        codec_id: Option<CodecId>,
        codec: Option<&Type>,
    ) -> syn::Result<()> {
        if let Some((_, previous, _)) = self.0.iter().find(|(ty, _, _)| ty == logical) {
            if *previous != codec_id {
                return Err(syn::Error::new_spanned(
                    logical,
                    "conflicting codecs for a logical symbol type",
                ));
            }
        } else {
            self.0.push((logical.clone(), codec_id, codec.cloned()));
        }
        Ok(())
    }

    pub fn check(program: &Program) -> syn::Result<Self> {
        let mut codecs = Self::default();
        for entry in &program.structures.entries {
            let keyed = match &entry.structure {
                Structure::Host { keyed, .. } | Structure::Built { keyed, .. } => keyed,
                _ => continue,
            };
            if let Some(input) = &keyed.provider.input {
                for column in &input.columns {
                    codecs.record(&column.logical, column.codec_id, column.codec.as_ref())?;
                }
            }
        }
        Ok(codecs)
    }
}

pub(crate) fn scalar(ty: &Type) -> proc_macro2::TokenStream {
    quote!(<#ty as ::gpu_datalog_runtime::Scalar>::COLUMN)
}

// Formal rule GPU-Runs: only declared run handles bind packed inputs; every
// lookup uses a leading packed prefix and every physical run remains visible.
pub fn bind(source: &Program) -> syn::Result<Program> {
    let analysis = cq_lang_structures::contract::check(source)?;
    let definitions = source
        .physical
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
        if analysis.facts(&keyed.provider).abi != Some(NativeAbi::Runs) {
            continue;
        }
        let input = keyed.provider.input.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(entry, "GPU-Runs: input schema is not recorded")
        })?;
        let types = &input.columns;
        let key_count = input.key_count;
        let host = &analysis.physical.hosts[&symbol_name(&entry.name)];
        let layout = &analysis.host_layouts[host];
        let row = &analysis.physical.rows[&symbol_name(&entry.name)];
        let origin =
            crate::rows::origin(&symbol_name(&entry.name), &definitions)?.ok_or_else(|| {
                syn::Error::new_spanned(entry, "GPU-Runs: scan has no declared input origin")
            })?;
        if layout.len() != types.len() {
            return Err(syn::Error::new_spanned(
                entry,
                "GPU-Runs: declared packed arity differs from input",
            ));
        }
        for (column, ty) in &row.types {
            if let Some(source) = origin.columns.get(column) {
                let position = layout.iter().position(|name| name == source).unwrap();
                let logical = &types[position].logical;
                if ty != logical {
                    return Err(syn::Error::new_spanned(
                        entry,
                        "GPU-Runs: declared packed column type differs from input",
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
            || keys
                .iter()
                .enumerate()
                .any(|(i, key)| origin.columns.get(key) != layout.get(i))
        {
            return Err(syn::Error::new_spanned(
                &entry.name,
                format!(
                    "GPU-Runs: lookup key {:?} (source columns {:?}) must be a leading prefix of sorted packed layout {:?} for input {}",
                    keys,
                    keys.iter()
                        .map(|key| origin.columns.get(key))
                        .collect::<Vec<_>>(),
                    layout,
                    origin.host,
                ),
            ));
        }
        keyed.provider = provider.clone();
    }
    target.structures.declare(&[])?;
    cq_lang_structures::contract::check(&target)?;
    Ok(target)
}

/// Formal rule GPU-Representation: provider and codec declarations, never
/// logical Rust type spellings, supply the facts checked by BE-RustRepresentation.
pub fn record_representation(
    source: &cq_lang_sequence::Program,
) -> syn::Result<cq_lang_sequence::Program> {
    let analysis = cq_lang_structures::contract::check(&source.structures)?;
    let mut hosts = BTreeMap::new();
    for entry in &source.structures.structures.entries {
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
                    "GPU-Runs: declared packed arity differs from input",
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
    for definition in &source.structures.physical.program.definitions {
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
    for definition in &source.structures.physical.program.definitions {
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
    for entry in &source.structures.structures.entries {
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
    let codecs = Codecs::check(&source.structures)?;
    for row in analysis.physical.rows.values() {
        for (column, logical) in &row.types {
            if codecs.get(logical).is_some() {
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

    #[test]
    fn authored_binding_annotations_record_identity_without_type_inspection() {
        let program: cq_lang_sequence::Program = syn::parse_quote! {
            physical {
                relation R(x:i64) = (&data.r);
                r = pipe rename R@0 {x -> x};
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
        let recorded = record_representation(&program).unwrap();
        assert!(recorded.representation.identity.contains("value"));
        let facts = cq_lang_sequence::contract::check(&recorded).unwrap();
        let mut operator = recorded.structures.physical.program.definitions[1]
            .operator
            .clone();
        assert!(!facts.rust_requires_host(&operator));
        let cq_lang_structures::physical::Operator::Bind(bind) = &mut operator else {
            unreachable!()
        };
        bind.annotation = None;
        assert!(facts.rust_requires_host(&operator));
    }

    #[test]
    fn codec_identity_uses_ids_not_type_payloads() {
        let logical = syn::parse_quote!(Logical);
        let codec = syn::parse_quote!(Codec);
        let alias = syn::parse_quote!(AnotherSpelling);
        let mut codecs = Codecs::default();
        codecs
            .record(&logical, Some(CodecId(7)), Some(&codec))
            .unwrap();
        codecs
            .record(&logical, Some(CodecId(7)), Some(&alias))
            .unwrap();
        assert_eq!(codecs.get(&logical), Some(&codec));
        assert!(
            codecs
                .record(&logical, Some(CodecId(8)), Some(&codec))
                .is_err()
        );
        assert!(codecs.record(&logical, None, None).is_err());

        let mut identity = Codecs::default();
        identity.record(&logical, None, None).unwrap();
        identity.record(&logical, None, None).unwrap();
        assert_eq!(identity.get(&logical), None);
        assert!(
            identity
                .record(&logical, Some(CodecId(7)), Some(&codec))
                .is_err()
        );
    }

    #[test]
    fn representation_provenance_does_not_require_single_column_intervals() {
        let program: cq_lang_sequence::Program = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64) = (&data.r);
                r = pipe rename R@0 {a -> x,b -> y};
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
        let recorded = record_representation(&program).unwrap();
        assert!(recorded.representation.identity.contains("x"));
        assert!(recorded.representation.identity.contains("y"));
        let analysis = cq_lang_sequence::check_grading(&recorded, &crate::machine()).unwrap();
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
        let source: Program = syn::parse_quote! {
            physical {
                relation R(x:i64) = (&data.r);
                relation D(x:i64) = (&data.d);
                d = pipe rename D@1 {x -> x};
                r = let rename R@0 {x -> x};
                joined = pipe probe first [x] d,r;
                result = let canon project joined keep {x};
                return result as answer(x);
            }
            structures {
                providers {
                    native = AnyDefaultSpelling { reads [exact, range, membership, resident], abi runs };
                    index = OtherDefaultSpelling { reads [exact] };
                    host_scan = Host { reads [] };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> {
                        reads [distinct], build inferred
                    };
                }
                r host scan = @native input { keys 1; columns [i64 => StoredScalar] } as Alias [];
                r = @index as Index [x];
                d host scan = @host_scan as Host [];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
            }
        };
        let target = bind(&source).unwrap();
        let Structure::Host { keyed: host, .. } = &source.structures.entries[0].structure else {
            unreachable!()
        };
        let Structure::Built { keyed: built, .. } = &target.structures.entries[1].structure else {
            unreachable!()
        };
        assert_eq!(built.provider, host.provider);
        assert_eq!(built.provider.ty, syn::parse_quote!(Alias));
    }
}
