//! Optional HW4 specification: requires the negation/aggregation implementation.
//! Run after completing HW4: cargo run -p mini-linq --example negation-aggregation --features hw4
use mini_linq::mini_linq;

mini_linq! {
    struct Allowed;
    relation Person(key: i32);
    relation Blocked(key: i32);
    allowed(person) :- Person(person), !Blocked(person);
}

mini_linq! {
    struct Spend;
    relation Person(key: i32);
    relation Blocked(person: i32, reason: i32);
    relation Purchase(person: i32, category: i32, amount: i32);
    spend(person, total) :- Person(person), !Blocked(person, _),
        agg total = sum(amount) in Purchase(person, _, amount);
}

mini_linq! {
    struct GlobalSum;
    relation Blocker(key: i32);
    relation Purchase(category: i32, amount: i32);
    summary(total) :- !Blocker(_),
        agg total = sum(amount) in Purchase(_, amount);
}

fn main() {
    // Full-key anti-semijoin: blocked rows remove people, never introduce them.
    let allowed = Allowed::run([(1,), (2,), (3,)], [(2,), (2,)]);
    assert_eq!(allowed, vec![(1,), (3,)]);
    println!("safe negation: {allowed:?}");

    // Person -> correlated anti-semijoin -> correlated aggregate -> projection.
    // Duplicate input tuples disappear, but distinct categories with equal
    // amounts both contribute. The empty group for person 3 produces zero.
    let spend = Spend::run(
        [(1,), (2,), (3,), (3,)],
        [(2, 7)],
        [(1, 10, 5), (1, 20, 5), (1, 20, 5), (2, 30, 99)],
    );
    assert_eq!(spend, vec![(1, 10), (3, 0)]);
    println!("correlated sum after negation: {spend:?}");

    // Unit -> global anti-semijoin -> uncorrelated aggregate. There is no
    // positive atom and no correlation key in this optional extension.
    let sum = GlobalSum::run([], [(1, 5), (2, 5)]);
    assert_eq!(sum, vec![(10,)]);
    assert_eq!(GlobalSum::run([], []), vec![(0,)]);
    assert!(GlobalSum::run([(9,)], [(1, 5)]).is_empty());
    println!("global sum: {sum:?}; empty sum: [(0,)]; blocked global result: []");
}
