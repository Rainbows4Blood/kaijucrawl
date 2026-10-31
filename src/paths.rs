//! Production paths are always under the current user's home. No CLI/config/env override.
use crate::domain::{Error, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct LocalPaths {
    root: PathBuf,
}
impl LocalPaths {
    pub fn current() -> Result<Self> {
        let base = directories::BaseDirs::new()
            .ok_or_else(|| Error::Config("Cannot locate your home folder".into()))?;
        Ok(Self::at(base.home_dir().join(".kaijucrawl")))
    }
    /// Dependency injection for disposable tests; never exposed as a user setting.
    #[doc(hidden)]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.toml")
    }
    pub fn secrets(&self) -> PathBuf {
        self.root.join("secrets.json")
    }
    pub fn database(&self) -> PathBuf {
        self.root.join("catalogue.sqlite3")
    }
    pub fn archives(&self) -> PathBuf {
        self.root.join("archives")
    }
    pub fn lock(&self) -> PathBuf {
        self.root.join("catalogue.lock")
    }
    pub fn ensure(&self) -> Result<()> {
        private_dir(&self.root)
    }
}
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut next = path;
    while !next.exists() {
        missing.push(next);
        next = next
            .parent()
            .ok_or_else(|| Error::Config("Invalid local directory".into()))?;
    }
    for directory in missing.into_iter().rev() {
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700).create(directory).or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists && directory.is_dir() {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        }
        #[cfg(not(unix))]
        if let Err(e) = builder.create(directory)
            && !(e.kind() == std::io::ErrorKind::AlreadyExists && directory.is_dir())
        {
            return Err(e.into());
        }
        if let Some(parent) = directory.parent() {
            sync_directory(parent)?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
pub(crate) fn private_file(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Config("Invalid local file path".into()))?;
    private_dir(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| Error::Io(e.error))?;
    sync_directory(parent)
}
