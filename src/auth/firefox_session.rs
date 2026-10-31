//! Read Firefox's current saved session, without restoring or modifying it.
use super::{BrowserProfile, Cookie, firefox_context, paste, relevant};
use crate::domain::{Error, ROBOTS_LIMIT, Result};
use serde::Deserialize;
use std::{fs::File, io::Read};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const COMPRESSED_LIMIT: u64 = 32 * 1024 * 1024;
const DECODED_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct Session {
    // Serde skips unrelated session data (tabs, history, form fields, etc.).
    #[serde(default)]
    cookies: Vec<SessionCookie>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct SessionCookie {
    host: String,
    value: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    secure: bool,
    expiry: Option<i64>,
    #[serde(default)]
    #[zeroize(skip)]
    origin_attributes: serde_json::Map<String, serde_json::Value>,
}

fn invalid() -> Error {
    Error::Config("Firefox session snapshot could not be read safely".into())
}

fn decode(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if bytes.len() < 12 || &bytes[..8] != b"mozLz40\0" {
        return Err(invalid());
    }
    let size = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| invalid())?) as usize;
    if size > DECODED_LIMIT {
        return Err(invalid());
    }
    let mut decoded = Zeroizing::new(vec![0; size]);
    let actual =
        lz4_flex::block::decompress_into(&bytes[12..], &mut decoded).map_err(|_| invalid())?;
    if actual != size {
        return Err(invalid());
    }
    Ok(decoded)
}

fn context(cookie: &SessionCookie) -> std::result::Result<super::FirefoxContext, &'static str> {
    let mut attrs = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in &cookie.origin_attributes {
        match key.as_str() {
            "userContextId" | "privateBrowsingId" => {
                let number = value
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("invalid Firefox session context identifier")?;
                attrs.append_pair(key, &number.to_string());
            }
            "partitionKey" | "firstPartyDomain" => {
                attrs.append_pair(
                    key,
                    value.as_str().ok_or("invalid Firefox session partition")?,
                );
            }
            // Firefox serializes this default even for ordinary desktop sessions.
            "geckoViewSessionContextId" if value.as_str() == Some("") => {}
            _ => return Err("unsupported Firefox session context attribute"),
        }
    }
    let attrs = attrs.finish();
    firefox_context(&if attrs.is_empty() {
        attrs
    } else {
        format!("^{attrs}")
    })
}

type ImportedSession = (Vec<Cookie>, Vec<String>);

pub(super) fn read(profile: &BrowserProfile, schema: u32) -> Result<Option<ImportedSession>> {
    // Only current snapshots: never fall back to previous/upgrade/backup sessions.
    let mut selected = None;
    for relative in [
        "sessionstore-backups/recovery.jsonlz4",
        "sessionstore.jsonlz4",
    ] {
        let path = profile.path.join(relative);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(invalid()),
        };
        let modified = metadata.modified().map_err(|_| invalid())?;
        if selected
            .as_ref()
            .is_none_or(|(_, previous)| modified > *previous)
        {
            selected = Some((path, modified));
        }
    }
    let Some((path, _)) = selected else {
        return Ok(None);
    };
    let mut bytes = Zeroizing::new(Vec::new());
    File::open(path)
        .map_err(|_| invalid())?
        .take(COMPRESSED_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > COMPRESSED_LIMIT {
        return Err(invalid());
    }
    let decoded = decode(&bytes)?;
    let session: Session = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    let mut cookies = Vec::new();
    let mut warnings = std::collections::BTreeSet::new();
    let mut total = 0;
    for mut entry in session.cookies {
        if !relevant(&entry.host) {
            continue;
        }
        let scope = match context(&entry) {
            Ok(scope) if scope.container == profile.container => scope,
            Ok(_) => continue,
            Err(reason) => {
                warnings.insert(format!("Firefox session cookies omitted: {reason}; use exact-host manual paste if needed"));
                if entry.name == "cf_clearance" {
                    warnings.insert(format!(
                        "cf_clearance omitted from session snapshot: {reason}"
                    ));
                }
                continue;
            }
        };
        if entry.path.is_empty() {
            entry.path = "/".into();
        }
        let pair = Zeroizing::new(format!("{}={}", entry.name, entry.value));
        if !entry.path.starts_with('/') || entry.value.contains(';') || paste(&pair).is_err() {
            warnings.insert("Malformed Firefox session cookies omitted; use manual paste".into());
            continue;
        }
        total += entry.name.len() + entry.value.len() + entry.host.len() + entry.path.len();
        if total > ROBOTS_LIMIT {
            return Err(invalid());
        }
        cookies.push(Cookie {
            name: std::mem::take(&mut entry.name),
            value: std::mem::take(&mut entry.value),
            host_only: !entry.host.starts_with('.'),
            domain: std::mem::take(&mut entry.host),
            path: std::mem::take(&mut entry.path),
            secure: entry.secure,
            expires: entry
                .expiry
                .filter(|e| *e > 0)
                .map(|e| if schema >= 16 { e / 1000 } else { e }),
            first_party_site: scope.first_party_site,
            // Firefox's session format does not preserve cookie creation times.
            created_micros: None,
        });
    }
    Ok(Some((cookies, warnings.into_iter().collect())))
}
