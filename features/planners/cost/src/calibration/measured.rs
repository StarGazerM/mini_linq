use super::*;

pub(super) fn provenance(run: &ev::RunIdentity, locator: String, method: &str) -> ev::Provenance {
    ev::Provenance {
        producer: "cq-cost planner calibration".into(), method: method.into(), locator,
        run: Some(run.clone()), artifact_hash: None,
    }
}
pub(super) fn count(id: String, value: u64, unit: ev::Unit, hash: &str, run: &ev::RunIdentity, locator: &str) -> ev::CountFact {
    ev::Fact {
        id, source: ev::Source::Calibrated, claim: ev::Claim::Equal, value: Some(value), lower: None, upper: None,
        dataset_hash: hash.into(), provenance: provenance(run, locator.into(), "complete generated insert-only-set invocation; exact counters"), unit,
    }
}

pub(super) fn finish(
    program: &round::Program,
    baseline: &ev::DatasetEvidence,
    request: &DatasetRequest,
    mut captured: Captured,
    run: &ev::RunIdentity,
) -> Result<(ev::DatasetEvidence, DeclaredEvidence)> {
    let source = &program.storage.incremental.strata.source;
    let original = source.declared_evidence.as_ref().ok_or_else(|| invalid("missing source attachment"))?;
    let mut manifest = baseline.dataset.manifest.clone();
    let mut bindings = original.bindings.clone();
    let mut derived = BTreeMap::new();
    for (index, declaration) in source.declarations.iter().enumerate().filter(|(_, d)| d.value.is_none()) {
        let existing = bindings.iter().find(|b| b.relation == declaration.name);
        let (name, columns) = if let Some(existing) = existing {
            require(existing.view == EvidenceView::Full, "derived source binding must be full")?;
            (existing.source.clone(), existing.columns.clone())
        } else {
            let name = format!("calibrated:{index}:{}", declaration.name);
            require(!manifest.schema.contains_key(&name), "generated derived evidence identity collides with an existing dataset relation")?;
            let columns = declaration.columns.iter().map(|c| ColumnBinding { column: c.name.clone(), source: c.name.to_string() }).collect::<Vec<_>>();
            bindings.push(RelationBinding { relation: declaration.name.clone(), source: name.clone(), view: EvidenceView::Full, columns: columns.clone() });
            (name, columns)
        };
        require(columns.len() == declaration.columns.len(), "row-only derived calibration requires complete source column bindings")?;
        let domains = columns.iter().map(|column| {
            let previous = manifest.schema.get(&name).and_then(|schema| schema.columns.get(&column.source));
            (column.source.clone(), previous.cloned().unwrap_or_else(|| ev::ScalarDomain::Unordered {
                limitation: "only final cardinality measured; no scalar, degree or distinct observation for this derived column".into(),
            }))
        }).collect();
        manifest.schema.insert(name.clone(), ev::ManifestRelation {
            semantics: ev::RelationSemantics::Set, columns: domains,
            null_handling: "row-only; scalar values not inspected".into(),
            set_certification: Some(ev::SetCertification::FullTupleDeduplicated),
        });
        derived.insert(index, name);
    }
    // Input values are observed before canonical dedup; set frequencies use the
    // actual successful admission callback. Hashes must match the planned input.
    for input in captured.inputs.values() {
        let actual = input.content_hash.as_ref().ok_or_else(|| invalid("missing input digest"))?;
        let file = manifest.files.get_mut(&input.path).ok_or_else(|| invalid("input digest has no manifest file"))?;
        if let Some(expected) = &file.content_hash {
            require(expected == actual, format!("decoded input identity differs for {}", input.path))?;
        } else {
            require(file.missing_as_empty && input.summary.is_some(), "unbound data digest is not an explicit empty input")?;
            // A generated zero-row traversal gives the empty decoded stream a
            // concrete semantic digest; it does not assert filesystem presence.
            let mut empty_spec = request.relations.iter().find(|r| r.relation.name == input.dataset_relation)
                .ok_or_else(|| invalid("missing empty input request"))?.relation.clone();
            empty_spec.semantics = ev::RelationSemantics::Set;
            let empty = cq_load::stats::CanonicalAccumulator::new(empty_spec).map_err(|e| invalid(e.to_string()))?;
            require(&empty.content_hash() == actual, "expected missing-as-empty input produced rows")?;
        }
        file.content_hash = Some(actual.clone());
        let schema = manifest.schema.get_mut(&input.dataset_relation).ok_or_else(|| invalid("missing input schema"))?;
        schema.semantics = ev::RelationSemantics::Set;
        schema.set_certification = Some(ev::SetCertification::FullTupleDeduplicated);
        let requested = request.relations.iter().find(|r| r.relation.name == input.dataset_relation).ok_or_else(|| invalid("missing input request"))?;
        let mut recorded = requested.clone();
        recorded.relation.semantics = ev::RelationSemantics::Set;
        manifest.loader_options.insert(format!("relation:{}", input.dataset_relation), serde_json::to_string(&recorded).map_err(|e| invalid(e.to_string()))?);
    }
    manifest.loader_options.insert("calibration-input-boundary".into(), "DL-InsertNew: raw order/duplicates hashed; exact successful canonical admissions counted".into());
    let dataset_hash = manifest.dataset_hash().map_err(evidence_error)?;
    let schema_hash = manifest.schema_hash().map_err(evidence_error)?;
    let mut evidence = ev::DatasetEvidence {
        schema: ev::SCHEMA.into(),
        dataset: ev::DatasetIdentity { name: baseline.dataset.name.clone(), hash: dataset_hash.clone(), date: request.date.clone(), schema_hash,
            formulation: baseline.dataset.formulation.clone(), manifest },
        relations: BTreeMap::new(), premises: Vec::new(), relative: Vec::new(),
        targets: baseline.targets.clone(), observations: ev::Observations::default(),
    };
    for input in captured.inputs.values_mut() {
        let mut provenance = provenance(run, input.path.clone(), "typed-row-stream-v1 before dedup; exact column/key frequencies inside successful DL-InsertNew; checked against canonical cardinality");
        provenance.artifact_hash = input.content_hash.clone();
        let summary = input.summary.take().ok_or_else(|| invalid("incomplete input summary"))?;
        evidence.relations.insert(input.dataset_relation.clone(), summary.bind(&dataset_hash, ev::Source::Calibrated, provenance).map_err(|e| invalid(e.to_string()))?);
    }
    for (stratum, observations) in captured.strata.iter().enumerate() {
        let final_rows = observations.final_rows.as_ref().ok_or_else(|| invalid("incomplete stratum"))?;
        for (&relation, &(full, tail)) in final_rows {
            let name = derived.get(&relation).ok_or_else(|| invalid("stratum member lacks derived evidence identity"))?;
            let locator = format!("stratum:{stratum}/relation:{relation}");
            let id = format!("calibration:{}:{name}", run.run_id);
            let recursive = !program.context.strata[stratum].steps.is_empty();
            let final_fact = count(format!("{id}:final_rows"), full, ev::Unit::Rows, &dataset_hash, run, &locator);
            let recursive_evidence = recursive.then(|| ev::RecursiveEvidence {
                stratum: stratum.to_string(), final_rows: final_fact.clone(),
                rounds: count(format!("{id}:rounds"), observations.deltas.len() as u64, ev::Unit::Rounds, &dataset_hash, run, &locator),
                round_convention: ev::ROUND_CONVENTION.into(), run: run.clone(),
                seed_rows: Some(count(format!("{id}:seed_rows"), observations.seed.as_ref().unwrap()[&relation], ev::Unit::Rows, &dataset_hash, run, &locator)),
                unconsumed_final_delta_rows: Some(count(format!("{id}:unconsumed_delta"), tail, ev::Unit::Rows, &dataset_hash, run, &locator)),
            });
            let columns = evidence.dataset.manifest.schema[name].columns.iter().map(|(name, domain)| (name.clone(), ev::ColumnEvidence {
                domain: domain.clone(), distinct: None, min: None, max: None, max_degree: None,
            })).collect();
            evidence.relations.insert(name.clone(), ev::RelationEvidence {
                semantics: ev::RelationSemantics::Set,
                scope: if recursive { ev::RelationScope::RecursiveFinal } else { ev::RelationScope::Frozen },
                columns, rows: (!recursive).then_some(final_fact), key_sets: Vec::new(), keys: Vec::new(), recursive: recursive_evidence,
            });
            if recursive {
                evidence.observations.rounds.push(ev::RoundObservations {
                    relation: name.clone(), stratum: stratum.to_string(), run: run.clone(), complete: true,
                    entries: observations.deltas.iter().enumerate().map(|(round, delta)| ev::RoundObservation {
                        round: count(format!("{id}:round:{}:index", round + 1), (round + 1) as u64, ev::Unit::Rounds, &dataset_hash, run, &locator),
                        delta_rows: count(format!("{id}:round:{}:delta", round + 1), delta[&relation], ev::Unit::Rows, &dataset_hash, run, &locator),
                    }).collect(),
                });
                evidence.premises.push(ev::Premise::DeltaSubset {
                    id: format!("{id}:delta_subset"), relations: vec![name.clone()], source: ev::Source::Declared,
                    dataset_hash: dataset_hash.clone(), provenance: provenance(run, locator.clone(), "checked insert-only set admission; every observed delta is a subset of final canonical rows"),
                });
                if tail == 0 {
                    evidence.premises.push(ev::Premise::DeltaPartition {
                        id: format!("{id}:delta_partition"), relation: name.clone(), includes_seed_delta: true,
                        insert_only_set: true, complete_consumption: true, source: ev::Source::Declared,
                        dataset_hash: dataset_hash.clone(), provenance: provenance(run, locator.clone(), "all executed deltas, including the seed delta, are disjoint and sum to observed final cardinality; no unconsumed tail"),
                    });
                }
            }
        }
    }
    for target in evidence.targets.values_mut() {
        for fact in [&mut target.build_per_row_h, &mut target.merge_per_row_m] {
            require(fact.source == ev::Source::Declared, "calibration preserves only explicitly declared target unit costs")?;
            if fact.dataset_hash != dataset_hash {
                fact.dataset_hash = dataset_hash.clone();
                fact.provenance.method.push_str(&format!("; retained declared policy from dataset {} after measured-schema enrichment", baseline.dataset.hash));
            }
        }
    }
    evidence.check().map_err(evidence_error)?;
    let attachment = DeclaredEvidence { document: Arc::new(evidence.clone()), bindings };
    attachment.check(&source.declarations)?;
    Ok((evidence, attachment))
}
