//! Equality-component memberships collected while facts are parsed.

use ahash::AHashSet;
use std::io;

/// Values observed in one inferred equality component, indexed by component ID.
#[derive(Debug, PartialEq, Eq)]
pub enum DomainValues {
    /// Distinct process-global symbol codes in ascending code order.
    Symbols(Vec<u32>),
    /// Inclusive integer bounds, or no observed values.
    Integers(Option<(i32, i32)>),
}

pub(crate) enum Domain {
    Symbols(AHashSet<u32>),
    Integers(Option<(i32, i32)>),
}

pub(crate) struct Domains<'a> {
    schema: &'a [(&'a str, &'a [u32])],
    values: Vec<Option<Domain>>,
}

impl<'a> Domains<'a> {
    pub(crate) fn new(schema: &'a [(&'a str, &'a [u32])]) -> Self {
        Self { schema, values: Vec::new() }
    }

    pub(crate) fn columns(
        &mut self,
        relation: &str,
        types: &[&str],
    ) -> io::Result<Option<&'a [u32]>> {
        let Some(&(_, columns)) = self.schema.iter().find(|(name, _)| *name == relation) else {
            return Ok(None);
        };
        if columns.len() != types.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                format!("domain schema for {relation} has the wrong number of columns")));
        }
        for (&id, &ty) in columns.iter().zip(types) {
            let id = id as usize;
            if self.values.len() <= id {
                self.values.resize_with(id + 1, || None);
            }
            let domain = &mut self.values[id];
            match (domain.as_ref(), ty) {
                (None, "string") => *domain = Some(Domain::Symbols(AHashSet::default())),
                (None, "i32") => *domain = Some(Domain::Integers(None)),
                (Some(Domain::Symbols(_)), "string") | (Some(Domain::Integers(_)), "i32") => {},
                _ => return Err(io::Error::new(io::ErrorKind::InvalidInput,
                    format!("incompatible types in domain {id} for {relation}"))),
            }
        }
        Ok(Some(columns))
    }

    pub(crate) fn get_mut(&mut self, id: u32) -> &mut Domain {
        self.values[id as usize].as_mut().expect("typed domain initialized before parsing")
    }

    pub(crate) fn finish(self) -> Vec<DomainValues> {
        self.values.into_iter().map(|domain| match domain {
            Some(Domain::Symbols(values)) => {
                let mut values: Vec<_> = values.into_iter().collect();
                values.sort_unstable();
                DomainValues::Symbols(values)
            }
            Some(Domain::Integers(bounds)) => DomainValues::Integers(bounds),
            None => DomainValues::Symbols(Vec::new()),
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_types_empty_components_without_observations() {
        let mut domains = Domains::new(&[("formalparam", &[0, 1, 1])]);
        domains.columns("formalparam", &["i32", "string", "string"]).unwrap();
        assert_eq!(domains.finish(), [
            DomainValues::Integers(None),
            DomainValues::Symbols(Vec::new()),
        ]);
        assert!(Domains::new(&[]).finish().is_empty());
    }
}
