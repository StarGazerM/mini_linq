use super::query;

#[path = "arguments.rs"]
mod arguments;
#[path = "bindings.rs"]
mod bindings;
#[path = "guards.rs"]
mod guards;
#[path = "heads.rs"]
mod heads;
#[path = "joins.rs"]
mod joins;

fn check<T: std::fmt::Debug + PartialEq>(name: &str, actual: Vec<T>, expected: Vec<T>) {
    assert_eq!(actual, expected, "{name}");
    println!("  {name}: {actual:?}");
}

pub fn run() {
    arguments::run();
    bindings::run();
    guards::run();
    heads::run();
    joins::run();
}
