//! The caller's query-independent configuration, read by the structure
//! selection and Physical lowering passes.
//!
//! Hosts are facts, not claims: `R keyed [a]` says the host value is keyed
//! on column `a` (so a demand on `a` is answered in place), `ordered` that
//! the key has an order, `canon` that the value is a set of its rows (each
//! row once). Whether the value's type implements the traits those facts
//! lead the program to use is rustc's check. The rest is strategy.
//!
//! ```text
//! Policy ::= "policy" "{" Host* ["join" "=" ("seek" | "merge") ";"]
//!            ["semijoin" "=" ("eager" | "prefer_lazy") ";"]
//!            ["project" "=" ("keep" | "canon") ";"] "}"
//! Host   ::= Ident ["keyed" "[" Ident,* "]" ["ordered"]] ["canon"] ";"
//! ```

use syn::{Ident, Token, token};

use crate::CommaList;

pub mod kw {
    syn::custom_keyword!(policy);
    syn::custom_keyword!(keyed);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(ordered);
    syn::custom_keyword!(join);
    syn::custom_keyword!(semijoin);
    syn::custom_keyword!(seek);
    syn::custom_keyword!(merge);
    syn::custom_keyword!(eager);
    syn::custom_keyword!(prefer_lazy);
    syn::custom_keyword!(project);
    syn::custom_keyword!(keep);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Policy {
    pub policy_token: kw::policy,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(parse_hosts)]
    #[to_tokens(|tokens, hosts| quote::TokenStreamExt::append_all(tokens, hosts))]
    pub hosts: Vec<Host>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::join)]
    pub join: Option<JoinChoice>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::semijoin)]
    pub semijoin: Option<SemijoinChoice>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::project)]
    pub project: Option<ProjectChoice>,
}

fn parse_hosts(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Host>> {
    let mut hosts = Vec::new();
    while input.peek(Ident)
        && !input.peek(kw::join)
        && !input.peek(kw::semijoin)
        && !input.peek(kw::project)
    {
        hosts.push(input.parse()?);
    }
    Ok(hosts)
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Host {
    pub name: Ident,
    #[syn(optional, peek = kw::keyed)]
    pub key: Option<Key>,
    #[syn(optional, peek = kw::canon)]
    pub canon: Option<kw::canon>,
    pub semi_token: Token![;],
}

/// `keyed [a, b] [ordered]`: the host value's key columns, by name.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Key {
    pub keyed_token: kw::keyed,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
    pub ordered: Option<kw::ordered>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinChoice {
    pub join_token: kw::join,
    pub eq_token: Token![=],
    pub rule: JoinRule,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum JoinRule {
    #[parse(peek = kw::seek)]
    Seek(kw::seek),
    #[parse(peek = kw::merge)]
    Merge(kw::merge),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct SemijoinChoice {
    pub semijoin_token: kw::semijoin,
    pub eq_token: Token![=],
    pub rule: SemijoinRule,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum SemijoinRule {
    #[parse(peek = kw::eager)]
    Eager(kw::eager),
    #[parse(peek = kw::prefer_lazy)]
    PreferLazy(kw::prefer_lazy),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ProjectChoice {
    pub project_token: kw::project,
    pub eq_token: Token![=],
    pub rule: ProjectRule,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum ProjectRule {
    #[parse(peek = kw::keep)]
    Keep(kw::keep),
    #[parse(peek = kw::canon)]
    Canon(kw::canon),
}

impl Policy {
    /// Deduplicate projections unless their kept columns retain an inferred
    /// input key. Defaults to `false` (`project = keep`).
    pub fn canon_projections(&self) -> bool {
        matches!(
            self.project,
            Some(ProjectChoice {
                rule: ProjectRule::Canon(_),
                ..
            })
        )
    }

    pub fn merge_joins(&self) -> bool {
        matches!(
            self.join,
            Some(JoinChoice {
                rule: JoinRule::Merge(_),
                ..
            })
        )
    }

    pub fn prefer_lazy(&self) -> bool {
        matches!(
            self.semijoin,
            Some(SemijoinChoice {
                rule: SemijoinRule::PreferLazy(_),
                ..
            })
        )
    }
}
