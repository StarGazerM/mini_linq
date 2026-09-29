//! Reads of a value: hosts through their declared layout and key, built
//! structures through their key, merge sides through their order.

use super::*;

/// A host read through a rename, in the rename's attribute names.
pub struct HostAccess {
    pub value: Ident,
    pub keyed: bool,
    /// Key columns in the host's listed order.
    pub key: Vec<String>,
    /// Remaining columns in the host's declared order.
    pub payload: Vec<String>,
}

/// The value answering a keyed read.
pub enum Access {
    Host(HostAccess),
    Built(Ident),
    /// A curried structure read on a proper prefix of its levels: the
    /// structure and the prefix's levels.
    Prefix(Ident, Vec<Vec<String>>),
}

/// One side of a merge.
pub enum Side {
    Host(HostAccess),
    Built(Ident),
    Pipe(TokenStream),
}

impl<'a> Context<'a> {
    /// The host behind a rename, with its key and payload in the rename's
    /// attribute names.
    pub fn host_access(&self, result: &str) -> HostAccess {
        let host = &self.hosts[&self.physical_analysis.hosts[result]];
        let inverse: BTreeMap<&String, &String> = self.physical_analysis.renames[result]
            .iter()
            .map(|(target, source)| (source, target))
            .collect();
        let key: Vec<String> = host
            .key
            .iter()
            .flatten()
            .map(|c| inverse[c].clone())
            .collect();
        let payload = host
            .layout
            .iter()
            .filter(|c| !host.key.as_ref().is_some_and(|k| k.contains(c)))
            .map(|c| inverse[c].clone())
            .collect();
        HostAccess {
            value: host.value.clone(),
            keyed: host.key.is_some(),
            key,
            payload,
        }
    }

    /// Enumerate a host in declared order, rebound to the rename's names.
    pub fn host_scan(&self, result: &str) -> TokenStream {
        let access = self.host_access(result);
        let value = &access.value;
        let row = self.pattern(result);
        let payload = access.payload.iter().map(|c| self.ident(result, c));
        if !access.keyed {
            return quote! {
                ::core::iter::Iterator::map(#value.iter(), move |(#(#payload,)*)| #row)
            };
        }
        let key = access.key.iter().map(|c| self.ident(result, c));
        quote! {
            ::core::iter::Iterator::flat_map(#value.iter(), move |((#(#key,)*), __cq_values)| {
                ::core::iter::Iterator::map(__cq_values.iter(), move |(#(#payload,)*)| #row)
            })
        }
    }

    /// The value answering a read of `result` keyed on sorted `columns`: a
    /// structure built on them, else the host key the rename translates to.
    pub fn keyed(&self, result: &Ident, columns: &[String]) -> syn::Result<Access> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Access::Built(structure));
        }
        if let Some((levels, depth)) = self.built_levels(&name, columns) {
            let prefix = levels[..depth].to_vec();
            return Ok(Access::Prefix(self.built_ident(&name, levels), prefix));
        }
        if self.analysis.host_key_for(&name, columns).is_some() {
            return Ok(Access::Host(self.host_access(&name)));
        }
        error(
            result,
            format!("no structure or host key answers the read of `{name}` on {columns:?}"),
        )
    }

    pub fn built(&self, name: &str, columns: &[String]) -> Option<Ident> {
        self.built_levels(name, columns)
            .filter(|(levels, depth)| levels.len() == *depth)
            .map(|(levels, _)| self.built_ident(name, levels))
    }

    /// A built structure whose leading levels' union is `columns`: its
    /// levels and how many of them a read on `columns` applies.
    pub fn built_levels(
        &self,
        name: &str,
        columns: &[String],
    ) -> Option<(&Vec<Vec<String>>, usize)> {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(name) else {
            return None;
        };
        structures.iter().find_map(|(levels, _)| {
            structures::prefix_depth(levels, columns).map(|depth| (levels, depth))
        })
    }

    pub fn built_ident(&self, name: &str, levels: &[Vec<String>]) -> Ident {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(name) else {
            unreachable!("built_levels found it");
        };
        structures
            .iter()
            .find(|(l, _)| l == levels)
            .map(|(_, s)| s.clone())
            .expect("built_levels found it")
    }

    /// `&(k…,)`: the key a host map is entered with, constructed from the
    /// bound columns by the one copy the map's owned key type imposes.
    pub fn host_key(&self, result: &str, access: &HostAccess) -> TokenStream {
        let key = access.key.iter().map(|c| {
            let ident = self.ident(result, c);
            quote!(::core::clone::Clone::clone(#ident))
        });
        quote!(&(#(#key,)*))
    }

    /// The cursor merging or ranging over `result` on sorted `columns`; a
    /// pipe sorted on them by its own merge is consumed here.
    pub fn ordered(
        &mut self,
        reader: &Ident,
        result: &Ident,
        columns: &[String],
    ) -> syn::Result<Side> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Side::Built(structure));
        }
        if self
            .analysis
            .host_key_for(&name, columns)
            .is_some_and(|k| k.ordered)
        {
            let access = self.host_access(&name);
            if access.key != columns {
                return error(
                    reader,
                    format!(
                        "`{reader}` reads `{name}` in the order {columns:?}, but host `{}` is ordered by {:?}",
                        access.value, access.key
                    ),
                );
            }
            return Ok(Side::Host(access));
        }
        if let Some(Operator::Merge(k)) = self.physical.get(&name).map(|d| &d.operator) {
            if sorted(&k.columns) == columns {
                if let Some(rows) = self.take_pipe(&name) {
                    return Ok(Side::Pipe(rows));
                }
            }
        }
        error(
            reader,
            format!(
                "no ordered structure, host key or sorted stream answers the read of `{name}` on {columns:?}"
            ),
        )
    }
}
