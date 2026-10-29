use std::collections::BTreeSet;

fn set<T: Ord>(rows: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    rows.into_iter().collect()
}

#[test]
fn bounded_counter_and_computed_probe_keys() {
    let seeds = [(0_i32,), (0,)];
    let links = [(1_i32, 10_i32), (2, 20), (3, 30), (4, 40)];
    let (counter, matched, input) = cq_datalog::datalog! {
        struct Counter;
        relation Seed(n:i32) = (seeds);
        relation Link(n:i32,v:i32) = (links);
        relation Count(n:i32);
        relation Matched(n:i32,v:i32);
        output Count, Matched, Link;
        Count(*n) :- Seed(n);
        Count(*n + 1) :- Count(n), where n < 4;
        Matched(*n,*v) :- Link(n,v), Count(n);
    };
    ascent::ascent! {
        struct Reference;
        relation seed(i32);
        relation link(i32,i32);
        relation count(i32);
        relation matched(i32,i32);
        count(*n) <-- seed(n);
        count(*n + 1) <-- count(n), if *n < 4;
        matched(*n,*v) <-- count(n), link(n,v);
    }
    let mut reference = Reference {
        seed: seeds.to_vec(),
        link: links.to_vec(),
        ..Default::default()
    };
    reference.run();
    let counter: Vec<_> = counter.collect();
    assert_eq!(counter.len(), 5);
    assert_eq!(set(counter), set(reference.count));
    assert_eq!(set(matched), set(reference.matched));
    assert_eq!(set(input), set(links));
}

#[test]
fn guarded_path_lengths_on_a_cycle() {
    let edges = [(0_i32, 1_i32), (1, 2), (2, 0), (0, 1)];
    let actual = cq_datalog::datalog! {
        struct Paths;
        relation Edge(x:i32,y:i32) = (edges);
        relation Path(x:i32,y:i32,length:i32);
        output Path;
        Path(*x,*y,1) :- Edge(x,y);
        Path(*x,*z,*length + 1) :- Path(x,y,length), Edge(y,z), where length < 5;
    };
    ascent::ascent! {
        struct Reference;
        relation edge(i32,i32);
        relation path(i32,i32,i32);
        path(*x,*y,1) <-- edge(x,y);
        path(*x,*z,*length + 1) <-- path(x,y,length), edge(y,z), if *length < 5;
    }
    let mut reference = Reference {
        edge: edges.to_vec(),
        ..Default::default()
    };
    reference.run();
    assert_eq!(set(actual), set(reference.path));
}

#[test]
fn fibonacci_and_let_bound_recursive_values() {
    let seed = [(0_u64, 0_u64, 1_u64)];
    let actual = cq_datalog::datalog! {
        struct Fibonacci;
        relation Seed(n:u64,a:u64,b:u64) = (seed);
        relation Fib(n:u64,a:u64,b:u64);
        output Fib;
        Fib(*n,*a,*b) :- Seed(n,a,b);
        Fib(next,*b,sum) :- Fib(n,a,b), where n < 12,
            let next = *n + 1, let sum = *a + *b;
    };
    ascent::ascent! {
        struct Reference;
        relation seed(u64,u64,u64);
        relation fib(u64,u64,u64);
        fib(*n,*a,*b) <-- seed(n,a,b);
        fib(next,*b,sum) <-- fib(n,a,b), if *n < 12,
            let next = *n + 1, let sum = *a + *b;
    }
    let mut reference = Reference {
        seed: seed.to_vec(),
        ..Default::default()
    };
    reference.run();
    let actual = set(actual);
    assert!(actual.contains(&(12, 144, 233)));
    assert_eq!(actual, set(reference.fib));
}

#[test]
fn nonlinear_computed_values_are_deduplicated() {
    let seed = [(1_i32,), (1,)];
    let actual = cq_datalog::datalog! {
        struct Sums;
        relation Seed(n:i32) = (seed);
        relation Sum(n:i32);
        output Sum;
        Sum(*n) :- Seed(n);
        Sum(*a + *b) :- Sum(a), Sum(b), if *a + *b <= 12;
    };
    ascent::ascent! {
        struct Reference;
        relation seed(i32);
        relation sum(i32);
        sum(*n) <-- seed(n);
        sum(*a + *b) <-- sum(a), sum(b), if *a + *b <= 12;
    }
    let mut reference = Reference {
        seed: seed.to_vec(),
        ..Default::default()
    };
    reference.run();
    let actual: Vec<_> = actual.collect();
    assert_eq!(actual.len(), 12);
    assert_eq!(set(actual), set(reference.sum));
}

#[test]
fn bounded_string_building_and_recursive_join_keys() {
    let seed = [(String::new(),)];
    let suffixes = [
        ("".to_owned(), "a".to_owned()),
        ("a".to_owned(), "b".to_owned()),
        ("ab".to_owned(), "c".to_owned()),
        ("abc".to_owned(), "d".to_owned()),
    ];
    let actual = cq_datalog::datalog! {
        struct Words;
        relation Seed(word:String) = (&seed);
        relation Suffix(word:String,suffix:String) = (&suffixes);
        relation Word(word:String);
        output Word;
        Word((*word).clone()) :- Seed(word);
        Word(next) :- Suffix(word,suffix), Word(word), if word.len() < 3,
            let next = format!("{}{}", word, suffix);
    };
    ascent::ascent! {
        struct Reference;
        relation seed(String);
        relation suffix(String,String);
        relation word(String);
        word(word.clone()) <-- seed(word);
        word(next) <-- word(word), suffix(word,suffix), if word.len() < 3,
            let next = format!("{}{}", word, suffix);
    }
    let mut reference = Reference {
        seed: seed.to_vec(),
        suffix: suffixes.to_vec(),
        ..Default::default()
    };
    reference.run();
    let actual = set(actual);
    assert_eq!(actual.len(), 4);
    assert_eq!(actual, set(reference.word));
}

#[test]
fn finite_generators_match_ascent_in_seed_and_recursive_rules() {
    for seeds in [vec![], vec![(0_i32,), (0,), (2,)]] {
        let links = [(1_i32, 10_i32), (2, 20), (2, 20), (3, 30), (4, 40)];
        let (generated, reach, matched) = cq_datalog::datalog! {
            struct Generated;
            relation Seed(x:i32) = (&seeds);
            relation Link(x:i32,y:i32) = (links);
            relation Generated(x:i32);
            relation Reach(x:i32);
            relation Matched(x:i32,y:i32);
            output Generated, Reach, Matched;
            Generated(y) :- Seed(x), for y in [*x, *x + 1, *x];
            Reach(*x) :- Generated(x);
            Reach(y) :- Reach(x), where x < 4, for y in [*x + 1, *x + 1];
            Matched(y,*label) :- Reach(x), for y in [*x + 1, *x + 1], Link(y,label);
        };
        ascent::ascent! {
            struct Reference;
            relation seed(i32);
            relation link(i32,i32);
            relation generated(i32);
            relation reach(i32);
            relation matched(i32,i32);
            generated(y) <-- seed(x), for y in [*x, *x + 1, *x];
            reach(*x) <-- generated(x);
            reach(y) <-- reach(x), if *x < 4, for y in [*x + 1, *x + 1];
            matched(y,*label) <-- reach(x), for y in [*x + 1, *x + 1], link(y,label);
        }
        let mut reference = Reference {
            seed: seeds,
            link: links.to_vec(),
            ..Default::default()
        };
        reference.run();
        let generated: Vec<_> = generated.collect();
        let reach: Vec<_> = reach.collect();
        let matched: Vec<_> = matched.collect();
        assert_eq!(generated.len(), set(&generated).len());
        assert_eq!(reach.len(), set(&reach).len());
        assert_eq!(matched.len(), set(&matched).len());
        assert_eq!(set(generated), set(reference.generated));
        assert_eq!(set(reach), set(reference.reach));
        assert_eq!(set(matched), set(reference.matched));
    }
}
