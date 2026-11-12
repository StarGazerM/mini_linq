//! Typed equality components: observed EDB memberships and declared IDB bounds.

use ahash::AHashSet;
use std::io;

/// Per-component declared numeric bounds (`None` for symbols) and EDB membership.
/// Bounds use a host-only primitive seam; no GPU runtime dependency is required.
pub type DomainType = (Option<(i128, i128)>, bool);

/// Values for every inferred equality component, indexed by component ID.
#[derive(Debug, PartialEq, Eq)]
pub enum DomainValues {
    /// Distinct process-global symbol codes in ascending code order.
    Symbols(Vec<u32>),
    /// Inclusive observed bounds, or checked declared bounds for IDB-only fields.
    Integers(Option<(i32, i32)>),
}

pub(crate) enum Domain {
    Symbols(AHashSet<u32>),
    Integers(Option<(i32, i32)>),
}

pub(crate) struct Domains<'a> {
    schema: &'a [(&'a str, &'a [u32])],
    values: Vec<Domain>,
}

impl<'a> Domains<'a> {
    pub(crate) fn new(
        schema: &'a [(&'a str, &'a [u32])],
        types: &[DomainType],
    ) -> io::Result<Self> {
        let values = types.iter().enumerate().map(|(id, &(bounds, edb))| {
            Ok(match bounds {
                None => Domain::Symbols(AHashSet::default()),
                Some((minimum, maximum)) => {
                    if minimum > maximum {
                        return Err(io::Error::new(io::ErrorKind::InvalidInput,
                            format!("invalid declared bounds for domain {id}")));
                    }
                    let bounds = if edb { None } else {
                        let minimum = i32::try_from(minimum);
                        let maximum = i32::try_from(maximum);
                        let (Ok(minimum), Ok(maximum)) = (minimum, maximum) else {
                            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                format!("declared bounds for domain {id} exceed the i32 loader")));
                        };
                        Some((minimum, maximum))
                    };
                    Domain::Integers(bounds)
                }
            })
        }).collect::<io::Result<Vec<_>>>()?;
        for &(relation, columns) in schema {
            if columns.iter().any(|&id| id as usize >= values.len()) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput,
                    format!("domain schema for {relation} references an untyped component")));
            }
        }
        Ok(Self { schema, values })
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
            match (&self.values[id], ty) {
                (Domain::Symbols(_), "string") | (Domain::Integers(_), "i32") => {},
                _ => return Err(io::Error::new(io::ErrorKind::InvalidInput,
                    format!("incompatible types in domain {id} for {relation}"))),
            }
        }
        Ok(Some(columns))
    }

    pub(crate) fn get_mut(&mut self, id: u32) -> &mut Domain {
        &mut self.values[id as usize]
    }

    pub(crate) fn finish(self) -> Vec<DomainValues> {
        self.values.into_iter().map(|domain| match domain {
            Domain::Symbols(values) => {
                let mut values: Vec<_> = values.into_iter().collect();
                values.sort_unstable();
                DomainValues::Symbols(values)
            }
            Domain::Integers(bounds) => DomainValues::Integers(bounds),
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_types_empty_components_without_observations() {
        let mut domains = Domains::new(&[("formalparam", &[0, 1, 1])], &[
            (Some((i32::MIN as i128, i32::MAX as i128)), true), (None, true),
        ]).unwrap();
        domains.columns("formalparam", &["i32", "string", "string"]).unwrap();
        assert_eq!(domains.finish(), [
            DomainValues::Integers(None),
            DomainValues::Symbols(Vec::new()),
        ]);
        assert!(Domains::new(&[], &[]).unwrap().finish().is_empty());
    }

    #[test]
    fn interior_and_trailing_idb_components_keep_declared_types() {
        let mut domains = Domains::new(&[
            ("derived_first", &[0]), ("formalparam", &[1, 2, 2]),
            ("derived_middle", &[3]), ("mainclass", &[4]), ("derived_last", &[5]),
        ], &[
            (Some((-100, 100)), false),
            (Some((i32::MIN as i128, i32::MAX as i128)), true),
            (None, true), (Some((-200, 200)), false), (None, true),
            (Some((i32::MIN as i128, i32::MAX as i128)), false),
        ]).unwrap();
        domains.columns("formalparam", &["i32", "string", "string"]).unwrap();
        domains.columns("mainclass", &["string"]).unwrap();
        *domains.get_mut(1) = Domain::Integers(Some((-2, 3)));
        assert_eq!(domains.finish(), [
            DomainValues::Integers(Some((-100, 100))),
            DomainValues::Integers(Some((-2, 3))),
            DomainValues::Symbols(Vec::new()),
            DomainValues::Integers(Some((-200, 200))),
            DomainValues::Symbols(Vec::new()),
            DomainValues::Integers(Some((i32::MIN, i32::MAX))),
        ]);
    }

    #[test]
    fn incomplete_or_unrepresentable_component_metadata_is_rejected() {
        assert!(Domains::new(&[("derived", &[1])], &[(None, false)]).is_err());
        assert!(Domains::new(&[("derived", &[0])], &[
            (Some((i64::MIN as i128, i64::MAX as i128)), false),
        ]).is_err());
        assert!(Domains::new(&[], &[(Some((2, 1)), false)]).is_err());
        let mut domains = Domains::new(&[("formalparam", &[0])], &[(None, true)]).unwrap();
        assert!(domains.columns("formalparam", &["i32"]).is_err());
    }
}
