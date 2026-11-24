//! Explicit front-end file publication, never invoked by compiler printers.
use std::io::{self, Write};
use std::path::Path;

pub fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> { write(path, bytes, true) }
/// Publish without overwriting an existing artifact, including a racing creator.
pub fn publish_new(path: &Path, bytes: &[u8]) -> io::Result<()> { write(path, bytes, false) }
fn write(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let filename = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output must name a file"))?.to_string_lossy();
    let temporary = path.with_file_name(format!(".{filename}.cq-cost-{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        if replace {
            std::fs::rename(&temporary, path)
        } else {
            std::fs::hard_link(&temporary, path)?;
            std::fs::remove_file(&temporary)
        }
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result
}
