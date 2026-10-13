//! Bounded private logs, restricted to the caller's owned filename and archives.
use anyhow::{Result, bail};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
pub struct RotatingLog {
    path: PathBuf,
    file: File,
    bytes: u64,
    maximum: u64,
    archives: usize,
}
impl RotatingLog {
    pub fn open(path: &Path, maximum: u64, archives: usize) -> Result<Self> {
        if maximum == 0 || maximum > 16 * 1024 * 1024 || archives == 0 || archives > 16 {
            bail!("Invalid log rotation limits");
        }
        for p in std::iter::once(path.to_owned()).chain((1..=archives).map(|n| archive_path(path, n))) {
            if let Ok(m) = std::fs::symlink_metadata(&p) {
                if !m.is_file() || m.file_type().is_symlink() {
                    bail!("Owned log must be a regular file");
                }
                if m.len() > maximum {
                    trim_tail(&p, maximum)?;
                }
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        let file = open_file(path)?;
        let bytes = file.metadata()?.len();
        let mut log = Self {
            path: path.to_owned(),
            file,
            bytes,
            maximum,
            archives,
        };
        if bytes >= maximum {
            log.rotate()?;
        }
        Ok(log)
    }
    fn rotate(&mut self) -> Result<()> {
        self.file.flush()?;
        let oldest = archive_path(&self.path, self.archives);
        if oldest.exists() {
            std::fs::remove_file(oldest)?;
        }
        for n in (1..self.archives).rev() {
            let old = archive_path(&self.path, n);
            if old.exists() {
                std::fs::rename(old, archive_path(&self.path, n + 1))?;
            }
        }
        std::fs::rename(&self.path, archive_path(&self.path, 1))?;
        self.file = open_file(&self.path)?;
        self.bytes = 0;
        Ok(())
    }
    pub fn write(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            if self.bytes >= self.maximum {
                self.rotate()?;
            }
            let count = bytes.len().min((self.maximum - self.bytes) as usize);
            self.file.write_all(&bytes[..count])?;
            self.bytes += count as u64;
            bytes = &bytes[count..];
        }
        Ok(())
    }
}
fn trim_tail(path: &Path, maximum: u64) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let size = file.metadata()?.len();
    let mut tail = vec![0; maximum as usize];
    file.seek(SeekFrom::Start(size - maximum))?;
    file.read_exact(&mut tail)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&tail)?;
    file.set_len(maximum)?;
    Ok(())
}
fn archive_path(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}
fn open_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_bounds_every_owned_file_and_preserves_unrelated_files() -> Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.work/log-tests");
        std::fs::create_dir_all(&root)?;
        let directory = root.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&directory)?;
        let path = directory.join("core.log");
        std::fs::write(&path, vec![0; 100])?;
        std::fs::write(directory.join("unrelated"), b"keep")?;
        let mut log = RotatingLog::open(&path, 16, 3)?;
        // Startup rotates an existing oversized file; remove its historical oversize on subsequent rotations.
        log.write(&[7; 128])?;
        drop(log);
        for n in 0..=3 {
            let p = if n == 0 { path.clone() } else { archive_path(&path, n) };
            assert!(std::fs::metadata(&p)?.len() <= 16);
            assert_eq!(std::fs::metadata(p)?.permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(std::fs::read(directory.join("unrelated"))?, b"keep");
        let link = directory.join("link");
        std::os::unix::fs::symlink(&path, &link)?;
        assert!(RotatingLog::open(&link, 16, 3).is_err());
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
