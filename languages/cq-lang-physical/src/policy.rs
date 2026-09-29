//! The caller's query-independent configuration, read by the structure
//! selection and Physical lowering passes.
//!
//! Policy contains only query-independent strategy. Host facts belong to declarations.
//!
//! ```text
//! Policy ::= "policy" "{" ["join" "=" ("seek" | "merge") ";"]
//!            ["semijoin" "=" ("eager" | "prefer_lazy") ";"]
//!            ["project" "=" ("keep" | "canon") ";"]
//!            ["intersect" "=" ("first" | "smallest") ";"] "}"
//! ```

use syn::{Token, token};

pub mod kw {
    syn::custom_keyword!(policy);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(join);
    syn::custom_keyword!(semijoin);
    syn::custom_keyword!(seek);
    syn::custom_keyword!(merge);
    syn::custom_keyword!(eager);
    syn::custom_keyword!(prefer_lazy);
    syn::custom_keyword!(project);
    syn::custom_keyword!(keep);
    syn::custom_keyword!(intersect);
    syn::custom_keyword!(first);
    syn::custom_keyword!(smallest);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Policy {
    pub policy_token: kw::policy,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::join)]
    pub join: Option<JoinChoice>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::semijoin)]
    pub semijoin: Option<SemijoinChoice>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::project)]
    pub project: Option<ProjectChoice>,
    #[syn(in = brace)]
    #[syn(optional, peek = kw::intersect)]
    pub intersect: Option<IntersectChoice>,
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

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct IntersectChoice {
    pub intersect_token: kw::intersect,
    pub eq_token: Token![=],
    pub rule: IntersectRule,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum IntersectRule {
    #[parse(peek = kw::first)]
    First(kw::first),
    #[parse(peek = kw::smallest)]
    Smallest(kw::smallest),
}

impl Policy {
    /// Compare candidate counts for supported binary intersections. Default: first.
    pub fn smallest_intersections(&self) -> bool {
        matches!(
            self.intersect,
            Some(IntersectChoice {
                rule: IntersectRule::Smallest(_),
                ..
            })
        )
    }
}
