use kaijucrawl::auth::{self, BrowserProfile};
use rusqlite::params;
use std::path::Path;
fn profile(path: &Path) -> BrowserProfile {
    BrowserProfile {
        browser: "Firefox".into(),
        name: "Synthetic only".into(),
        path: path.into(),
        container: 0,
    }
}

fn session_database(path: &Path, schema: u32) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(path.join("cookies.sqlite")).unwrap();
    db.pragma_update(None, "user_version", schema).unwrap();
    db.execute_batch("CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT,creationTime INTEGER)").unwrap();
    db
}

fn session_snapshot(path: &Path, value: serde_json::Value) -> Vec<u8> {
    let mut bytes = b"mozLz40\0".to_vec();
    bytes.extend(lz4_flex::block::compress_prepend_size(
        &serde_json::to_vec(&value).unwrap(),
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

#[test]
fn firefox_session_snapshot_adds_json_cookie_and_keeps_database_values() {
    let temp = tempfile::tempdir().unwrap();
    let db = session_database(temp.path(), 16);
    db.execute_batch("INSERT INTO moz_cookies VALUES('osCsid','DATABASE_VALUE','.drivethrurpg.com','/',1,9999999999000,'',20)").unwrap();
    let path = temp.path().join("sessionstore-backups/recovery.jsonlz4");
    let original = session_snapshot(
        &path,
        serde_json::json!({"cookies": [
        {"host": ".drivethrurpg.com", "name": "siteSettings", "value": "{\"language\":\"en\",\"currency\":\"USD\"}", "path": "/", "secure": true, "originAttributes": {"userContextId":0,"privateBrowsingId":0,"firstPartyDomain":"","partitionKey":"(https,drivethrurpg.com)","geckoViewSessionContextId":""}},
        {"host": ".drivethrurpg.com", "name": "osCsid", "value": "OLD_SNAPSHOT"},
        {"host": ".drivethrurpg.com", "name": "siteSettings", "value": "DUPLICATE", "originAttributes": {"partitionKey":"(https,drivethrurpg.com)"}}
    ], "windows": [{"tabs": [{"unrelated": "DO_NOT_IMPORT"}]}]}),
    );
    let cookies = auth::import(&profile(temp.path())).unwrap();
    assert_eq!(cookies.session_count, Some(1));
    assert_eq!(cookies.imported.len(), 2);
    assert!(cookies.warnings.is_empty());
    assert_eq!(
        cookies
            .header(&url::Url::parse("https://www.drivethrurpg.com/robots.txt").unwrap())
            .unwrap(),
        "osCsid=DATABASE_VALUE; siteSettings={\"language\":\"en\",\"currency\":\"USD\"}"
    );
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert_eq!(
        db.query_row("SELECT value FROM moz_cookies", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "DATABASE_VALUE"
    );
}

#[test]
fn firefox_session_snapshot_enforces_scope_container_and_context() {
    let temp = tempfile::tempdir().unwrap();
    let _db = session_database(temp.path(), 16);
    let mut entries = vec![];
    for (name, host, attrs) in [
        ("default", ".drivethrurpg.com", serde_json::json!({})),
        (
            "selected",
            "www.drivethrurpg.com",
            serde_json::json!({"userContextId":2,"partitionKey":"(https,drivethrurpg.com)"}),
        ),
        (
            "foreign",
            ".unrelated.invalid",
            serde_json::json!({"userContextId":2}),
        ),
        (
            "private",
            ".drivethrurpg.com",
            serde_json::json!({"userContextId":2,"privateBrowsingId":1}),
        ),
        (
            "partition",
            ".drivethrurpg.com",
            serde_json::json!({"userContextId":2,"partitionKey":"(https,other.invalid)"}),
        ),
        (
            "ancestor",
            ".drivethrurpg.com",
            serde_json::json!({"userContextId":2,"partitionKey":"(https,drivethrurpg.com,f)"}),
        ),
        (
            "gecko",
            ".drivethrurpg.com",
            serde_json::json!({"userContextId":2,"geckoViewSessionContextId":"UNTRUSTED"}),
        ),
        (
            "unknown",
            ".drivethrurpg.com",
            serde_json::json!({"userContextId":2,"unknown":"UNTRUSTED"}),
        ),
        (
            "malformed",
            ".drivethrurpg.com",
            serde_json::json!({"userContextId":"2"}),
        ),
    ] {
        entries.push(serde_json::json!({"host":host,"name":name,"value":"SYNTHETIC_SECRET","path":"/account","secure":true,"originAttributes":attrs}));
    }
    entries.push(serde_json::json!({"host":".drivethrurpg.com","name":"injected","value":"x; injection=y","originAttributes":{"userContextId":2}}));
    session_snapshot(
        &temp.path().join("sessionstore.jsonlz4"),
        serde_json::json!({"cookies":entries}),
    );
    let mut selected = profile(temp.path());
    selected.container = 2;
    let cookies = auth::import(&selected).unwrap();
    assert_eq!(cookies.imported.len(), 1);
    assert_eq!(cookies.imported[0].name, "selected");
    assert!(
        cookies
            .header(&url::Url::parse("https://www.drivethrurpg.com/account").unwrap())
            .is_some()
    );
    for url in [
        "https://www.drivethrurpg.com/robots.txt",
        "https://api.drivethrurpg.com/account",
        "http://www.drivethrurpg.com/account",
        "https://www.drivethrurpg.com:8443/account",
    ] {
        assert!(cookies.header(&url::Url::parse(url).unwrap()).is_none());
    }
    assert!(!cookies.warnings.is_empty());
    assert!(!cookies.warnings.join(" ").contains("SYNTHETIC_SECRET"));
    assert!(!cookies.warnings.join(" ").contains("UNTRUSTED"));
}

#[test]
fn firefox_current_session_selection_never_falls_back_to_old_backups() {
    let temp = tempfile::tempdir().unwrap();
    let _db = session_database(temp.path(), 16);
    let recovery = temp.path().join("sessionstore-backups/recovery.jsonlz4");
    let clean = temp.path().join("sessionstore.jsonlz4");
    let snapshot = |name| serde_json::json!({"cookies":[{"host":".drivethrurpg.com","name":name,"value":"synthetic"}]});
    session_snapshot(
        &temp.path().join("sessionstore-backups/recovery.baklz4"),
        snapshot("old_backup"),
    );
    assert_eq!(
        auth::import(&profile(temp.path())).unwrap().session_count,
        None
    );
    session_snapshot(&clean, snapshot("clean"));
    assert_eq!(
        auth::import(&profile(temp.path())).unwrap().imported[0].name,
        "clean"
    );
    session_snapshot(&recovery, snapshot("recovery"));
    std::fs::File::options()
        .write(true)
        .open(&clean)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10),
            ),
        )
        .unwrap();
    assert_eq!(
        auth::import(&profile(temp.path())).unwrap().imported[0].name,
        "recovery"
    );
    std::fs::write(&recovery, b"corrupt snapshot").unwrap();
    let cookies = auth::import(&profile(temp.path())).unwrap();
    assert!(cookies.imported.is_empty());
    assert_eq!(cookies.session_count, None);
    assert!(cookies.warnings.join(" ").contains("session snapshot"));
}

#[test]
fn firefox_session_rejects_corruption_and_oversized_decoding_without_leaking_data() {
    let temp = tempfile::tempdir().unwrap();
    let _db = session_database(temp.path(), 16);
    let mut oversized = b"mozLz40\0".to_vec();
    oversized.extend(u32::MAX.to_le_bytes());
    for bytes in [
        b"SYNTHETIC_PRIVATE_BAD_FORMAT".to_vec(),
        oversized,
        b"mozLz40\0\x08\x00\x00\x00INVALID_LZ4".to_vec(),
    ] {
        std::fs::write(temp.path().join("sessionstore.jsonlz4"), bytes).unwrap();
        let cookies = auth::import(&profile(temp.path())).unwrap();
        assert_eq!(cookies.session_count, None);
        assert!(cookies.imported.is_empty());
        assert!(cookies.warnings.join(" ").contains("manual paste"));
        assert!(!cookies.warnings.join(" ").contains("PRIVATE"));
    }
}

#[test]
fn firefox_session_expiry_uses_schema_units() {
    for (schema, multiplier) in [(15, 1_i64), (16, 1000), (17, 1000)] {
        let temp = tempfile::tempdir().unwrap();
        let _db = session_database(temp.path(), schema);
        let now = chrono::Utc::now().timestamp();
        session_snapshot(
            &temp.path().join("sessionstore.jsonlz4"),
            serde_json::json!({"cookies":[
                {"host":".drivethrurpg.com","name":"expired","value":"synthetic","expiry":(now-100)*multiplier},
                {"host":".drivethrurpg.com","name":"valid","value":"synthetic","expiry":(now+3600)*multiplier}
            ]}),
        );
        let cookies = auth::import(&profile(temp.path())).unwrap();
        assert_eq!(
            cookies
                .header(&url::Url::parse("https://www.drivethrurpg.com/").unwrap())
                .as_deref(),
            Some("valid=synthetic")
        );
    }
}

#[test]
fn firefox_cookie_header_orders_longer_paths_then_older_creation() {
    let temp = tempfile::tempdir().unwrap();
    let db = session_database(temp.path(), 16);
    for (name, path, created) in [
        ("new", "/", Some(20)),
        ("unknown", "/", None),
        ("old", "/", Some(10)),
        ("deep", "/account", Some(30)),
    ] {
        db.execute("INSERT INTO moz_cookies VALUES(?1,'synthetic','.drivethrurpg.com',?2,1,9999999999000,'',?3)", params![name, path, created]).unwrap();
    }
    assert_eq!(
        auth::import(&profile(temp.path()))
            .unwrap()
            .header(&url::Url::parse("https://www.drivethrurpg.com/account").unwrap())
            .as_deref(),
        Some("deep=synthetic; old=synthetic; new=synthetic; unknown=synthetic")
    );
}
#[test]
fn firefox_readonly_wal_container_scope_and_partition_filter() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("cookies.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT)").unwrap();
    for (name, host, attrs) in [
        ("default", ".drivethrurpg.com", ""),
        ("container", "api.drivethrurpg.com", "^userContextId=2"),
        (
            "partition",
            ".drivethrurpg.com",
            "^partitionKey=%28https%2Cexample.invalid%29",
        ),
        ("other", "unrelated.invalid", ""),
    ] {
        db.execute(
            "INSERT INTO moz_cookies VALUES(?1,'synthetic',?2,'/api',1,9999999999,?3)",
            params![name, host, attrs],
        )
        .unwrap();
    }
    let p = profile(temp.path());
    let imported = auth::import(&p).unwrap();
    assert_eq!(imported.imported.len(), 1);
    let c = &imported.imported[0];
    assert_eq!(c.name, "default");
    assert!(!c.host_only);
    assert_eq!(c.path, "/api");
    assert!(c.secure);
    assert_eq!(imported.warnings.len(), 1);
    let mut p = p;
    p.container = 2;
    let imported = auth::import(&p).unwrap();
    assert_eq!(imported.imported.len(), 1);
    assert_eq!(imported.imported[0].name, "container");
    assert!(imported.imported[0].host_only);
    assert_eq!(
        db.query_row("SELECT count(*) FROM moz_cookies", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
}
#[test]
fn discovery_uses_profile_manifests_not_account_mixing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("one")).unwrap();
    std::fs::create_dir(temp.path().join("two")).unwrap();
    std::fs::write(temp.path().join("profiles.ini"),"[Profile0]\nName=First\nPath=one\nIsRelative=1\n[Profile1]\nName=Second\nPath=two\nIsRelative=1\n").unwrap();
    let profiles = auth::profiles_at("Firefox", temp.path());
    assert_eq!(profiles.len(), 2);
    assert_ne!(profiles[0].path, profiles[1].path);
    std::fs::write(temp.path().join("Local State"),r#"{"profile":{"info_cache":{"one":{"name":"One"},"two":{"name":"Two"},"../bad":{"name":"Ignore"}}}}"#).unwrap();
    assert_eq!(auth::profiles_at("Chrome", temp.path()).len(), 2);
}

#[test]
fn firefox_first_party_clearance_retains_partition_and_request_scope() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("cookies.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA user_version=15; CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT,isPartitionedAttributeSet INTEGER,isHttpOnly INTEGER)").unwrap();
    for (name, host, path, attrs) in [
        (
            "cf_clearance",
            ".drivethrurpg.com",
            "/",
            "^partitionKey=%28https%2Cdrivethrurpg.com%29",
        ),
        ("osCsid", ".drivethrurpg.com", "/", ""),
        (
            "website_only",
            "www.drivethrurpg.com",
            "/",
            "^partitionKey=%28https%2Cdrivethrurpg.com%29",
        ),
        (
            "account_only",
            ".drivethrurpg.com",
            "/account",
            "^partitionKey=%28https%2Cdrivethrurpg.com%29",
        ),
        (
            "other_site",
            ".drivethrurpg.com",
            "/",
            "^partitionKey=%28https%2Cother.invalid%29",
        ),
        (
            "other_container",
            ".drivethrurpg.com",
            "/",
            "^userContextId=7&partitionKey=%28https%2Cdrivethrurpg.com%29",
        ),
    ] {
        db.execute(
            "INSERT INTO moz_cookies VALUES(?1,'SYNTHETIC_ONLY',?2,?3,1,9999999999,?4,1,1)",
            params![name, host, path, attrs],
        )
        .unwrap();
    }
    let cookies = auth::import(&profile(temp.path())).unwrap();
    assert_eq!(cookies.imported.len(), 4);
    for host in ["www.drivethrurpg.com", "api.drivethrurpg.com"] {
        let url = url::Url::parse(&format!("https://{host}/robots.txt")).unwrap();
        let header = cookies.header(&url).unwrap();
        assert!(header.contains("cf_clearance=SYNTHETIC_ONLY"));
        assert!(header.contains("osCsid=SYNTHETIC_ONLY"));
        assert_eq!(header.contains("website_only="), host.starts_with("www."));
        for excluded in ["account_only", "other_site", "other_container"] {
            assert!(!header.contains(excluded));
        }
        assert!(
            cookies
                .diagnostics(&url)
                .contains("cf_clearance: eligible (matching first-party partition)")
        );
    }
    let clearance = cookies
        .imported
        .iter()
        .find(|c| c.name == "cf_clearance")
        .unwrap();
    assert_eq!(
        clearance.first_party_site.as_deref(),
        Some("https://drivethrurpg.com")
    );
    for url in [
        "http://www.drivethrurpg.com/",
        "https://www.drivethrurpg.com:8443/",
        "https://unapproved.drivethrurpg.com/",
        "https://unrelated.invalid/",
    ] {
        assert!(!clearance.matches(&url::Url::parse(url).unwrap(), 1));
    }
    let mut changed_context = clearance.clone();
    changed_context.first_party_site = Some("https://other.invalid".into());
    assert!(!changed_context.matches(
        &url::Url::parse("https://www.drivethrurpg.com/").unwrap(),
        1
    ));
    assert_eq!(
        db.query_row("SELECT count(*) FROM moz_cookies", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        6
    );
}

#[test]
fn firefox_context_decoding_accepts_same_site_and_rejects_ambiguous_contexts() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("cookies.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT); INSERT INTO moz_cookies VALUES('cf_clearance','SYNTHETIC_SECRET','.drivethrurpg.com','/',1,9999999999,'')").unwrap();
    for attrs in [
        "^partitionKey=%28https%2Cdrivethrurpg.com%29",
        "^partitionKey=(https,drivethrurpg.com,443)",
        "^partitionKey=drivethrurpg.com",
        "^firstPartyDomain=drivethrurpg.com",
        "^firstPartyDomain=%28https%2Cdrivethrurpg.com%29",
        "^partitionKey=(https,drivethrurpg.com)&firstPartyDomain=drivethrurpg.com&privateBrowsingId=0",
    ] {
        db.execute("UPDATE moz_cookies SET originAttributes=?1", [attrs])
            .unwrap();
        let cookies = auth::import(&profile(temp.path())).unwrap();
        assert_eq!(cookies.imported.len(), 1, "{attrs}");
        assert_eq!(
            cookies.imported[0].first_party_site.as_deref(),
            Some("https://drivethrurpg.com")
        );
        assert!(cookies.warnings.is_empty());
    }
    for attrs in [
        "^partitionKey=(https,other.invalid)",
        "^partitionKey=(http,drivethrurpg.com)",
        "^partitionKey=(https,drivethrurpg.com,8443)",
        "^partitionKey=(https,drivethrurpg.com,f)",
        "^partitionKey=(https,drivethrurpg.com,443,f)",
        "^partitionKey=(https,drivethrurpg.com)&firstPartyDomain=other.invalid",
        "^partitionKey=(https,drivethrurpg.com)&privateBrowsingId=1",
        "^partitionKey=(https,drivethrurpg.com)&unknown=UNTRUSTED_METADATA",
        "^userContextId=invalid",
        "^userContextId=4294967296",
        "^userContextId=0&userContextId=1",
        "^partitionKey=(https,drivethrurpg.com)&partitionKey=other.invalid",
        "^partitionKey=(https,drivethrurpg.com)&%70artitionKey=other.invalid",
        "^partitionKey=(https,drivethrurpg.com)&invalid",
        "^partitionKey=%ZZ",
        "partitionKey=(https,drivethrurpg.com)",
    ] {
        db.execute("UPDATE moz_cookies SET originAttributes=?1", [attrs])
            .unwrap();
        let cookies = auth::import(&profile(temp.path())).unwrap();
        assert!(cookies.imported.is_empty(), "{attrs}");
        let warnings = cookies.warnings.join("\n");
        assert!(warnings.contains("cf_clearance omitted"));
        assert!(!warnings.contains("SYNTHETIC_SECRET") && !warnings.contains("UNTRUSTED_METADATA"));
    }
    db.execute("UPDATE moz_cookies SET originAttributes='^userContextId=2&partitionKey=%28https%2Cdrivethrurpg.com%29'", []).unwrap();
    assert!(
        auth::import(&profile(temp.path()))
            .unwrap()
            .imported
            .is_empty()
    );
    let mut selected = profile(temp.path());
    selected.container = 2;
    assert_eq!(auth::import(&selected).unwrap().imported.len(), 1);
}

#[test]
fn firefox_expiry_units_follow_cookie_schema_version() {
    for (version, multiplier) in [(15, 1_i64), (16, 1000), (17, 1000)] {
        let temp = tempfile::tempdir().unwrap();
        let db = rusqlite::Connection::open(temp.path().join("cookies.sqlite")).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        db.execute_batch("CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT)").unwrap();
        let now = chrono::Utc::now().timestamp();
        for (name, expiry) in [("valid", now + 3600), ("expired", now - 3600)] {
            db.execute("INSERT INTO moz_cookies VALUES(?1,'SYNTHETIC_ONLY','.drivethrurpg.com','/',1,?2,'^partitionKey=%28https%2Cdrivethrurpg.com%29')", params![name, expiry * multiplier]).unwrap();
        }
        let cookies = auth::import(&profile(temp.path())).unwrap();
        assert_eq!(
            cookies
                .header(&url::Url::parse("https://www.drivethrurpg.com/robots.txt").unwrap())
                .as_deref(),
            Some("valid=SYNTHETIC_ONLY")
        );
    }
}

#[test]
fn firefox_imports_clearance_httponly_and_empty_values_without_exposing_them() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("cookies.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT,isHttpOnly INTEGER)").unwrap();
    for (name, value, host, path, expiry, attrs) in [
        (
            "cf_clearance",
            "CLEARANCE_SECRET",
            ".drivethrurpg.com",
            "/",
            9999999999_i64,
            "",
        ),
        (
            "__cf_bm",
            "BOT_SECRET",
            "www.drivethrurpg.com",
            "/",
            9999999999,
            "",
        ),
        ("empty", "", ".drivethrurpg.com", "/", 9999999999, ""),
        ("expired", "EXPIRED_SECRET", ".drivethrurpg.com", "/", 1, ""),
        (
            "path_only",
            "PATH_SECRET",
            ".drivethrurpg.com",
            "/account",
            9999999999,
            "",
        ),
        (
            "cf_clearance",
            "PARTITION_SECRET",
            ".drivethrurpg.com",
            "/",
            9999999999,
            "^partitionKey=%28https%2Cexample.invalid%29",
        ),
        (
            "cf_clearance",
            "CONTAINER_SECRET",
            ".drivethrurpg.com",
            "/",
            9999999999,
            "^userContextId=7",
        ),
    ] {
        db.execute(
            "INSERT INTO moz_cookies VALUES(?1,?2,?3,?4,1,?5,?6,1)",
            params![name, value, host, path, expiry, attrs],
        )
        .unwrap();
    }
    let mut cookies = auth::import(&profile(temp.path())).unwrap();
    let website = url::Url::parse("https://www.drivethrurpg.com/robots.txt").unwrap();
    let api = url::Url::parse("https://api.drivethrurpg.com/robots.txt").unwrap();
    assert_eq!(cookies.imported.len(), 5);
    let header = cookies.header(&website).unwrap();
    assert!(header.contains("cf_clearance=CLEARANCE_SECRET"));
    assert!(header.contains("__cf_bm=BOT_SECRET"));
    assert!(header.contains("empty="));
    for excluded in ["EXPIRED", "PATH", "PARTITION", "CONTAINER"] {
        assert!(!header.contains(excluded));
    }
    assert!(!cookies.header(&api).unwrap().contains("__cf_bm"));
    assert_eq!(
        cookies.diagnostics(&website),
        "Eligible cookies: 3; cf_clearance: eligible"
    );
    assert_eq!(
        cookies.diagnostics(&api),
        "Eligible cookies: 2; cf_clearance: eligible"
    );
    assert!(
        cookies
            .warnings
            .iter()
            .any(|w| w.contains("cf_clearance omitted"))
    );
    assert!(!cookies.warnings.join("\n").contains("SECRET"));
    cookies
        .imported
        .iter_mut()
        .filter(|c| c.name == "cf_clearance")
        .for_each(|c| c.expires = Some(1));
    assert!(cookies.diagnostics(&website).contains("expired or outside"));
    cookies.raw.push((
        "https://www.drivethrurpg.com/".into(),
        "session=PASTED_SECRET".into(),
    ));
    assert_eq!(
        cookies.diagnostics(&website),
        "Eligible cookies: 1; cf_clearance: not in pasted header"
    );
    cookies.raw[0].1 = "cf_clearance=PASTED_SECRET".into();
    assert_eq!(
        cookies.diagnostics(&website),
        "Eligible cookies: 1; cf_clearance: eligible (pasted; expiry unknown)"
    );
}
#[test]
fn inaccessible_profiles_offer_paste() {
    let temp = tempfile::tempdir().unwrap();
    let error = auth::import(&profile(temp.path()))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("manual"));
}
#[test]
fn chromium_plain_cookies_partitioned_and_unrelated_rows() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("Cookies")).unwrap();
    db.execute_batch("CREATE TABLE cookies(name TEXT,value TEXT,host_key TEXT,path TEXT,is_secure INTEGER,expires_utc INTEGER,top_frame_site_key TEXT,encrypted_value BLOB); INSERT INTO cookies VALUES('session','synthetic','www.drivethrurpg.com','/',1,14000000000000000,'',X''); INSERT INTO cookies VALUES('partition','synthetic','.drivethrurpg.com','/',1,14000000000000000,'https://example.invalid',X''); INSERT INTO cookies VALUES('unrelated','','unrelated.invalid','/',1,14000000000000000,'',X'76323000')").unwrap();
    let mut p = profile(temp.path());
    p.browser = "Chrome".into();
    let imported = auth::import(&p).unwrap();
    assert_eq!(imported.imported.len(), 1);
    assert_eq!(imported.warnings.len(), 1);
}
#[cfg(windows)]
#[test]
fn windows_app_bound_cookie_graceful_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("Cookies")).unwrap();
    db.execute_batch("CREATE TABLE cookies(name TEXT,value TEXT,host_key TEXT,path TEXT,is_secure INTEGER,expires_utc INTEGER,top_frame_site_key TEXT,encrypted_value BLOB); INSERT INTO cookies VALUES('session','','www.drivethrurpg.com','/',1,14000000000000000,'',X'76323000')").unwrap();
    let mut p = profile(temp.path());
    p.browser = "Edge".into();
    let e = auth::import(&p).err().unwrap().to_string();
    assert!(e.contains("app-bound"));
    assert!(e.contains("manual paste"));
}
