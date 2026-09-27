//! Reads of a value: hosts through their declared layout and key, built
//! structures through their key, merge sides through their order.

use super::*;

/// A host read through a rename, in the rename's attribute names.
pub(super) struct HostAccess {
    pub(super) value: Ident,
    pub(super) keyed: bool,
    /// Key columns in the host's listed order.
    pub(super) key: Vec<String>,
    /// Remaining columns in the host's declared order.
    pub(super) payload: Vec<String>,
}

/// The value answering a keyed read.
pub(super) enum Access {
    Host(HostAccess),
    Built(Ident),
}

/// One side of a merge.
pub(super) enum Side {
    Host(HostAccess),
    Built(Ident),
    Pipe(TokenStream),
}

impl<'a> Printer<'a> {
    /// The host behind a rename, with its key and payload in the rename's
    /// attribute names.
    pub(super) fn host_access(&self, result: &str) -> HostAccess {
        let host = &self.hosts[&self.analysis.physical.hosts[result]];
        let inverse: BTreeMap<&String, &String> = self.analysis.physical.renames[result]
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
    pub(super) fn host_scan(&self, result: &str) -> TokenStream {
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
    pub(super) fn keyed(&self, result: &Ident, columns: &[String]) -> syn::Result<Access> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Access::Built(structure));
        }
        if self.analysis.host_key_for(&name, columns).is_some() {
            return Ok(Access::Host(self.host_access(&name)));
        }
        error(
            result,
            format!("no structure or host key answers the read of `{name}` on {columns:?}"),
        )
    }

    pub(super) fn built(&self, name: &str, columns: &[String]) -> Option<Ident> {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(name) else {
            return None;
        };
        structures
            .iter()
            .find(|(c, _)| c == columns)
            .map(|(_, s)| s.clone())
    }

    /// `&(k…,)`: the key a host map is entered with, constructed from the
    /// bound columns by the one copy the map's owned key type imposes.
    pub(super) fn host_key(&self, result: &str, access: &HostAccess) -> TokenStream {
        let key = access.key.iter().map(|c| {
            let ident = self.ident(result, c);
            quote!(::core::clone::Clone::clone(#ident))
        });
        quote!(&(#(#key,)*))
    }

    /// The cursor merging or ranging over `result` on sorted `columns`; a
    /// pipe sorted on them by its own merge is consumed here.
    pub(super) fn ordered(
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
        if let Some(Operator::Merge(_, k)) = self.physical.get(&name).map(|d| &d.operator) {
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
