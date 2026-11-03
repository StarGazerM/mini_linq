//! Authored modules check plan contracts independently of selection.
use cq_lang_structures as structures;
use quote::{ToTokens, quote};

#[test]
fn codec_declarations_intern_once_and_preserve_explicit_identity() {
    use structures::provider::{CodecId, InputSchema, intern_codecs};
    let mut input: InputSchema = syn::parse_quote!(input {
        keys 0;
        columns [
            A => Code codec Codec,
            B => Code codec Codec,
            C => Code codec [7] Other,
            D => Code codec [8] Other
        ]
    });
    intern_codecs(&mut input.columns).unwrap();
    assert_eq!(input.columns[0].codec_id, input.columns[1].codec_id);
    assert_eq!(input.columns[2].codec_id, Some(CodecId(7)));
    assert_eq!(input.columns[3].codec_id, Some(CodecId(8)));
    assert_ne!(input.columns[2].codec_id, input.columns[3].codec_id);
    let printed: InputSchema = syn::parse2(input.to_token_stream()).unwrap();
    assert_eq!(input, printed);
    let id = input.columns[0].codec_id;
    input.columns[0].codec = Some(syn::parse_quote!(RenamedOpaqueCodec));
    intern_codecs(&mut input.columns).unwrap();
    assert_eq!(input.columns[0].codec_id, id);
}

#[test]
fn codec_identity_exhaustion_is_a_parse_error() {
    use structures::provider::{CodecId, InputSchema, intern_codecs};
    let mut input: InputSchema = syn::parse_quote!(input {
        keys 0;
        columns [u32 => u32 codec [4294967295] Existing, u32 => u32 codec Fresh]
    });
    assert!(intern_codecs(&mut input.columns).is_err());
    input.columns[1].codec = Some(syn::parse_quote!(Existing));
    intern_codecs(&mut input.columns).unwrap();
    assert_eq!(input.columns[1].codec_id, Some(CodecId(u32::MAX)));

    let mut last: InputSchema = syn::parse_quote!(input {
        keys 0;
        columns [u32 => u32 codec [4294967294] Existing, u32 => u32 codec Last]
    });
    intern_codecs(&mut last.columns).unwrap();
    assert_eq!(last.columns[1].codec_id, Some(CodecId(u32::MAX)));

    let parsed = syn::parse_str::<structures::Structures>(
        "structures {
            providers {}
            r host scan = @input input { keys 0; columns [
                u32 => u32 codec [4294967295] Existing, u32 => u32 codec Fresh
            ] } as Input [];
        }",
    );
    assert!(parsed.is_err());
}

#[test]
fn logical_declarations_intern_once_and_preserve_explicit_identity() {
    use structures::provider::{InputSchema, LogicalTypeId, intern_logical_types};
    let mut input: InputSchema = syn::parse_quote!(input {
        keys 0;
        columns [
            A => Code,
            A => Code,
            [7] B => Code,
            [8] B => Code
        ]
    });
    intern_logical_types(&mut input.columns).unwrap();
    assert_eq!(
        input.columns[0].logical_type_id,
        input.columns[1].logical_type_id
    );
    assert_eq!(input.columns[2].logical_type_id, Some(LogicalTypeId(7)));
    assert_eq!(input.columns[3].logical_type_id, Some(LogicalTypeId(8)));
    assert_ne!(
        input.columns[2].logical_type_id,
        input.columns[3].logical_type_id
    );
    let printed: InputSchema = syn::parse2(input.to_token_stream()).unwrap();
    assert_eq!(input, printed);
    let id = input.columns[0].logical_type_id;
    input.columns[0].logical = syn::parse_quote!(RenamedOpaqueLogical);
    intern_logical_types(&mut input.columns).unwrap();
    assert_eq!(input.columns[0].logical_type_id, id);
}

#[test]
fn logical_identity_exhaustion_is_a_declaration_error() {
    use structures::provider::{InputSchema, intern_logical_types};
    let mut complete: InputSchema = syn::parse_quote!(input {
        keys 0; columns [[4294967295] Logical => Carrier]
    });
    assert!(intern_logical_types(&mut complete.columns).is_ok());
    let mut exhausted: InputSchema = syn::parse_quote!(input {
        keys 0; columns [[4294967295] Logical => Carrier, Other => Carrier]
    });
    assert!(intern_logical_types(&mut exhausted.columns).is_err());
    let mut allocation: InputSchema = syn::parse_quote!(input {
        keys 0; columns [[4294967294] Logical => Carrier, Other => Carrier]
    });
    assert!(intern_logical_types(&mut allocation.columns).is_err());
}

#[test]
fn input_declarations_share_the_relation_identity_namespace() {
    let program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a: First, b: Second);
            r = pipe rename R@0 {a -> a, b -> b};
            return r as answer(a,b);
        }
        structures {
            providers { reader = Reader { reads [], build typed }; }
            r host scan = @reader input {
                keys 0; columns [Second => Second, First => First]
            } as Reader [];
        }
    };
    let input = program.structures.entries[0]
        .structure
        .provider()
        .unwrap()
        .0
        .input
        .as_ref()
        .unwrap();
    let columns = &program.physical.program.declarations[0].columns;
    assert_eq!(input.columns[0].logical_type_id, columns[1].logical_type_id);
    assert_eq!(input.columns[1].logical_type_id, columns[0].logical_type_id);
}

/// Provider tokens that entry rewrites in these tests introduce (STR-Provider).
fn declarations() -> Vec<structures::Declaration> {
    let base: structures::Providers = syn::parse_quote!(providers {
        host_scan = ::core::iter::Map<_, _> { reads [], build typed };
        vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
        sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
        hash_index = ::cq_storage::HashIndex<'_, _, _> {
            reads [exact, membership], supports [keys], build typed, join_filter
        };
        adaptive_keyset = ::cq_storage::AdaptiveKeySet<_, _, _> { reads [membership], build typed, shared };
        alias = Alias { reads [exact, range, membership], supports [retain, expand, keys], build inferred };
        cursor_alias = CursorAlias<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
        directory = ::custom::Directory<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
        directory_alias = DirectoryAlias<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
        frozen_device_index = ::cq_gpu_runtime::FrozenDeviceIndex<_> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
        index = ::custom::Index<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter
        };
        index_alias = IndexAlias {
            reads [exact, range, membership], supports [retain, expand, keys], build inferred
        };
        key_only = KeyOnly { reads [exact, range, membership], supports [retain, expand, keys], build inferred };
        my_trie = MyTrie { reads [exact, range, membership], build inferred };
        other_trie = OtherTrie { reads [exact, range, membership], build inferred };
        payload_alias = PayloadAlias { reads [exact, range, membership], build inferred };
        same_alias = SameAlias { reads [exact, range, membership], supports [retain, expand, keys], build inferred };
        scan = Scan { reads [exact, range, membership], supports [retain, expand, keys], build inferred };
        snapshot = ::custom::Snapshot<_> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
        snapshot_alias = SnapshotAlias<_> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
        user_index = UserIndex { reads [exact, range, membership], build inferred };
    });
    base.declarations
}

/// Declares the tokens entry rewrites introduced and prunes replaced ones.
fn declared(program: &structures::Program) -> structures::Program {
    let mut program = program.clone();
    program.structures.declare(&declarations()).unwrap();
    program
}

fn check(program: &structures::Program) -> syn::Result<structures::Analysis> {
    structures::contract::check(&declared(program))
}

fn check_rewrite(source: &structures::Program, target: &structures::Program) -> syn::Result<()> {
    structures::check_rewrite(&declared(source), &declared(target))
}

#[test]
fn constant_occurrences_use_ordinary_storage() {
    let source: structures::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = let canon rename R@0 {a -> x} constants {b = 7};
        return r as answer(x);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r rows = @sort as ::cq_storage::dedup::Sort;
        r canon = rows;
    }};
    check(&source).unwrap();
    assert_eq!(
        syn::parse2::<structures::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for rows in [
        quote::quote!(r rows = @vec as ::std::vec::Vec<_> payload [] constants {};),
        quote::quote!(r rows = @vec as ::std::vec::Vec<_> payload [x] constants {b = 7};),
    ] {
        assert!(syn::parse2::<structures::Entry>(rows).is_err());
    }
    let indexed: structures::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32,c:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,c -> y} constants {b = 7};
        j = let canon probe first [x] l,r;
        return j as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r = @hash_index as ::cq_storage::HashIndex<'_,_,_> [x];
        j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
    }};
    let facts = check(&indexed).unwrap();
    assert!(!facts.filtered.contains_key("r"));
    assert!(!facts.retained.contains_key("r"));
    assert!(!facts.host_read("r"));
    for entry in [
        syn::parse_quote!(
            r = @hash_index as ::cq_storage::HashIndex<'_,_,_> [x] payload [x,y] constants {};
        ),
        syn::parse_quote!(r host result(j) = @hash_index as ::cq_storage::HashIndex<'_,_,_> [x];),
    ] {
        let mut invalid = indexed.clone();
        invalid.structures.entries[2] = entry;
        assert!(check(&invalid).is_err());
    }
}

fn smallest_expand_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,c -> c};
        s = let rename S@2 {a -> a,c -> c};
        candidate = pipe probe smallest [b] l,r;
        checked = let canon semi eager [a,c] candidate,s;
        return checked as answer(a,b,c);
    } structures { providers { directory = ::custom::Directory { reads [exact, range, membership], supports [retain, expand, keys], build inferred }; directory_alias = DirectoryAlias { reads [exact, range, membership], supports [retain, expand, keys], build inferred }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r = @directory as ::custom::Directory [b][c] supports [retain, expand];
        s = @directory_alias as DirectoryAlias [a][c] supports [expand];
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        r host scan = @host_scan as ::core::iter::Map<_, _> [];
        s host scan = @host_scan as ::core::iter::Map<_, _> [];
        r rows = @vec as ::std::vec::Vec<_>;
        s rows = @vec as ::std::vec::Vec<_>;
        checked rows = @sort as ::cq_storage::dedup::Sort; checked canon = rows;
        candidate stage = batch 256 expand;
    } }
}

fn resident_expand_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {a -> x,b -> y,c -> z};
        p = let canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; snapshot = ::custom::Snapshot<_> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host result(p) = @snapshot as ::custom::Snapshot<_> [x,y] supports [expand];
        p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows;
        p stage = batch 256 expand;
    }}
}

#[test]
fn expansion_accepts_declared_exact_host_cursors() {
    for provider in [
        syn::parse_quote!(r host result(p) = @snapshot as ::custom::Snapshot<_> [x,y] supports [expand];),
        syn::parse_quote!(r host result(p) = @snapshot_alias as SnapshotAlias<_> [x][y] supports [retain, expand];),
        syn::parse_quote!(r host result(p) = @frozen_device_index as ::cq_gpu_runtime::FrozenDeviceIndex<_> [y,x] supports [expand];),
    ] {
        let mut program = resident_expand_program();
        program.structures.entries[2] = provider;
        let facts = check(&program).unwrap();
        assert_eq!(facts.stages["p"], 256);
        assert!(facts.expands.contains("p"));
        assert!(!facts.built.contains_key("r"));
    }
}
#[test]
fn composite_prefix_ranges_support_expansion_and_shared_witnesses() {
    let source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            r = let rename R@1 {a -> x,b -> y,c -> z};
            s = pipe semi eager [x,y] l,r;
            p = let canon probe first [x,y] s,r;
            return p as answer(x,y,z);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; directory = ::custom::Directory<_,_> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @host_scan as ::core::iter::Map<_,_> [];
            r = @directory as ::custom::Directory<_,_> [x][y] supports [expand];
            r host scan = @host_scan as ::core::iter::Map<_,_> [];
            p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows;
            s share = lookup r [x,y] first s force p;
            s repr = tuples witness { r [x,y] at s as ::cq_storage::RunCursor<'_,_> };
            p stage = batch 256 expand;
        }
    };
    let facts = check(&source).unwrap();
    assert!(facts.expands.contains("p"));
    assert!(facts.witnesses.contains_key("s"));
    for provider in [
        syn::parse_quote!(r = @directory as ::custom::Directory<_,_> [x,y];),
        syn::parse_quote!(r = @directory_alias as DirectoryAlias<_,_> [x,y] supports [retain];),
        syn::parse_quote!(r = @directory as ::custom::Directory<_,_> [x][z][y] supports [expand];),
    ] {
        let mut invalid = source.clone();
        invalid.structures.entries[1] = provider;
        assert!(check(&invalid).is_err());
    }
}

#[test]
fn expansion_rejects_missing_support_and_wrong_exact_keys() {
    for provider in [
        syn::parse_quote!(r host result(p) = @snapshot as ::custom::Snapshot<_,_> [x,y];),
        syn::parse_quote!(r host result(p) = @snapshot_alias as SnapshotAlias<_> [x,y] supports [retain];),
        syn::parse_quote!(r host result(p) = @frozen_device_index as ::cq_gpu_runtime::FrozenDeviceIndex<_> [x,y];),
    ] {
        let mut program = resident_expand_program();
        program.structures.entries[2] = provider;
        assert!(check(&program).is_err());
    }
    let mut hashed = resident_expand_program();
    hashed.physical.program.definitions[1] =
        syn::parse_quote!(r = let rename R@1 {a -> x,b -> y,c -> z};);
    hashed.structures.entries[2] = syn::parse_quote!(r = @index as ::custom::Index<_,_> [x,y];);
    hashed
        .structures
        .entries
        .push(syn::parse_quote!(r rows = @vec as ::std::vec::Vec<_>;));
    assert!(check(&hashed).is_err());
    hashed
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::Stage { .. }));
    check(&hashed).unwrap();

    let mut mismatch = resident_expand_program();
    mismatch.structures.entries[2] = syn::parse_quote!(r host result(p) = @snapshot as ::custom::Snapshot<_> [x] supports [expand];);
    assert!(check(&mismatch).is_err());
}

fn membership_bridge_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        relation M(b:i64,tag:i64); relation S(b:i64,c:i64); relation T(c:i64,d:i64);
        l = pipe rename L@0 {a -> a};
        r = let rename R@1 {a -> a,b -> b};
        m = pipe rename M@2 {b -> b,tag -> tag};
        selected = let point select m if (tag = 7);
        s = let rename S@3 {b -> b,c -> c};
        t = let rename T@4 {c -> c,d -> d};
        first = pipe probe first [a] l,r;
        member = pipe probe first [b] first,selected;
        after = pipe probe first [b] member,s;
        last = pipe probe first [c] after,t;
        out = let canon project last keep {a,b,c,d,tag};
        return out as answer(a,b,c,d,tag);
    } structures { providers { adaptive_keyset = ::cq_storage::AdaptiveKeySet<_, _, _> { reads [membership], build typed, shared }; exact_cursor = ::custom::ExactCursor<_,_> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        selected = @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64> [b] membership payload [b] constants {tag = 7};
        r = @exact_cursor as ::custom::ExactCursor<_,_> [a] supports [expand];
        s = @exact_cursor as ::custom::ExactCursor<_,_> [b] supports [expand];
        t = @exact_cursor as ::custom::ExactCursor<_,_> [c] supports [expand];
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        m host scan = @host_scan as ::core::iter::Map<_,_> [];
        m host result(selected) = @host_scan as ::core::iter::Map<_,_> [tag];
        s host scan = @host_scan as ::core::iter::Map<_,_> [];
        t host scan = @host_scan as ::core::iter::Map<_,_> [];
        out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
    } }
}

#[test]
fn stages_cross_certified_bitmap_membership_with_recorded_constants() {
    let mut program = membership_bridge_program();
    program.structures.entries.extend([
        syn::parse_quote!(first stage = batch 256 expand;),
        syn::parse_quote!(after stage = batch 256 expand;),
        syn::parse_quote!(last stage = batch 256 expand;),
    ]);
    let facts = check(&program).unwrap();
    assert!(
        facts
            .membership
            .contains(&("selected".into(), vec!["b".into()]))
    );
    assert_eq!(facts.stages.len(), 3);
    assert!(!facts.expands.contains("member"));

    // The bridge is a total step, not an independent expand-capable cursor.
    program
        .structures
        .entries
        .push(syn::parse_quote!(member stage = batch 256 expand;));
    assert!(check(&program).is_err());
}

#[test]
fn stages_reject_uncertified_payload_probe_bridges() {
    let mut program = membership_bridge_program();
    program.structures.entries[0] = syn::parse_quote!(
        selected = @hash_index as ::cq_storage::HashIndex<'_,_,_> [b] payload [b] constants {tag = 7};
    );
    check(&program).unwrap();
    program.structures.entries.extend([
        syn::parse_quote!(first stage = batch 256 expand;),
        syn::parse_quote!(after stage = batch 256 expand;),
    ]);
    assert!(check(&program).is_err());
}

#[test]
fn stage_membership_bridges_require_proven_constants_and_complete_keys() {
    for entry in [
        syn::parse_quote!(selected = @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64> [b] membership payload [b] constants {tag = 8};),
        syn::parse_quote!(selected = @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64> [b] membership payload [b] constants {};),
        syn::parse_quote!(selected = @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64> [b,tag] membership payload [b] constants {tag = 7};),
    ] {
        let mut program = membership_bridge_program();
        program.structures.entries[0] = entry;
        program.structures.entries.extend([
            syn::parse_quote!(first stage = batch 256 expand;),
            syn::parse_quote!(after stage = batch 256 expand;),
        ]);
        assert!(check(&program).is_err());
    }
}

fn smallest_shared_prefixes() -> structures::Program {
    let mut program = smallest_expand_program();
    program
        .physical
        .program
        .definitions
        .insert(3, syn::parse_quote!(pre_r = pipe semi eager [b] l,r;));
    program
        .physical
        .program
        .definitions
        .insert(4, syn::parse_quote!(pre_s = pipe semi eager [a] pre_r,s;));
    program.physical.program.definitions[5] =
        syn::parse_quote!(candidate = pipe probe smallest [b] pre_s,r;);
    program.structures.entries.extend([
        syn::parse_quote!(pre_r share = lookup r [b] first pre_r force candidate;),
        syn::parse_quote!(pre_s share = lookup s [a] first pre_s force candidate;),
        syn::parse_quote!(pre_r repr = tuples witness { r [b] at pre_r as ::cq_storage::RunCursor<'_, _> };),
        syn::parse_quote!(pre_s repr = tuples witness { s [a] at pre_s as ::cq_storage::RunCursor<'_, _> };),
    ]);
    program
}

#[test]
fn smallest_force_consumes_two_independent_directory_witnesses() {
    let program = smallest_shared_prefixes();
    let facts = check(&program).unwrap();
    assert_eq!(facts.witnesses.len(), 2);
    assert_eq!(facts.shares["pre_r"].force, "candidate");
    assert_eq!(facts.shares["pre_s"].force, "candidate");
    assert_eq!(
        syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(),
        program
    );
}

#[test]
fn smallest_prefix_witness_requires_preserving_chain_and_no_escape() {
    let mut escaped = smallest_shared_prefixes();
    escaped.physical.program.definitions[4].canon = Some(Default::default());
    escaped.physical.program.definitions[4].mode = syn::parse_quote!(let);
    escaped.structures.entries.extend([
        syn::parse_quote!(pre_s rows = @sort as ::cq_storage::dedup::Sort;),
        syn::parse_quote!(pre_s canon = rows;),
    ]);
    assert!(check(&escaped).is_err());

    let mut changed = smallest_shared_prefixes();
    changed.physical.program.definitions[4] = syn::parse_quote!(pre_s = pipe semi eager [a] l,s;);
    assert!(check(&changed).is_err());

    let mut mismatch = smallest_shared_prefixes();
    let entry = mismatch.structures.entries.last_mut().unwrap();
    *entry = syn::parse_quote!(pre_s repr = tuples witness { r [b] at pre_s as ::cq_storage::RunCursor<'_, _> };);
    assert!(check(&mismatch).is_err());
}

#[test]
fn smallest_expansion_requires_support_on_both_selected_candidates() {
    let source = smallest_expand_program();
    let facts = check(&source).unwrap();
    assert_eq!(facts.stages["candidate"], 256);
    assert!(facts.expands.contains("candidate"));
    assert!(!facts.expands.contains("checked"));
    assert_eq!(
        syn::parse2::<structures::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for (index, entry) in [
        (0, syn::parse_quote!(r = @other_trie as OtherTrie [b][c];)),
        (1, syn::parse_quote!(s = @other_trie as OtherTrie [a][c];)),
        (
            0,
            syn::parse_quote!(r = @directory_alias as DirectoryAlias<_, _> [b][c] supports [retain];),
        ),
    ] {
        let mut missing = source.clone();
        missing.structures.entries[index] = entry;
        assert!(check(&missing).is_err());
        missing.structures.entries.pop();
        check(&missing).unwrap();
    }
}

#[test]
fn smallest_expansion_rejects_missing_or_misshaped_candidate_facts() {
    for (index, entry) in [
        (
            0,
            syn::parse_quote!(r = @directory as ::custom::Directory [b,c] supports [expand];),
        ),
        (
            1,
            syn::parse_quote!(s = @directory as ::custom::Directory [c][a] supports [expand];),
        ),
        (
            1,
            syn::parse_quote!(s = @directory as ::custom::Directory [a] supports [expand];),
        ),
    ] {
        let mut missing = smallest_expand_program();
        missing.structures.entries[index] = entry;
        assert!(check(&missing).is_err());
    }
    let mut missing = smallest_expand_program();
    missing.structures.entries.remove(1);
    assert!(check(&missing).is_err());
    // Without Smallest, the prefix is not a payload cursor over all built levels.
    let mut ordinary = smallest_expand_program();
    ordinary.physical.program.definitions[3] =
        syn::parse_quote!(candidate = pipe probe first [b] l,r;);
    assert!(check(&ordinary).is_err());
    let mut force = smallest_expand_program();
    force.physical.program.definitions[4] =
        syn::parse_quote!(checked = let canon probe first [a,c] candidate,s;);
    force
        .structures
        .entries
        .push(syn::parse_quote!(checked stage = batch 256 expand;));
    assert!(check(&force).is_err());
    let mut second = smallest_expand_program();
    second.structures.entries.pop();
    second
        .structures
        .entries
        .push(syn::parse_quote!(checked stage = batch 256 expand;));
    assert!(check(&second).is_err());
}

fn program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x}; r = let rename R@1 {a -> x,b -> y};
        j = let canon probe first [x] l,r; return j as answer(x,y);
    } structures { providers { user_index = UserIndex { reads [exact, range, membership], build inferred }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r = @user_index as UserIndex [x]; r rows = @vec as ::std::vec::Vec<_>; r host scan = @host_scan as ::core::iter::Map<_, _> []; l host scan = @host_scan as ::core::iter::Map<_, _> []; j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows; } }
}

#[test]
fn only_built_lets_have_entries_and_host_reads_make_no_claims() {
    let p = program();
    let a = check(&p).unwrap();
    assert!(a.host_read("l"));
    assert!(!a.host_read("r"));
    assert!(a.built_on("r", &["x".into()]).is_some());
    assert_eq!(a.host_layouts["R"], ["a", "b"]);
    assert!(!a.unique.contains_key("l"));
    assert!(!a.unique.contains_key("r"));
    assert_eq!(
        syn::parse2::<structures::Program>(p.to_token_stream()).unwrap(),
        p
    );
    for entries in [
        "r = @i as I [y];",
        "r = @i as I [x]; r = @j as J [x];",
        "l = @i as I [x];",
        "R = @i as I [a];",
        "r = @i as I [x][x];",
        "r = @i as I [absent];",
        "",
    ] {
        let mut q = p.clone();
        q.structures = syn::parse_str(&format!(
            "structures {{ providers {{
                i = I {{ reads [exact, range, membership], build inferred }};
                j = J {{ reads [exact, range, membership], build inferred }};
            }} {entries} }}"
        ))
        .unwrap();
        q.structures.declare(&[]).unwrap();
        assert!(check(&q).is_err(), "{entries}");
    }
    let mut q = p;
    q.physical.program.definitions[1].mode = syn::parse_quote!(pipe);
    assert!(check(&q).is_err());
    q.structures = syn::parse_quote!(structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; user_index = UserIndex { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        r host scan = @host_scan as ::core::iter::Map<_, _> [];
        r host result(j) = @user_index as UserIndex [x]; j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
    });
    check(&q).unwrap();
}

#[test]
fn canonicality_comes_only_from_plan_boundaries_and_key_preservation() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = let canon rename R@0 {a -> a,b -> b};
        e = pipe bind r with n: i32 = (1);
        p = pipe project e keep {a,b}; return p as answer(a,b);
    } structures { providers { sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; } r canon = rows; r rows = @sort as ::cq_storage::dedup::Sort; r host scan = @host_scan as ::core::iter::Map<_, _> []; } };
    let a = check(&p).unwrap();
    assert_eq!(a.unique["p"], ["a", "b"]);
    let mut q = p.clone();
    q.physical.program.definitions[0].canon = None;
    assert!(check(&q).is_err());
    let mut q = p;
    q.physical.program.definitions[2] = syn::parse_quote!(p = pipe project e keep {a};);
    q.physical.program.head = syn::parse_quote!(answer(a));
    assert!(check(&q).is_err());
}

#[test]
fn ordered_premises_require_storage_or_a_same_key_merge_stream() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32);
        l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a,b -> b}; s = pipe rename S@2 {b -> b};
        j = let merge[a] l,r; k = let canon merge[b] j,s; return k as answer(a,b);
    } structures { providers { ordered = Ordered { reads [exact, range, membership], build inferred }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; }
        j = @ordered as Ordered [b]; j rows = @vec as ::std::vec::Vec<_>; k rows = @sort as ::cq_storage::dedup::Sort; k canon = rows;
        l host scan = @host_scan as ::core::iter::Map<_, _> []; r host scan = @host_scan as ::core::iter::Map<_, _> []; s host scan = @host_scan as ::core::iter::Map<_, _> [];
        l host result(j) = @ordered as Ordered [a]; r host result(j) = @ordered as Ordered [a]; s host result(k) = @ordered as Ordered [b];
    } };
    check(&p).unwrap();
    let mut q = p.clone();
    q.structures.entries.clear();
    assert!(check(&q).is_err());
    q.physical.program.definitions[3].mode = syn::parse_quote!(pipe);
    assert!(check(&q).is_err());
    let mut q = p;
    q.physical.program.definitions[3].mode = syn::parse_quote!(pipe);
    assert!(check(&q).is_err());
}

#[test]
fn leveled_entries_cover_prefixes_but_do_not_prove_uniqueness() {
    let mut p = program();
    p.structures.entries[0] = syn::parse_quote!(r = @my_trie as MyTrie [][x];);
    let a = check(&p).unwrap();
    assert_eq!(a.built_on("r", &["x".into()]).unwrap().1, 2);
    assert_eq!(a.built_on("r", &[]).unwrap().1, 1);
    p.physical.program.definitions[2].canon = None;
    assert!(check(&p).is_err());
}

#[test]
fn complete_read_structures_take_precedence_over_candidate_prefixes() {
    let source: structures::Program = syn::parse_quote!(physical {
        relation L(x:i32); relation R(x:i32,y:i32);
        l = pipe rename L@0 {x -> x}; r = let rename R@1 {x -> x,y -> y};
        j = pipe probe first [x] l,r;
        checked = let canon probe first [x,y] j,r;
        return checked as answer(x,y);
    } structures { providers { curried = Curried { reads [exact, range, membership], build inferred }; full = Full { reads [exact, range, membership], build inferred }; payload = Payload { reads [exact, range, membership], build inferred }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r = @curried as Curried [x][y]; r = @full as Full [x,y]; r = @payload as Payload [x];
        r rows = @vec as ::std::vec::Vec<_>; r host scan = @host_scan as ::core::iter::Map<_, _> [];
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        checked rows = @sort as ::cq_storage::dedup::Sort; checked canon = rows;
    });
    let mut reordered = source.clone();
    reordered.structures.entries.reverse();
    for program in [&source, &reordered] {
        let analysis = check(program).unwrap();
        assert_eq!(
            analysis.built_on("r", &["x".into()]).unwrap(),
            (&syn::parse_quote!(@payload as Payload), 1)
        );
        assert_eq!(
            analysis.built_on("r", &["x".into(), "y".into()]).unwrap(),
            (&syn::parse_quote!(@full as Full), 1)
        );
    }
    let mut missing = source.clone();
    missing.structures.entries.remove(2);
    assert!(check(&missing).is_err());
    let mut duplicate = source.clone();
    duplicate
        .structures
        .entries
        .push(source.structures.entries[0].clone());
    assert!(check(&duplicate).is_err());
}

#[test]
fn a_const_only_read_cannot_justify_a_built_entry() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation R(a: i32);
        r = let rename R@0 { a -> x };
        p = let canon point select r if (x = 5);
        return p as answer(x);
    } structures { providers { vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r rows = @vec as ::std::vec::Vec<_>; r host scan = @host_scan as ::core::iter::Map<_, _> []; p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows; } };
    check(&p).unwrap();
    let mut invalid = p;
    invalid
        .structures
        .entries
        .push(syn::parse_quote!(r = @user_index as UserIndex [x];));
    assert!(check(&invalid).is_err());
}

#[test]
fn const_reads_can_coexist_with_storage_justified_by_a_stream() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32);
        l = pipe rename L@0 { a -> x }; r = let rename R@1 { a -> x };
        p = let point select r if (x = 5);
        j = pipe probe first [x] l,r;
        out = let canon semi eager [x] j,p;
        return out as answer(x);
    } structures { providers { user_index = UserIndex { reads [exact, range, membership], build inferred }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r = @user_index as UserIndex [x]; p = @user_index as UserIndex [x]; r rows = @vec as ::std::vec::Vec<_>; r host scan = @host_scan as ::core::iter::Map<_, _> []; p rows = @vec as ::std::vec::Vec<_>; out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows; l host scan = @host_scan as ::core::iter::Map<_, _> []; } };
    check(&p).unwrap();
}

#[test]
fn payload_probes_must_cover_every_built_key_level() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32, payload: f64);
        l = pipe rename L@0 { a -> x };
        r = let rename R@1 { a -> x, payload -> y };
        j = pipe probe first [x] l,r;
        out = let canon project j keep {x};
        return out as answer(x);
    } structures { providers { user_trie = UserTrie { reads [exact, range, membership], build inferred }; } r = @user_trie as UserTrie [x][y]; } };
    assert!(check(&p).is_err());
}

#[test]
fn representation_and_stage_entries_round_trip() {
    for text in [
        "r repr = tuples;",
        "r repr = positions { H lineage { a -> c }, J lineage { b -> d } } owned [x, y];",
        "r stage = batch 128;",
        "s share = lookup b [x] first s force p;",
        "s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_, _> };",
        "s repr = positions { L lineage { x -> a } } owned [] witness { b [x] at s as Cursor };",
        "p stage = batch 32 expand;",
    ] {
        let entry: structures::Entry = syn::parse_str(text).unwrap();
        assert_eq!(
            syn::parse2::<structures::Entry>(entry.to_token_stream()).unwrap(),
            entry
        );
    }
}

fn represented_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = pipe rename R@0 {a -> x,b -> y};
        s = pipe select r if (x >= 0);
        b = pipe bind s with z: i32 = (x + y);
        out = let canon project b keep {x,y,z};
        return out as answer(x,y,z);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r host scan = @host_scan as ::core::iter::Map<_, _> [];
        out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
    } }
}

#[test]
fn representation_and_staging_are_checked_additive_rewrites() {
    let source = represented_program();
    let defaults = check(&source).unwrap();
    assert!(defaults.repr.is_empty());
    assert!(defaults.stages.is_empty());
    let mut target = source.clone();
    target.structures.entries.extend([
        syn::parse_quote!(r repr = tuples;),
        syn::parse_quote!(out repr = positions { R lineage { x -> a, y -> b } } owned [z];),
        syn::parse_quote!(s stage = batch 128;),
    ]);
    check_rewrite(&source, &target).unwrap();
    let facts = check(&target).unwrap();
    assert!(matches!(
        facts.repr["out"],
        structures::Repr::Positions { .. }
    ));
    assert_eq!(facts.stages["s"], 128);
}

#[test]
fn representation_rejects_uncovered_noncomputed_and_forged_columns() {
    for entry in [
        "out repr = positions { R lineage { x -> a } } owned [z];",
        "out repr = positions { R lineage { x -> a, y -> b } } owned [x,z];",
        "out repr = positions { R lineage { x -> a, y -> a } } owned [z];",
        "out repr = positions { R lineage { x -> b, y -> a } } owned [z];",
        "out repr = positions { Missing lineage { x -> a, y -> b } } owned [z];",
    ] {
        let mut program = represented_program();
        program
            .structures
            .entries
            .push(syn::parse_str(entry).unwrap());
        assert!(check(&program).is_err(), "{entry}");
    }
}

#[test]
fn stages_reject_unsupported_adapters_cursor_producers_and_zero_batches() {
    for entry in [
        syn::parse_quote!(b stage = batch 16;),
        syn::parse_quote!(out stage = batch 16;),
        syn::parse_quote!(r stage = batch 0;),
    ] {
        let mut program = represented_program();
        program.structures.entries.push(entry);
        assert!(check(&program).is_err());
    }
    let mut cursor = program();
    cursor
        .structures
        .entries
        .push(syn::parse_quote!(j stage = batch 16;));
    assert!(check(&cursor).is_err());

    let mut guard = represented_program();
    guard.physical.program.definitions[1] = syn::parse_quote!(s = pipe guard r if (*x >= 0););
    guard
        .structures
        .entries
        .push(syn::parse_quote!(s stage = batch 16;));
    assert!(check(&guard).is_err());
}

#[test]
fn representation_and_stage_decisions_are_unique_per_result() {
    for entries in [
        vec![
            syn::parse_quote!(r repr = tuples;),
            syn::parse_quote!(r repr = tuples;),
        ],
        vec![
            syn::parse_quote!(s stage = batch 16;),
            syn::parse_quote!(s stage = batch 32;),
        ],
    ] {
        let mut program = represented_program();
        program.structures.entries.extend(entries);
        assert!(check(&program).is_err());
    }
}

#[test]
fn joined_representations_carry_both_bases_without_relaxing_read_lineage() {
    let mut program = program();
    program.structures.entries.push(syn::parse_quote!(
        j repr = positions { L lineage {x -> a}, R lineage {y -> b} } owned [];
    ));
    let facts = check(&program).unwrap();
    assert!(facts.repr.contains_key("j"));
    assert!(!facts.lineage.contains_key("j"));
}

#[test]
fn stages_accept_relational_constraints_multiple_boundaries_and_retained_producers() {
    let mut program = represented_program();
    program.physical.program.definitions[1] = syn::parse_quote!(
        s = pipe select r if (x >= 0);
    );
    program.structures.entries.extend([
        syn::parse_quote!(r stage = batch 8;),
        syn::parse_quote!(s stage = batch 16;),
    ]);
    check(&program).unwrap();

    program
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::Stage { .. }));
    program.physical.program.definitions[2].mode = syn::parse_quote!(let);
    program.structures.entries.extend([
        syn::parse_quote!(b rows = @vec as ::std::vec::Vec<_>;),
        syn::parse_quote!(out stage = batch 16;),
    ]);
    check(&program).unwrap();
}

#[test]
fn generator_bind_round_trip_drops_inherited_uniqueness() {
    let source: structures::Program = syn::parse_quote! { physical {
        u = pipe unit;
        g = pipe bind u with for x = ([1, 1, 2]);
        return canon g as answer(x);
    } structures { providers { sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        g head = @sort as ::cq_storage::dedup::Sort;
    } };
    let facts = check(&source).unwrap();
    assert!(!facts.unique.contains_key("g"));
    assert!(facts.physical.rows["g"].computed.contains("x"));
    assert_eq!(
        syn::parse2::<structures::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let mut invalid = source.clone();
    invalid.physical.program.return_canon = None;
    assert!(check(&invalid).is_err());
    let mut invalid = source;
    invalid.physical.program.definitions[1].operator =
        syn::parse_quote!(bind u with for x = ([*x]));
    assert!(check(&invalid).is_err());
}

fn shared_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        b = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,b;
        p = let canon probe first [x] s,b;
        return p as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; exact_cursor = ::custom::ExactCursor<_,_> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        b host scan = @host_scan as ::core::iter::Map<_, _> [];
        b rows = @vec as ::std::vec::Vec<_>;
        b = @exact_cursor as ::custom::ExactCursor<_,_> [x] supports [retain, expand, keys];
        p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows;
        s share = lookup b [x] first s force p;
        s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_, _> };
    } }
}

#[test]
fn lookup_sharing_witness_and_expansion_are_additive() {
    let mut target = shared_program();
    target
        .structures
        .entries
        .push(syn::parse_quote!(p stage = batch 32 expand;));
    let facts = check(&target).unwrap();
    assert_eq!(facts.shares["s"].force, "p");
    assert!(facts.witnesses.contains_key("s"));
    assert!(facts.expands.contains("p"));
    assert_eq!(
        syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
        target
    );
    let mut source = target.clone();
    source.structures.entries.truncate(6);
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn retained_snapshot_witness_requires_identical_provider_and_key() {
    let mut program: structures::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        b = pipe rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,b;
        p = let canon probe first [x] s,b;
        return p as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; snapshot = ::custom::Snapshot<_,_,Layout0> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        b host scan = @host_scan as ::core::iter::Map<_,_> [];
        b host result(s) = @snapshot as ::custom::Snapshot<_,_,Layout0> [x] supports [retain, expand];
        b host result(p) = @snapshot as ::custom::Snapshot<_,_,Layout0> [x] supports [expand];
        p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows;
        s share = lookup b [x] first s force p;
        s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_,_> };
        p stage = batch 256 expand;
    } };
    let facts = check(&program).unwrap();
    assert!(facts.witnesses.contains_key("s"));
    assert!(facts.expands.contains("p"));
    program.structures.entries[3] = syn::parse_quote! {
        b host result(p) = @snapshot as ::custom::Snapshot<_,_,Layout1> [x] supports [expand];
    };
    assert!(check(&program).is_err());
}

#[test]
fn sharing_rejects_probe_before_semi() {
    let mut p = shared_program();
    p.physical.program.definitions.swap(2, 3);
    assert!(check(&p).is_err());
}

#[test]
fn witness_requires_matching_share() {
    let mut p = shared_program();
    p.structures.entries.remove(6);
    assert!(check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_canon() {
    let mut p = shared_program();
    p.physical.program.definitions[2].canon = Some(Default::default());
    p.physical.program.definitions[2].mode = syn::parse_quote!(let);
    p.structures.entries.extend([
        syn::parse_quote!(s rows = @sort as ::cq_storage::dedup::Sort;),
        syn::parse_quote!(s canon = rows;),
    ]);
    assert!(check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_head() {
    let mut p = shared_program();
    p.physical.program.returned = syn::parse_quote!(s);
    p.physical.program.head = syn::parse_quote!(answer(x));
    p.physical.program.definitions[2].canon = Some(Default::default());
    p.structures
        .entries
        .push(syn::parse_quote!(s canon = rows;));
    assert!(check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_retained_let() {
    let mut p = shared_program();
    p.physical.program.definitions[2].mode = syn::parse_quote!(let);
    p.structures
        .entries
        .push(syn::parse_quote!(s rows = @vec as ::std::vec::Vec<_>;));
    assert!(check(&p).is_err());
}

#[test]
fn retain_does_not_imply_expand() {
    let mut p = shared_program();
    p.structures.entries[3] =
        syn::parse_quote!(b = @cursor_alias as CursorAlias<_,_> [x] supports [retain];);
    p.structures
        .entries
        .push(syn::parse_quote!(p stage = batch 16 expand;));
    assert!(check(&p).is_err());
}

#[test]
fn expand_cannot_stage_across_bind() {
    let mut p = shared_program();
    p.physical
        .program
        .definitions
        .push(syn::parse_quote!(q = pipe bind p with z:i32 = (1);));
    p.physical.program.returned = syn::parse_quote!(q);
    p.physical.program.head = syn::parse_quote!(answer(x, y, z));
    p.structures.entries.extend([
        syn::parse_quote!(p stage = batch 16 expand;),
        syn::parse_quote!(q stage = batch 16;),
    ]);
    assert!(check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_union_arm() {
    let mut p = shared_program();
    p.physical.program.definitions[3] = syn::parse_quote!(p = pipe probe first [x] s,b;);
    p.physical.program.definitions.extend([
        syn::parse_quote!(q = pipe project p keep {x};),
        syn::parse_quote!(u = let canon union s,q;),
    ]);
    p.physical.program.returned = syn::parse_quote!(u);
    p.physical.program.head = syn::parse_quote!(answer(x));
    p.structures.entries[4] = syn::parse_quote!(u rows = @sort as ::cq_storage::dedup::Sort;);
    p.structures.entries[5] = syn::parse_quote!(u canon = rows;);
    assert!(check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_fused_prefix() {
    let mut p = shared_program();
    p.physical.program.definitions[3] = syn::parse_quote!(p = pipe probe first [x] s,b;);
    p.physical.program.definitions.extend([
        syn::parse_quote!(q = pipe probe first [x] s,b;),
        syn::parse_quote!(u = let canon union fused p,q block @2 free [x] interface [y] over s;),
    ]);
    p.physical.program.returned = syn::parse_quote!(u);
    p.physical.program.head = syn::parse_quote!(answer(x, y));
    p.structures.entries[4] = syn::parse_quote!(u rows = @sort as ::cq_storage::dedup::Sort;);
    p.structures.entries[5] = syn::parse_quote!(u canon = rows;);
    assert!(check(&p).is_err());
}

#[test]
fn canonical_consumer_accepts_bag_return_and_forbids_head_dedup() {
    let program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            consumer canonical;
            r = pipe rename R@0 {a -> x};
            return r as answer(*x % 2);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; } r host scan = @host_scan as ::core::iter::Map<_,_> []; }
    };
    let facts = check(&program).unwrap();
    assert!(!facts.unique.contains_key("r"));
    assert!(!program.physical.program.head_dedup());
    assert!(facts.head_dedup.is_empty());
    let mut missing = program.clone();
    missing.physical.program.consumer_canonical = None;
    assert!(check(&missing).is_err());
    let mut invalid = program;
    invalid.structures.entries.push(syn::parse_quote!(
        r head = @sort as ::cq_storage::dedup::Sort;
    ));
    assert!(check(&invalid).is_err());
}

#[test]
fn discharged_host_uniqueness_does_not_survive_a_lossy_projection() {
    let program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            host_set R discharged;
            r = pipe rename R@0 {a -> x,b -> y};
            p = pipe project r keep {x,y};
            return p as answer(x,y);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; } r host scan = @host_scan as ::core::iter::Map<_,_> []; }
    };
    let facts = check(&program).unwrap();
    assert_eq!(facts.unique["p"], ["x", "y"]);
    let mut lossy = program;
    lossy.physical.program.definitions[1] = syn::parse_quote!(p = pipe project r keep {x};);
    lossy.physical.program.head = syn::parse_quote!(answer(x));
    assert!(check(&lossy).is_err());
}

#[test]
fn provider_support_packets_roundtrip_and_expand_implies_retain_only() {
    use structures::ProviderCapability::{Expand, Keys, Retain};
    for text in [
        "@provider as ::custom::Provider<_,_> [y,x] supports [retain, expand, keys]",
        "@provider_alias as ProviderAlias [x][y] supports [expand]",
        "@provider_alias as ProviderAlias [x] supports [retain]",
        "@provider_alias as ProviderAlias [x] supports [keys]",
        "@provider_alias as ProviderAlias [x] supports []",
        "@provider_alias as ProviderAlias [x]",
    ] {
        let keyed: structures::Keyed = syn::parse_str(text).unwrap();
        assert_eq!(
            syn::parse2::<structures::Keyed>(keyed.to_token_stream()).unwrap(),
            keyed
        );
    }
    let expanded: structures::Keyed = syn::parse_quote!(@alias as Alias [x] supports [expand]);
    assert!(expanded.supports(Expand));
    assert!(expanded.supports(Retain));
    assert!(!expanded.supports(Keys));
    let retained: structures::Keyed = syn::parse_quote!(@alias as Alias [x] supports [retain]);
    assert!(retained.supports(Retain));
    assert!(!retained.supports(Expand));
    assert!(!retained.supports(Keys));
    assert!(syn::parse_str::<structures::Keyed>("@alias as Alias [x] supports [unknown]").is_err());
}

#[test]
fn support_validation_rejects_duplicates_empty_keys_and_membership_cursors() {
    for packet in [
        syn::parse_quote!(r = @alias as Alias [x] supports [retain, retain];),
        syn::parse_quote!(r = @alias as Alias [x] supports [expand, expand];),
        syn::parse_quote!(r = @alias as Alias [x] supports [keys, keys];),
    ] {
        let mut invalid = program();
        invalid.structures.entries[0] = packet;
        assert!(check(&invalid).is_err());
    }
    let mut empty = program();
    empty.structures.entries[2] =
        syn::parse_quote!(r host scan = @scan as Scan [] supports [keys];);
    assert!(check(&empty).is_err());
    for packet in [
        syn::parse_quote!(selected = @key_only as KeyOnly [b] supports [retain]
            membership payload [b] constants {tag = 7};),
        syn::parse_quote!(selected = @key_only as KeyOnly [b] supports [expand]
            membership payload [b] constants {tag = 7};),
    ] {
        let mut invalid = membership_bridge_program();
        invalid.structures.entries[0] = packet;
        assert!(check(&invalid).is_err());
    }
}

#[test]
fn support_follows_selected_layout_not_any_matching_type_or_prefix() {
    use structures::ProviderCapability::{Expand, Retain};
    let mut source = program();
    // Both layouts answer x, but a complete key takes precedence over a prefix,
    // even when its identical Rust type has no declared support.
    source.structures.entries[0] =
        syn::parse_quote!(r = @same_alias as SameAlias [x][y] supports [expand];);
    source
        .structures
        .entries
        .push(syn::parse_quote!(r = @same_alias as SameAlias [x];));
    let facts = check(&source).unwrap();
    assert_eq!(facts.built_on("r", &["x".into()]).unwrap().1, 1);
    assert!(!facts.provider_support("r", &["x".into()], None, Retain));
    assert!(facts.provider_support("r", &["y".into(), "x".into()], None, Expand));
    assert!(!facts.provider_support("r", &["y".into()], None, Expand));
    assert!(!facts.provider_support("r", &["x".into()], Some("j"), Expand));
    source.structures.entries.reverse();
    let facts = check(&source).unwrap();
    assert!(!facts.provider_support("r", &["x".into()], None, Retain));
}

#[test]
fn abstract_sharing_does_not_require_a_retained_cursor_but_witnesses_do() {
    let types: [structures::Provider; 2] = [
        syn::parse_quote!(@index as ::custom::Index<_,_>),
        syn::parse_quote!(@index_alias as IndexAlias),
    ];
    for ty in types {
        let mut source = shared_program();
        source.structures.entries[3] = syn::parse_quote!(b = #ty [x];);
        assert!(check(&source).is_err());
        source.structures.entries.pop();
        assert!(check(&source).unwrap().shares.contains_key("s"));
    }
}

#[test]
fn host_support_is_exact_to_reader_key_and_snapshot_layout() {
    use structures::ProviderCapability::Expand;
    let mut source = resident_expand_program();
    source
        .physical
        .program
        .definitions
        .insert(2, syn::parse_quote!(s = pipe semi eager [x,y] l,r;));
    source.physical.program.definitions[3] =
        syn::parse_quote!(p = let canon probe first [x,y] s,r;);
    source.structures.entries.extend([
        syn::parse_quote!(r host result(s) = @snapshot as ::custom::Snapshot<_> [x,y] supports [retain];),
        syn::parse_quote!(s share = lookup r [x,y] first s force p;),
        syn::parse_quote!(s repr = tuples witness { r [x,y] at s as _ };),
    ]);
    let facts = check(&source).unwrap();
    assert!(facts.provider_support("r", &["y".into(), "x".into()], Some("p"), Expand));
    assert!(!facts.provider_support("r", &["x".into(), "y".into()], Some("s"), Expand));
    assert!(!facts.provider_support("r", &["x".into()], Some("p"), Expand));
    assert!(!facts.provider_support("r", &["x".into(), "y".into()], Some("other"), Expand));
    assert!(structures::retained_host_read(&source, "r", "s", &["y".into(), "x".into()]).is_some());
    assert!(structures::retained_host_read(&source, "r", "s", &[]).is_none());
    assert!(structures::retained_host_read(&source, "r", "s", &["x".into(), "x".into()]).is_none());
    for provider in [
        syn::parse_quote!(r host result(p) = @snapshot as ::custom::Snapshot<_> [x][y] supports [expand];),
        syn::parse_quote!(r host result(p) = @snapshot as ::custom::Snapshot<_> [y,x] supports [expand];),
    ] {
        source.structures.entries[2] = provider;
        assert!(check(&source).is_err());
    }
}

#[test]
fn join_filter_uses_declared_keys_and_preserves_scalar_and_totality_proofs() {
    let source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            p = let canon probe first [x] l,r;
            return p as answer(x,y);
        } structures { providers { scan = Scan { reads [exact, range, membership], build inferred }; index = ::custom::Index<_,_> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; filter = Filter { reads [exact, range, membership], build inferred }; }
            l host scan = @scan as Scan [];
            r host scan = @scan as Scan [];
            r = @index as ::custom::Index<_,_> [x] supports [keys];
            p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows;
            l join_filter = @filter as Filter  build r [x] covering p marker ::cq_load::ColumnAt<0>;
        }
    };
    let facts = check(&source).unwrap();
    assert_eq!(
        structures::join_filter_candidates(&source, &facts, "p").len(),
        1
    );
    for packet in [
        syn::parse_quote!(r = @index as ::custom::Index<_,_> [x];),
        syn::parse_quote!(r = @index_alias as IndexAlias [x] supports [retain, expand];),
    ] {
        let mut missing = source.clone();
        missing.structures.entries[2] = packet;
        assert!(check(&missing).is_err());
    }
    let mut alias = source.clone();
    alias.structures.entries[2] =
        syn::parse_quote!(r = @index_alias as IndexAlias [x] supports [keys];);
    check(&alias).unwrap();
    let mut scalar = source.clone();
    scalar.physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            p = let canon probe first [x] l,r;
            return p as answer(x,y);
        }
    };
    check(&scalar).unwrap();
    let mut partial = source.clone();
    partial.physical = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x};
            raw = pipe rename R@1 {a -> x,b -> y};
            r = let bind raw with z: i64 = (1);
            p = let canon probe first [x] l,r;
            return p as answer(x,y,z);
        }
    };
    partial.structures.entries[1] = syn::parse_quote!(raw host scan = @scan as Scan [];);
    assert!(check(&partial).is_err());
    let mut lazy = source.clone();
    lazy.physical.program.definitions[2] = syn::parse_quote!(p = let canon probe lazy [x] l,r;);
    assert!(check(&lazy).is_err());
}

#[test]
fn candidate_support_is_independent_of_complete_payload_selection() {
    use structures::ProviderCapability::Expand;
    let mut source = smallest_shared_prefixes();
    source.structures.entries.push(syn::parse_quote!(
        r = @payload_alias as PayloadAlias [b];
    ));
    let facts = check(&source).unwrap();
    assert!(!facts.provider_support("r", &["b".into()], None, Expand));
    assert!(facts.candidate_support("r", &["b".into()], "c", Expand));
    assert!(facts.witnesses.contains_key("pre_r"));
    assert!(facts.expands.contains("candidate"));
    assert_eq!(
        facts.built_on("r", &["b".into()]).unwrap().0,
        &syn::parse_quote!(@payload_alias as PayloadAlias)
    );
    assert_eq!(
        facts.candidate_on("r", &["b".into()], "c").unwrap(),
        &syn::parse_quote!(@directory as ::custom::Directory)
    );
    // A capable ordinary payload cannot confer support on the directory.
    source.structures.entries[0] = syn::parse_quote!(r = @directory as ::custom::Directory [b][c];);
    *source.structures.entries.last_mut().unwrap() =
        syn::parse_quote!(r = @payload_alias as PayloadAlias [b] supports [expand];);
    assert!(check(&source).is_err());
}

#[test]
fn declared_retain_does_not_relax_eager_sharing() {
    let mut source = shared_program();
    source.physical.program.definitions[2] = syn::parse_quote!(s = pipe semi lazy [x] l,b;);
    assert!(check(&source).is_err());
}

#[test]
fn canon_reference_rejects_legacy_provider_syntax() {
    assert!(
        syn::parse_str::<structures::Entry>("r canon = ::std::collections::HashSet<_>;").is_err()
    );
    let entry: structures::Entry = syn::parse_quote!(r canon = rows;);
    assert!(matches!(
        entry.structure,
        structures::Structure::Dedup {
            index: structures::CanonIndex::Rows(_),
            ..
        }
    ));
}

#[test]
fn ordered_canon_requires_complete_direct_source() {
    let source: structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            r = pipe canon rename R@0 {a -> x,b -> y};
            return r as answer(x,y);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; }
            r host scan = @host_scan as ::core::iter::Map<_,_> [];
            r canon = host;
        }
    };
    check(&source).unwrap();
    let projected: structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            r = pipe rename R@0 {a -> x,b -> y};
            p = let canon project r keep {x};
            return p as answer(x);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; }
            r host scan = @host_scan as ::core::iter::Map<_,_> [];
            p canon = host;
        }
    };
    assert!(check(&projected).is_err());
}

fn provider_program(structures: proc_macro2::TokenStream) -> structures::Program {
    syn::parse2(quote::quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> x}; r = let rename R@1 {a -> x,b -> y};
            j = let canon probe first [x] l,r; return j as answer(x,y);
        }
        #structures
    })
    .unwrap()
}

fn provider_entries() -> proc_macro2::TokenStream {
    quote::quote! {
        r rows = @vec as ::std::vec::Vec<_>;
        r host scan = @host_scan as ::core::iter::Map<_, _> [];
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
    }
}

fn reject_providers(
    table: proc_macro2::TokenStream,
    built: proc_macro2::TokenStream,
) {
    let entries = provider_entries();
    let program = provider_program(quote::quote!(structures {
        providers {
            vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
            host_scan = ::core::iter::Map<_, _> { reads [], build typed };
            sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
            #table
        }
        #built
        #entries
    }));
    assert!(structures::contract::check(&program).is_err());
}

#[test]
fn providers_table_round_trips_every_fact() {
    let providers: structures::Providers = syn::parse_quote!(providers {
        sorted_run = ::cq_storage::SortedRun<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed,
            join_filter, shared
        };
        trie = ::cq_storage::Trie<'_, _, _> { reads [exact, membership], build typed, nested 2 };
        sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared };
        sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
        hash_set = ::cq_storage::HashSet<_> { reads [membership, ordered, resident], build typed, accounting keys };
        packed = ::cq_storage::dedup::Packed { reads [distinct], build inferred, packed_bits 64 };
        range = OpaqueRange { reads [range], abi range, contiguous_range };
        relation = OpaqueRadix { reads [exact], abi radix_relation };
        columns = OpaqueColumns { reads [], abi columns, bounds resident_columns };
    });
    assert_eq!(providers.declarations.len(), 9);
    assert_eq!(
        syn::parse2::<structures::Providers>(providers.to_token_stream()).unwrap(),
        providers
    );
    let entries = provider_entries();
    let program = provider_program(quote::quote!(structures {
        providers {
            hash_set = ::cq_storage::HashSet<_> { reads [membership], build typed, accounting keys, shared };
            trie = ::cq_storage::Trie<'_, _, _> { reads [exact, membership], build typed, nested 2 };
            vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
            host_scan = ::core::iter::Map<_, _> { reads [], build typed };
            sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
        }
        r = @trie(@hash_set as ::cq_storage::HashSet<_>)
            as ::cq_storage::Trie<'_, _, ::cq_storage::HashSet<_>> [x];
        #entries
    }));
    assert_eq!(
        syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(),
        program
    );
    let analysis = structures::contract::check(&program).unwrap();
    let (built, _) = analysis.built_on("r", &["x".into()]).unwrap();
    assert_eq!(
        built.tokens().map(ToString::to_string).collect::<Vec<_>>(),
        ["trie", "hash_set"]
    );
    assert_eq!(
        built,
        &syn::parse_quote!(@trie(@hash_set as ::cq_storage::HashSet<_>)
            as ::cq_storage::Trie<'_, _, ::cq_storage::HashSet<_>>)
    );
}

#[test]
fn str_provider_rejects_undeclared_unreferenced_and_duplicate_tokens() {
    let index = quote::quote!(index = Index { reads [exact, membership], supports [keys], build inferred };);
    let built = quote::quote!(r = @index as Index [x];);
    reject_providers(quote!(), built.clone());
    reject_providers(
        quote!(#index),
        quote!(),
    );
    reject_providers(
        quote!(#index #index),
        built,
    );
}

#[test]
fn str_provider_rejects_misshaped_instances_and_undeclared_support() {
    let trie = quote::quote!(trie = ::cq_storage::Trie<'_, _, _> { reads [exact, membership], build typed, nested 2 };);
    let index = quote::quote!(index = Index { reads [exact, membership], supports [keys], build inferred };);
    let sharded = quote::quote!(sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0 };);
    reject_providers(
        quote!(#trie),
        quote!(r = @trie as ::cq_storage::Trie<'_, _, _> [x][y];),
    );
    reject_providers(
        quote!(#index),
        quote!(r = @index(@index as Index) as Index [x];),
    );
    reject_providers(
        quote!(#sharded #index),
        quote!(r = @sharded(@sharded(@index as Index) as ::cq_storage::sharded::Sharded<Index>)
            as ::cq_storage::sharded::Sharded<::cq_storage::sharded::Sharded<Index>> [x];),
    );
    reject_providers(
        quote!(#index),
        quote!(r = @index as Index [x] supports [expand];),
    );
}

#[test]
fn str_routed_restricts_inner_facts_to_exact_membership_and_keys() {
    let entries = provider_entries();
    let program = provider_program(quote::quote!(structures {
        providers {
            sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared };
            sorted_run = ::cq_storage::SortedRun<_, _> {
                reads [exact, range, membership], supports [retain, expand, keys], build typed,
                join_filter, shared
            };
            vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
            host_scan = ::core::iter::Map<_, _> { reads [], build typed };
            sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
        }
        r = @sharded(@sorted_run as ::cq_storage::SortedRun<_, _>)
            as ::cq_storage::sharded::Sharded<::cq_storage::SortedRun<_, _>> [x];
        #entries
    }));
    let analysis = structures::contract::check(&program).unwrap();
    let (built, _) = analysis.built_on("r", &["x".into()]).unwrap();
    let facts = analysis.facts(built);
    assert_eq!(
        facts.reads,
        [
            structures::Capability::Exact,
            structures::Capability::Membership
        ]
    );
    assert_eq!(facts.supports, [structures::ProviderCapability::Keys]);
    assert!(facts.join_filter);
    assert!(facts.shared);
    assert_eq!(facts, program.structures.providers.facts(built).unwrap());
    assert!(
        !analysis
            .facts(built.inner.as_deref().unwrap())
            .supports
            .is_empty()
    );
}

#[test]
fn native_input_round_trip_preserves_opaque_types_and_explicit_codecs() {
    let provider: structures::Provider = syn::parse_quote!(
        @native input {
            keys 1;
            columns [
                <Logical as Trait>::Key => Storage,
                String => Symbol<String, Codec> codec Codec,
                Symbol<String, Codec> => Symbol<String, Codec>
            ]
        } as unrelated::Alias
    );
    assert_eq!(
        syn::parse2::<structures::Provider>(provider.to_token_stream()).unwrap(),
        provider
    );
    let input = provider.input.as_ref().unwrap();
    assert_eq!(input.key_count, 1);
    assert!(input.columns[0].codec.is_none());
    assert_eq!(input.columns[1].codec, Some(syn::parse_quote!(Codec)));
    assert!(input.columns[2].codec.is_none());
    let declaration: structures::Declaration =
        syn::parse_quote!(native = unrelated::Alias { reads [], abi radix_relation };);
    assert!(declaration.instance().input.is_none());
}

#[test]
fn native_declaration_mutations_reject_inconsistent_facts() {
    for table in [
        quote!(index = Alias { reads [range], contiguous_range };),
        quote!(index = Alias { reads [range], abi radix_relation, contiguous_range };),
        quote!(index = Alias { reads [exact], abi range, contiguous_range };),
    ] {
        reject_providers(
            table,
            quote!(r = @index as Alias [x];),
        );
    }
    for table in [
        quote!(index = Alias { reads [], bounds resident_columns };),
        quote!(index = Alias { reads [], abi radix_relation, bounds resident_columns };),
    ] {
        reject_providers(
            table,
            quote!(r = @index as Alias [x];),
        );
    }
    reject_providers(
        quote!(index = Alias<_> { reads [], abi radix_relation, nested 0 };),
        quote!(r = @index as Alias<_> [x];),
    );
}

fn native_input_program(abi: proc_macro2::TokenStream) -> structures::Program {
    let advice = (abi.to_string() == "radix_relation").then(|| quote!(
        radix { word KeyCarrier; offset u64; ratio 4; radix_bits 0; merge_tile 32 }
    ));
    provider_program(quote! {
        structures {
            providers {
                vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
                host_scan = ::core::iter::Map<_, _> { reads [], build typed };
                sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
                index = Index { reads [exact], build inferred };
                native = Arbitrary { reads [], abi #abi };
            }
            r = @index as Index [x];
            r rows = @vec as ::std::vec::Vec<_>;
            r host scan = @native input {
                keys 2; columns [i32 => Carrier, i32 => Encoded codec Codec]
            } #advice as CompletelyDifferent [];
            l host scan = @host_scan as ::core::iter::Map<_, _> [];
            j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
        }
    })
}

#[test]
fn native_host_schema_mutations_are_checked_without_type_recognition() {
    for abi in [quote!(radix_relation), quote!(columns)] {
        let program = native_input_program(abi);
        structures::contract::check(&program).unwrap();
        assert_eq!(
            syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(),
            program
        );
        for missing in [false, true] {
            let mut mutated = program.clone();
            for entry in &mut mutated.structures.entries {
                if let Some(provider) = entry.structure.provider_mut()
                    && provider.token == "native"
                {
                    if missing {
                        provider.input = None;
                    } else {
                        provider.input.as_mut().unwrap().key_count = 3;
                    }
                }
            }
            assert!(structures::contract::check(&mutated).is_err());
        }
        for wrong_identity in [false, true] {
            let mut mutated = program.clone();
            for entry in &mut mutated.structures.entries {
                if let Some(provider) = entry.structure.provider_mut()
                    && provider.token == "native"
                {
                    let input = provider.input.as_mut().unwrap();
                    if wrong_identity {
                        input.columns[0].logical_type_id = Some(structures::LogicalTypeId(99));
                    } else {
                        input.columns.pop();
                    }
                }
            }
            assert!(structures::contract::check(&mutated).is_err());
        }
        let mut renamed = program.clone();
        for entry in &mut renamed.structures.entries {
            if let Some(provider) = entry.structure.provider_mut()
                && provider.token == "native"
            {
                provider.input.as_mut().unwrap().columns[0].logical = syn::parse_quote!(Other);
            }
        }
        assert!(structures::contract::check(&renamed).is_ok());
    }
}

#[test]
fn native_fact_parser_rejects_unknown_and_duplicate_declarations() {
    for declaration in [
        quote!(native = Alias { abi unknown };),
        quote!(native = Alias { abi range, abi radix_relation };),
        quote!(native = Alias { contiguous_range, contiguous_range };),
        quote!(native = Alias { bounds unknown };),
        quote!(native = Alias { bounds resident_columns, bounds resident_columns };),
    ] {
        assert!(syn::parse2::<structures::Declaration>(declaration).is_err());
    }
}
