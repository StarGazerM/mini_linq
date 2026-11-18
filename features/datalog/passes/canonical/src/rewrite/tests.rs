use super::*;
use incremental::strata::source;

fn storage_of(source: source::Program, delta_first: bool) -> storage::Program {
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let policy = if delta_first {
        cq_datalog_incremental::DriverPolicy::DeltaFirst
    } else {
        cq_datalog_incremental::DriverPolicy::WrittenOrder
    };
    let incremental = cq_datalog_incremental::compile_with_driver(&strata, policy).unwrap();
    cq_datalog_storage::compile(&incremental).unwrap()
}

fn canonical<'a>(program: &'a storage::Program, relation: &str) -> &'a storage::Layout {
    let index = program
        .incremental
        .strata
        .source
        .declarations
        .iter()
        .position(|declaration| declaration.name == relation)
        .unwrap();
    &program.layouts[program.providers[index].canonical]
}

fn reversed_read() -> storage::Program {
    storage_of(
        syn::parse_quote! {
            struct Reversed;
            relation E(x:i32,y:i64) = (e);
            relation R(x:i32,y:i64);
            relation S(x:i32,z:i32);
            output S;
            R(*x,*y) :- E(x,y);
            S(*x,*z) :- E(x,y), R(z,y);
        },
        false,
    )
}

#[test]
fn an_unread_canonical_layout_takes_a_read_order_and_its_respelled_type() {
    let source = reversed_read();
    assert_eq!(canonical(&source, "R").columns, [0, 1]);
    let target = storage(&source, Ordered).unwrap();
    let layout = canonical(&target, "R");
    // R is read only through `[y, x]`; the membership layout follows it.
    assert_eq!((layout.columns.as_slice(), layout.key), ([1, 0].as_slice(), 2));
    let expected: syn::Type = syn::parse_quote!(::std::collections::HashSet<
        (i64, i32,),
        ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
    >);
    assert_eq!(layout.provider.ty, expected);
    // Nothing else moves: S has no read layout to follow, E is read-only.
    assert_eq!(canonical(&target, "S"), canonical(&source, "S"));
    assert_eq!(canonical(&target, "E"), canonical(&source, "E"));
    assert_eq!(target.reads, source.reads);
    assert_eq!(target.layouts.len(), source.layouts.len());
    // Selecting again finds the canonical order already read.
    assert_eq!(storage(&target, Ordered).unwrap(), target);
}

#[test]
fn a_canonical_layout_that_is_read_or_shares_a_read_order_stays() {
    // `R(x, y)` with both columns bound reads the canonical layout itself.
    let probed = storage_of(
        syn::parse_quote! {
            struct Probed;
            relation E(x:i32,y:i32) = (e);
            relation R(x:i32,y:i32);
            relation S(x:i32,y:i32);
            output S;
            R(*x,*y) :- E(x,y);
            S(*x,*y) :- E(x,y), R(x,y), R(_,y);
        },
        false,
    );
    assert_eq!(storage(&probed, Ordered).unwrap(), probed);
    // A scan reads `[x, y]` with an empty key: the canonical order is in use.
    let scanned = storage_of(
        syn::parse_quote! {
            struct Scanned;
            relation E(x:i32,y:i32) = (e);
            relation R(x:i32,y:i32);
            relation S(x:i32,y:i32);
            output S;
            R(*x,*y) :- E(x,y);
            S(*x,*y) :- R(x,y), E(y,x), R(_,x);
        },
        false,
    );
    assert_eq!(storage(&scanned, Ordered).unwrap(), scanned);
}

#[test]
fn a_full_read_in_the_relations_own_component_outranks_an_earlier_layout() {
    let source = storage_of(
        syn::parse_quote! {
            struct Component;
            relation E(x:i32,y:i32,z:i32) = (e);
            relation P(x:i32,y:i32,z:i32);
            relation Q(x:i32);
            output Q;
            Q(*x) :- E(x,y,_), P(_,y,_);
            P(*x,*y,*z) :- E(x,y,z);
            P(*x,*y,*w) :- P(x,y,z), E(z,_,w), P(w,_,z);
        },
        true,
    );
    let orders = source
        .layouts
        .iter()
        .filter(|layout| layout.relation == "P")
        .map(|layout| (layout.columns.clone(), layout.key))
        .collect::<Vec<_>>();
    // Q's rule comes first and reads `[y, ..]`; P's own rules read the others.
    assert_eq!(orders[0], (vec![1, 0, 2], 1));
    assert_eq!(canonical(&source, "P").columns, [0, 1, 2]);
    let target = storage(&source, Ordered).unwrap();
    assert_eq!(canonical(&target, "P").columns, [0, 2, 1]);
}

#[test]
fn the_independent_check_rejects_other_changes() {
    let source = reversed_read();
    let target = storage(&source, Ordered).unwrap();
    check_storage(&source, &target).unwrap();
    let index = target.providers[1].canonical;
    // An order no layout of the relation reads.
    let mut unread = source.clone();
    let s = unread.providers[2].canonical;
    unread.layouts[s].columns = vec![1, 0];
    assert!(check_storage(&source, &unread).is_err());
    // The right order with a stale provider type.
    let mut stale = target.clone();
    stale.layouts[index].provider = source.layouts[index].provider.clone();
    assert!(check_storage(&source, &stale).is_err());
    // A layout that is not canonical.
    let mut other = target.clone();
    let read = (0..other.layouts.len()).find(|&i| i != index && other.layouts[i].relation == "R").unwrap();
    other.layouts[read].key = 0;
    assert!(check_storage(&source, &other).is_err());
}
