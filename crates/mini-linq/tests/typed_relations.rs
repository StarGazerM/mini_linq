#![cfg(feature = "compiled-workloads")]

use std::collections::HashSet;

use mini_linq::mini_linq;

type City = &'static str;
type Road = (City, City);

mini_linq! {
    struct TwoHopRoads;
    relation road(c0: City, c1: City);
    two_hop(from, to) :- road(from, via), road(via, to).
}

type PersonId = u64;

mini_linq! {
    struct PurchaseNames;
    relation Person(c0: PersonId, c1: String);
    relation Purchase(c0: u64, c1: u64);
    purchase_name(name, amount) :- Person(person, name), Purchase(person, amount).
}

#[test]
fn the_course_road_instance_runs_without_an_integer_encoding() {
    let roads: HashSet<Road> =
        HashSet::from([("Logan", "Salt Lake City"), ("Salt Lake City", "Provo")]);

    assert_eq!(TwoHopRoads::run(roads.clone()), vec![("Logan", "Provo")],);
    assert_eq!(roads.len(), 2, "query evaluation must not update its input");
}

#[test]
fn generated_indexes_and_outputs_support_non_copy_columns() {
    let names = [
        (2, String::from("Grace")),
        (1, String::from("Ada")),
        (1, String::from("Ada")),
    ];
    let purchases = [(1, 20), (1, 10), (2, 30)];

    assert_eq!(
        PurchaseNames::run(names, purchases),
        vec![
            (String::from("Ada"), 10),
            (String::from("Ada"), 20),
            (String::from("Grace"), 30),
        ],
    );
}
