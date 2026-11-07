use cq_lang_sequence::structures::{CanonSource, Capability, Provider};
use cq_lang_sequence::{Analysis, Catalog, Obligation, Program};

pub(crate) fn check(program: &Program, analysis: &Analysis, catalog: &Catalog) -> syn::Result<()> {
    // Formal rule PL-Catalog: a program declaration of a token the target
    // declares is that declaration; the catalog is keyed by token id.
    for declaration in &analysis.structures.providers.declarations {
        if catalog
            .declaration(&declaration.token)
            .is_some_and(|declared| declared != declaration)
        {
            return Err(syn::Error::new_spanned(
                &declaration.token,
                format!(
                    "PL-Catalog: declaration of provider token `{}` differs from the target catalog",
                    declaration.token
                ),
            ));
        }
    }
    // Positional selection is a constructor contract, not a Membership read
    // obligation. Recheck its recorded token even when planning skips PL-Select.
    for selection in analysis.structures.selections.values() {
        if !catalog.providers.iter().any(|entry| {
            entry.covers(&selection.provider) && !entry.levels.is_empty()
        }) {
            return Err(syn::Error::new_spanned(
                &selection.provider,
                "PL-Catalog: recorded positional selection constructor is not admitted by the target catalog",
            ));
        }
    }
    for obligation in &analysis.obligations {
        let available = match obligation {
            Obligation::Provider {
                provider,
                capability,
                instance,
                ..
            } => provider_available(analysis, catalog, provider, capability, instance),
            // The canon boundary is admitted at its recorded graded level only.
            Obligation::Canon {
                canonical,
                answer_index,
                level,
                ..
            } => level.as_ref().is_some_and(|level| {
                canon_available(
                    catalog,
                    canonical.as_ref(),
                    answer_index.as_ref(),
                    |registered| registered.contains(level),
                )
            }),
            Obligation::Codec {
                ordered, codec_id, ..
            } => codec_available(analysis, catalog, *codec_id, *ordered),
            Obligation::Failure { .. } => true,
        };
        if !available {
            let reads = if let Obligation::Provider { provider, .. } = obligation {
                analysis
                    .provider_reads
                    .iter()
                    .filter(|(_, providers)| providers.contains(provider))
                    .map(|(name, _)| format!("{name}@{}", analysis.levels[name].join(".")))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                String::new()
            };
            return Err(syn::Error::new(
                program.structures.physical.program.returned.span(),
                format!("P-Decompose: unmet catalog obligation {obligation:?}; reads [{reads}]"),
            ));
        }
    }
    Ok(())
}

/// Formal rule PL-Distinct: whether the target discharges a canon boundary
/// reading `canonical` (`None`: the returned answer boundary, reading its
/// recorded identity index `answer_index`). `admits` tells whether a
/// registered level set covers the boundary's execution level.
///
/// A canonical rows record, or the answer's identity index, needs a catalog
/// entry of its recorded token declaring Distinct; an answer boundary without
/// a recorded identity index is unmet. A Built reference is discharged by its
/// own Provider obligations; a host carrier is read only at host.
pub(crate) fn canon_available(
    catalog: &Catalog,
    canonical: Option<&CanonSource>,
    answer_index: Option<&Provider>,
    admits: impl Fn(&[Vec<String>]) -> bool,
) -> bool {
    let provider = match canonical {
        Some(CanonSource::Rows(provider)) => provider,
        Some(CanonSource::Index(_)) => return true,
        Some(CanonSource::Host) => return admits(&[vec!["host".to_owned()]]),
        None => match answer_index {
            Some(provider) => provider,
            None => return false,
        },
    };
    catalog.providers.iter().any(|entry| {
        entry.covers(provider)
            && catalog
                .facts(&entry.provider)
                .is_some_and(|facts| facts.reads(&Capability::Distinct))
            && admits(&entry.levels)
    })
}

/// A BE-Code obligation needs the catalog token of its exact recorded codec
/// (`None`: identity), which must certify monotone encoding when `ordered`
/// and register every graded level; no other token discharges it.
pub(crate) fn codec_available(
    analysis: &Analysis,
    catalog: &Catalog,
    codec_id: Option<cq_lang_sequence::structures::CodecId>,
    ordered: bool,
) -> bool {
    catalog.codecs.iter().any(|token| {
        token.codec_id == codec_id
            && (!ordered || token.ordered)
            && analysis
                .levels
                .values()
                .all(|level| token.levels.contains(level))
    })
}

/// Check the complete target promise, including scheduled read levels.
pub(crate) fn provider_available(
    analysis: &Analysis,
    catalog: &Catalog,
    provider: &str,
    capability: &Capability,
    instance: &Provider,
) -> bool {
    let levels = provider_levels(analysis, provider);
    provider_admitted(
        analysis,
        catalog,
        provider,
        capability,
        instance,
        levels.iter().any(|level| level.len() > 1),
        |registered| levels.iter().all(|level| registered.contains(level)),
    )
}

/// The single provider admission predicate shared by the final catalog check
/// and placement's prediction of a read level. `below_host` tells whether a
/// read executes below host; `admits` whether a registered level set covers
/// the read levels. The catalog entry of the recorded instance's tokens
/// supplies the levels; its declared facts the capabilities.
pub(crate) fn provider_admitted(
    analysis: &Analysis,
    catalog: &Catalog,
    provider: &str,
    capability: &Capability,
    instance: &Provider,
    below_host: bool,
    admits: impl Fn(&[Vec<String>]) -> bool,
) -> bool {
    // Formal rule PL-Resident: Only an explicit resident token admits an
    // unretained host handle below host; all read/level facts still apply.
    let needs_resident = analysis.structures.host_read(provider) && below_host;
    catalog.providers.iter().any(|entry| {
        entry.covers(instance)
            && catalog.facts(&entry.provider).is_some_and(|facts| {
                (!needs_resident || facts.reads(&Capability::Resident)) && facts.reads(capability)
            })
            && admits(&entry.levels)
    })
}

fn provider_levels(analysis: &Analysis, provider: &str) -> Vec<Vec<String>> {
    let mut levels = Vec::new();
    for (name, providers) in &analysis.provider_reads {
        if providers.contains(provider) {
            if let Some(level) = analysis.levels.get(name) {
                let level = analysis
                    .dispatches
                    .iter()
                    .find(|dispatch| dispatch.inner_stores.contains(name))
                    .map_or(level, |dispatch| &dispatch.producer_level);
                if !levels.contains(level) {
                    levels.push(level.clone());
                }
            }
        }
    }
    levels
}
#[cfg(test)]
mod tests {
    use super::*;
    use cq_lang_sequence::structures::Declaration;

    #[test]
    fn resident_host_reads_require_explicit_complete_catalog_facts() {
        let structures = syn::parse_quote! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i32);
                l = pipe rename L {a -> a};
                r = pipe rename R {a -> a,b -> b};
                answer = let canon probe first [a] l,r;
                return answer as answer(a,b);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; opaque_native = OpaqueNative<i32> { reads [exact, range, membership], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                l host scan = @reader as Reader [];
                r host scan = @opaque_native as OpaqueNative<i32> [];
                r host result(answer) = @opaque_native as OpaqueNative<i32> [a];
                answer rows = @sort as ::cq_storage::dedup::Sort;
                answer canon = rows;
            }
        };
        let program = cq_sequence_lowering::compile(&structures).unwrap();
        let mut analysis = cq_lang_sequence::contract::check(&program).unwrap();
        let level = vec!["host".into(), "device".into(), "block".into()];
        for (reader, providers) in &analysis.provider_reads {
            if providers.contains("r") {
                analysis.levels.insert(reader.clone(), level.clone());
            }
        }
        let instance = syn::parse_quote!(@opaque_native as OpaqueNative<i32>);
        let declaration: Declaration = syn::parse_quote!(
            opaque_native = OpaqueNative<_> { reads [exact, range, membership], build typed };
        );
        let mut catalog = Catalog::default();
        catalog.register(declaration, vec![level]);
        for capability in [Capability::Exact, Capability::Range, Capability::Membership] {
            assert!(!provider_available(
                &analysis,
                &catalog,
                "r",
                &capability,
                &instance
            ));
        }
        catalog.declarations[0]
            .facts
            .reads
            .push(Capability::Resident);
        for capability in [Capability::Exact, Capability::Range, Capability::Membership] {
            assert!(provider_available(
                &analysis,
                &catalog,
                "r",
                &capability,
                &instance
            ));
        }
        let mut missing = catalog.clone();
        missing.declarations[0]
            .facts
            .reads
            .retain(|c| *c != Capability::Membership);
        assert!(!provider_available(
            &analysis,
            &missing,
            "r",
            &Capability::Membership,
            &instance
        ));
        let mut wrong_level = catalog.clone();
        wrong_level.providers[0].levels = vec![vec!["host".into()]];
        assert!(!provider_available(
            &analysis,
            &wrong_level,
            "r",
            &Capability::Range,
            &instance
        ));
        assert!(!provider_available(
            &analysis,
            &catalog,
            "r",
            &Capability::Range,
            &syn::parse_quote!(@unrelated as OpaqueNative<i32>)
        ));
    }

    /// `L` declares `codec Codec` for its i32 column; `b` is an i64 column
    /// without a declared codec.
    fn encoded() -> Program {
        let structures = syn::parse_quote! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i64);
                l = pipe rename L {a -> a};
                r = pipe rename R {a -> a,b -> b};
                answer = let canon probe first [a] l,r;
                return answer as answer(a,b);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; opaque_native = OpaqueNative<i32> { reads [exact, range, membership], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                l host scan = @reader input { keys 0; columns [i32 => Code codec Codec] } as Reader [];
                r host scan = @opaque_native as OpaqueNative<i32> [];
                r host result(answer) = @opaque_native as OpaqueNative<i32> [a];
                answer rows = @sort as ::cq_storage::dedup::Sort;
                answer canon = rows;
            }
        };
        cq_sequence_lowering::compile(&structures).unwrap()
    }

    fn at(level: &[&str]) -> impl Fn(&[Vec<String>]) -> bool {
        let level: Vec<String> = level.iter().map(|name| (*name).to_owned()).collect();
        move |registered| registered.contains(&level)
    }

    #[test]
    fn codec_obligations_record_the_declared_codec_and_admit_only_its_token() {
        let mut analysis = cq_lang_sequence::contract::check(&encoded()).unwrap();
        let declared = cq_lang_sequence::structures::CodecId(0);
        let codecs: Vec<_> = analysis
            .obligations
            .iter()
            .filter_map(|obligation| match obligation {
                Obligation::Codec {
                    columns,
                    ordered,
                    codec_id,
                    ..
                } => Some((columns.clone(), *ordered, *codec_id)),
                _ => None,
            })
            .collect();
        // The canon heading splits by the recorded codec of each column.
        assert!(
            codecs.contains(&(vec!["a".into()], false, Some(declared))),
            "{codecs:?}"
        );
        assert!(
            codecs.contains(&(vec!["b".into()], false, None)),
            "{codecs:?}"
        );
        let token = |codec_id: Option<cq_lang_sequence::structures::CodecId>, ordered| {
            cq_lang_sequence::CodecToken {
                codec: codec_id.map(|_| syn::parse_quote!(Codec)),
                codec_id,
                ordered,
                levels: vec![vec!["host".into()]],
            }
        };
        let mut catalog = Catalog {
            codecs: vec![token(None, true)],
            ..Catalog::default()
        };
        // Legal: identity columns use the identity token.
        assert!(codec_available(&analysis, &catalog, None, true));
        // Illegal: neither identity nor another codec discharges `Codec`.
        assert!(!codec_available(&analysis, &catalog, Some(declared), false));
        catalog
            .codecs
            .push(token(Some(cq_lang_sequence::structures::CodecId(1)), true));
        assert!(!codec_available(&analysis, &catalog, Some(declared), false));
        // Legal: the exact token, certifying order only when it declares it.
        catalog.codecs.push(token(Some(declared), false));
        assert!(codec_available(&analysis, &catalog, Some(declared), false));
        assert!(!codec_available(&analysis, &catalog, Some(declared), true));
        // A later token may supply order even when the first matching token does not.
        catalog.codecs.push(token(Some(declared), true));
        assert!(codec_available(&analysis, &catalog, Some(declared), true));
        // Illegal: a graded level the token does not register.
        analysis
            .levels
            .insert("answer".into(), vec!["host".into(), "device".into()]);
        assert!(!codec_available(&analysis, &catalog, None, true));
        let mut graded = token(Some(declared), true);
        graded.levels.push(vec!["host".into(), "device".into()]);
        catalog.codecs.push(graded);
        assert!(codec_available(&analysis, &catalog, Some(declared), true));
    }

    #[test]
    fn codec_admission_rejects_a_mutated_id_without_inspecting_its_spelling() {
        use cq_lang_sequence::structures::CodecId;
        let analysis = cq_lang_sequence::contract::check(&encoded()).unwrap();
        let mut catalog = Catalog {
            codecs: vec![cq_lang_sequence::CodecToken {
                codec_id: Some(CodecId(0)),
                codec: Some(syn::parse_quote!(Codec)),
                ordered: true,
                levels: vec![vec!["host".into()]],
            }],
            ..Catalog::default()
        };
        assert!(codec_available(&analysis, &catalog, Some(CodecId(0)), true));
        catalog.codecs[0].codec_id = Some(CodecId(1));
        assert!(!codec_available(
            &analysis,
            &catalog,
            Some(CodecId(0)),
            true
        ));
        // Equal spelling never merges explicitly different declarations.
        assert!(codec_available(&analysis, &catalog, Some(CodecId(1)), true));
        catalog.codecs[0].codec = Some(syn::parse_quote!(AnotherOpaqueSpelling));
        assert!(codec_available(&analysis, &catalog, Some(CodecId(1)), true));
    }

    #[test]
    fn same_spelling_with_distinct_declared_ids_is_a_conflicting_encoding() {
        let mut program = encoded();
        program.structures.structures.entries[1]
            .structure
            .provider_mut()
            .unwrap()
            .input = Some(syn::parse_quote!(input {
            keys 0; columns [[0] i32 => Code codec [0] Codec, [1] i64 => i64]
        }));
        assert!(cq_lang_sequence::contract::check(&program).is_ok());
        let input = program.structures.structures.entries[1]
            .structure
            .provider_mut()
            .unwrap()
            .input
            .as_mut()
            .unwrap();
        // The same spelling under an explicitly different declaration is not
        // the same encoding of the shared logical i32 column.
        input.columns[0].codec_id = Some(cq_lang_sequence::structures::CodecId(1));
        assert!(cq_lang_sequence::contract::check(&program).is_err());
    }

    #[test]
    fn equal_logical_spellings_with_distinct_ids_select_distinct_codecs() {
        let structures = syn::parse_quote! {
            physical {
                relation L(a:i32 [0]); relation R(a:i32 [0],b:i32 [1]);
                l = pipe rename L {a -> a};
                r = pipe rename R {a -> a,b -> b};
                answer = let canon probe first [a] l,r;
                return answer as answer(a,b);
            }
            structures { providers {
                reader = Reader { reads [exact, range, membership], build inferred };
                sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
            }
                l host scan = @reader input { keys 0; columns [[0] i32 => Code codec [0] Codec] } as Reader [];
                r host scan = @reader input { keys 0; columns [[0] i32 => Code codec [0] Codec, [1] i32 => Code codec [1] Codec] } as Reader [];
                r host result(answer) = @reader as Reader [a];
                answer rows = @sort as ::cq_storage::dedup::Sort;
                answer canon = rows;
            }
        };
        let program = cq_sequence_lowering::compile(&structures).unwrap();
        let analysis = cq_lang_sequence::contract::check(&program).unwrap();
        for (column, id) in [("a", 0), ("b", 1)] {
            assert!(analysis.obligations.iter().any(|obligation| matches!(
                obligation,
                Obligation::Codec { columns, codec_id: Some(recorded), .. }
                    if columns == &[column.to_owned()] && recorded.0 == id
            )));
        }
    }

    #[test]
    fn codec_declarations_require_recorded_ids_after_parsing() {
        let mut program = encoded();
        let input = program
            .structures
            .structures
            .entries
            .iter_mut()
            .find_map(|entry| entry.structure.provider_mut()?.input.as_mut())
            .unwrap();
        input.columns[0].codec_id = None;
        assert!(cq_lang_sequence::contract::check(&program).is_err());
    }

    #[test]
    fn logical_type_identity_mutations_are_rejected() {
        for logical_type_id in [None, Some(cq_lang_sequence::structures::LogicalTypeId(99))] {
            let mut program = encoded();
            let input = program
                .structures
                .structures
                .entries
                .iter_mut()
                .find_map(|entry| entry.structure.provider_mut()?.input.as_mut())
                .unwrap();
            input.columns[0].logical_type_id = logical_type_id;
            assert!(cq_lang_sequence::contract::check(&program).is_err());
        }
    }

    #[test]
    fn canon_admission_reads_the_recorded_token_and_graded_level() {
        let program = encoded();
        let mut analysis = cq_lang_sequence::contract::check(&program).unwrap();
        let sort: Declaration = syn::parse_quote!(
            sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
        );
        let instance = sort.instance();
        let mut catalog = Catalog::default();
        catalog.register(sort, vec![vec!["host".into()]]);
        // Illegal: an answer boundary without a recorded identity index is
        // unmet although a Distinct entry exists.
        assert!(!canon_available(&catalog, None, None, at(&["host"])));
        // Legal: its recorded identity index at a registered level.
        assert!(canon_available(
            &catalog,
            None,
            Some(&instance),
            at(&["host"])
        ));
        assert!(!canon_available(
            &catalog,
            None,
            Some(&instance),
            at(&["host", "device"])
        ));
        assert!(!canon_available(
            &catalog,
            None,
            Some(&syn::parse_quote!(@unrelated as ::cq_storage::dedup::Sort)),
            at(&["host"])
        ));
        // The final check reads the level recorded by grading, never a default.
        analysis
            .obligations
            .retain(|obligation| matches!(obligation, Obligation::Canon { .. }));
        assert!(analysis.obligations.iter().all(|obligation| matches!(
            obligation,
            Obligation::Canon {
                canonical: Some(CanonSource::Rows(_)),
                level: None,
                ..
            }
        )));
        assert!(check(&program, &analysis, &catalog).is_err());
        let mut record = |recorded: &[&str]| {
            for obligation in &mut analysis.obligations {
                if let Obligation::Canon { level, .. } = obligation {
                    *level = Some(recorded.iter().map(|name| (*name).to_owned()).collect());
                }
            }
            check(&program, &analysis, &catalog)
        };
        record(&["host"]).unwrap();
        assert!(record(&["host", "device"]).is_err());
    }
}
