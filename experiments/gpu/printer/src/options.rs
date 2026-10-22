use std::collections::BTreeSet;
use syn::{
    Ident, LitInt, Token,
    parse::{Parse, ParseStream},
};

pub(crate) const DEFAULT_RETRY_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum OutputPolicy {
    #[default]
    Resumable,
    Retry,
}

/// Control choices do not alter any Structures representation decision.
#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub items: usize,
    pub frontier: u32,
    pub capacity: Option<Ident>,
    pub output: OutputPolicy,
    pub datalog_keys: Vec<(Ident, Vec<syn::Type>)>,
    pub datalog_dictionary: Option<Ident>,
    pub datalog_pool: Option<Ident>,
    pub defer_completion: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            items: 4,
            frontier: 8,
            capacity: None,
            output: OutputPolicy::default(),
            datalog_keys: Vec::new(),
            datalog_dictionary: None,
            datalog_pool: None,
            defer_completion: false,
        }
    }
}
impl Parse for Options {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut options = Self::default();
        let mut seen = BTreeSet::new();
        while !input.is_empty() {
            let option: Ident = input.parse()?;
            if !seen.insert(option.to_string()) {
                return Err(syn::Error::new_spanned(
                    option,
                    "CQ_GPU_DUPLICATE_ARGUMENT: repeated printer option",
                ));
            }
            let args;
            syn::parenthesized!(args in input);
            match option.to_string().as_str() {
                "tile" => {
                    let threads: LitInt = args.parse()?;
                    args.parse::<Token![,]>()?;
                    let items: LitInt = args.parse()?;
                    let items = items.base10_parse::<usize>()?;
                    if threads.base10_parse::<u32>()? != 256 || items != 4 {
                        return Err(syn::Error::new_spanned(
                            option,
                            "CQ_GPU_TILE: only tile(256,4) is supported",
                        ));
                    }
                    options.items = items;
                }
                "frontier" => {
                    options.frontier = args.parse::<LitInt>()?.base10_parse()?;
                }
                "directory" => {
                    let directory: Ident = args.parse()?;
                    if directory != "rank" {
                        return Err(syn::Error::new_spanned(
                            directory,
                            "CQ_GPU_DIRECTORY: only directory(rank) is supported",
                        ));
                    }
                }
                "capacity" => {
                    options.capacity = Some(args.parse()?);
                }
                "output" => {
                    let policy: Ident = args.parse()?;
                    if policy != "retry" {
                        return Err(syn::Error::new_spanned(
                            policy,
                            "CQ_GPU_OUTPUT_POLICY: only output(retry) is supported",
                        ));
                    }
                    options.output = OutputPolicy::Retry;
                }
                "datalog_dictionary" => {
                    options.datalog_dictionary = Some(args.parse()?);
                }
                "datalog_pool" => {
                    options.datalog_pool = Some(args.parse()?);
                }
                "completion" => {
                    let boundary: Ident = args.parse()?;
                    if boundary != "round" && boundary != "query" {
                        return Err(syn::Error::new_spanned(
                            boundary,
                            "CQ_GPU_COMPLETION: expected completion(query) or completion(round)",
                        ));
                    }
                    options.defer_completion = boundary == "round";
                }
                "datalog_keys" => {
                    let mut names = BTreeSet::new();
                    while !args.is_empty() {
                        let name: Ident = args.parse()?;
                        if !names.insert(name.to_string()) {
                            return Err(syn::Error::new_spanned(
                                name,
                                "CQ_GPU_DATALOG_KEYS: duplicate key host",
                            ));
                        }
                        args.parse::<Token![:]>()?;
                        let tuple: syn::TypeTuple = args.parse()?;
                        options
                            .datalog_keys
                            .push((name, tuple.elems.into_iter().collect()));
                        if args.is_empty() {
                            break;
                        }
                        args.parse::<Token![,]>()?;
                    }
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        option,
                        "CQ_GPU_ARGUMENT: unknown printer option",
                    ));
                }
            }
            if !args.is_empty() {
                return Err(args.error("CQ_GPU_ARGUMENT: unexpected option arguments"));
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(options)
    }
}
