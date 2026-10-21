// The unselected branch must be discarded before resolving its dependencies.
oracle_shared::oracle_engine!(ascent, mini, {
    missing_ascent_dependency::expand!();
});
oracle_shared::oracle_engine!(mini, ascent, {
    missing_mini_dependency::expand!();
});
oracle_shared::oracle_engine!(mini, mini, {
    const MINI_SELECTED: bool = true;
});
oracle_shared::oracle_engine!(ascent, ascent, {
    const ASCENT_SELECTED: bool = true;
});

#[test]
fn selected_branches_expand_without_other_engine_dependencies() {
    const { assert!(MINI_SELECTED && ASCENT_SELECTED) };
}

#[test]
fn golden_root_preserves_the_existing_corpus() {
    assert!(
        oracle_shared::golden_root()
            .join("tc_left/chain.csv")
            .is_file()
    );
}
