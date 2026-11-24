//! Native host reads (STR-NativeHost): structure selection records, for a
//! keyed read of a host, the declared host capability that answers it
//! (IF-HostDecl). Every other host read records its fallback provider
//! (`host` entries); constant points and windows without a native record scan
//! and filter. The record is checked against the input's declaration only:
//! the capability must be declared on exactly the read's key, and must be the
//! one the reader's operator needs. No Rust type is inspected; generated code
//! makes one direct call per record and rustc checks the host's trait.
//! ```text
//! Entry ::= Ident "native" ReaderRef "=" HostRead "[" Ident,* "]" [Support] ";"
//! ```
//! Keys name result attributes; a `scan` record names the host columns of the
//! scan's leading literal constants, read by one native point read.
use super::*;
use cq_syntax::HostRead;

/// A recorded native read: its declared capability and key.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct NativeRead {
    pub read: cq_syntax::host::HostReadToken,
    pub key: Level,
    #[parse(parse_support)]
    pub supports: Option<ProviderSupport>,
}

/// Checked facts of one native record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Native {
    pub read: HostRead,
    /// Key columns in recorded order.
    pub key: Vec<String>,
    /// The explicitly indexed declaration slot of a non-prefix one-column key.
    pub slot: Option<usize>,
    /// The host stores borrowed key columns (IF-HostDecl `borrowed`).
    pub borrowed: bool,
    pub supports: Option<ProviderSupport>,
}

impl Native {
    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.supports
            .as_ref()
            .is_some_and(|support| support.supports(capability))
    }
}

/// The capability a reader's operator needs from a native host read.
fn needed(
    physical: &crate::physical::Program,
    _program: &Program,
    analysis: &Analysis,
    name: &str,
    reader: &ReaderRef<String>,
    key: &[String],
) -> Vec<HostRead> {
    match reader {
        ReaderRef::Scan => vec![HostRead::Exact],
        ReaderRef::Candidates => vec![HostRead::Candidates],
        ReaderRef::Membership => vec![HostRead::Membership],
        ReaderRef::Result(result) => {
            let Some(definition) = physical
                .program
                .definitions
                .iter()
                .find(|d| symbol_name(&d.name) == *result)
            else {
                return Vec::new();
            };
            match &definition.operator {
                Operator::Point(..) | Operator::Probe(..) => {
                    let full = analysis.physical.headings[name] == key;
                    if full && matches!(definition.operator, Operator::Probe(..)) {
                        vec![HostRead::Exact, HostRead::Membership]
                    } else {
                        vec![HostRead::Exact]
                    }
                }
                Operator::Range(..) => vec![HostRead::Range, HostRead::Ordered],
                Operator::Merge(..) => vec![HostRead::Ordered],
                // An eager semi may open the retained exact cursor its share
                // forces later (ST-Share); otherwise it tests membership.
                Operator::Semi(..) => vec![HostRead::Membership, HostRead::Exact],
                Operator::Anti(..) => vec![HostRead::Membership],
                _ => Vec::new(),
            }
        }
    }
}

/// Formal rule STR-NativeHost: check one `native` entry against its host
/// declaration and record it. Returns false for other entries.
pub(crate) fn entry(
    physical: &crate::physical::Program,
    program: &Program,
    entry: &Entry,
    analysis: &mut Analysis,
) -> syn::Result<bool> {
    let Structure::Native { reader, read, .. } = &entry.structure else {
        return Ok(false);
    };
    let name = symbol_name(&entry.name);
    let scan = matches!(reader, ReaderRef::Scan);
    if !(analysis.host_read(&name) || scan && analysis.physical.hosts.contains_key(&name)) {
        return error(
            &entry.name,
            "a native read requires a reusable host rename or host scan",
        );
    }
    let input = &analysis.physical.hosts[&name];
    let declaration = physical
        .inputs()
        .find(|input_decl| symbol_name(&input_decl.name) == *input)
        .expect("checked host input");
    let recorded: Vec<String> = read.key.columns.iter().map(symbol_name).collect();
    let mut key = recorded.clone();
    key.sort();
    if key.is_empty() || key.windows(2).any(|pair| pair[0] == pair[1]) {
        return error(&read.key, "a native read key names distinct columns");
    }
    // Host column names of the key: a scan names them directly; other reads
    // name result attributes, renamed back to the host's declared columns.
    let mut host_key = Vec::new();
    for column in &key {
        let host_column = if scan {
            Some(column.clone())
        } else {
            analysis.physical.renames[&name].get(column).cloned()
        };
        let Some(host_column) = host_column else {
            return error(&read.key, format!("`{name}` has no column `{column}`"));
        };
        host_key.push(host_column);
    }
    host_key.sort();
    let Some(host) = declaration.host(&host_key) else {
        return error(
            &read.key,
            format!("`{input}` declares no host read on {host_key:?} (IF-HostDecl)"),
        );
    };
    // The recorded key is the host's key layout: its columns in declaration
    // order. A permuted key would probe the host with a reordered tuple.
    let layout = recorded.iter().map(|column| {
        if scan {
            column.clone()
        } else {
            analysis.physical.renames[&name][column].clone()
        }
    });
    let declared = declaration
        .columns
        .iter()
        .map(|column| symbol_name(&column.name))
        .filter(|column| host_key.contains(column));
    if !layout.eq(declared) {
        return error(
            &read.key,
            "a native read key lists the host key in declaration order",
        );
    }
    let capability = read.read.read;
    if !host.reads(capability) {
        return error(
            &read.read,
            format!(
                "`{input}` does not declare a `{}` host read on {host_key:?}",
                capability.name()
            ),
        );
    }
    let reader_name = reader.key();
    let valid = match &reader_name {
        ReaderRef::Scan => {
            let Some(Operator::Rename(rename)) = physical
                .program
                .definitions
                .iter()
                .find(|d| symbol_name(&d.name) == name)
                .map(|d| &d.operator)
            else {
                unreachable!("checked host rename")
            };
            let literals: BTreeSet<String> = rename
                .constants
                .iter()
                .flat_map(|constants| &constants.equalities)
                .map(|equality| symbol_name(&equality.left))
                .collect();
            let mut prefix: Vec<String> = declaration
                .columns
                .iter()
                .map(|column| symbol_name(&column.name))
                .take_while(|column| literals.contains(column))
                .collect();
            prefix.sort();
            prefix == key
        }
        ReaderRef::Candidates | ReaderRef::Membership => {
            physical::candidate_prefixes(physical, &analysis.physical)
                .iter()
                .flat_map(|candidate| &candidate.sides)
                .any(|(side, prefix)| {
                    *side == name
                        && key
                            == if matches!(reader_name, ReaderRef::Candidates) {
                                prefix.clone()
                            } else {
                                analysis.physical.headings[&name].clone()
                            }
                })
        }
        ReaderRef::Result(result) => {
            let keyed = analysis
                .physical
                .keyed_reads
                .get(&name)
                .into_iter()
                .flatten()
                .any(|(r, columns, _)| r == result && *columns == key);
            let point = physical.program.definitions.iter().any(|d| {
                symbol_name(&d.name) == *result
                    && matches!(&d.operator, Operator::Point(_, select)
                    if select.condition.constant_keys().is_some_and(|keys| {
                        keys.keys().cloned().collect::<Vec<_>>() == key
                    }))
            });
            let range = physical.program.definitions.iter().any(|d| {
                symbol_name(&d.name) == *result
                    && matches!(&d.operator, Operator::Range(_, select)
                        if select.condition.range_bounds().is_some_and(|(column, _)| [column] == *key))
            });
            keyed || point || range
        }
    };
    if !valid {
        return error(reader, "native read does not match a host read on its key");
    }
    if !needed(physical, program, analysis, &name, &reader_name, &key).contains(&capability) {
        return error(
            &read.read,
            format!(
                "the reader needs another host read than `{}`",
                capability.name()
            ),
        );
    }
    let prefix = declaration.prefix(&host_key);
    if capability == HostRead::Ordered
        && matches!(&reader_name, ReaderRef::Result(result) if physical.program.definitions.iter().any(|d| symbol_name(&d.name) == *result && matches!(d.operator, Operator::Merge(..))))
    {
        // A merge reads keys in sorted column order; the host's order is its
        // declaration order, so both must agree.
        let order: Vec<String> = declaration
            .columns
            .iter()
            .take(key.len())
            .map(|column| {
                analysis.physical.renames[&name]
                    .iter()
                    .find(|(_, source)| **source == symbol_name(&column.name))
                    .map(|(target, _)| target.clone())
                    .expect("total host rename")
            })
            .collect();
        if order != key {
            return error(
                &read.key,
                "a native merge reads a prefix declared in sorted key order",
            );
        }
    }
    let slot = (!prefix).then(|| {
        declaration
            .columns
            .iter()
            .position(|column| symbol_name(&column.name) == host_key[0])
            .expect("checked declared column")
    });
    if slot.is_some()
        && matches!(&reader_name, ReaderRef::Result(result) if physical.program.definitions.iter().any(|d| symbol_name(&d.name) == *result && matches!(d.operator, Operator::Point(..))))
    {
        return error(&read.key, "a native point reads a leading host key");
    }
    if let Some(support) = &read.supports {
        let mut seen = BTreeSet::new();
        for token in &support.capabilities {
            let capability = token.capability();
            if !seen.insert(capability) {
                return error(token, "duplicate provider capability");
            }
            let declared = match capability {
                ProviderCapability::Retain => cq_syntax::HostSupport::Retain,
                ProviderCapability::Expand => cq_syntax::HostSupport::Expand,
                ProviderCapability::Keys => {
                    return error(token, "a native host read supports only retain and expand");
                }
            };
            if !host.supports(declared) {
                return error(
                    token,
                    format!("`{input}` does not declare this host support"),
                );
            }
        }
        if read.read.read != HostRead::Exact || !matches!(reader_name, ReaderRef::Result(_)) {
            return error(support, "retained native reads are exact keyed reads");
        }
        analysis.host_support.insert(
            (name.clone(), reader_name.clone(), key.clone()),
            support.clone(),
        );
    }
    // A full-row-key native membership probe observes existence only.
    if capability == HostRead::Membership
        && matches!(&reader_name, ReaderRef::Result(result) if physical.program.definitions.iter().any(|d| symbol_name(&d.name) == *result && matches!(d.operator, Operator::Probe(..))))
    {
        analysis.membership.insert((name.clone(), key.clone()));
    }
    let native = Native {
        read: capability,
        key: recorded,
        slot,
        borrowed: host.borrowed.is_some(),
        supports: read.supports.clone(),
    };
    if analysis
        .natives
        .insert((name, reader_name, key), native)
        .is_some()
    {
        return error(entry, "duplicate native host read");
    }
    Ok(true)
}

/// Every keyed read has at most one record: native or fallback.
pub(crate) fn check(analysis: &Analysis) -> syn::Result<()> {
    for (name, reader, key) in analysis.natives.keys() {
        if matches!(reader, ReaderRef::Scan)
            && analysis.physical.canon.contains(name)
            && analysis.physical.modes[name] == ModeKind::Pipe
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "a native scan cannot replace an ordered canonical host source",
            ));
        }
        if !matches!(reader, ReaderRef::Scan)
            && analysis
                .fallbacks
                .contains_key(&(name.clone(), reader.clone(), key.clone()))
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{name}` records both a native read and a fallback on {key:?}"),
            ));
        }
    }
    Ok(())
}
