//! Plain schema data and typed views of already validated integer fixtures.
use crate::dataset::{Dataset, RelationData};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    I32,
    I64,
    Date,
    Decimal,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relation {
    pub name: &'static str,
    pub columns: &'static [(&'static str, ColumnType)],
}

pub fn declarations(inputs: &[Relation]) -> Vec<cq_syntax::RelationDecl> {
    inputs
        .iter()
        .map(|input| cq_syntax::RelationDecl {
            relation_token: Default::default(),
            name: syn::Ident::new(input.name, proc_macro2::Span::call_site()),
            columns_paren: Default::default(),
            columns: input
                .columns
                .iter()
                .map(|&(name, kind)| cq_syntax::ColumnDecl {
                    name: syn::Ident::new(name, proc_macro2::Span::call_site()),
                    colon_token: Default::default(),
                    ty: syn::Type::Path(syn::TypePath {
                        qself: None,
                        path: {
                            let name = syn::Ident::new(
                                match kind {
                                    ColumnType::I32 => "i32",
                                    ColumnType::I64 => "i64",
                                    ColumnType::Date => "Date",
                                    ColumnType::Decimal => "Decimal",
                                    ColumnType::Text => "String",
                                },
                                proc_macro2::Span::call_site(),
                            );
                            if matches!(kind, ColumnType::Date | ColumnType::Decimal) {
                                let mut path: syn::Path =
                                    syn::Ident::new("cq_load", proc_macro2::Span::call_site())
                                        .into();
                                path.segments.push(name.into());
                                path
                            } else {
                                name.into()
                            }
                        },
                    }),
                })
                .collect(),
            value: None,
            semi_token: Default::default(),
        })
        .collect()
}

/// Construct only after schema, manifest and provenance validation have passed.
pub struct Inputs {
    relations: Vec<RelationData>,
}
impl Inputs {
    pub fn from_dataset(dataset: Dataset) -> Self {
        Self {
            relations: dataset.relations,
        }
    }

    fn rows<T: IntegerRow>(&mut self, name: &str) -> std::vec::IntoIter<Vec<i32>> {
        let position = self
            .relations
            .iter()
            .position(|r| r.name == name)
            .unwrap_or_else(|| panic!("missing or already bound relation {name}"));
        let relation = self.relations.remove(position);
        assert_eq!(relation.arity, T::ARITY, "{name} tuple width");
        relation.rows.into_iter()
    }

    pub fn take<T: IntegerRow>(&mut self, name: &str) -> Vec<T> {
        self.rows::<T>(name).map(T::from_row).collect()
    }

    pub fn take_hash<T: IntegerRow>(&mut self, name: &str) -> HashMap<(i32,), Vec<T::Tail>> {
        let mut groups = HashMap::<_, Vec<_>>::new();
        for row in self.rows::<T>(name) {
            let (key, value) = T::from_row(row).split();
            groups.entry(key).or_default().push(value);
        }
        groups
    }

    pub fn take_btree<T: IntegerRow>(&mut self, name: &str) -> BTreeMap<(i32,), Vec<T::Tail>> {
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for row in self.rows::<T>(name) {
            let (key, value) = T::from_row(row).split();
            groups.entry(key).or_default().push(value);
        }
        groups
    }
}

pub trait IntegerRow: Sized {
    const ARITY: usize;
    type Tail;
    fn from_row(row: Vec<i32>) -> Self;
    fn split(self) -> ((i32,), Self::Tail);
}

macro_rules! integer_row {
    (@ty $name:ident) => { i32 };
    ($arity:literal; $first:ident $(, $rest:ident)*) => {
        impl IntegerRow for (i32, $(integer_row!(@ty $rest),)*) {
            const ARITY: usize = $arity;
            type Tail = ($(integer_row!(@ty $rest),)*);
            fn from_row(row: Vec<i32>) -> Self {
                let [$first, $($rest),*]: [i32; $arity] = row.try_into().expect("validated row width");
                ($first, $($rest,)*)
            }
            fn split(self) -> ((i32,), Self::Tail) {
                let ($first, $($rest,)*) = self;
                (($first,), ($($rest,)*))
            }
        }
    };
}
integer_row!(1; a);
integer_row!(2; a,b);
integer_row!(3; a,b,c);
integer_row!(4; a,b,c,d);
integer_row!(5; a,b,c,d,e);
integer_row!(6; a,b,c,d,e,f);
