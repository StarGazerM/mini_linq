use super::*;
pub(super) fn result_row_type(source: &index_requirements::Module) -> syn::Type {
    let program = &source.relational_plan.program;
    let relations = program
        .inputs()
        .map(|input| (cq_ir::symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut variables = BTreeMap::<String, syn::Type>::new();
    for item in &program.query.body {
        let atom = item.positive_atom().expect("positive R1/reference fixture");
        let input = relations
            .get(&cq_ir::symbol_name(&atom.relation))
            .expect("well-formed body relation is declared");
        for (variable, column) in atom.variables().zip(&input.columns) {
            variables
                .entry(cq_ir::symbol_name(variable))
                .or_insert_with(|| column.ty.clone());
        }
    }
    let output_types = program.query.head.variables().map(|variable| {
        variables
            .get(&cq_ir::symbol_name(variable))
            .expect("well-formed result variable is bound")
    });
    syn::parse2(quote!((#(#output_types,)*))).unwrap()
}

pub(super) fn identifier_occurrences(node: &syn::Block, name: &str) -> usize {
    let mut counter = IdentifierCounter { name, count: 0 };
    counter.visit_block(node);
    counter.count
}

pub(super) fn expression_identifier_occurrences(node: &syn::Expr, name: &str) -> usize {
    let mut counter = IdentifierCounter { name, count: 0 };
    counter.visit_expr(node);
    counter.count
}

pub(super) fn statement_identifier_occurrences(node: &syn::Stmt, name: &str) -> usize {
    let mut counter = IdentifierCounter { name, count: 0 };
    counter.visit_stmt(node);
    counter.count
}

struct IdentifierCounter<'a> {
    name: &'a str,
    count: usize,
}

impl<'ast> Visit<'ast> for IdentifierCounter<'_> {
    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        if cq_ir::symbol_name(ident) == self.name {
            self.count += 1;
        }
    }
}

#[derive(Default)]
pub(super) struct PullMacroCollector<'ast> {
    pub(super) macros: Vec<&'ast syn::Macro>,
}

impl<'ast> Visit<'ast> for PullMacroCollector<'ast> {
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "pull")
        {
            self.macros.push(invocation);
        }
        syn::visit::visit_macro(self, invocation);
    }
}

pub(super) fn is_absolute_mini_linq_pull(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == "pull"
}

pub(super) fn is_absolute_mini_linq_iterator_pipeline(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == "iterator_pipeline"
}

pub(super) fn exactly_one_struct<'a>(file: &'a syn::File, name: &str) -> &'a syn::ItemStruct {
    let matches = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item) if item.ident == name => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected exactly one struct `{name}`");
    matches[0]
}

pub(super) fn inherent_methods<'a>(
    file: &'a syn::File,
    program_name: &str,
) -> Vec<&'a syn::ImplItemFn> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item)
                if item.trait_.is_none() && type_is_named(&item.self_ty, program_name) =>
            {
                Some(item)
            }
            _ => None,
        })
        .flat_map(|item| &item.items)
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) => Some(method),
            _ => None,
        })
        .collect()
}

pub(super) fn exactly_one_method<'a>(
    methods: &'a [&syn::ImplItemFn],
    name: &str,
    program_name: &str,
) -> &'a syn::ImplItemFn {
    let matches = methods
        .iter()
        .copied()
        .filter(|method| method.sig.ident == name)
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "{program_name} must define exactly one `{name}` method",
    );
    matches[0]
}

pub(super) fn type_is_named(ty: &syn::Type, name: &str) -> bool {
    matches!(
        ty,
        syn::Type::Path(path)
            if path.qself.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == name
    )
}

pub(super) fn local_binding_name(statement: &syn::Stmt) -> Option<String> {
    let syn::Stmt::Local(local) = statement else {
        return None;
    };
    pattern_identifier(&local.pat).map(ToString::to_string)
}

pub(super) fn pattern_identifier(pattern: &syn::Pat) -> Option<&syn::Ident> {
    match pattern {
        syn::Pat::Ident(pattern) => Some(&pattern.ident),
        syn::Pat::Paren(pattern) => pattern_identifier(&pattern.pat),
        syn::Pat::Type(pattern) => pattern_identifier(&pattern.pat),
        _ => None,
    }
}
