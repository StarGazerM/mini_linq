# Regression gates

`tools/regression-targets.tsv` is the current executable inventory. It lists
workspace tests, examples, external manifests and scenario scripts, with their
source paths. Run `bash tools/test_regressions.sh`; every target gets a log and
exit status under `target/regression-logs/`, and any failure makes the command
fail. The runner continues so all failures are visible.

The relational fixtures retain independently authored queries, data and
answers. Complete-program scenario inputs exercise each pass with only its
endpoint languages available. Set interpreters and compiled Rust tests check
semantics independently of structural correspondence.
