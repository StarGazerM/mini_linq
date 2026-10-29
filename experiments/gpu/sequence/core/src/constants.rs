//! Expose literal atom arguments before selecting physical storage providers.
use std::collections::BTreeSet;
use cq_lang_structures::physical::{self, Operator, Program, symbol_name};
use cq_lang_structures::syntax::EqualityValue;
use quote::{format_ident, quote};

fn derive(source: &Program) -> syn::Result<Program> {
    let analysis = physical::contract::check(source)?;
    let mut used: BTreeSet<String> = analysis.headings.values().flatten().cloned()
        .chain(source.program.definitions.iter().map(|definition| symbol_name(&definition.name)))
        .collect();
    let mut fresh = |stem: &str| {
        let mut suffix = 0usize;
        loop {
            let name = format!("__cq_constant_{stem}_{suffix}");
            if used.insert(name.clone()) { break format_ident!("{name}"); }
            suffix += 1;
        }
    };
    let mut target = source.clone();
    let mut definitions = Vec::new();
    for definition in &source.program.definitions {
        let Operator::Rename(rename) = &definition.operator else {
            definitions.push(definition.clone());
            continue;
        };
        let Some(constants) = &rename.constants else {
            definitions.push(definition.clone());
            continue;
        };
        if constants.equalities.is_empty() {
            definitions.push(definition.clone());
            continue;
        }
        let raw = fresh("source");
        let filtered = fresh("filtered");
        let mut renamed = rename.clone();
        renamed.constants = None;
        let keep: Vec<_> = rename.mapping.iter().map(|mapping| mapping.target.clone()).collect();
        let mut conditions = Vec::new();
        for equality in &constants.equalities {
            let column = &equality.left;
            let EqualityValue::Literal(value) = &equality.right else {
                return Err(syn::Error::new_spanned(definition, "PH-ConstantGuard requires literal occurrence arguments"));
            };
            let variable = fresh("column");
            renamed.mapping.push(syn::parse_quote!(#column -> #variable));
            conditions.push(quote!(#variable = #value));
        }
        definitions.push(syn::parse_quote!(#raw = pipe #renamed;));
        definitions.push(syn::parse_quote!(#filtered = pipe select #raw if (#(#conditions),*);));
        let mut projected = definition.clone();
        projected.operator = syn::parse_quote!(project #filtered keep {#(#keep),*});
        definitions.push(projected);
    }
    target.program.definitions = definitions;
    Ok(target)
}

// Formal rule PH-ConstantGuard: fresh literal columns, relational restrictions and projection preserve the renamed atom's bag.
pub fn compile(source: &Program) -> syn::Result<Program> {
    let target = derive(source)?;
    check(source, &target)?;
    Ok(target)
}

/// Re-derive the complete expected rewrite, including freshness and all records.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    let expected = derive(source)?;
    if target != &expected {
        return Err(syn::Error::new_spanned(target, "PH-ConstantGuard differs from the source-derived literal guard rewrite"));
    }
    physical::contract::check(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Program {
        syn::parse_quote! { physical {
            relation R(x:i32,y:i32);
            r = let rename R@0 {y -> y} constants {x = 7};
            return r as answer(y);
        } }
    }
    #[test]
    fn literal_guards_preserve_the_output_and_are_checked() {
        let source = source();
        let target = compile(&source).unwrap();
        assert_eq!(target.program.definitions.len(), 3);
        assert_eq!(source.program.head, target.program.head);
        check(&source, &target).unwrap();
        let mut changed = target.clone();
        let Operator::Select(select) = &mut changed.program.definitions[1].operator else { panic!() };
        select.condition = syn::parse_quote!(__cq_constant_column_0 = 8);
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.program.definitions.remove(1);
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.program.definitions[0].name = syn::parse_quote!(not_the_fresh_name);
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn unchanged_atoms_remain_exact() {
        let source: Program = syn::parse_quote! { physical {
            relation R(x:i32); r = pipe rename R@0 {x -> x}; return r as answer(x);
        } };
        assert_eq!(compile(&source).unwrap(), source);
    }
}
