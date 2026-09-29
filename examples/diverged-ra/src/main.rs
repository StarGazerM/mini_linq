fn main() {
    let mut rows = diverged_ra::run_probe(&[(1, 2), (1, 9), (2, 3)], &[(1, 5), (2, 7)]);
    rows.sort();
    assert_eq!(rows, [(2, 5), (3, 7)]);
    println!("theta_join: {rows:?}");
}
