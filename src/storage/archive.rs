use crate::{
    domain::*,
    paths::{LocalPaths, private_dir, sync_directory},
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, PathBuf},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Archives {
    root: PathBuf,
}
pub fn hash(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}
pub fn compress(body: &[u8]) -> Result<Vec<u8>> {
    let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    w.write_all(body)?;
    Ok(w.finish()?)
}
pub fn decompress(body: &[u8]) -> Result<Vec<u8>> {
    if body.len() > PRODUCT_LIMIT + 65536 {
        return Err(Error::Integrity);
    }
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(body)
        .take(PRODUCT_LIMIT as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| Error::Integrity)?;
    if out.len() > PRODUCT_LIMIT {
        return Err(Error::Integrity);
    }
    Ok(out)
}
impl Archives {
    pub fn new(paths: &LocalPaths) -> Self {
        Self {
            root: paths.archives(),
        }
    }
    fn staging(&self) -> Result<PathBuf> {
        let staging = self.root.join(".staging");
        for path in [&self.root, &staging] {
            if let Ok(meta) = std::fs::symlink_metadata(path)
                && meta.file_type().is_symlink()
            {
                return Err(Error::Integrity);
            }
        }
        Ok(staging)
    }
    /// Called only while acquiring exclusive catalogue ownership, before work starts.
    pub(crate) async fn cleanup_staging(&self) -> Result<()> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let staging = store.staging()?;
            let entries = match std::fs::read_dir(&staging) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            // Stream only our flat staging namespace, never recurse into archives.
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if entry.file_type()?.is_file()
                    && (name.starts_with("archive-") || name.starts_with("check-"))
                {
                    std::fs::remove_file(entry.path())?;
                }
            }
            sync_directory(&staging)
        })
        .await
        .map_err(|_| Error::Cancelled)?
    }
    pub fn prepare(
        &self,
        catalogue: Uuid,
        target: &str,
        id: &str,
        body: &[u8],
    ) -> Result<(Archive, Vec<u8>)> {
        if body.len() > PRODUCT_LIMIT
            || target.is_empty()
            || !target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(Error::Integrity);
        }
        ProductId::new(id)?;
        let compressed = compress(body)?;
        let sha256 = hash(body);
        Ok((
            Archive {
                key: format!("{catalogue}/{target}/{id}/{sha256}.json.gz"),
                sha256,
                decoded_bytes: body.len() as i64,
                compressed_bytes: compressed.len() as i64,
                compression: "gzip".into(),
                archived_at: Utc::now(),
            },
            compressed,
        ))
    }
    fn path(&self, a: &Archive) -> Result<PathBuf> {
        self.staging()?;
        let p = std::path::Path::new(&a.key);
        if a.compression != "gzip"
            || !(0..=PRODUCT_LIMIT as i64).contains(&a.decoded_bytes)
            || !(0..=(PRODUCT_LIMIT + 65536) as i64).contains(&a.compressed_bytes)
            || a.key.is_empty()
            || a.key.contains('\\')
            || a.key.contains(':')
            || p.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Error::Integrity);
        }
        let mut path = self.root.clone();
        for part in p.components() {
            path.push(part);
            if let Ok(meta) = std::fs::symlink_metadata(&path)
                && meta.file_type().is_symlink()
            {
                return Err(Error::Integrity);
            }
        }
        Ok(path)
    }
    fn read_sync(&self, a: &Archive) -> Result<Option<Vec<u8>>> {
        let path = self.path(a)?;
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take((PRODUCT_LIMIT + 65537) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() as i64 != a.compressed_bytes {
            return Err(Error::Integrity);
        }
        let body = decompress(&bytes)?;
        if body.len() as i64 != a.decoded_bytes || hash(&body) != a.sha256 {
            return Err(Error::Integrity);
        }
        Ok(Some(body))
    }
    pub async fn read(&self, a: &Archive) -> Result<Option<Vec<u8>>> {
        let store = self.clone();
        let a = a.clone();
        tokio::task::spawn_blocking(move || store.read_sync(&a))
            .await
            .map_err(|_| Error::Cancelled)?
    }
    pub async fn write_verified(&self, a: &Archive, compressed: Vec<u8>) -> Result<()> {
        let store = self.clone();
        let a = a.clone();
        tokio::task::spawn_blocking(move || {
            if store.read_sync(&a)?.is_some() {
                return Ok(());
            }
            let decoded = decompress(&compressed)?;
            if hash(&decoded) != a.sha256
                || decoded.len() as i64 != a.decoded_bytes
                || compressed.len() as i64 != a.compressed_bytes
            {
                return Err(Error::Integrity);
            }
            let path = store.path(&a)?;
            let parent = path.parent().ok_or(Error::Integrity)?;
            private_dir(parent)?;
            let staging = store.staging()?;
            private_dir(&staging)?;
            let mut file = tempfile::Builder::new()
                .prefix("archive-")
                .tempfile_in(&staging)?;
            file.write_all(&compressed)?;
            file.as_file().sync_all()?;
            // Atomic no-clobber publication. A stale writer cannot replace an archive.
            if let Err(e) = file.persist_noclobber(&path)
                && e.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(e.error.into());
            }
            sync_directory(parent)?;
            sync_directory(&staging)?;
            store.read_sync(&a)?.ok_or(Error::Integrity)?;
            Ok(())
        })
        .await
        .map_err(|_| Error::Cancelled)?
    }
    pub async fn test(&self) -> Result<()> {
        self.test_cancel(&tokio_util::sync::CancellationToken::new())
            .await
    }
    pub async fn test_cancel(&self, cancel: &tokio_util::sync::CancellationToken) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let staging = store.staging()?;
            private_dir(&staging)?;
            let mut file = tempfile::Builder::new()
                .prefix("check-")
                .tempfile_in(&staging)?;
            file.write_all(b"kaijucrawl local storage check")?;
            file.as_file().sync_all()?;
            if std::fs::read(file.path())? != b"kaijucrawl local storage check" {
                return Err(Error::Integrity);
            }
            file.close()?;
            sync_directory(&staging)
        })
        .await
        .map_err(|_| Error::Cancelled)?
    }
}
