use kaijucrawl::{
    auth::{self, Cookie, Cookies},
    config::{Config, RuntimeSettings, Secrets},
    domain::*,
    http::{Limiter, Policy, retry_after},
    parser::extract,
    storage::archive,
    targets::{drivethru as d, sitemap},
    telemetry,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
fn parsed(v: Value) -> Result<Product> {
    extract::parse(&serde_json::to_vec(&v).unwrap(), "42", &d::detail("42"))
}
fn good() -> Value {
    json!({"id":42,"name":"Title","description":"Full description"})
}

#[test]
fn identity_is_decimal_and_target_scoped() {
    assert_eq!(
        ProductId::new("000184467440737095516160000")
            .unwrap()
            .as_str(),
        "184467440737095516160000"
    );
    for x in ["0", "-2", "2.1", "1e3", "true", ""] {
        assert!(ProductId::new(x).is_err());
    }
}
#[test]
fn aliases_and_host_security() {
    for u in [
        "https://www.drivethrurpg.com/en/product/00042/slug?x=1#f",
        "https://drivethrurpg.com/product/42",
        "https://www.drivethrurpg.com/product_info.php?products_id=42",
    ] {
        assert_eq!(d::product_url(u).unwrap().as_str(), "42");
    }
    for u in [
        "https://evil.invalid/product/42",
        "https://user@www.drivethrurpg.com/product/42",
        "https://www.drivethrurpg.com:444/product/42",
        "https://www.drivethrurpg.com/download/42",
        "http://www.drivethrurpg.com/product/42",
    ] {
        assert!(d::product_url(u).is_none());
    }
}
#[test]
fn redirect_cannot_change_product_or_site() {
    let original = url::Url::parse(&d::detail("42")).unwrap();
    for next in [
        d::detail("43"),
        d::detail("42").replace("siteId=10", "siteId=11"),
        "https://evil.invalid/".into(),
        d::canonical("42"),
    ] {
        assert!(!d::redirect(
            &original,
            &url::Url::parse(&next).unwrap(),
            Some("42")
        ));
    }
    assert!(d::redirect(&original, &original, Some("42")));
}
#[test]
fn completeness_never_fabricates_full_description() {
    assert!(parsed(good()).is_ok());
    for p in [
        json!({"id":42,"name":"Title","shortDescription":"only short"}),
        json!({"id":42,"description":"full"}),
        json!({"id":42,"name":"Title","description":"<script>fake</script>"}),
    ] {
        let Err(Error::Job(f)) = parsed(p) else {
            panic!("should fail")
        };
        assert_eq!(f.kind, FailureKind::Incomplete);
        assert!(!f.missing_fields.is_empty());
    }
}
#[test]
fn direct_envelopes_only_and_ambiguity_rejected() {
    for p in [
        json!({"product":good()}),
        json!({"data":good()}),
        json!({"data":[good()]}),
        good(),
    ] {
        assert!(parsed(p).is_ok());
    }
    for p in [
        json!({"data":[good(),good()]}),
        json!({"recommendations":[good()]}),
        json!({"id":43,"recommendations":[good()]}),
        json!({"id":42,"productId":43,"title":"T","description":"D"}),
        json!({"id":42}),
    ] {
        assert!(parsed(p).is_err());
    }
}
#[test]
fn included_relationships_match_type_and_id() {
    let p=parsed(json!({"data":{"type":"products","id":"42","attributes":{"title":"T","description":"D"},"relationships":{"publisher":{"data":{"type":"publishers","id":"7"}}}},"included":[{"type":"creators","id":"7","attributes":{"name":"Wrong"}},{"type":"publishers","id":"7","attributes":{"name":"Right"}}]})).unwrap();
    assert_eq!(p.fields["publisher"], "Right");
    assert_eq!(p.fields["publisher_id"], "7");
    assert!(p.fields["raw_metadata"]["api_response"].is_object());
}
#[test]
fn rejects_gates_but_not_product_topics() {
    for k in [
        "ageGate",
        "ageGateRequired",
        "requiresAgeVerification",
        "adultContentRestricted",
        "authenticationRequired",
        "accessDenied",
    ] {
        let mut v = good();
        v[k] = json!(true);
        assert!(parsed(v).is_err());
    }
    let mut p = good();
    p["description"] =
        json!("A story about captcha and age verification required by fictional robots");
    assert!(parsed(p.clone()).is_ok());
    assert!(d::classify(200, &serde_json::to_vec(&p).unwrap()).is_none());
}
#[test]
fn ordinary_html_security_scripts_and_hidden_dialogs_are_not_access_denials() {
    for body in [
        r#"<!doctype html><html><head><title>DriveThruRPG</title><script src="/cdn-cgi/challenge-platform/scripts/jsd/main.js"></script></head><body><h1>Browse games</h1><script>const translations = {"error":"session expired", "login":"login required", "gate":"age verification required"}; window['cf-chl-widget'] = 'captcha';</script></body></html>"#,
        r#"<html><title>Store</title><body><h1>Books</h1><form><div class="g-recaptcha"></div></form><div hidden><h2>Login required</h2></div><div aria-hidden="true"><h2>Session expired</h2></div><div style="display: none !important"><h2>Content restricted</h2></div><div style="visibility: hidden"><h1>Access denied</h1></div><template><h1>Verify you are human</h1></template><noscript><h1>Login required</h1></noscript></body></html>"#,
        r#"<html><body><h1>Games about captcha challenges</h1><p>A story about robots, session expired messages and age verification required by a fictional gatekeeper.</p><!-- <title>Just a moment...</title> --></body></html>"#,
        r#"<html><body><h1>Welcome</h1><script src="/captcha.js"></script><div id="cf-chl-example" data-message="login required"></div></body></html>"#,
        "User-agent: *\nDisallow: /captcha\n# login required on account pages",
        "<urlset><url><loc>https://www.drivethrurpg.com/en/product/42/captcha-challenge</loc></url></urlset>",
    ] {
        assert!(d::classify(200, body.as_bytes()).is_none(), "{body}");
    }
}

#[test]
fn explicit_html_and_text_gate_evidence_is_still_rejected() {
    for (body, evidence) in [
        (
            "<html><title>Just a moment...</title><body>Checking</body></html>",
            "page title",
        ),
        (
            "<html><title>Attention Required! | Cloudflare</title></html>",
            "page title",
        ),
        (
            "<html><body><h1> VERIFY <span>YOU ARE</span> HUMAN! </h1></body></html>",
            "page heading",
        ),
        ("<html><h2>Login&nbsp;required</h2></html>", "page heading"),
        (
            "<html><body><div>Session expired.</div></body></html>",
            "page message",
        ),
        (
            "<html><body><form id='challenge-form' action='/cdn-cgi/challenge-platform/verify'></form></body></html>",
            "challenge form",
        ),
        ("  Invalid session\n", "text response"),
    ] {
        let failure = d::classify(200, body.as_bytes()).expect("gate must be rejected");
        assert_eq!(failure.kind, FailureKind::Authentication);
        assert_eq!(failure.status, Some(200));
        assert!(failure.reason.contains(evidence));
        assert!(!failure.reason.contains(body));
    }
    for body in [
        "<html><h1>Age verification required</h1></html>",
        "Adult content is disabled",
    ] {
        assert_eq!(
            d::classify(200, body.as_bytes()).unwrap().kind,
            FailureKind::Restricted
        );
    }
    for status in [401, 403] {
        assert_eq!(
            d::classify(status, b"<html><h1>Store</h1></html>")
                .unwrap()
                .kind,
            FailureKind::Authentication
        );
    }
    for (status, kind) in [
        (404, FailureKind::Missing),
        (410, FailureKind::Missing),
        (429, FailureKind::Transient),
        (500, FailureKind::Transient),
    ] {
        assert_eq!(d::classify(status, b"captcha.js").unwrap().kind, kind);
    }
}
#[test]
fn full_metadata_fidelity() {
    let body = include_bytes!("fixtures/full_product.json");
    let p = extract::parse(body, "42", &d::detail("42")).unwrap();
    let r = serde_json::to_value(&p).unwrap();
    assert_eq!(p.title, "Synthetic Kaiju Handbook");
    assert!(p.description_text.contains("Play together."));
    assert!(!p.description_text.contains("secret"));
    assert_eq!(r["price"], "0");
    assert_eq!(r["base_price"], "12345678901234567890.12345678901234567890");
    assert_eq!(r["currency"], "EUR");
    assert_eq!(r["rating_count"], 4);
    assert_eq!(r["review_count"], 2);
    assert_eq!(r["authors"], json!(["A. Writer"]));
    assert_eq!(r["filter_groups"]["genre"], json!(["Fantasy"]));
    assert_eq!(r["language"], "English, German");
    assert_eq!(r["offers"][0]["raw"]["nested"], json!({"keep":[1,2]}));
    assert_eq!(r["files"][0]["size_bytes"], 1048576);
    assert_eq!(r["previews"].as_array().unwrap().len(), 7);
    assert_eq!(r["images"].as_array().unwrap().len(), 9);
    assert_eq!(r["related_products"].as_array().unwrap().len(), 4);
    assert_eq!(r["flags"].as_object().unwrap().len(), 19);
    assert_eq!(
        r["raw_metadata"]["public_api"]["unknownNested"]["exact"].to_string(),
        "0.123456789012345678901234567890"
    );
    for key in [
        "slug",
        "product_url",
        "source_url",
        "source_kind",
        "url",
        "api_url",
        "publisher",
        "publisher_id",
        "publisher_metadata",
        "description_html",
        "short_description",
        "authors",
        "artists",
        "editors",
        "contributors",
        "credits",
        "page_count",
        "isbn",
        "sku",
        "edition",
        "price",
        "base_price",
        "special_price",
        "on_sale",
        "currency",
        "currency_source",
        "lowest_prices",
        "offers",
        "formats",
        "dates",
        "date_created",
        "date_modified",
        "date_available",
        "file_last_modified",
        "categories",
        "filters",
        "primary_filters",
        "filter_groups",
        "genres",
        "themes",
        "rule_systems",
        "product_types",
        "languages",
        "language",
        "rating",
        "rating_count",
        "review_count",
        "review_ratings",
        "reviews",
        "ranking",
        "files",
        "file_size_reported",
        "images",
        "cover_image",
        "previews",
        "description_links",
        "is_bundle",
        "bundle_id",
        "related_products",
        "flags",
        "parser_version",
        "record_schema_version",
        "extraction_status",
        "missing_fields",
        "raw_metadata",
    ] {
        assert!(r.get(key).is_some(), "missing field {key}");
    }
}
#[test]
fn decimals_integers_and_currency_rules() {
    for invalid in [json!(true), json!("NaN"), json!("Infinity")] {
        assert!(extract::decimal(&invalid).is_null());
    }
    let mut p = good();
    p["price"] = json!(0);
    p["pageCount"] = json!(1.5);
    p["rating"] = json!(0);
    p["ratingCount"] = json!(0);
    let p = parsed(p).unwrap();
    assert_eq!(p.fields["price"], "0");
    assert_eq!(p.fields["currency_source"], "frontend_base_currency");
    assert!(p.fields["page_count"].is_null());
    assert!(p.fields["rating"].is_null());
}
#[test]
fn description_cleanup_preserves_boundaries() {
    let (h, t) = extract::clean(
        "<p>to<b>get</b>her</p><p>A<br>B</p><table><tr><td>x</td><td>y</td></tr></table><img onerror='x' src='javascript:bad'><iframe>bad</iframe><a href='data:x' srcdoc='x'>ok</a>",
    );
    assert!(t.starts_with("together\n\nA\nB"));
    for s in ["onerror", "javascript:", "srcdoc", "iframe", "data:"] {
        assert!(!h.contains(s));
    }
}
#[test]
fn sitemap_streaming_ignores_extensions_and_reads_gzip() {
    let xml=b"<urlset xmlns:image='image'><url><loc>https://www.drivethrurpg.com/en/product/42?a=1&amp;b=2</loc><lastmod>old</lastmod><image:image><image:loc>https://evil.invalid/</image:loc></image:image></url></urlset>";
    for body in [xml.to_vec(), archive::compress(xml).unwrap()] {
        let mut entries = vec![];
        sitemap::parse(body.as_slice(), |e| {
            entries.extend(e);
            Ok(())
        })
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].location.ends_with("?a=1&b=2"));
        assert_eq!(entries[0].lastmod.as_deref(), Some("old"));
    }
}
#[test]
fn sitemap_malformed_and_dtd_rejected() {
    for xml in [
        "<bad/>",
        "<urlset><url><loc>x</loc></url>",
        "<!DOCTYPE x SYSTEM 'https://evil.invalid'><urlset/>",
        "<urlset><url><loc>x</loc></urlset>",
    ] {
        assert!(sitemap::parse(xml.as_bytes(), |_| Ok(())).is_err());
    }
}
#[test]
fn sitemap_batches_bounded() {
    let xml = format!(
        "<urlset>{}</urlset>",
        "<url><loc>https://www.drivethrurpg.com/product/42</loc></url>".repeat(2001)
    );
    let mut counts = vec![];
    sitemap::parse(xml.as_bytes(), |v| {
        counts.push(v.len());
        Ok(())
    })
    .unwrap();
    assert_eq!(counts, vec![1000, 1000, 1]);
}
#[test]
fn sitemap_namespace_prefixes_keep_extensions_separate() {
    let xml = br#"<sm:urlset xmlns:sm="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1"><sm:url><sm:loc>https://www.drivethrurpg.com/product/42</sm:loc><image:loc>https://evil.invalid/other</image:loc><sm:lastmod>2026-10-26</sm:lastmod></sm:url></sm:urlset>"#;
    let mut entries = vec![];
    sitemap::parse(xml.as_slice(), |v| {
        entries.extend(v);
        Ok(())
    })
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].location,
        "https://www.drivethrurpg.com/product/42"
    );
    assert_eq!(entries[0].lastmod.as_deref(), Some("2026-10-26"));
}
#[test]
fn cookie_paste_rejects_header_injection() {
    assert_eq!(
        auth::paste("Cookie: session=synthetic; x=1").unwrap(),
        "session=synthetic; x=1"
    );
    for s in [
        "a=b\r\nAuthorization: x",
        "Authorization: Bearer x",
        "a=b\nx=y",
        "a=b; broken",
        "=x",
        "a=b\t",
        "a=b\0",
        "a=truncated…",
    ] {
        assert!(auth::paste(s).is_err());
    }
}
#[test]
fn cookie_paste_preserves_browser_json_values_and_order() {
    let raw = r#"cf_clearance=SYNTHETIC; siteSettings={"language": "en", "currency":"USD"}; session=abc==; empty="#;
    assert_eq!(auth::paste(raw).unwrap(), raw);
    assert_eq!(auth::paste(&format!("Cookie: {raw}")).unwrap(), raw);
    let mut secrets = Secrets::default();
    secrets.website_cookie = raw.into();
    assert_eq!(
        auth::Cookies::manual(&secrets)
            .unwrap()
            .header(&url::Url::parse(d::WEBSITE).unwrap())
            .as_deref(),
        Some(raw)
    );
}
#[test]
fn cookie_scopes_path_expiry_secure() {
    let mut c = Cookie {
        first_party_site: None,
        created_micros: None,
        name: "s".into(),
        value: "synthetic".into(),
        domain: "drivethrurpg.com".into(),
        host_only: true,
        path: "/api".into(),
        secure: true,
        expires: Some(100),
    };
    let u = url::Url::parse("https://api.drivethrurpg.com/api/v1").unwrap();
    assert!(!c.matches(&u, 1));
    c.host_only = false;
    assert!(c.matches(&u, 1));
    assert!(!c.matches(&u, 100));
    assert!(!c.matches(
        &url::Url::parse("https://api.drivethrurpg.com/apix").unwrap(),
        1
    ));
    assert!(!c.matches(
        &url::Url::parse("http://api.drivethrurpg.com/api").unwrap(),
        1
    ));
    assert!(!c.matches(
        &url::Url::parse("https://evildrivethrurpg.com/api").unwrap(),
        1
    ));
}
#[test]
fn manual_headers_never_cross_host() {
    let mut s = Secrets::default();
    s.website_cookie = "s=synthetic".into();
    let cookies = Cookies::manual(&s).unwrap();
    assert!(
        cookies
            .header(&url::Url::parse(d::WEBSITE).unwrap())
            .is_some()
    );
    assert!(cookies.header(&url::Url::parse(d::API).unwrap()).is_none());
    assert!(
        cookies
            .header(&url::Url::parse("https://drivethrurpg.com/").unwrap())
            .is_none()
    );
}
#[test]
fn robot_specific_groups_delay_and_request_rate() {
    let p=Policy::parse("User-agent: *\nDisallow: /private\nCrawl-delay: 99\nUser-agent: KaijuCrawl\nDisallow: /blocked\nAllow: /blocked/public\nCrawl-delay: 0.2\nRequest-rate: 2/3\nSitemap: https://evil.invalid/sitemap.xml".into(),"KaijuCrawl/0.1").unwrap();
    assert_eq!(p.interval, Duration::from_millis(1500));
    assert!(p.allows("KaijuCrawl/0.1", "https://example.invalid/private"));
    assert!(!p.allows("KaijuCrawl/0.1", "https://example.invalid/blocked"));
    assert!(p.allows("KaijuCrawl/0.1", "https://example.invalid/blocked/public"));
    assert!(p.sitemaps.is_empty());
}
#[tokio::test]
async fn independent_target_limits_even_for_shared_host() {
    let a = Limiter::new(2);
    let b = Limiter::new(2);
    let c = CancellationToken::new();
    a.cooldown(1).await;
    let now = Instant::now();
    b.pace(&c).await.unwrap();
    assert!(now.elapsed() < Duration::from_millis(200));
    let c2 = c.clone();
    let pending = tokio::spawn(async move { a.pace(&c2).await });
    c.cancel();
    assert!(pending.await.unwrap().is_err());
}
#[test]
fn configuration_no_secret_serialization_and_settings_bounds() {
    let c = Config::default();
    let text = toml::to_string(&c).unwrap();
    for forbidden in [
        "password",
        "database_url",
        "cookie",
        "secret_key",
        "advanced",
        "runtime",
        "workers",
        "interval_ms",
    ] {
        assert!(!text.contains(forbidden));
    }
    let mut a = RuntimeSettings {
        workers: 0,
        ..Default::default()
    };
    assert!(a.validate().is_err());
    a = RuntimeSettings::default();
    a.total_secs = 1;
    assert!(a.validate().is_err());
    a = RuntimeSettings::default();
    a.flush_ms = 0;
    assert!(a.validate().is_ok());
    assert_eq!(retry_after("120"), 120);
}
#[test]
fn terminal_sequences_removed() {
    assert_eq!(
        telemetry::display("hello\x1b[31mred\x1b[0m\x1b]0;title\x07\x00"),
        "hellored"
    );
}
#[tokio::test]
async fn immutable_archive_roundtrip_and_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let paths = kaijucrawl::paths::LocalPaths::at(temp.path());
    let a = archive::Archives::new(&paths);
    a.test().await.unwrap();
    let body = serde_json::to_vec(&good()).unwrap();
    let (archive, gz) = a
        .prepare(uuid::Uuid::new_v4(), d::KEY, "42", &body)
        .unwrap();
    assert!(a.read(&archive).await.unwrap().is_none());
    a.write_verified(&archive, gz.clone()).await.unwrap();
    a.write_verified(&archive, gz).await.unwrap();
    assert_eq!(a.read(&archive).await.unwrap().unwrap(), body);
    for key in [
        "../outside.json.gz",
        "/outside.json.gz",
        "C:/outside.json.gz",
        "folder\\outside.json.gz",
    ] {
        let mut invalid = archive.clone();
        invalid.key = key.into();
        assert!(a.read(&invalid).await.is_err());
    }
    std::fs::write(paths.archives().join(&archive.key), b"corrupt").unwrap();
    assert!(a.read(&archive).await.is_err());
    assert!(
        a.write_verified(&archive, archive::compress(&body).unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(paths.archives().join(&archive.key)).unwrap(),
        b"corrupt"
    );
}
