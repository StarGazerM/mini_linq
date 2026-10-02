# Reviewed core public API

Run `python3 tools/check_public_api.py` to reject unreviewed API changes.
`python3 tools/check_public_api.py --update` writes snapshots for an intentional
review; it is never called by CI. Snapshots use rustdoc JSON from the pinned
Rust toolchain, with all runtime features enabled. Facades have no Cargo features.
The cq-testkit snapshot includes its optional DuckDB and golden helpers.

Each sorted line pairs an exported path with its normalized JSON signature.
Fields, generic bounds, associated items, local trait implementations and macro
reexports are included; numeric rustdoc IDs become stable paths. Spans, docs,
implementation bodies and blanket implementations are excluded. The API list
comes from rustdoc, including derive-generated APIs, rather than a source grep.

The wave-1 signatures, decisions and pipeline protocol are documented in
`facade/cq-stage/src/lib.rs` and `doc/DESIGN.md`; the wave-2 handoff is also in
`/tmp/codex/core_api.md` for this working session.
