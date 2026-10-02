//! A push dialect for `T X = bottom + Set X`: one panic aborts evaluation.
//! Physical reads feed nested loops and a set sink. Rust binds and guards remain
//! opaque partial functions in their ancestry order; I/O, mutation and evaluation
//! counts are outside the meaning. Only relational steps admit RA laws.
use cq_lang_structures as structures;
use cq_print::{Context, Repr, access::Access, error, index, sorted};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{ToTokens, quote};
use structures::physical;
use structures::physical::Operator;
use structures::syntax::symbol_name;

#[proc_macro]
pub fn set_sink(input: TokenStream) -> TokenStream {
    cq_stage::stage(input.into(), None, |request: &cq_protocol::Request| {
        Ok(match &request.task {
            cq_protocol::Task::New { .. } => quote!(::std::collections::HashSet::new()),
            cq_protocol::Task::Insert { state, value, .. } => {
                quote!({ ::cq_storage::require::leibniz(#value); (#state).insert(::core::clone::Clone::clone(#value)) })
            }
        })
    })
    .into()
}

#[proc_macro]
pub fn push(input: TokenStream) -> TokenStream {
    cq_stage::stage(
        input.into(),
        None,
        |source: &cq_stage::Policed<structures::Program, physical::policy::Policy>| {
            compile(&source.language)
        },
    )
    .into()
}

struct Push<'a> {
    context: Context<'a>,
}
impl<'a> std::ops::Deref for Push<'a> {
    type Target = Context<'a>;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

fn compile(module: &structures::Program) -> syn::Result<syn::Expr> {
    let analysis = structures::contract::check(module)?;
    let mut push = Push {
        context: Context::new(module, &analysis),
    };
    for p in &module.physical.program.definitions {
        let name = symbol_name(&p.name);
        push.context.register(&name);
        match p.mode {
            physical::Mode::Pipe(_) => {}
            physical::Mode::Let(_) => {
                let row = push.pattern(&name);
                let body = push.emit(&name, quote!(__push_rows.push(#row);))?;
                let source = quote!({
                    let mut __push_rows = ::std::vec::Vec::new();
                    #body
                    ::core::iter::IntoIterator::into_iter(__push_rows)
                });
                let repr = push.context.materialize(&name, p.canon.is_some(), source);
                push.context.reprs.insert(name, repr);
            }
        }
    }
    let returned = &module.physical.program.returned;
    let name = symbol_name(returned);
    let mut fields = Vec::new();
    for expression in &module.physical.program.head.args {
        let Some(variable) = structures::syntax::rust::variable(expression) else {
            return error(
                expression,
                "this example push dialect supports variable heads",
            );
        };
        fields.push(push.ident(&name, &symbol_name(variable)));
    }
    let body = push.emit(
        &name,
        quote! {
            mini_linq::distinct_state! { operator #returned; insert(__push_out, &(#(#fields,)*)) };
        },
    )?;
    let statements = &push.statements;
    syn::parse2(quote!({
        #[allow(unused_imports)] use mini_linq::index_provider;
        #(#statements)*
        let mut __push_out = mini_linq::distinct_state! { operator #returned; new };
        #body
        ::core::iter::IntoIterator::into_iter(__push_out)
    }))
}

impl Push<'_> {
    fn emit(&self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.physical[result];
        if let Some(Repr::Let { rows, structures }) = self.reprs.get(result) {
            let _ = structures;
            let Some(storage) = rows else {
                return error(&definition.name, "unmaterialized scan");
            };
            let pattern = self.pattern(result);
            return Ok(
                quote!(#[allow(unused_variables)] for &#pattern in #storage.iter() { #sink }),
            );
        }
        if definition.canon.is_some() {
            let row = self.pattern(result);
            let id = &definition.name;
            let body = self.operator(result, quote!(
                if mini_linq::distinct_state! { operator #id; insert(__push_seen, &#row) } { #sink }
            ))?;
            return Ok(quote!({
                let mut __push_seen = mini_linq::distinct_state! { operator #id; new };
                #body
            }));
        }
        self.operator(result, sink)
    }

    fn operator(&self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let p = self.physical[result];
        match &p.operator {
            Operator::Unit(_) => Ok(sink),
            Operator::Rename(_) => {
                let rows = self.host_scan(result);
                let pattern = self.pattern(result);
                Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
            }
            Operator::Project(project) => self.emit(&symbol_name(&project.input), sink),
            Operator::Point(_, select) => {
                if let Some(rows) = self.host_point(result, select) {
                    let pattern = self.pattern(result);
                    Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
                } else {
                    let condition = cq_print::expressions::relational_condition(&select.condition);
                    self.emit(&symbol_name(&select.input), quote!(if #condition { #sink }))
                }
            }
            // This push consumer realizes a range by scanning and filtering.
            // CQ Rust comparisons can now generate Range without custom syntax.
            Operator::Select(select) | Operator::Range(_, select) => {
                let condition = cq_print::expressions::relational_condition(&select.condition);
                self.emit(&symbol_name(&select.input), quote!(if #condition { #sink }))
            }
            Operator::Guard(guard) => {
                let condition = &guard.condition;
                self.emit(&symbol_name(&guard.input), quote!(if #condition { #sink }))
            }
            Operator::Bind(bind) => {
                let pattern = &bind.pattern;
                let annotation = &bind.annotation;
                let expression = &bind.expression;
                let shadows = self.shadows(&symbol_name(&bind.input));
                self.emit(
                    &symbol_name(&bind.input),
                    quote! { let #pattern #annotation = { #(#shadows)* #expression }; #sink },
                )
            }
            Operator::Probe(_, k) => {
                let left = symbol_name(&k.left);
                let right = symbol_name(&k.right);
                let columns = sorted(&k.columns);
                let (lookup, payload, host) = match self.keyed(&k.right, &columns)? {
                    Access::Host(access) => (
                        self.host_lookup(&left, &access, false),
                        access.payload,
                        false,
                    ),
                    Access::Built(structure) => (
                        self.chained_lookup(&left, &structure, &right, &columns),
                        self.built_payload(&right, &self.levels_of(&right, &structure)),
                        false,
                    ),
                    Access::Prefix(_, _) => {
                        return error(
                            &k.right,
                            "prefix payload probes are outside this example dialect",
                        );
                    }
                };
                let row = self.row(result, |c| {
                    match payload.iter().position(|p| p == c && !columns.contains(p)) {
                        Some(i) => {
                            let i = index(i);
                            if host {
                                quote!(&__push_v.#i)
                            } else {
                                quote!(__push_v.#i)
                            }
                        }
                        None => self.ident(&left, c).to_token_stream(),
                    }
                });
                let pattern = self.pattern(result);
                self.emit(&left, quote!(for __push_v in #lookup { #[allow(unused_variables)] let #pattern = #row; #sink }))
            }
            Operator::Semi(_, k) => {
                let left = symbol_name(&k.left);
                let right = symbol_name(&k.right);
                let test = match self.keyed(&k.right, &sorted(&k.columns))? {
                    Access::Host(access) => self.host_lookup(&left, &access, true),
                    Access::Built(structure) => self.chained_contains(
                        &left,
                        &structure,
                        &right,
                        &self.levels_of(&right, &structure),
                    ),
                    Access::Prefix(structure, levels) => {
                        self.chained_contains(&left, &structure, &right, &levels)
                    }
                };
                self.emit(&left, quote!(if #test { #sink }))
            }
            _ => error(
                &p.name,
                "this push example implements unit, rename, project, select, point, range, bind, guard, probe and semi",
            ),
        }
    }
}
