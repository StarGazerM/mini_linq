use cq_lang_physical as physical;
use quote::ToTokens;
#[test]
fn computed_outputs_use_the_declared_logical_identity() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: Logical [7]);
        relation answer(value: Logical [8]);
        r = pipe rename R {a -> x};
        b = pipe bind r with y: Logical = (*x);
        p = let canon project b keep {y};
        return p as answer(y);
    }};
    let facts = physical::contract::check(&source).unwrap();
    assert_eq!(
        facts.rows["b"].logical_type_ids["x"],
        cq_syntax::LogicalTypeId(7)
    );
    assert_eq!(
        facts.rows["b"].logical_type_ids["y"],
        cq_syntax::LogicalTypeId(8)
    );
}

#[test]
fn constant_occurrences_drop_source_columns_and_require_retention() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = let canon rename R {a -> x} constants {b = -7};
        return r as answer(x);
    }};
    let facts = physical::contract::check(&source).unwrap();
    assert_eq!(facts.headings["r"], ["x"]);
    assert_eq!(facts.renames["r"].len(), 1);
    assert_eq!(
        syn::parse2::<physical::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for definition in [
        syn::parse_quote!(r = pipe canon rename R {a -> x} constants {b = -7};),
        syn::parse_quote!(r = let canon rename R {a -> x} constants {a = 7};),
        syn::parse_quote!(r = let canon rename R {a -> x} constants {missing = 7};),
        syn::parse_quote!(r = let canon rename R {a -> x} constants {b = x};),
        syn::parse_quote!(r = let canon rename R {a -> x} constants {};),
    ] {
        let mut invalid = source.clone();
        invalid.program.definitions[0] = definition;
        assert!(physical::contract::check(&invalid).is_err());
    }
}

const SOURCE: &str = "physical {
    relation R(a:i32,b:i32); relation S(b:i32,c:i32);
    r = pipe rename R {a -> x,b -> y}; s = let rename S {b -> y,c -> z};
    j = pipe probe first [y] r,s; f = pipe guard j if (*x < *z);
    e = pipe bind f with w:i32 = (*x + *z);
    p = let canon project e keep {x,w}; return p as answer(x,w);
}";
#[test]
fn complete_program_round_trip_and_analysis() {
    let source: physical::Program = syn::parse_str(SOURCE).unwrap();
    let a = physical::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<physical::Program>(source.to_token_stream()).unwrap(),
        source
    );
    assert_eq!(a.headings["p"], ["w", "x"]);
    assert_eq!(a.hosts["r"], "R");
    assert_eq!(a.renames["r"]["x"], "a");
    assert_eq!(a.rows["p"].bindings["x"], "x");
    assert!(a.rows["p"].computed.contains("w"));
    assert_eq!(a.key_reads["s"], [("j".into(), vec!["y".into()])]);
    assert!(a.scanned.contains("r"));
    assert!(!a.scanned.contains("s"));
}
#[test]
fn invalid_operators_modes_and_heads_are_rejected() {
    for (before, after) in [
        ("rename R", "rename Missing"),
        ("a -> x,b -> y", "a -> x,a -> y"),
        ("a -> x,b -> y", "a -> x,b -> x"),
        ("a -> x,b -> y", "a -> x"),
        ("a -> x,b -> y", "a -> x,q -> y"),
        ("r,s", "r,missing"),
        ("s = let", "r = let"),
        ("probe first [y]", "probe first [x]"),
        ("probe first [y]", "probe first [y,y]"),
        ("= (*x + *z)", "= (*q + *w)"),
        ("with w:i32", "with x:i32"),
        ("keep {x,w}", "keep {x,missing}"),
        ("keep {x,w}", "keep {x,x}"),
        ("answer(x,w)", "answer(x)"),
        ("return p", "return missing"),
        (
            "return p",
            "unused = pipe rename R {a -> a,b -> b}; return p",
        ),
        ("guard j if (*x < *z)", "guard p if (*x < *z)"),
    ] {
        let changed: physical::Program = syn::parse_str(&SOURCE.replace(before, after)).unwrap();
        assert!(physical::contract::check(&changed).is_err(), "{before} -> {after}");
    }
}
#[test]
fn merge_and_point_record_supply_kinds() {
    let source: physical::Program = syn::parse_str("physical { relation R(a:i32); relation S(a:i32); r = let rename R {a -> x}; s = let rename S {a -> x}; j = pipe merge[x] r,s; f = pipe point select j if (x = 1); return f as answer(x); }").unwrap();
    let a = physical::contract::check(&source).unwrap();
    assert_eq!(a.merge_reads["r"][0].0, "j");
    assert_eq!(a.merge_reads["s"][0].0, "j");
    assert_eq!(a.keyed_reads["j"][0].2, physical::syntax::SupplyKind::Const);
}

const HASH_CHAIN: &str = "physical {
    relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
    l = let canon rename L {a -> a};
    r = let canon rename R {a -> a, b -> b};
    s = let canon rename S {b -> b, c -> c};
    j0 = let canon probe first [a] l, r;
    j1 = let canon probe first [b] j0, s;
    x = let canon semi eager [a] j1, l;
    p = let canon project x keep {a, c}; return p as answer(a, c);
}";

#[test]
fn intermediate_lets_admit_canon_and_semijoins_record_key_reads() {
    let source: physical::Program = syn::parse_str(HASH_CHAIN).unwrap();
    let analysis = physical::contract::check(&source).unwrap();
    assert_eq!(
        analysis.canon,
        ["l", "r", "s", "j0", "j1", "x", "p"]
            .map(String::from)
            .into()
    );
    assert_eq!(analysis.key_reads["l"], [("x".into(), vec!["a".into()])]);
    assert_eq!(physical::ordinals(&analysis, "r", &["a".into()]), [0]);
}

#[test]
fn a_merge_result_read_by_key_stays_materialized() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        l = let rename L {a -> a};
        r = let rename R {a -> a, b -> b};
        s = let rename S {b -> b, c -> c};
        j0 = let merge[a] l, r;
        j1 = let merge[b] j0, s;
        x = pipe semi eager [a] l, j1;
        p = let canon project x keep {a}; return p as answer(a);
    }};
    let analysis = physical::contract::check(&source).unwrap();
    assert_eq!(analysis.modes["j1"], physical::ModeKind::Let);
    assert_eq!(analysis.key_reads["j1"], [("x".into(), vec!["a".into()])]);
    let mut invalid = source;
    invalid.program.definitions[4] = syn::parse_quote!(j1 = pipe merge[b] j0, s;);
    assert!(physical::contract::check(&invalid).is_err());
}

#[test]
fn point_reads_require_literal_keys_and_canon_host_pipes_require_linear_reads() {
    let mut p: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32); r = pipe rename R {a -> x,b -> y};
        w = let canon point select r if (x = 2); return w as answer(x,y);
    });
    physical::contract::check(&p).unwrap();
    p.program.definitions[1] = syn::parse_quote!(w = let canon point select r if (x = y););
    assert!(physical::contract::check(&p).is_err());
    let mut p: physical::Program = syn::parse_str(HASH_CHAIN).unwrap();
    p.program.definitions[0].mode = syn::parse_quote!(pipe);
    assert!(physical::contract::check(&p).is_err());
    p.program.definitions[0].canon = None;
    physical::contract::check(&p).unwrap();
}

#[test]
fn rust_branches_are_independent_and_types_are_host_obligations() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32); r = pipe rename R {a -> x};
        g = pipe guard r if (*x != 0);
        b = let bind r with y:i32 = (10 / *x);
        j = pipe probe first [x] g,b; return j as answer(x,y);
    });
    physical::contract::check(&p).unwrap();
    let p: physical::Program = syn::parse_quote!(physical {
        relation R(price:f64); r = pipe rename R {price -> price};
        p = let canon point select r if (price = 0.0); return p as answer(price);
    });
    physical::contract::check(&p).unwrap();
    assert!(
        syn::parse_str::<physical::Program>(
            "physical { e = pipe extend u with {x:i32} value (1); return e as answer(x); }"
        )
        .is_err()
    );
}

#[test]
fn rewrite_accepts_physical_choices_and_rejects_changed_logical_operands() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); relation S(a:i32);
        r = let rename R {a -> x}; s = let rename S {a -> x};
        j = pipe probe first [x] r,s; return j as answer(x);
    }};
    let mut merged = source.clone();
    merged.program.definitions[2].operator = syn::parse_quote!(merge[x] r,s);
    physical::check_rewrite(&source, &merged).unwrap();
    physical::check_rewrite(&merged, &source).unwrap();
    for operator in [
        syn::parse_quote!(merge[x] s,r),
        syn::parse_quote!(semi eager [x] r,s),
    ] {
        let mut target = source.clone();
        target.program.definitions[2].operator = operator;
        physical::contract::check(&target).unwrap();
        assert!(physical::check_rewrite(&source, &target).is_err());
    }
}

#[test]
fn rewrite_preserves_selection_and_rust_payload_without_an_ra_dependency() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); r = pipe rename R {a -> x};
        s = pipe select r if (x = 2);
        g = pipe guard s if (accept(x)); return g as answer(x);
    }};
    let mut point = source.clone();
    point.program.definitions[1].operator = syn::parse_quote!(point select r if (x = 2));
    physical::check_rewrite(&source, &point).unwrap();
    let mut changed_predicate = point.clone();
    changed_predicate.program.definitions[1].operator =
        syn::parse_quote!(point select r if (x = 3));
    let mut changed_payload = point;
    changed_payload.program.definitions[2].operator = syn::parse_quote!(guard s if (other(x)));
    for target in [changed_predicate, changed_payload] {
        physical::contract::check(&target).unwrap();
        assert!(physical::check_rewrite(&source, &target).is_err());
    }
}

#[test]
fn generator_bind_round_trip_checks_and_key_transfer() {
    let source: physical::Program = syn::parse_quote! { physical {
        u = pipe unit;
        g = pipe bind u with for x = ([1, 1, 2]);
        return canon g as answer(x);
    } };
    let facts = physical::contract::check(&source).unwrap();
    assert!(facts.rows["g"].computed.contains("x"));
    assert_eq!(
        syn::parse2::<physical::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let op = &source.program.definitions[1].operator;
    assert!(physical::inherited_key(op, &facts.headings["g"], false, |_| Some(&[])).is_none());
    for replacement in [
        "bind u with for x = ([*x])",
        "bind absent with for x = ([1])",
    ] {
        let mut invalid = source.clone();
        invalid.program.definitions[1].operator = syn::parse_str(replacement).unwrap();
        assert!(physical::contract::check(&invalid).is_err());
    }
    let mut scalar = source;
    let physical::Operator::Bind(bind) = &mut scalar.program.definitions[1].operator else {
        panic!()
    };
    bind.for_token = None;
    assert_eq!(
        physical::inherited_key(
            &scalar.program.definitions[1].operator,
            &["x".into()],
            false,
            |_| Some(&[])
        ),
        Some(Vec::new())
    );
}

#[test]
fn consumer_context_round_trips_and_remains_distinct() {
    let program: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); relation S(a:i32);
        consumer canonical;
        r = pipe rename R {a -> x};
        s = pipe rename S {a -> x};
        u = pipe union r,s;
        return u as answer(x);
    }};
    let facts = physical::contract::check(&program).unwrap();
    assert!(facts.consumer_canonical);
    assert_eq!(
        syn::parse2::<physical::Program>(program.to_token_stream()).unwrap(),
        program
    );
    let mut missing = program.clone();
    missing.program.consumer_canonical = None;
    assert_ne!(missing, program);
    assert!(physical::contract::check(&missing).is_err());
    assert!(physical::check_rewrite(&program, &missing).is_err());
    let mut redundant = program;
    redundant.program.return_canon = Some(Default::default());
    physical::contract::check(&redundant).unwrap();
}

#[test]
fn collapsing_computed_heads_record_their_dedup_obligation() {
    let program: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = pipe rename R {a -> x,b -> y};
        return dedup r as answer(*x % 2);
    }};
    physical::contract::check(&program).unwrap();
    assert!(program.program.head_dedup());
    assert_eq!(
        syn::parse2::<physical::Program>(program.to_token_stream()).unwrap(),
        program
    );
    // Without the obligation, distinct (x, y) valuations could collapse.
    let mut missing = program.clone();
    missing.program.return_dedup = None;
    assert!(physical::contract::check(&missing).is_err());
    // A canonicalized return already records the answer dedup.
    let mut canon = missing.clone();
    canon.program.return_canon = Some(Default::default());
    physical::contract::check(&canon).unwrap();
    assert!(canon.program.head_dedup());
    // A consumer canonical boundary owns answer identity.
    let mut consumer = missing;
    consumer.program.consumer_canonical = Some(syn::parse_quote!(consumer canonical;));
    physical::contract::check(&consumer).unwrap();
    // An injective head admits a harmless extra dedup.
    let extra: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        r = pipe rename R {a -> x};
        return dedup r as answer(x);
    }};
    physical::contract::check(&extra).unwrap();
}

#[test]
fn context_fact_grammar_rejects_duplicates_and_malformed_packets() {
    assert!(syn::parse_str::<physical::Program>(
        "physical { consumer canonical; consumer canonical; u = pipe unit; return u as answer(); }"
    ).is_err());
    for arguments in [
        "consumer = canonical, consumer = canonical",
        "host_set R discharged",
        "host_set R",
        "consumer canonical",
        "consumer = unknown",
    ] {
        assert!(
            syn::parse_str::<physical::FactPacket>(arguments).is_err(),
            "{arguments}"
        );
    }
    let packet: physical::FactPacket =
        syn::parse_str("consumer = canonical,").unwrap();
    assert!(packet.context().consumer_canonical);
}

#[test]
fn uniqueness_derives_operator_keys_and_local_return_obligations() {
    let cases: &[(&str, &str, &[(&str, &[&str])])] = &[
        (
            "unit and scalar filters preserve the empty key",
            "physical {
                u = pipe unit; b = pipe bind u with x:i32 = (1);
                s = pipe select b if (x = 1);
                g = pipe guard s if (*x > 0);
                p = pipe project g keep {x};
                return canon p as answer(x);
            }",
            &[("u", &[]), ("b", &[]), ("s", &[]), ("g", &[]), ("p", &[])],
        ),
        (
            "input relations are unconditionally sets",
            "physical {
                relation R(a:i32); r = pipe rename R {a -> x};
                return canon r as answer(x);
            }",
            &[("r", &["x"])],
        ),
        (
            "rename sorts the full input heading",
            "physical {
                relation R(a:i32,b:i32);
                r = pipe rename R {a -> y,b -> x};
                return canon r as answer(x,y);
            }",
            &[("r", &["x", "y"])],
        ),
        (
            "constants remove fixed columns from the input key",
            "physical {
                relation R(a:i32,b:i32);
                r = let rename R {a -> x} constants {b = 7};
                return canon r as answer(x);
            }",
            &[("r", &["x"])],
        ),
        (
            "all fixed host columns leave the empty key",
            "physical {
                relation R(a:i32);
                r = let rename R {} constants {a = 7};
                return canon r as answer();
            }",
            &[("r", &[])],
        ),
        (
            "natural join combines and deduplicates both operand keys",
            "physical {
                relation R(a:i32,b:i32); relation S(b:i32,c:i32);
                r = pipe rename R {a -> x,b -> y};
                s = let rename S {b -> y,c -> z};
                j = pipe probe first [y] r,s;
                return canon j as answer(x,y,z);
            }",
            &[("r", &["x", "y"]), ("s", &["y", "z"]), ("j", &["x", "y", "z"])],
        ),
        (
            "merge shares the natural join key rule",
            "physical {
                relation R(a:i32); relation S(a:i32,b:i32);
                r = let rename R {a -> x}; s = let rename S {a -> x,b -> y};
                j = pipe merge[x] r,s;
                return canon j as answer(x,y);
            }",
            &[("r", &["x"]), ("s", &["x", "y"]), ("j", &["x", "y"])],
        ),
        (
            "join requires both keys",
            "physical {
                relation R(a:i32);
                r = pipe rename R {a -> x};
                u = pipe unit; s = let bind u with for x:i32 = ([1,1]);
                j = pipe probe first [x] r,s;
                return canon j as answer(x);
            }",
            &[("r", &["x"]), ("u", &[])],
        ),
        (
            "anti and semi preserve only the left key",
            "physical {
                relation R(a:i32,b:i32);
                r = pipe rename R {a -> x,b -> y};
                u = pipe unit; s = let bind u with for x:i32 = ([1,1]);
                a = pipe anti eager [x] r,s;
                m = pipe semi eager [x] a,s;
                return canon m as answer(x,y);
            }",
            &[("r", &["x", "y"]), ("u", &[]), ("a", &["x", "y"]), ("m", &["x", "y"])],
        ),
        (
            "a unique right side cannot canonicalize membership output",
            "physical {
                relation S(a:i32);
                u = pipe unit; r = pipe bind u with for x:i32 = ([1,1]);
                s = let rename S {a -> x};
                a = pipe anti eager [x] r,s;
                m = pipe semi eager [x] a,s;
                return canon m as answer(x);
            }",
            &[("u", &[]), ("s", &["x"])],
        ),
        (
            "fixed point key remains empty through range guard bind and projection",
            "physical {
                relation R(a:i32);
                r = pipe rename R {a -> x};
                p = pipe point select r if (x = 2);
                w = pipe range select p if (x >= 1);
                g = pipe guard w if (*x != 0);
                b = pipe bind g with y:i32 = (*x + 1);
                k = pipe project b keep {x,y};
                return canon k as answer(x,y);
            }",
            &[("r", &["x"]), ("p", &[]), ("w", &[]), ("g", &[]),
                ("b", &[]), ("k", &[])],
        ),
        (
            "dropping a key column loses uniqueness",
            "physical {
                relation R(a:i32,b:i32);
                r = pipe rename R {a -> x,b -> y};
                p = pipe project r keep {x};
                return canon p as answer(x);
            }",
            &[("r", &["x", "y"])],
        ),
        (
            "generator bind loses even the empty key",
            "physical {
                u = pipe unit; b = pipe bind u with for x:i32 = ([1,1]);
                p = pipe project b keep {x};
                return canon p as answer(x);
            }",
            &[("u", &[])],
        ),
        (
            "group keys exclude fixed captures",
            "physical {
                relation R(a:i32,b:i32);
                capture { let fixed:i32 = (1); }
                r = pipe rename R {a -> x,b -> y};
                g = pipe group r by [fixed,x] { n:usize = count(); };
                return canon g as answer(x,n);
            }",
            &[("r", &["x", "y"]), ("g", &["x"])],
        ),
        (
            "capture-only grouping has the empty key",
            "physical {
                relation R(a:i32);
                capture { let fixed:i32 = (1); }
                r = pipe rename R {a -> x};
                g = pipe group r by [fixed] { n:usize = count(); };
                return canon g as answer(n);
            }",
            &[("r", &["x"]), ("g", &[])],
        ),
        (
            "global grouping has the empty key",
            "physical {
                u = pipe unit; g = pipe group u by [] { n:usize = count(); };
                return canon g as answer(n);
            }",
            &[("u", &[]), ("g", &[])],
        ),
        (
            "disjoint union requires unique arms and supplies the full heading",
            "physical {
                relation R(a:i32,b:i32); relation S(a:i32,b:i32);
                r = pipe rename R {a -> x,b -> y};
                s = pipe rename S {a -> x,b -> y};
                a = pipe anti eager [x] s,r;
                u = pipe union r,a block free[x] interface[y] fallback;
                return canon u as answer(x,y);
            }",
            &[("r", &["x", "y"]), ("s", &["x", "y"]),
                ("a", &["x", "y"]), ("u", &["x", "y"])],
        ),
        (
            "disjointness cannot repair a duplicate arm",
            "physical {
                relation R(a:i32,b:i32);
                r = pipe rename R {a -> x,b -> y};
                seed = pipe unit; b = pipe bind seed with for x:i32 = ([1,1]);
                s = pipe bind b with y:i32 = (1);
                a = pipe anti eager [x] s,r;
                u = pipe union r,a block free[x] interface[y] fallback;
                return canon u as answer(x,y);
            }",
            &[("r", &["x", "y"]), ("seed", &[])],
        ),
        (
            "ordinary union can overlap even with unique arms",
            "physical {
                relation R(a:i32); relation S(a:i32);
                r = pipe rename R {a -> x}; s = pipe rename S {a -> x};
                u = pipe union r,s; return canon u as answer(x);
            }",
            &[("r", &["x"]), ("s", &["x"])],
        ),
        (
            "recorded canon supplies a missing full-heading key",
            "physical {
                relation R(a:i32,b:i32);
                r = pipe rename R {a -> x,b -> y};
                p = let canon project r keep {x};
                return canon p as answer(x);
            }",
            &[("r", &["x", "y"]), ("p", &["x"])],
        ),
        (
            "redundant canon preserves a stronger inherited key",
            "physical {
                u = pipe unit; b = let canon bind u with x:i32 = (1);
                return canon b as answer(x);
            }",
            &[("u", &[]), ("b", &[])],
        ),
    ];
    for &(name, source, expected) in cases {
        let mut program: physical::Program = syn::parse_str(source).unwrap();
        let facts = physical::contract::check(&program)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let keys = physical::uniqueness(&program, &facts.headings, &facts.disjoint, |d, _| d.canon.is_some());
        let expected: std::collections::BTreeMap<String, Vec<String>> = expected
            .iter()
            .map(|(name, key)| (
                (*name).into(),
                key.iter().map(|column| (*column).into()).collect(),
            ))
            .collect();
        assert_eq!(keys, expected, "{name}");
        program.program.return_canon = None;
        let local = physical::contract::check(&program);
        if keys.contains_key(&physical::symbol_name(&program.program.returned)) {
            assert!(local.is_ok(), "{name}: {local:?}");
        } else {
            assert_eq!(
                local.unwrap_err().to_string(),
                "return requires canonical complete input valuations",
                "{name}",
            );
        }
        program.program.consumer_canonical = Some(syn::parse_quote!(consumer canonical;));
        physical::contract::check(&program).unwrap();
        assert_eq!(
            physical::uniqueness(&program, &facts.headings, &facts.disjoint, |d, _| d.canon.is_some()),
            keys,
            "{name}: consumer canonicality is not an operator key",
        );
    }
}

#[test]
fn final_boundaries_and_answer_dedup_do_not_canonicalize_group_inputs() {
    for boundary in ["", "canon ", "dedup "] {
        for consumer in ["", "consumer canonical;"] {
            let source = format!(
                "physical {{
                    {consumer}
                    seed = pipe unit; r = pipe bind seed with for x:i32 = ([1,1]);
                    g = pipe group r by [] {{ n:usize = count(); }};
                    return {boundary}g as answer(n);
                }}"
            );
            let mut program: physical::Program = syn::parse_str(&source).unwrap();
            assert_eq!(
                physical::contract::check(&program).unwrap_err().to_string(),
                "group requires canonical complete input valuations",
                "{source}",
            );
            program.program.definitions[1].mode = syn::parse_quote!(let);
            program.program.definitions[1].canon = Some(Default::default());
            physical::contract::check(&program).unwrap();
        }
    }
    let dedup: physical::Program = syn::parse_quote! { physical {
        seed = pipe unit; r = pipe bind seed with for x:i32 = ([1,1]);
        return dedup r as answer(x);
    }};
    assert_eq!(
        physical::contract::check(&dedup).unwrap_err().to_string(),
        "return requires canonical complete input valuations",
    );
}

#[test]
fn uniqueness_policy_observes_inherited_keys_and_supplies_effective_boundaries() {
    let program: physical::Program = syn::parse_quote! { physical {
        u = pipe unit;
        g = pipe bind u with for x:i32 = ([1,1]);
        b = pipe bind g with y:i32 = (*x + 1);
        p = pipe project b keep {x};
        return canon p as answer(x);
    }};
    let facts = physical::contract::check(&program).unwrap();
    let mut inherited = Vec::new();
    let keys = physical::uniqueness(&program, &facts.headings, &facts.disjoint, |definition, key| {
        let name = physical::symbol_name(&definition.name);
        inherited.push((name.clone(), key.map(<[String]>::to_vec)));
        name == "g"
    });
    assert_eq!(
        inherited,
        [
            ("u".into(), Some(vec![])),
            ("g".into(), None),
            ("b".into(), Some(vec!["x".into()])),
            ("p".into(), Some(vec!["x".into()])),
        ],
    );
    assert_eq!(keys["u"], Vec::<String>::new());
    assert_eq!(keys["g"], ["x"]);
    assert_eq!(keys["b"], ["x"]);
    assert_eq!(keys["p"], ["x"]);
}

#[test]
fn union_disjointness_requires_actual_column_preserving_exclusion() {
    for (body, disjoint) in [
        ("u = pipe union r,s block free[x] interface[y] fallback;", false),
        ("a = pipe anti eager [x] s,r; u = pipe union r,a;", true),
        ("a = pipe anti eager [x] s,r; u = pipe union r,a block free[x] interface[y] fallback;", true),
        ("t = pipe rename T {x -> x,y -> y}; a = pipe anti eager [x] s,t; u = pipe union r,a block free[x] interface[y] fallback;", false),
        ("a = pipe anti eager [x] s,r; p = pipe project a keep {y}; b = pipe bind p with x:i32 = (0); u = pipe union r,b block free[x] interface[y] fallback;", false),
        ("t = pipe rename T {x -> x,y -> y}; a1 = pipe anti eager [x] s,r; t1 = pipe anti eager [x] t,r; a2 = pipe anti eager [x] t1,s; u = pipe union r,a1,a2 block free[x] interface[y] fallback;", true),
        ("t = pipe rename T {x -> x,y -> y}; a1 = pipe anti eager [x] s,r; a2 = pipe anti eager [x] t,r; u = pipe union r,a1,a2 block free[x] interface[y] fallback;", false),
    ] {
        let mut program: physical::Program = syn::parse_str(&format!(
            "physical {{ relation R(x:i32,y:i32); relation S(x:i32,y:i32);
             relation T(x:i32,y:i32);
             r = pipe rename R {{x -> x,y -> y}};
             s = pipe rename S {{x -> x,y -> y}};
             {body} return canon u as answer(x,y); }}"
        )).unwrap();
        let facts = physical::contract::check(&program).unwrap();
        assert_eq!(facts.disjoint.contains("u"), disjoint, "{body}");
        program.program.return_canon = None;
        assert_eq!(physical::contract::check(&program).is_ok(), disjoint, "{body}");
        program.program.definitions.push(syn::parse_quote!(
            grouped = pipe group u by [] { n:usize = count(); };
        ));
        program.program.returned = syn::parse_quote!(grouped);
        program.program.head = syn::parse_quote!(answer(n));
        assert_eq!(physical::contract::check(&program).is_ok(), disjoint, "{body}");
    }
}

#[test]
fn explicit_closed_equalities_preserve_keys_across_constant_column_projection() {
    for (condition, fixed) in [("x = 7", true), ("7 = x", true), ("x = @(limit)", true), ("x > 7", false)] {
        let mut program: physical::Program = syn::parse_str(&format!(
            "physical {{ relation R(x:i32,y:i32);
             r = pipe rename R {{x -> x,y -> y}};
             s = pipe select r if ({condition});
             p = pipe project s keep {{y}};
             return canon p as answer(y); }}"
        )).unwrap();
        let facts = physical::contract::check(&program).unwrap();
        let keys = physical::uniqueness(&program, &facts.headings, &facts.disjoint, |d, _| d.canon.is_some());
        assert_eq!(keys.contains_key("p"), fixed, "{condition}");
        if fixed {
            assert_eq!(keys["s"], ["y"]);
            assert_eq!(keys["p"], ["y"]);
        }
        program.program.return_canon = None;
        assert_eq!(physical::contract::check(&program).is_ok(), fixed, "{condition}");
    }
}
