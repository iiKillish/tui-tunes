use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{fs, path::Path};

/// Replace the file at `path` with `contents`, so that a crash, a full disk or another write at
/// the same time can't leave it half written: they go to a temporary file beside it first, which
/// is flushed to disk and then moved over it. With `private`, the file is readable only by the
/// current user (on unix), from the moment it's created; else it keeps the permissions it had.
/// A symlink (say into a dotfiles repository) stays one: the file it points to is replaced.
pub fn write_atomically(path: &Path, contents: &[u8], private: bool) -> std::io::Result<()> {
    let resolved = fs::canonicalize(path);
    let path = resolved.as_deref().unwrap_or(path);
    let permissions = fs::metadata(path).ok().map(|m| m.permissions());
    // Unique within the process too, for threads writing the same file at once.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let temp_path = path.with_file_name(temp_name);
    // A file left by a crash would keep its permissions, so start over.
    let _ = fs::remove_file(&temp_path);

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let written = options.open(&temp_path).and_then(|mut file| {
        if !private && let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(contents)?;
        file.sync_all()
    });
    let result = written.and_then(|()| fs::rename(&temp_path, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

pub trait Serializer {
    /// Configuration and credential file helper
    /// Creates a default configuration if none exist, otherwise will optionally overwrite
    /// the file if it fails to parse
    fn load_or_generate_default<
        P: AsRef<Path>,
        T: serde::Serialize + serde::de::DeserializeOwned,
        F: Fn() -> Result<T, String>,
    >(
        &self,
        path: P,
        default: F,
        default_on_parse_failure: bool,
    ) -> Result<T, String> {
        let path = path.as_ref();
        // Nothing exists so just write the default and return it
        if !path.exists() {
            let value = default()?;
            return self.write(path, value);
        }

        let result = self.load(path);
        if default_on_parse_failure && result.is_err() {
            let value = default()?;
            return self.write(path, value);
        }
        result.map_err(|e| format!("Unable to parse {}: {}", path.to_string_lossy(), e))
    }

    fn load<P: AsRef<Path>, T: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        path: P,
    ) -> Result<T, String>;
    fn write<P: AsRef<Path>, T: serde::Serialize>(&self, path: P, value: T) -> Result<T, String>;
}

pub struct TomlSerializer {}
impl Serializer for TomlSerializer {
    fn load<P: AsRef<Path>, T: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        path: P,
    ) -> Result<T, String> {
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("Unable to read {}: {}", path.as_ref().to_string_lossy(), e))?;
        toml::from_str(&contents).map_err(|e| {
            format!(
                "Unable to parse toml {}: {}",
                path.as_ref().to_string_lossy(),
                e
            )
        })
    }

    fn write<P: AsRef<Path>, T: serde::Serialize>(&self, path: P, value: T) -> Result<T, String> {
        let content =
            toml::to_string_pretty(&value).map_err(|e| format!("Failed serializing value: {e}"))?;
        write_atomically(path.as_ref(), content.as_bytes(), false)
            .map(|_| value)
            .map_err(|e| {
                format!(
                    "Failed writing content to {}: {}",
                    path.as_ref().display(),
                    e
                )
            })
    }
}

pub struct CborSerializer {}
impl Serializer for CborSerializer {
    fn load<P: AsRef<Path>, T: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        path: P,
    ) -> Result<T, String> {
        let contents = std::fs::read(&path)
            .map_err(|e| format!("Unable to read {}: {}", path.as_ref().to_string_lossy(), e))?;
        serde_cbor::from_slice(&contents).map_err(|e| {
            format!(
                "Unable to parse CBOR {}: {}",
                path.as_ref().to_string_lossy(),
                e
            )
        })
    }

    fn write<P: AsRef<Path>, T: serde::Serialize>(&self, path: P, value: T) -> Result<T, String> {
        let content =
            serde_cbor::to_vec(&value).map_err(|e| format!("Failed serializing value: {e}"))?;
        write_atomically(path.as_ref(), &content, false)
            .map(|_| value)
            .map_err(|e| {
                format!(
                    "Failed writing content to {}: {}",
                    path.as_ref().display(),
                    e
                )
            })
    }
}

pub static TOML: TomlSerializer = TomlSerializer {};
pub static CBOR: CborSerializer = CborSerializer {};

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ttunes-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_writes_replace_the_file_and_leave_nothing_else() {
        let dir = temp_dir("atomic-write");
        let path = dir.join("state.cbor");
        fs::write(&path, "old").unwrap();
        write_atomically(&path, b"new", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let files: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(files.len(), 1);

        // A file that can't be written (its folder is gone) leaves no temporary file.
        assert!(write_atomically(&dir.join("missing/state.cbor"), b"x", false).is_err());

        let value: Vec<u32> = vec![1, 2, 3];
        CBOR.write(&path, &value).unwrap();
        assert_eq!(CBOR.load::<_, Vec<u32>>(&path).unwrap(), value);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_atomic_writes_are_only_for_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private-write");
        let path = dir.join("token.json");
        write_atomically(&path, b"{}", true).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_writes_keep_symlinks_and_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("symlink-write");
        let target = dir.join("dotfiles-config.toml");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_atomically(&link, b"new", false).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        fs::remove_dir_all(&dir).unwrap();
    }
}
