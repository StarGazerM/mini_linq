//! Check the public boundary from a separate crate, including negative cases.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Consumer(PathBuf);

impl Drop for Consumer {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn external_access_requires_traits_and_cannot_reach_storage_helpers() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let consumer = Consumer(
        std::env::temp_dir().join(format!("cq-storage-privacy-{}-{stamp}", std::process::id())),
    );
    fs::create_dir_all(consumer.0.join("src")).unwrap();
    fs::write(
        consumer.0.join("Cargo.toml"),
        format!(
            "[package]\nname = \"storage-privacy-check\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [workspace]\n[dependencies]\ncq-storage = {{ path = {:?} }}\n",
            env!("CARGO_MANIFEST_DIR"),
        ),
    )
    .unwrap();

    let check = |name: &str, source: &str, expected: Option<&[&str]>| {
        fs::write(consumer.0.join("src/main.rs"), source).unwrap();
        // This target must differ from the workspace target: the outer Cargo
        // process is still running this test and may hold its build lock.
        let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["check", "--offline", "--quiet", "--color", "never"])
            .arg("--manifest-path")
            .arg(consumer.0.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", consumer.0.join("target"))
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        match expected {
            None => assert!(output.status.success(), "{name}: {diagnostic}"),
            Some(messages) => {
                assert!(!output.status.success(), "{name} unexpectedly compiled");
                for message in messages {
                    assert!(
                        diagnostic.contains(message),
                        "{name}: missing {message:?} in {diagnostic}"
                    );
                }
            }
        }
    };

    check(
        "ingestion_and_trait_access",
        r#"
        use cq_storage::{BlockDescriptor, BlockMask, BlockRead, BlockSelection,
            BlockStore, BuildBlocks, CoordinateBounds, MakeSelection, RangeSummary,
            ZoneSummary};
        use std::collections::HashSet;
        use std::num::NonZeroUsize;
        fn main() {
            let size = NonZeroUsize::new(2).unwrap();
            let store = BlockStore::from_rows([[1], [2]], size);
            let _packed = BlockStore::<1>::pack_unique(HashSet::from([[1]]), size);
            let all = BlockMask::all(&store);
            let mut selected = BlockMask::empty(&store);
            let mut summary = ZoneSummary::<1>::default();
            summary.observe(&[1]);
            for block in store.blocks() {
                assert_eq!(block.store().identity(), store.identity());
                assert!(summary.may_match(block.bounds(), &[0]));
                assert!(all.block_nonempty(block.id()));
                for (row, _) in block.rows() {
                    assert!(all.contains(block.id(), row));
                    assert!(selected.insert(block.id(), row));
                }
            }
            assert_eq!(selected.len(), 2);
            assert!(!selected.is_empty());
        }
        "#,
        None,
    );

    // Neither an address nor an owner identity has to support hashing merely
    // to implement metadata traversal. These empty-domain implementations also
    // show that coordinate equality belongs to the selection representation.
    check(
        "opaque_coordinates_need_no_equality_or_hashing",
        r#"
        use cq_storage::{BlockDescriptor, BlockRead, BlockSelection};
        #[derive(Clone, Copy)]
        struct Address;
        #[derive(Clone, Copy, Eq, PartialEq)]
        struct Identity(*const Store);
        struct Store { _marker: u8 }
        #[derive(Clone, Copy)]
        struct Descriptor<'a>(&'a Store);
        impl BlockRead for Store {
            type Row = [i64; 1];
            type BlockId = Address;
            type RowId = Address;
            type Identity = Identity;
            type Descriptor<'a> = Descriptor<'a>;
            fn identity(&self) -> Identity { Identity(std::ptr::from_ref(self)) }
            fn blocks(&self) -> impl Iterator<Item = Descriptor<'_>> {
                std::iter::empty()
            }
        }
        impl<'a> BlockDescriptor<'a> for Descriptor<'a> {
            type Store = Store;
            fn store(&self) -> &'a Store { self.0 }
            fn id(&self) -> Address { Address }
            fn rows(self) -> impl Iterator<Item = (Address, &'a [i64; 1])> {
                std::iter::empty()
            }
        }
        struct EmptySelection;
        impl BlockSelection for EmptySelection {
            type BlockId = Address;
            type RowId = Address;
            fn contains(&self, _: Address, _: Address) -> bool { false }
            fn insert(&mut self, _: Address, _: Address) -> bool {
                panic!("no valid coordinates in this domain")
            }
            fn block_nonempty(&self, _: Address) -> bool { false }
            fn len(&self) -> usize { 0 }
        }
        fn main() {
            assert_eq!(Store { _marker: 0 }.blocks().count(), 0);
            assert!(EmptySelection.is_empty());
        }
        "#,
        None,
    );

    // Compile each failure independently so one inaccessible operation cannot
    // conceal another accidentally public operation in the same consumer.
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "block_read_trait_required",
            "let _ = store.blocks();",
            &["error[E0599]", "BlockRead"],
        ),
        (
            "descriptor_trait_required",
            "let block = cq_storage::BlockRead::blocks(&store).next().unwrap(); let _ = block.rows();",
            &["error[E0599]", "BlockDescriptor"],
        ),
        (
            "bounds_trait_required",
            "let block = cq_storage::BlockRead::blocks(&store).next().unwrap(); let _ = block.bounds();",
            &["error[E0599]", "CoordinateBounds"],
        ),
        (
            "selection_trait_required",
            "let mask = <cq_storage::BlockMask as cq_storage::MakeSelection<_>>::all(&store); let _ = mask.contains(0, 0);",
            &["error[E0599]", "BlockSelection"],
        ),
        (
            "summary_trait_required",
            "let mut summary = cq_storage::ZoneSummary::<1>::default(); summary.observe(&[1]);",
            &["error[E0599]", "RangeSummary"],
        ),
        (
            "metadata_is_private",
            "let _ = store.metadata;",
            &["error[E0616]", "metadata"],
        ),
        (
            "payload_is_private",
            "let _ = store.payload;",
            &["error[E0616]", "payload"],
        ),
        (
            "implementation_module_is_private",
            "let _: Option<cq_storage::blocks::BlockStore<1>> = None;",
            &["error[E0603]", "blocks"],
        ),
        (
            "metadata_shape_helper_is_private",
            "let _ = store.block_lengths();",
            &["error[E0624]", "block_lengths"],
        ),
        (
            "random_access_is_absent_even_with_traits",
            "use cq_storage::*; let _ = store.get(0, 0);",
            &["error[E0599]", "get"],
        ),
        (
            "legacy_set_constructor_is_absent_even_with_traits",
            "use cq_storage::*; let _ = BlockStore::<1>::from_set(std::collections::HashSet::new(), size);",
            &["error[E0599]", "from_set"],
        ),
        (
            "bitmap_shape_constructor_is_absent",
            "let _ = cq_storage::BlockMask::all([1]);",
            &["error[E0599]", "MakeSelection"],
        ),
    ];
    for &(name, body, expected) in cases {
        check(
            name,
            &format!(
                "use cq_storage::BlockStore;\nfn main() {{\n\
                 let size = std::num::NonZeroUsize::new(2).unwrap();\n\
                 let store = BlockStore::from_rows([[1], [2]], size);\n\
                 {body}\n}}\n"
            ),
            Some(expected),
        );
    }
}
