//! Terminal-parameterised positional execution fixtures. An optional bracketed
//! queue lowers Structures before the terminal; omit it for Structures terminals.
//! Invokers provide mini-linq, cq-storage and cq-load as test dependencies.

/// Instantiate common positional queries and their authored expected answers.
#[macro_export]
macro_rules! positional_cases {
    ($($printer:ident)::+) => {
        $crate::positional_cases!(@pipeline [$($printer)::+;] $($printer)::+);
    };
    ([$($first:ident)::+ $(, $pass:path)*] $($printer:ident)::+) => {
        $crate::positional_cases!(@pipeline [$($first)::+; $($pass,)* $($printer)::+,] $($printer)::+);
    };
    (@pipeline [$($first:ident)::+; $($pass:path,)*] $($printer:ident)::+) => {
struct PayloadOnly<T>(T);
impl<T: cq_storage::IndexRead> Clone for PayloadOnly<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<I: Iterator, T: cq_storage::Build<I>> cq_storage::Build<I> for PayloadOnly<T> {
    fn build(source: I) -> Self {
        Self(T::build(source))
    }
}
impl<T: cq_storage::IndexRead> cq_storage::IndexRead for PayloadOnly<T> {
    type Key = T::Key;
    type Item = T::Item;
    type Cursor = T::Cursor;
    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.0.lookup(key)
    }
}

#[derive(Clone, Copy)]
struct UnusedRight;
impl cq_storage::rows::Rows for UnusedRight {
    type Row = (&'static i64,);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        std::iter::from_fn(|| panic!("an empty left must not force its RHS"))
    }
}

#[test]
fn conflicting_points_keep_all_residuals_before_the_next_read() {
    let rows = [(1_i64, 10_i64), (2, 10), (3, 20)];
    let result: Vec<_> = mini_linq::compile! {
        pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(R)); }
        struct Conflicting;
        relation R(a:i64,b:i64) = (rows);
        answer(a) :- R(a,b), if *a == 1 && *a == 2 && *b == 10;
    }
    .collect();
    assert!(result.is_empty());
}
#[test]
fn nonleading_filtered_index_keeps_payload_and_constant_reconstruction() {
    let left = [(1_i64,), (2,), (3,)];
    let right = [
        (1_i64, "yes".to_owned(), 10_i64),
        (2, "no".to_owned(), 20),
        (3, "yes".to_owned(), 30),
    ];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(L,R)); }
        struct Filtered;
        relation L(a:i64) = (left);
        relation R(a:i64,label:String,value:i64) = (right);
        answer(a,label,value) :- L(a), R(a,label,value), if label == "yes";
    }
    .map(|(a, l, v)| (*a, l.clone(), *v))
    .collect();
    assert_eq!(
        result,
        BTreeSet::from([(1, "yes".to_owned(), 10), (3, "yes".to_owned(), 30)])
    );
}

#[test]
fn empty_positional_left_does_not_force_lazy_semi_rhs() {
    let left = [(1_i64,), (2,)];
    let right = UnusedRight;
    let result: Vec<_> = mini_linq::passes::select! {
        arguments { positional(L) };
        pipeline { $($first)::+, $($pass,)* };
        physical {
            relation L(a:i64) = (left); relation R(b:i64) = (right);
            l = pipe rename L@0 {a -> x}; empty = pipe range select l if (x > 100);
            r = pipe rename R@1 {b -> x};
            s = pipe semi lazy [x] empty,r;
            out = pipe canon project s keep {x}; return out as answer(x);
        }
    }
    .collect();
    assert!(result.is_empty());
}

#[test]
fn custom_aliases_and_bare_build_provider_keep_selected_source_contracts() {
    type Mask<H> = cq_storage::Selection<H>;
    type Keys<K> = cq_storage::AdaptiveKeySet<K, cq_storage::I64Codec, 64>;
    type Payload<'a, K, V> = PayloadOnly<cq_storage::HashIndex<'a, K, V>>;
    let left = [(1_i64, 10_i64), (1, 20), (2, 30)];
    let right = [(1_i64, 1_i64), (2, 0)];
    let actual: BTreeSet<_> = $($first)::+! {
        pipeline { $($pass,)* };
        physical {
            relation L(a:i64,b:i64) = (left); relation R(c:i64,d:i64) = (right);
            l = pipe rename L@0 {a -> x,b -> y}; r = pipe rename R@1 {c -> x,d -> z};
            s = let point select r if (z = 1); t = let semi eager [x] l,s;
            u = pipe semi eager [x] s,t; j = pipe probe first [x] u,t;
            out = pipe canon project j keep {x,y}; return out as answer(x,y);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            s selection = Mask<_> base R lineage{x -> c,z -> d} summary none;
            s keyset = Keys<_> [x] owned base R lineage{x -> c,z -> d};
            t selection = Mask<_> base L lineage{x -> a,y -> b} summary per_block;
            t = Payload [x];
            t keyset = Keys<_> [x] owned base L lineage{x -> a,y -> b};
            s read = positional base R lineage{x -> c,z -> d} column d marker ::cq_load::ColumnAt<1> predicate physical;
            t read = positional base L lineage{x -> a,y -> b} column a marker ::cq_load::ColumnAt<0> predicate membership s[x] summary exact;
            u read = positional base R lineage{x -> c,z -> d} column c marker ::cq_load::ColumnAt<0> predicate membership t[x] summary exact;
            out canon = ::std::collections::HashSet<_>;
        }
    }.map(|(x,y)| (*x,*y)).collect();
    assert_eq!(actual, BTreeSet::from([(1, 10), (1, 20)]));
}

#[test]
fn positional_bindings_do_not_shadow_captured_parameters() {
    let __cq_positional_r = 2_i64;
    let rows = [(1_i64,), (2,), (3,)];
    let values: Vec<_> = mini_linq::compile! {
        pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(r)); }
        struct Hygiene;
        relation r(x:i64) = (rows);
        answer(x) :- r(x), if *x < __cq_positional_r;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(values, vec![1]);
}

#[test]
fn a_computed_left_key_can_test_an_owned_set_without_becoming_a_selection() {
    let left = [(1_i64,), (7,)];
    let right = [(2_i64, 1_i64), (8, 0)];
    let values: Vec<_> = mini_linq::passes::select! {
        arguments { positional(L,R) };
        pipeline { $($first)::+, $($pass,)* };
        physical {
            relation L(a:i64) = (left); relation R(b:i64,c:i64) = (right);
            l = pipe rename L@0 {a -> a};
            computed = pipe bind l with x:i64 = (*a + 1);
            r = pipe rename R@1 {b -> x,c -> z};
            s = let point select r if (z = 1);
            t = let semi eager [x] computed,s;
            back = pipe semi eager [x] s,t;
            out = pipe canon project back keep {x}; return out as answer(x);
        }
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(values, vec![2]);
}

#[test]
fn raw_identifiers_survive_recorded_base_and_column_lineage() {
    let rows = [(1_i64,), (2,)];
    let values: Vec<_> = mini_linq::compile! {
        pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(r#type)); }
        struct Raw;
        relation r#type(r#match:i64) = (rows);
        answer(r#loop) :- r#type(r#loop), if *r#loop > 1;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(values, vec![2]);
}

    };
}

/// The retained positional-mask query used with several host layouts.
#[macro_export]
macro_rules! positional_reduced {
    ($($printer:ident)::+, $left:expr, $right:expr) => {
        $crate::positional_reduced!(@typed [] $($printer)::+, i64, $left, $right)
    };
    ([$($pass:path),*] $($printer:ident)::+, $left:expr, $right:expr) => {
        $crate::positional_reduced!(@typed [$($pass),*] $($printer)::+, i64, $left, $right)
    };
    (@typed $($printer:ident)::+, $ty:ty, $left:expr, $right:expr) => {
        $crate::positional_reduced!(@typed [] $($printer)::+, $ty, $left, $right)
    };
    (@typed [$($pass:path),*] $($printer:ident)::+, $ty:ty, $left:expr, $right:expr) => {
        mini_linq::passes::select! {
            arguments { positional(L, R) };
            pipeline { $($pass,)* $($printer)::+ };
            physical {
                relation L(a:$ty, b:$ty) = ($left);
                relation R(c:$ty, flag:$ty) = ($right);
                l = pipe rename L@0 { a -> x, b -> y };
                window = pipe range select l if (y >= 10, y < 40);
                r = pipe rename R@1 { c -> x, flag -> f };
                selected = let point select r if (f = 1);
                projected = pipe project window keep {x,y};
                reduced = let semi eager [x] projected, selected;
                back = pipe semi eager [x] selected, reduced;
                joined = pipe probe first [x] back, reduced;
                result = pipe canon project joined keep {x,y};
                return result as answer(x,y);
            }
        }
        .map(|(x, y)| (*x, *y))
        .collect::<::std::collections::BTreeSet<_>>()
    };
}
