//! Catalog adapters retain tuple typing while reading the authored CQ schema.
//! Erasure to integer rows happens only at the dataset/oracle boundary.

pub(crate) type ReadRelations =
    fn(&std::path::Path) -> Result<Vec<crate::dataset::RelationData>, crate::dataset::DatasetError>;

macro_rules! capture {
    ($vis:vis struct $query:ident; $($rest:tt)*) => {
        pub(super) fn read_relations(directory: &::std::path::Path)
            -> Result<Vec<crate::dataset::RelationData>, crate::dataset::DatasetError>
        {
            let mut relations = Vec::new();
            crate::loading::relations!(relations, directory; $($rest)*);
            Ok(relations)
        }
    };
}
pub(crate) use capture;

macro_rules! relations {
    ($rows:ident, $directory:ident;
     relation $name:ident($($column:ident: $ty:ty),* $(,)?); $($rest:tt)*) => {
        {
            ::mini_linq::relation_loaders! {
                relation $name($($column: $ty),*);
            }
            let path = $directory.join(crate::dataset::input_file_name($rows.len()));
            let tuples = <relation_loader!($name)>::csv(path, ::cq_load::CsvOptions::default())?;
            let rows = tuples.into_iter().map(|($($column,)*)| vec![$($column),*]).collect();
            $rows.push(crate::dataset::RelationData::new(
                stringify!($name), [$(stringify!($column)),*].len(), rows,
            )?);
        }
        crate::loading::relations!($rows, $directory; $($rest)*);
    };
    ($rows:ident, $directory:ident; $($query:tt)*) => {};
}
pub(crate) use relations;
