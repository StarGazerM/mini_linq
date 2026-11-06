//! Projection of recorded dataset input through checked Round relation views.
//! This maps identities only; no number is interpreted and no order is selected.
use crate::{InputSource, RoundContext, storage};
use cq_syntax::{RelationDecl, declared_evidence::{DeclaredEvidence, EvidenceView}};

/// Map the source dataset attachment onto exactly this hole's declared views.
/// Canonical providers, delta layouts, scans and filtered key sets retain their
/// source relation/column correspondence; synthetic unit keys have no mapping.
pub fn for_hole(
    source: &storage::Program,
    context: &RoundContext,
    hole: usize,
    declarations: &[RelationDecl],
) -> syn::Result<Option<DeclaredEvidence>> {
    let Some(evidence) = &source.incremental.strata.source.declared_evidence else {
        return Ok(None);
    };
    let scan = context.strata.iter().flat_map(|s| s.seeds.iter().chain(&s.steps))
        .find(|rule| rule.hole == hole).and_then(|rule| rule.scan.as_ref());
    let mut bindings = Vec::new();
    for declaration in declarations {
        let mut origin = None;
        if let Some(scan) = scan.filter(|scan| scan.binding == declaration.name) {
            origin = Some(match scan.source {
                InputSource::Delta(relation) => (relation, EvidenceView::Delta),
                InputSource::Layout(index) => {
                    let layout = context.layouts.get(index).ok_or_else(|| {
                        syn::Error::new_spanned(declaration, "evidence scan references a missing layout")
                    })?;
                    (layout.relation, EvidenceView::Full)
                }
            });
        }
        if origin.is_none() {
            origin = context.relations.iter().enumerate()
                .find(|(_, r)| r.delta.as_ref() == Some(&declaration.name))
                .map(|(index, _)| (index, EvidenceView::Delta));
        }
        if origin.is_none() {
            origin = context.layouts.iter().find(|l| l.name == declaration.name)
                .map(|layout| (layout.relation, EvidenceView::Full));
        }
        if origin.is_none() {
            origin = context.delta_layouts.iter().find(|l| l.name == declaration.name)
                .map(|layout| (layout.relation, EvidenceView::Delta));
        }
        if origin.is_none() {
            origin = context.strata.iter().flat_map(|s| &s.keys)
                .find(|key| key.name == declaration.name && !key.columns.is_empty())
                .map(|key| (key.relation, EvidenceView::Subset));
        }
        let Some((index, mut view)) = origin else { continue };
        let relation = context.relations.get(index).ok_or_else(|| {
            syn::Error::new_spanned(declaration, "evidence view references a missing relation")
        })?;
        if declaration.columns.len() < relation.declaration.columns.len() && view == EvidenceView::Full {
            view = EvidenceView::Subset;
        }
        if let Some(binding) = evidence.project_relation(&relation.declaration.name, declaration, view)? {
            bindings.push(binding);
        }
    }
    let projected = DeclaredEvidence { document: evidence.document.clone(), bindings };
    projected.check(declarations)?;
    Ok(Some(projected))
}

/// Iteration is owned by the checked component, not merely by the intrinsic
/// source SCC: a legal union of SCCs also iterates its acyclic members.
pub fn check_iteration_scope(program: &crate::Program) -> syn::Result<()> {
    let source = &program.storage.incremental.strata.source;
    let Some(evidence) = &source.declared_evidence else { return Ok(()) };
    for binding in &evidence.bindings {
        let relation = source.declarations.iter().position(|d| d.name == binding.relation)
            .ok_or_else(|| syn::Error::new_spanned(binding, "evidence names an absent source relation"))?;
        if source.declarations[relation].value.is_some() { continue; }
        let (index, component) = program.context.strata.iter().enumerate()
            .find(|(_, component)| component.members.contains(&relation))
            .ok_or_else(|| syn::Error::new_spanned(binding, "derived evidence has no checked Round component"))?;
        let facts = &evidence.document.relations[&binding.source];
        let iterated = !component.steps.is_empty();
        let expected = if iterated {
            cq_syntax::evidence::RelationScope::RecursiveFinal
        } else {
            cq_syntax::evidence::RelationScope::Frozen
        };
        if facts.scope != expected {
            return Err(syn::Error::new_spanned(binding, "evidence iteration scope disagrees with its checked Round component"));
        }
        if let Some(recursive) = &facts.recursive {
            if recursive.stratum != index.to_string() {
                return Err(syn::Error::new_spanned(binding, "evidence stratum must name its checked Round component index"));
            }
        }
    }
    Ok(())
}
