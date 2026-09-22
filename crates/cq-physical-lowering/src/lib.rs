//! Realize a fully selected Physical program through external macro callbacks.
//!
//! The driver knows no concrete provider names or storage algorithms.
//! It schedules Input/Index/Lower requests and validates their replies.
//! Separately selected exact operator rules live in rename, indexed_join and project.
//! Physical choices are immutable. Unsupported representations fail instead of
//! acquiring an implicit collection, clone, or alternate provider.

mod composition;
pub mod indexed_join;
pub mod project;
pub mod rename;
mod syntax;

use cq_ir::{
    physical, provider_protocol as protocol, relational_plan, rust_access_plan, symbol_name,
};
use proc_macro2::TokenStream;
use protocol::{Binding, Environment, Reply, State, Task};
use quote::{ToTokens, quote};
use syn::{Path, Stmt};

/// Begin realization in a host expression scope. Referenced host relation names
/// are Rust expressions in that scope; this entry does not generate a host API.
pub fn begin(
    plan: physical::Module,
    resume_path: Path,
    access_path: Path,
) -> syn::Result<TokenStream> {
    physical::contract::check_structure(&plan)?;
    advance(
        plan,
        Environment::default(),
        Vec::new(),
        resume_path,
        access_path,
    )
}

/// Resume one provider response. Only this driver interprets its opaque state.
pub fn resume(reply: Reply) -> syn::Result<TokenStream> {
    let state: State = syn::parse2(reply.resume)?;
    physical::contract::check_structure(&state.plan)?;
    let tasks = schedule(&state.plan);
    check_prefix(&state.plan, &tasks, &state.environment)?;
    let expected = tasks.get(state.environment.bindings.len()).ok_or_else(|| {
        syn::Error::new_spanned(
            &state.pending,
            "reply received after all physical tasks completed",
        )
    })?;
    if expected.to_token_stream().to_string() != state.pending.to_token_stream().to_string() {
        return Err(syn::Error::new_spanned(
            &state.pending,
            "continuation task does not match the next physical request",
        ));
    }
    check_reply(&state.plan, expected, &reply.result)?;
    let mut environment = state.environment;
    environment.bindings.push(reply.result);
    let mut emitted = state.emitted;
    // Check the fragments in statement position. Block-like Rust expressions
    // (including `if` and `for`) need no semicolon there; other tail values do.
    // The sentinel is only parsed for validation and is never emitted.
    let statements = &reply.emitted;
    syn::parse2::<syn::Block>(quote! { { #(#statements)* let _: () = (); } }).map_err(|_| {
        syn::Error::new_spanned(
            quote! { #(#statements)* },
            "provider emitted code must contain statements, not a trailing value expression",
        )
    })?;
    emitted.extend(reply.emitted);
    advance(
        state.plan,
        environment,
        emitted,
        state.resume_path,
        state.access_path,
    )
}

fn advance(
    plan: physical::Module,
    environment: Environment,
    emitted: Vec<Stmt>,
    resume_path: Path,
    access_path: Path,
) -> syn::Result<TokenStream> {
    let tasks = schedule(&plan);
    check_prefix(&plan, &tasks, &environment)?;
    let Some(task) = tasks.get(environment.bindings.len()) else {
        return finish(&plan, &environment, &emitted, &access_path);
    };
    if let Task::Index { name, .. } = task {
        let definition = plan
            .index_requirements
            .indexes
            .iter()
            .find(|index| symbol_name(&index.name) == symbol_name(name))
            .expect("validated index identity");
        match environment.relation(&definition.requirement.relation) {
            Some(Binding::Stored { .. }) => {}
            Some(Binding::Region { .. }) => {
                return Err(syn::Error::new_spanned(
                    &definition.requirement,
                    "cannot build an index from an Access region; its selected producer must explicitly establish stored representation",
                ));
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    &definition.requirement,
                    "index source representation has not been established",
                ));
            }
        }
    }
    let selected = match task {
        Task::Input { .. } => &plan.assignments.relations,
        Task::Index { .. } => &plan.assignments.indexes,
        Task::Lower { .. } => &plan.assignments.lowerings,
    }
    .iter()
    .find(|binding| symbol_name(&binding.name) == symbol_name(task.name()))
    .expect("validated complete assignments");
    let path = &selected.implementation;
    let state: State = syn::parse2(quote! {
        plan { #plan } environment { #environment } emitted { #(#emitted)* }
        pending { #task } continuations (#resume_path, #access_path)
    })?;
    let request: protocol::Request = syn::parse2(quote! {
        reply_to (#resume_path) { #state }
        plan { #plan } environment { #environment } request { #task }
    })?;
    Ok(quote! { #path! { #request } })
}

/// Serialized task order is derived only from dependencies and explicit IX use
/// references. Index columns are never recomputed by realization.
fn schedule(plan: &physical::Module) -> Vec<Task> {
    let ra = &plan.index_requirements.relational_plan.plan;
    let mut tasks = Vec::new();
    let mut inputs = std::collections::BTreeSet::new();
    for definition in &ra.definitions {
        if let relational_plan::Operator::Rename(rename) = &definition.operator {
            let name = &rename.relation;
            if inputs.insert(symbol_name(name)) {
                tasks.push(syn::parse_quote! { input #name from (#name) });
            }
        }
    }
    let mut indexes = std::collections::BTreeSet::new();
    for definition in &ra.definitions {
        for usage in &plan.index_requirements.uses {
            if symbol_name(&usage.result) == symbol_name(&definition.result)
                && indexes.insert(symbol_name(&usage.index))
            {
                let name = &usage.index;
                tasks.push(syn::parse_quote! { index #name });
            }
        }
        let name = &definition.result;
        tasks.push(syn::parse_quote! { lower #name });
    }
    tasks
}

fn check_prefix(
    plan: &physical::Module,
    tasks: &[Task],
    environment: &Environment,
) -> syn::Result<()> {
    if environment.bindings.len() > tasks.len() {
        return Err(syn::Error::new_spanned(
            environment,
            "physical environment contains extra or repeated replies",
        ));
    }
    for (task, result) in tasks.iter().zip(&environment.bindings) {
        check_reply(plan, task, result)?;
    }
    Ok(())
}

fn check_reply(plan: &physical::Module, task: &Task, result: &Binding) -> syn::Result<()> {
    if symbol_name(task.name()) != symbol_name(result.name()) {
        return Err(syn::Error::new_spanned(
            result.name(),
            format!(
                "physical reply must establish requested identity `{}`",
                task.name()
            ),
        ));
    }
    let correct_kind = matches!(
        (task, result),
        (Task::Input { .. }, Binding::Stored { .. })
            | (Task::Index { .. }, Binding::Indexed { .. })
            | (
                Task::Lower { .. },
                Binding::Stored { .. } | Binding::Region { .. }
            )
    );
    if !correct_kind {
        return Err(syn::Error::new_spanned(
            result,
            "physical reply kind does not match the pending input, index, or lowering request",
        ));
    }
    if let Binding::Region { name, access, .. } = result {
        rust_access_plan::contract::check(access)?;
        let ra = &plan.index_requirements.relational_plan;
        let infos = relational_plan::check_in(ra.program.inputs(), &ra.plan)?;
        let heading = &infos[&symbol_name(name)].heading;
        let actual: Vec<_> = access.head.atom.variables().map(symbol_name).collect();
        let expected: Vec<_> = heading.iter().cloned().collect();
        if !access.head.atom.is_normalized() || actual != expected {
            return Err(syn::Error::new_spanned(
                &access.head,
                "Access result must expose the exact normalized RA heading in logical column order",
            ));
        }
    }
    Ok(())
}

fn finish(
    plan: &physical::Module,
    environment: &Environment,
    emitted: &[Stmt],
    access_path: &Path,
) -> syn::Result<TokenStream> {
    let output = &plan.index_requirements.relational_plan.plan.output;
    let Some(Binding::Region { access, .. }) = environment.relation(&output.input) else {
        return Err(syn::Error::new_spanned(
            &output.input,
            "the selected final lowering must return an Access region; no implicit scan or materialization is supplied",
        ));
    };
    let expected: Vec<_> = output.head.variables().map(symbol_name).collect();
    let actual: Vec<_> = access.head.atom.variables().map(symbol_name).collect();
    if !output.head.is_normalized() || expected != actual {
        return Err(syn::Error::new_spanned(
            &output.head,
            "this open-region realization requires normalized output in logical heading order; the selected final lowering must explicitly implement another layout",
        ));
    }
    let mut access = access.clone();
    // The public predicate name is output metadata, not a storage decision.
    access.head.atom.relation = output.head.relation.clone();
    Ok(quote! { { #(#emitted)* #access_path! { #access } } })
}
