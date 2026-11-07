//! Persisted host-FOR base columns: the `pack_for` headers and payload of one
//! manifest column, bound to that column file's identity (path, length,
//! modification time) and, for plain columns, the SHA-256 of its stored bytes
//! as hashed when the file was written. Loading a valid file reads only packed
//! bytes; the recorded hash certifies verified keys without rereading them.
use crate::manifest::{Manifest, Result};
use gpu_device_columns::{Bounds, VectorHeader, check_packed_layout};
use std::{
    fs,
    io::{BufReader, BufWriter, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

const MAGIC: [u8; 8] = *b"CQFOR\x02\0\0";
/// Hex SHA-256 width; an absent hash is stored as zero bytes.
const HASH: usize = 64;

pub struct Packed {
    pub rows: usize,
    pub bounds: Bounds,
    pub headers: Vec<VectorHeader>,
    pub payload: Vec<u64>,
    /// SHA-256 (hex) of the source column's stored bytes; plain columns only.
    pub hash: Option<String>,
}

pub fn path(directory: &Path, manifest: &Manifest, table: &str, name: &str) -> PathBuf {
    directory
        .join(format!("sf{}", manifest.sf))
        .join(format!("{table}.{name}.for"))
}

/// Identity of a source column file, observed without reading its values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    path: PathBuf,
    length: u64,
    modified: u64,
}

pub fn identity(manifest: &Manifest, table: &str, name: &str) -> Result<Identity> {
    let path = fs::canonicalize(manifest.root.join(&manifest.column(table, name)?.file))?;
    let metadata = fs::metadata(&path)?;
    let modified = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(Identity {
        path,
        length: metadata.len(),
        modified: u64::try_from(modified)?,
    })
}

fn bytes_of<T: Copy>(values: &[T]) -> &[u8] {
    // SAFETY: only instantiated with u64 and the padding-free repr(C) VectorHeader.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

fn bytes_of_mut<T: Copy>(values: &mut [T]) -> &mut [u8] {
    // SAFETY: as above; every bit pattern is a valid u64/VectorHeader.
    unsafe {
        std::slice::from_raw_parts_mut(values.as_mut_ptr().cast(), std::mem::size_of_val(values))
    }
}

fn read_u64(input: &mut impl Read) -> Result<u64> {
    let mut word = [0u8; 8];
    input.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

/// Fixed fields after the path and hash: length, modified, flag, rows, min,
/// max, headers, words.
const FIELDS: u64 = 8;

/// Read a persisted column if present. A file bound to a different source
/// identity is an error, never silently reused or replaced. Every allocation
/// is bounded by the manifest row count or the file's own length.
pub fn read(file: &Path, manifest: &Manifest, table: &str, name: &str) -> Result<Option<Packed>> {
    let input = match fs::File::open(file) {
        Ok(input) => input,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file_length = input.metadata()?.len();
    let mut input = BufReader::new(input);
    let stale = || format!("{}: stale or malformed packed column", file.display());
    let source = identity(manifest, table, name)?;
    let expected_path = source.path.as_os_str().as_encoded_bytes();
    let mut magic = [0u8; 8];
    input.read_exact(&mut magic)?;
    if magic != MAGIC || read_u64(&mut input)? != expected_path.len() as u64 {
        return Err(stale().into());
    }
    let mut recorded = vec![0u8; expected_path.len()];
    input.read_exact(&mut recorded)?;
    if recorded != expected_path
        || read_u64(&mut input)? != source.length
        || read_u64(&mut input)? != source.modified
    {
        return Err(stale().into());
    }
    let hashed = read_u64(&mut input)?;
    let mut hash = [0u8; HASH];
    input.read_exact(&mut hash)?;
    let hash = match hashed {
        0 => None,
        1 => Some(String::from_utf8(hash.to_vec()).map_err(|_| stale())?),
        _ => return Err(stale().into()),
    };
    let rows = manifest.relations[table].rows;
    if read_u64(&mut input)? != rows as u64 {
        return Err(stale().into());
    }
    let (min, max) = (read_u64(&mut input)? as i64, read_u64(&mut input)? as i64);
    let bounds = if rows == 0 {
        Bounds::Empty
    } else {
        Bounds::Inclusive { min, max }
    };
    let vectors = rows.div_ceil(gpu_device_columns::VECTOR_ROWS);
    if read_u64(&mut input)? != vectors as u64 {
        return Err(stale().into());
    }
    let words = read_u64(&mut input)?;
    let size = (8 + 8 + expected_path.len() as u64 + HASH as u64 + 8 * FIELDS)
        .checked_add(vectors as u64 * 32)
        .and_then(|size| size.checked_add(words.checked_mul(8)?));
    if size != Some(file_length) {
        return Err(stale().into());
    }
    let mut headers = vec![VectorHeader::default(); vectors];
    let mut payload = vec![0u64; usize::try_from(words)?];
    input.read_exact(bytes_of_mut(&mut headers))?;
    input.read_exact(bytes_of_mut(&mut payload))?;
    check_packed_layout(rows, &headers, payload.len())?;
    Ok(Some(Packed {
        rows,
        bounds,
        headers,
        payload,
        hash,
    }))
}

static TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Persist a column packed from values read while the source had identity
/// `source`; a source changed since then is an error. Publication is atomic
/// (exclusive per-call temporary file, sync, rename).
pub fn write(
    file: &Path,
    manifest: &Manifest,
    table: &str,
    name: &str,
    source: &Identity,
    packed: &Packed,
) -> Result<()> {
    if identity(manifest, table, name)? != *source {
        return Err(format!("{}: source changed while packing", source.path.display()).into());
    }
    fs::create_dir_all(file.parent().ok_or("cache file has no directory")?)?;
    let temporary = file.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let mut output = BufWriter::new(
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?,
    );
    let (min, max) = match packed.bounds {
        Bounds::Empty => (0, 0),
        Bounds::Inclusive { min, max } => (min, max),
    };
    let path = source.path.as_os_str().as_encoded_bytes();
    let mut hash = [0u8; HASH];
    if let Some(recorded) = &packed.hash {
        if recorded.len() != HASH {
            return Err("source hash is not a hex SHA-256".into());
        }
        hash.copy_from_slice(recorded.as_bytes());
    }
    output.write_all(&MAGIC)?;
    output.write_all(&(path.len() as u64).to_le_bytes())?;
    output.write_all(path)?;
    for value in [source.length, source.modified, u64::from(packed.hash.is_some())] {
        output.write_all(&value.to_le_bytes())?;
    }
    output.write_all(&hash)?;
    for value in [
        packed.rows as u64,
        min as u64,
        max as u64,
        packed.headers.len() as u64,
        packed.payload.len() as u64,
    ] {
        output.write_all(&value.to_le_bytes())?;
    }
    output.write_all(bytes_of(&packed.headers))?;
    output.write_all(bytes_of(&packed.payload))?;
    output
        .into_inner()
        .map_err(|error| error.into_error())?
        .sync_all()?;
    fs::rename(&temporary, file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_columns_roundtrip_and_reject_a_changed_source() {
        let root = std::env::temp_dir().join(format!("cq-packed-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let values: Vec<i64> = (0..2500).map(|i| i * 7 - 300).collect();
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        fs::write(root.join("t.a.bin"), &bytes).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"schema_version":2,"sf":1,"byte_order":"little","relations":{"t":{"rows":2500,
                "columns":[{"name":"a","type":"i64","logical_type":"i64","encoding":"plain","file":"t.a.bin"}]}}}"#,
        )
        .unwrap();
        let manifest = Manifest::load_with_manifest(&root, &root.join("manifest.json")).unwrap();
        let file = path(&root.join("cache"), &manifest, "t", "a");
        assert!(read(&file, &manifest, "t", "a").unwrap().is_none());
        let source = identity(&manifest, "t", "a").unwrap();
        let (read_values, hash) = manifest.read_column_checked("t", "a", true).unwrap();
        assert_eq!(read_values, values);
        let (headers, payload) = gpu_device_columns::pack_for(&values);
        let packed = Packed {
            rows: values.len(),
            bounds: Bounds::from_values(&values),
            headers,
            payload,
            hash,
        };
        write(&file, &manifest, "t", "a", &source, &packed).unwrap();
        let loaded = read(&file, &manifest, "t", "a").unwrap().unwrap();
        assert_eq!(loaded.rows, packed.rows);
        assert_eq!(loaded.bounds, packed.bounds);
        assert_eq!(loaded.payload, packed.payload);
        assert_eq!(loaded.hash, packed.hash);
        assert!(loaded.hash.is_some());
        assert_eq!(bytes_of(&loaded.headers), bytes_of(&packed.headers));
        // A truncated file is rejected before its declared payload is allocated.
        let full = fs::read(&file).unwrap();
        fs::write(&file, &full[..full.len() - 8]).unwrap();
        assert!(read(&file, &manifest, "t", "a").is_err());
        fs::write(&file, &full).unwrap();
        // A rewritten source column is neither served from nor written over the old identity.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(root.join("t.a.bin"), &bytes).unwrap();
        assert!(read(&file, &manifest, "t", "a").is_err());
        assert!(write(&file, &manifest, "t", "a", &source, &packed).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
