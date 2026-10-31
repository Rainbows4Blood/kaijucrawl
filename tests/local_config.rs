use kaijucrawl::{
    config::{Config, Secrets},
    paths::LocalPaths,
};

#[test]
fn fixed_home_paths_and_remaining_configuration_migration() {
    let current = LocalPaths::current().unwrap();
    assert_eq!(
        current.root(),
        directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .join(".kaijucrawl")
    );
    assert_eq!(current.database().file_name().unwrap(), "catalogue.sqlite3");
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path().join("new-home"));
    let legacy = temp.path().join("legacy.toml");
    let old = "selected = false\nbucket = 'discarded'\nendpoint = 'https://unused.invalid'\n[advanced]\nworkers = 0\ninterval_ms = 86400000\n[runtime]\nworkers = 999999\nuser_agent = 'IGNORED'\n[browser]\nbrowser = 'Firefox'\nname = 'Synthetic'\npath = 'synthetic-profile'\ncontainer = 2\n";
    std::fs::write(&legacy, old).unwrap();
    let c = Config::load_at(&paths, Some(&legacy)).unwrap();
    assert!(!c.selected);
    let cpus = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    assert_eq!(c.runtime.workers, cpus);
    assert_eq!(c.runtime.parsers, cpus);
    assert_eq!(
        c.runtime.robots_agent,
        concat!("KaijuCrawl/", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        c.runtime.user_agent,
        kaijucrawl::targets::drivethru::USER_AGENT
    );
    assert_eq!(c.browser.as_ref().unwrap().container, 2);
    let text = std::fs::read_to_string(paths.settings()).unwrap();
    for removed in [
        "bucket",
        "endpoint",
        "database",
        "prefix",
        "region",
        "password",
        "advanced",
        "runtime",
        "workers",
        "interval_ms",
    ] {
        assert!(!text.contains(removed));
    }
    assert_eq!(std::fs::read_to_string(&legacy).unwrap(), old);
    std::fs::write(&legacy, "invalid TOML").unwrap();
    assert_eq!(
        Config::load_at(&paths, Some(&legacy))
            .unwrap()
            .runtime
            .workers,
        cpus
    );
    // Existing local files also ignore removed options, not just legacy migration.
    std::fs::write(paths.settings(), old).unwrap();
    let current = Config::load_at(&paths, None).unwrap();
    assert_eq!(current.runtime.workers, cpus);
    assert_eq!(current.runtime.parsers, cpus);
    current.save_at(&paths).unwrap();
    assert!(
        !std::fs::read_to_string(paths.settings())
            .unwrap()
            .contains("workers")
    );
}

#[test]
fn local_secrets_are_separate_atomic_and_clearable() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path().join("data"));
    Config::default().save_at(&paths).unwrap();
    let s: Secrets = serde_json::from_value(serde_json::json!({
        "website_cookie": "session=SYNTHETIC_ONLY",
        "database_url": "unused legacy database setting",
        "access_key": "unused legacy access key",
        "secret_key": "unused legacy secret key"
    }))
    .unwrap();
    s.persist_at(&paths).unwrap();
    let saved = std::fs::read_to_string(paths.secrets()).unwrap();
    assert!(!saved.contains("legacy") && !saved.contains("database_url"));
    assert_eq!(
        Secrets::load_at(&paths).unwrap().website_cookie,
        s.website_cookie
    );
    assert!(
        !std::fs::read_to_string(paths.settings())
            .unwrap()
            .contains("SYNTHETIC_ONLY")
    );
    Secrets::default().persist_at(&paths).unwrap();
    assert!(Secrets::load_at(&paths).unwrap().website_cookie.is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(paths.secrets())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(paths.root())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::write(paths.secrets(), b"corrupt").unwrap();
    assert!(Secrets::load_at(&paths).is_err());
}
