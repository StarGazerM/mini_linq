//! Dependency graphs of the stages the facade recorded. The calibrator parses and
//! checks each printed program itself; it never infers lineage from generated Rust.
use cq_stage::{Annotation, Language};
use cq_stage::calibration::{Node, Stage};
use cq_syntax::symbol_name;
use proc_macro2::Span;
use quote::{ToTokens, quote};
use syn::{Ident, parse::Parser};

/// Parse and check a recorded stage as language `L`.
pub fn checked<L: Language>(stage: &Stage) -> syn::Result<L> {
    if !stage.is::<L>() {
        return Err(syn::Error::new(Span::call_site(), format!(
            "ST-Calibration: recorded {} stage is not {}", stage.language, std::any::type_name::<L>())));
    }
    let value: L = syn::parse_str(&stage.program)?;
    value.check()?;
    Ok(value)
}

/// Reparse a recorded valuation against its separately recorded normalized CQ.
pub fn checked_valuation(stage: &Stage) -> syn::Result<cq_stage::Valuation<cq_lang_cq::Program, cq_lang_vo::Program>> {
    let [cq] = stage.bases.as_slice() else {
        return Err(syn::Error::new(Span::call_site(), "ST-Calibration: VO requires one CQ base"));
    };
    if !stage.is::<cq_lang_vo::Program>() {
        return Err(syn::Error::new(Span::call_site(), "ST-Calibration: expected a VO annotation"));
    }
    let cq = syn::parse_str(cq)?;
    let valuation = (|input: syn::parse::ParseStream<'_>| {
        cq_lang_vo::Program::parse(input, &cq)
    }).parse_str(&stage.program)?;
    let stack = cq_stage::Valuation::new(cq, valuation);
    stack.check()?;
    Ok(stack)
}

/// Reparse an independently recorded Structures annotation against its Physical base.
pub fn checked_structures(stage: &Stage) -> syn::Result<cq_stage::Structures<cq_lang_physical::Program, cq_lang_structures::Program>> {
    let [physical] = stage.bases.as_slice() else {
        return Err(syn::Error::new(Span::call_site(), "ST-Calibration: Structures requires one Physical base"));
    };
    if !stage.is::<cq_lang_structures::Program>() {
        return Err(syn::Error::new(Span::call_site(), "ST-Calibration: expected a Structures annotation"));
    }
    let physical = syn::parse_str(physical)?;
    let structures = (|input: syn::parse::ParseStream<'_>| {
        <cq_lang_structures::Program as Annotation<&cq_lang_physical::Program>>::parse(input, &physical)
    }).parse_str(&stage.program)?;
    let stack = cq_stage::Structures::new(physical, structures);
    stack.check()?;
    Ok(stack)
}

/// Reparse Sequence against its separately recorded Physical and Structures bases.
pub fn checked_sequence(stage: &Stage) -> syn::Result<cq_stage::Sequence<cq_lang_physical::Program, cq_lang_structures::Program, cq_lang_sequence::Program>> {
    let [physical, structures] = stage.bases.as_slice() else {
        return Err(syn::Error::new(Span::call_site(), "ST-Calibration: Sequence requires Physical and Structures bases"));
    };
    if !stage.is::<cq_lang_sequence::Program>() {
        return Err(syn::Error::new(Span::call_site(), "ST-Calibration: expected a Sequence annotation"));
    }
    let physical = syn::parse_str(physical)?;
    let structures = (|input: syn::parse::ParseStream<'_>| {
        <cq_lang_structures::Program as Annotation<&cq_lang_physical::Program>>::parse(input, &physical)
    }).parse_str(structures)?;
    let sequence = (|input: syn::parse::ParseStream<'_>| {
        <cq_lang_sequence::Program as Annotation<(&cq_lang_physical::Program, &cq_lang_structures::Program)>>::parse(input, (&physical, &structures))
    }).parse_str(&stage.program)?;
    let stack = cq_stage::Sequence::new(physical, structures, sequence);
    stack.check()?;
    Ok(stack)
}

/// Named dependency graph of a recorded VO, RA, IX, Physical, Structures or
/// Sequence stage. Annotation stages retain source definitions in separate bases.
pub fn nodes(stage: &Stage) -> syn::Result<Vec<Node>> {
    if stage.is::<cq_lang_vo::Program>() {
        Ok(vo(&checked_valuation(stage)?.valuation))
    } else if stage.is::<cq_lang_ra::Program>() {
        Ok(ra(&checked(stage)?))
    } else if stage.is::<cq_lang_ix::Program>() {
        Ok(ra(&checked::<cq_lang_ix::Program>(stage)?.ra))
    } else if stage.is::<cq_lang_physical::Program>() {
        Ok(physical(&checked(stage)?))
    } else if stage.is::<cq_lang_structures::Program>() {
        Ok(physical(&checked_structures(stage)?.physical))
    } else if stage.is::<cq_lang_sequence::Program>() {
        Ok(physical(&checked_sequence(stage)?.physical))
    } else {
        Err(syn::Error::new(Span::call_site(), "ST-Calibration: language has no checked trace adapter"))
    }
}

fn slot(index: usize) -> Ident {
    Ident::new(&format!("__cq_operand_{index}"), Span::call_site())
}

fn vo(program: &cq_lang_vo::Program) -> Vec<Node> {
    use cq_lang_vo::Step;
    fn collect(steps: &[Step], prefix: &str, nodes: &mut Vec<Node>) {
        for (i, step) in steps.iter().enumerate() {
            let name = format!("{prefix}{i}");
            nodes.push(Node { name: name.clone(),
                operator: step.to_token_stream().to_string(), inputs: Vec::new() });
            if let Step::Block(block, _) = step {
                for (branch, arm) in block.branches.iter().enumerate() {
                    collect(&arm.steps, &format!("{name}/{branch}/"), nodes);
                }
            }
        }
    }
    let mut nodes = Vec::new();
    collect(&program.valuation.steps, "", &mut nodes);
    nodes
}

fn ra(program: &cq_lang_ra::Program) -> Vec<Node> {
    use cq_lang_ra::Operator;
    program.plan.definitions.iter().map(|d| {
        let inputs: Vec<&Ident> = match &d.operator {
            Operator::Unit(_) | Operator::Rename(_) => Vec::new(),
            Operator::Union(u) => u.operands.iter().collect(),
            Operator::Anti(a) => vec![&a.left, &a.right],
            Operator::NaturalJoin(j) => vec![&j.left, &j.right],
            Operator::Semijoin(j) => vec![&j.left, &j.right],
            Operator::Group(g) => vec![&g.input],
            Operator::Project(p) => vec![&p.input],
            Operator::Select(s) => vec![&s.input],
            Operator::Bind(b) => vec![&b.input],
            Operator::Guard(g) => vec![&g.input],
        };
        let mut logical = d.operator.clone();
        match &mut logical {
            Operator::Unit(_) | Operator::Rename(_) => (),
            Operator::Union(u) => {
                for (i, operand) in u.operands.iter_mut().enumerate() { *operand = slot(i); }
                if let Some(origin) = &mut u.origin {
                    if let Some(over) = &mut origin.over { over.prefix = slot(0); }
                    if let Some(domain) = &mut origin.domain { domain.input = slot(0); }
                }
            }
            Operator::Anti(a) => { a.left = slot(0); a.right = slot(1); }
            Operator::NaturalJoin(j) => { j.left = slot(0); j.right = slot(1); }
            Operator::Semijoin(j) => { j.left = slot(0); j.right = slot(1); }
            Operator::Group(g) => g.input = slot(0),
            Operator::Project(p) => p.input = slot(0),
            Operator::Select(s) => s.input = slot(0),
            Operator::Bind(b) => b.input = slot(0),
            Operator::Guard(g) => g.input = slot(0),
        }
        Node { name: symbol_name(&d.result),
            operator: logical.to_token_stream().to_string(),
            inputs: inputs.into_iter().map(symbol_name).collect() }
    }).collect()
}

fn physical(program: &cq_lang_physical::Program) -> Vec<Node> {
    use cq_lang_physical::Operator;
    program.program.definitions.iter().map(|d| {
        let left = slot(0);
        let right = slot(1);
        let logical = match &d.operator {
            Operator::Unit(u) => u.to_token_stream(),
            Operator::Rename(r) => r.to_token_stream(),
            Operator::Union(u) => {
                let mut u = u.logical();
                for (i, operand) in u.operands.iter_mut().enumerate() { *operand = slot(i); }
                if let Some(origin) = &mut u.origin {
                    if let Some(over) = &mut origin.over { over.prefix = slot(0); }
                    if let Some(domain) = &mut origin.domain { domain.input = slot(0); }
                }
                u.to_token_stream()
            }
            Operator::Anti(_, k) => {
                let columns = &k.columns;
                quote!(anti [#columns] #left, #right)
            }
            Operator::Probe(..) | Operator::Merge(..) => quote!(natural_join #left with #right),
            Operator::Semi(..) => quote!(semijoin #left with #right),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                let mut s = s.clone(); s.input = left; s.to_token_stream()
            }
            Operator::Group(g) => { let mut g = g.clone(); g.input = left; g.to_token_stream() }
            Operator::Project(p) => { let mut p = p.clone(); p.input = left; p.to_token_stream() }
            Operator::Bind(b) => { let mut b = b.clone(); b.input = left; b.to_token_stream() }
            Operator::Guard(g) => { let mut g = g.clone(); g.input = left; g.to_token_stream() }
        };
        Node {
            name: symbol_name(&d.name),
            operator: logical.to_string(),
            inputs: d.operator.operands().into_iter().map(symbol_name).collect(),
        }
    }).collect()
}
