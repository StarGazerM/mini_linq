//! Write each compiler boundary for the planned example.
//! cargo run -p cq-compose --example planned_boundaries -- doc/planned-example
#[path = "../example-support/planned_boundaries.rs"]
mod planned_boundaries;

fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .expect("supply the output directory");
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, file) in planned_boundaries::artifacts() {
        std::fs::write(directory.join(name), prettyplease::unparse(&file)).unwrap();
    }
}
