use crate::{config::Secrets, domain::*, targets::drivethru as d};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop};
mod firefox_session;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub expires: Option<i64>,
    /// Normalized top-level site, retained for supported Firefox partitions/FPI.
    pub first_party_site: Option<String>,
    pub created_micros: Option<i64>,
}
impl Cookie {
    pub fn matches(&self, u: &Url, now: i64) -> bool {
        let Some(host) = u.host_str() else {
            return false;
        };
        let domain = self.domain.trim_start_matches('.');
        let domain_ok = host == domain
            || (!self.host_only && host.strip_suffix(domain).is_some_and(|s| s.ends_with('.')));
        domain_ok
            && self
                .first_party_site
                .as_deref()
                .is_none_or(|site| d::cookie_site(u) == Some(site))
            && (!self.secure || u.scheme() == "https")
            && self.expires.is_none_or(|e| e > now)
            && (u.path() == self.path
                || (u.path().starts_with(&self.path)
                    && (self.path.ends_with('/')
                        || u.path().as_bytes().get(self.path.len()) == Some(&b'/'))))
    }
}
#[derive(Clone, Default)]
pub struct Cookies {
    pub imported: Vec<Cookie>,
    pub raw: Vec<(String, String)>,
    pub warnings: Vec<String>,
    /// Added cookies from Firefox's current session snapshot; None if unavailable.
    pub session_count: Option<usize>,
}
impl Cookies {
    pub fn manual(secrets: &Secrets) -> Result<Self> {
        let mut raw = vec![];
        for (origin, value) in [
            (d::WEBSITE, &secrets.website_cookie),
            (d::API, &secrets.api_cookie),
        ] {
            if !value.is_empty() {
                raw.push((origin.into(), paste(value)?));
            }
        }
        Ok(Self {
            raw,
            ..Default::default()
        })
    }
    pub fn header(&self, u: &Url) -> Option<String> {
        let origin = format!("{}/", u.origin().ascii_serialization());
        if let Some((_, v)) = self.raw.iter().find(|(o, _)| o == &origin) {
            return Some(v.clone());
        }
        let mut cs: Vec<_> = self
            .imported
            .iter()
            .filter(|c| c.matches(u, chrono::Utc::now().timestamp()))
            .collect();
        cs.sort_by_key(|c| {
            (
                std::cmp::Reverse(c.path.len()),
                c.created_micros.unwrap_or(i64::MAX),
            )
        });
        let s = cs
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        if s.is_empty() { None } else { Some(s) }
    }
    pub fn loaded(&self) -> bool {
        !self.imported.is_empty() || !self.raw.is_empty()
    }
    /// Describe eligibility for this exact request without exposing cookie values.
    pub fn diagnostics(&self, u: &Url) -> String {
        let origin = format!("{}/", u.origin().ascii_serialization());
        let (count, clearance) = if let Some((_, raw)) = self.raw.iter().find(|(o, _)| o == &origin)
        {
            let names: Vec<_> = raw
                .split(';')
                .filter_map(|part| part.trim().split_once('=').map(|(name, _)| name))
                .collect();
            let clearance = if names.contains(&"cf_clearance") {
                "eligible (pasted; expiry unknown)"
            } else {
                "not in pasted header"
            };
            (names.len(), clearance)
        } else {
            let now = chrono::Utc::now().timestamp();
            let count = self.imported.iter().filter(|c| c.matches(u, now)).count();
            let clearance = if self
                .imported
                .iter()
                .any(|c| c.name == "cf_clearance" && c.matches(u, now))
            {
                if self.imported.iter().any(|c| {
                    c.name == "cf_clearance" && c.matches(u, now) && c.first_party_site.is_some()
                }) {
                    "eligible (matching first-party partition)"
                } else {
                    "eligible"
                }
            } else if self.imported.iter().any(|c| c.name == "cf_clearance") {
                "loaded but expired or outside this request's scope"
            } else {
                "not loaded"
            };
            (count, clearance)
        };
        format!("Eligible cookies: {count}; cf_clearance: {clearance}")
    }
}
pub fn paste(value: &str) -> Result<String> {
    if value.len() > ROBOTS_LIMIT || value.chars().any(|c| c.is_control()) {
        return Err(Error::Config(
            "Cookie input must be a single line, at most 1 MiB".into(),
        ));
    }
    let value = value.trim();
    let value = if value
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("cookie:"))
    {
        value[7..].trim()
    } else {
        value
    };
    if value.is_empty() {
        return Err(Error::Config("Cookie input is empty".into()));
    }
    for part in value.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            return Err(Error::Config("Each cookie must be name=value".into()));
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            // Browsers can send legacy values such as literal JSON containing
            // quotes, commas and spaces. Preserve the header exactly; names
            // remain tokens and controls/non-ASCII cannot enter a header line.
            || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
        {
            return Err(Error::Config(
                "Malformed cookie or additional HTTP header".into(),
            ));
        }
    }
    Ok(value.into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserProfile {
    pub browser: String,
    pub name: String,
    pub path: PathBuf,
    pub container: u32,
}
fn profile_roots() -> Vec<(String, PathBuf)> {
    let Some(base) = directories::BaseDirs::new() else {
        return vec![];
    };
    let mut roots = vec![];
    #[cfg(windows)]
    {
        let local = base.data_local_dir();
        let roaming = base.config_dir();
        roots.push(("Firefox".into(), roaming.join("Mozilla/Firefox")));
        for (n, p) in [
            ("Chrome", "Google/Chrome/User Data"),
            ("Edge", "Microsoft/Edge/User Data"),
            ("Brave", "BraveSoftware/Brave-Browser/User Data"),
        ] {
            roots.push((n.into(), local.join(p)));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let home = base.home_dir();
        roots.push(("Firefox".into(), home.join(".mozilla/firefox")));
        roots.push((
            "Firefox".into(),
            home.join("snap/firefox/common/.mozilla/firefox"),
        ));
        for (n, p) in [
            ("Chrome", "google-chrome"),
            ("Edge", "microsoft-edge"),
            ("Brave", "BraveSoftware/Brave-Browser"),
        ] {
            roots.push((n.into(), base.config_dir().join(p)));
        }
    }
    roots
}
pub fn discover_profiles() -> Vec<BrowserProfile> {
    profile_roots()
        .into_iter()
        .flat_map(|(b, p)| profiles_at(&b, &p))
        .collect()
}
pub fn profiles_at(browser: &str, root: &Path) -> Vec<BrowserProfile> {
    let mut profiles = vec![];
    if browser == "Firefox" {
        let Ok(manifest) = std::fs::read_to_string(root.join("profiles.ini")) else {
            return profiles;
        };
        let mut path = None;
        let mut name = String::new();
        let mut relative = true;
        for line in manifest.lines().chain(std::iter::once("[end]")) {
            let line = line.trim();
            if line.starts_with('[') {
                if let Some(p) = path.take() {
                    let p = if relative {
                        root.join(p)
                    } else {
                        PathBuf::from(p)
                    };
                    if p.is_dir() {
                        profiles.push(BrowserProfile {
                            browser: browser.into(),
                            name: name.clone(),
                            path: p,
                            container: 0,
                        });
                    }
                }
                name.clear();
                relative = true;
            } else if let Some((k, v)) = line.split_once('=') {
                match k {
                    "Path" => path = Some(v.to_string()),
                    "Name" => name = v.into(),
                    "IsRelative" => relative = v == "1",
                    _ => {}
                }
            }
        }
    } else {
        let Ok(manifest) = std::fs::read(root.join("Local State")) else {
            return profiles;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&manifest) else {
            return profiles;
        };
        if let Some(cache) = value["profile"]["info_cache"].as_object() {
            for (dir, p) in cache {
                if dir.contains('/') || dir.contains('\\') || dir == ".." {
                    continue;
                }
                let path = root.join(dir);
                if path.is_dir() {
                    profiles.push(BrowserProfile {
                        browser: browser.into(),
                        name: p["name"].as_str().unwrap_or(dir).into(),
                        path,
                        container: 0,
                    });
                }
            }
        }
    }
    profiles
}
pub fn containers(profile: &BrowserProfile) -> Vec<(u32, String)> {
    let mut out = vec![(0, "Default (no container)".into())];
    if profile.browser != "Firefox" {
        return out;
    }
    if let Ok(bytes) = std::fs::read(profile.path.join("containers.json"))
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(xs) = v["identities"].as_array()
    {
        for x in xs {
            if let Some(n) = x["userContextId"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
            {
                out.push((n, x["name"].as_str().unwrap_or("Named container").into()));
            }
        }
    }
    out
}
fn import_error() -> Error {
    Error::Config(
        "Browser cookies unavailable, locked, or unsupported; use manual Paste cookies".into(),
    )
}
fn relevant(host: &str) -> bool {
    d::COOKIE_DOMAINS.contains(&host)
}
struct FirefoxContext {
    container: u32,
    first_party_site: Option<String>,
}
fn firefox_context(context: &str) -> std::result::Result<FirefoxContext, &'static str> {
    let mut out = FirefoxContext {
        container: 0,
        first_party_site: None,
    };
    if context.is_empty() {
        return Ok(out);
    }
    let suffix = context
        .strip_prefix('^')
        .ok_or("malformed Firefox context")?;
    if suffix.split('&').any(|part| !part.contains('=')) {
        return Err("malformed Firefox context");
    }
    let mut seen = std::collections::HashSet::new();
    // Firefox serializes OriginAttributes using URLParams, including percent encoding.
    for (key, value) in url::form_urlencoded::parse(suffix.as_bytes()) {
        if !seen.insert(key.clone()) {
            return Err("duplicate Firefox context attribute");
        }
        match key.as_ref() {
            "userContextId" => {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("invalid Firefox container identifier");
                }
                out.container = value
                    .parse()
                    .map_err(|_| "invalid Firefox container identifier")?;
            }
            "privateBrowsingId" if value == "0" => {}
            "privateBrowsingId" => return Err("private-browsing context is not selected"),
            "partitionKey" | "firstPartyDomain" if value.is_empty() => {}
            "partitionKey" | "firstPartyDomain" => {
                let site =
                    Url::parse(d::COOKIE_SITE).map_err(|_| "unsupported adapter cookie site")?;
                let host = site.host_str().ok_or("unsupported adapter cookie site")?;
                // Support schemeful keys and older host-only keys. Legacy keys
                // are narrowed to the adapter's HTTPS first-party request context.
                // No foreign-ancestor flag, other scheme/port/site, or unknown
                // attributes may be flattened into an unpartitioned cookie.
                let matches = value == host
                    || value == format!("({},{host})", site.scheme())
                    || site
                        .port_or_known_default()
                        .is_some_and(|port| value == format!("({},{host},{port})", site.scheme()));
                if !matches {
                    return Err("partition is outside the supported first-party site/context");
                }
                out.first_party_site = Some(d::COOKIE_SITE.into());
            }
            _ => return Err("unsupported Firefox context attribute"),
        }
    }
    Ok(out)
}
pub fn import(profile: &BrowserProfile) -> Result<Cookies> {
    let firefox = profile.browser == "Firefox";
    let path = if firefox {
        profile.path.join("cookies.sqlite")
    } else {
        let network = profile.path.join("Network/Cookies");
        if network.exists() {
            network
        } else {
            profile.path.join("Cookies")
        }
    };
    // SQLite's read transaction provides a consistent view including WAL. Never
    // copy only the main database, modify it, or terminate a browser.
    let mut db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| import_error())?;
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|_| import_error())?;
    let tx = db.transaction().map_err(|_| import_error())?;
    // Firefox schema 16 changed expiry from seconds to milliseconds.
    let firefox_schema: u32 = if firefox {
        tx.pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| import_error())?
    } else {
        0
    };
    let mut out = Cookies::default();
    let mut total = 0usize;
    let base_sql = if firefox {
        "SELECT name,value,host,path,isSecure,expiry,originAttributes,NULL FROM moz_cookies WHERE host IN (?1,?2,?3,?4,?5,?6)"
    } else {
        "SELECT name,value,host_key,path,is_secure,expires_utc,top_frame_site_key,encrypted_value FROM cookies WHERE host_key IN (?1,?2,?3,?4,?5,?6)"
    };
    let (table, creation_column) = if firefox {
        ("moz_cookies", "creationTime")
    } else {
        ("cookies", "creation_utc")
    };
    let has_creation: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
            [table, creation_column],
            |r| r.get(0),
        )
        .map_err(|_| import_error())?;
    let sql = if has_creation {
        base_sql.replacen(" FROM ", &format!(",{creation_column} FROM "), 1)
    } else {
        base_sql.replacen(" FROM ", ",NULL FROM ", 1)
    };
    let mut statement = tx.prepare(&sql).map_err(|_| import_error())?;
    let mut rows = statement
        .query(rusqlite::params_from_iter(d::COOKIE_DOMAINS))
        .map_err(|_| import_error())?;
    while let Some(row) = rows.next().map_err(|_| import_error())? {
        let host: String = row.get(2).map_err(|_| import_error())?;
        if !relevant(&host) {
            continue;
        }
        let name: String = row.get(0).map_err(|_| import_error())?;
        let context: String = row.get(6).map_err(|_| import_error())?;
        let mut first_party_site = None;
        if firefox {
            match firefox_context(&context) {
                Ok(scope) if scope.container == profile.container => {
                    first_party_site = scope.first_party_site
                }
                Ok(_) => continue,
                Err(reason) => {
                    out.warnings.push(format!(
                        "Firefox cookies omitted: {reason}; use exact-host manual paste if needed"
                    ));
                    if name == "cf_clearance" {
                        out.warnings.push(format!("cf_clearance omitted: {reason}"));
                    }
                    continue;
                }
            }
        } else if !context.is_empty() {
            out.warnings
                .push("Partitioned Chromium cookies omitted".into());
            if name == "cf_clearance" {
                out.warnings
                    .push("cf_clearance omitted: unsupported Chromium partition".into());
            }
            continue;
        }
        let mut value: String = row.get(1).map_err(|_| import_error())?;
        if !firefox && value.is_empty() {
            let encrypted: Vec<u8> = row.get(7).map_err(|_| import_error())?;
            if !encrypted.is_empty() {
                value = decrypt_chromium(&encrypted, &profile.browser, &host)?;
            }
        }
        paste(&format!("{name}={value}"))?;
        let expiry: i64 = row.get(5).map_err(|_| import_error())?;
        let expires = if expiry <= 0 {
            None
        } else if firefox {
            Some(if firefox_schema >= 16 {
                expiry / 1000
            } else {
                expiry
            })
        } else {
            Some(expiry / 1_000_000 - 11_644_473_600)
        };
        let path: String = row.get(3).map_err(|_| import_error())?;
        total += name.len() + value.len() + host.len() + path.len();
        if total > ROBOTS_LIMIT {
            return Err(Error::Config(
                "Relevant imported cookies exceed 1 MiB".into(),
            ));
        }
        out.imported.push(Cookie {
            name,
            value,
            host_only: !host.starts_with('.'),
            domain: host,
            path,
            secure: row.get(4).map_err(|_| import_error())?,
            expires,
            first_party_site,
            created_micros: row.get::<_, Option<i64>>(8).map_err(|_| import_error())?,
        });
    }
    drop(rows);
    drop(statement);
    drop(tx);
    if firefox {
        match firefox_session::read(profile, firefox_schema) {
            Ok(Some((session, warnings))) => {
                out.warnings.extend(warnings);
                // Current database values win over a potentially older session
                // snapshot. Also reject duplicate identities within the snapshot.
                let mut identities: std::collections::BTreeSet<_> = out.imported.iter().map(|c|
                    (c.name.clone(), c.domain.clone(), c.path.clone(), c.first_party_site.clone())
                ).collect();
                let additional: Vec<_> = session.into_iter().filter(|c| identities.insert(
                    (c.name.clone(), c.domain.clone(), c.path.clone(), c.first_party_site.clone())
                )).collect();
                total += additional.iter().map(|c| c.name.len() + c.value.len() + c.domain.len() + c.path.len()).sum::<usize>();
                if total > ROBOTS_LIMIT {
                    return Err(Error::Config("Relevant imported cookies exceed 1 MiB".into()));
                }
                out.session_count = Some(additional.len());
                out.imported.extend(additional);
            }
            Ok(None) => {},
            Err(_) => out.warnings.push("Firefox session snapshot unavailable, corrupt, too large or unsupported; only database cookies loaded. Use manual paste for the complete current header".into()),
        }
    }
    out.warnings.sort();
    out.warnings.dedup();
    Ok(out)
}
#[cfg(windows)]
fn decrypt_chromium(_encrypted: &[u8], _browser: &str, _host: &str) -> Result<String> {
    Err(Error::Config("Windows Chromium encrypted cookies are unsupported (including app-bound encryption). Use Firefox import or manual paste; no browser protection is bypassed".into()))
}
#[cfg(target_os = "linux")]
fn decrypt_chromium(encrypted: &[u8], browser: &str, host: &str) -> Result<String> {
    let mut password = if encrypted.starts_with(b"v10") {
        b"peanuts".to_vec()
    } else if encrypted.starts_with(b"v11") {
        let application = match browser {
            "Chrome" => "chrome",
            "Edge" => "microsoft-edge",
            "Brave" => "brave",
            _ => return Err(import_error()),
        };
        let service =
            dbus_secret_service::SecretService::connect(dbus_secret_service::EncryptionType::Dh)
                .map_err(|_| import_error())?;
        let items = service
            .search_items(std::collections::HashMap::from([(
                "application",
                application,
            )]))
            .map_err(|_| import_error())?;
        let item = items.unlocked.first().ok_or_else(|| {
            Error::Config(
                "Linux browser keyring is locked or unavailable; unlock it or use manual paste"
                    .into(),
            )
        })?;
        item.get_secret().map_err(|_| import_error())?
    } else {
        return Err(import_error());
    };
    let result = decrypt_linux_cookie(encrypted, &password, host);
    password.zeroize();
    result
}
#[cfg(target_os = "linux")]
fn decrypt_linux_cookie(encrypted: &[u8], password: &[u8], host: &str) -> Result<String> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    use sha2::{Digest, Sha256};
    if encrypted.len() < 3 {
        return Err(import_error());
    }
    let mut key = [0; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1, &mut key);
    let bytes = cbc::Decryptor::<aes::Aes128>::new((&key).into(), (&[b' '; 16]).into())
        .decrypt_padded_vec_mut::<Pkcs7>(&encrypted[3..])
        .map_err(|_| import_error())?;
    key.zeroize();
    let hash = Sha256::digest(host.as_bytes());
    let bytes = if bytes.starts_with(&hash) {
        &bytes[32..]
    } else {
        &bytes
    };
    String::from_utf8(bytes.to_vec()).map_err(|_| import_error())
}
#[cfg(all(test, target_os = "linux"))]
mod crypto_tests {
    use super::*;
    #[test]
    fn chromium_keyring_secret_and_host_digest() {
        use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
        use sha2::{Digest, Sha256};
        let password = b"synthetic keyring secret";
        let host = ".drivethrurpg.com";
        let mut key = [0; 16];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1, &mut key);
        let mut plaintext = Sha256::digest(host.as_bytes()).to_vec();
        plaintext.extend_from_slice(b"synthetic-cookie");
        let mut encrypted = b"v11".to_vec();
        encrypted.extend(
            cbc::Encryptor::<aes::Aes128>::new((&key).into(), (&[b' '; 16]).into())
                .encrypt_padded_vec_mut::<Pkcs7>(&plaintext),
        );
        assert_eq!(
            decrypt_linux_cookie(&encrypted, password, host).unwrap(),
            "synthetic-cookie"
        );
        assert!(decrypt_linux_cookie(&encrypted, b"wrong", host).is_err());
        assert!(decrypt_linux_cookie(&encrypted, password, "wrong.invalid").is_err());
    }
}
