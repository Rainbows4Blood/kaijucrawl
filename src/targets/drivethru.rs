use crate::domain::{Error, Failure, FailureKind, ProductId, Result};
use url::Url;
mod access;
pub const KEY: &str = "drivethru";
pub const WEBSITE: &str = "https://www.drivethrurpg.com/";
pub const API: &str = "https://api.drivethrurpg.com/";
/// Fixed request compatibility profile for User-Agent-dependent HTTP 502s.
/// This fixed profile does not change our identity for robots policy or pacing.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:158.0) Gecko/20100101 Firefox/158.0";
/// Same-site context for this adapter's direct website and API requests.
pub const COOKIE_SITE: &str = "https://drivethrurpg.com";
pub fn cookie_site(url: &Url) -> Option<&'static str> {
    (approved(url, false) || approved(url, true)).then_some(COOKIE_SITE)
}
pub const ROOT: &str = "https://www.drivethrurpg.com/sitemap.xml";
pub const IMAGE: &str = "https://d1vzi28wh99zvq.cloudfront.net/";
pub const PREVIEW: &str = "https://watermark.drivethrurpg.com/";
pub const FRONTEND_CURRENCY: &str = "USD";
pub const COOKIE_DOMAINS: [&str; 6] = [
    "drivethrurpg.com",
    ".drivethrurpg.com",
    "www.drivethrurpg.com",
    ".www.drivethrurpg.com",
    "api.drivethrurpg.com",
    ".api.drivethrurpg.com",
];
pub fn canonical(id: &str) -> String {
    format!("{WEBSITE}en/product/{id}")
}
pub fn detail(id: &str) -> String {
    format!("{API}api/vBeta/products/{id}?groupId=1&siteId=10")
}
pub fn approved(url: &Url, api: bool) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && match url.host_str() {
            Some("api.drivethrurpg.com") => api,
            Some("drivethrurpg.com" | "www.drivethrurpg.com") => !api,
            _ => false,
        }
}
pub fn product_url(raw: &str) -> Option<ProductId> {
    let u = Url::parse(raw).ok()?;
    if !approved(&u, false) {
        return None;
    }
    if u.path() == "/product_info.php" {
        let ids: Vec<_> = u
            .query_pairs()
            .filter(|(k, _)| k == "products_id")
            .collect();
        return if ids.len() == 1 {
            ProductId::new(&ids[0].1).ok()
        } else {
            None
        };
    }
    let p: Vec<_> = u.path().trim_matches('/').split('/').collect();
    let i = if p.first() == Some(&"product") {
        1
    } else if p.len() >= 3
        && p[1] == "product"
        && p[0].len() <= 5
        && p[0].chars().all(|c| c.is_ascii_alphabetic() || c == '-')
    {
        2
    } else {
        return None;
    };
    ProductId::new(p.get(i)?).ok()
}
pub fn sitemap_url(raw: &str) -> Option<String> {
    let mut u = Url::parse(raw).ok()?;
    if !approved(&u, false) {
        return None;
    }
    let path = u.path().to_ascii_lowercase();
    if !(path.ends_with(".xml") || path.ends_with(".xml.gz") || path.ends_with("/sitemap")) {
        return None;
    }
    u.set_fragment(None);
    Some(u.into())
}
pub fn redirect(original: &Url, next: &Url, product: Option<&str>) -> bool {
    if let Some(id) = product {
        approved(next, true)
            && original.origin() == next.origin()
            && next.path() == format!("/api/vBeta/products/{id}")
            && next.query_pairs().count() == 2
            && next.query_pairs().any(|(k, v)| k == "siteId" && v == "10")
            && next.query_pairs().any(|(k, v)| k == "groupId" && v == "1")
    } else {
        approved(next, original.host_str() == Some("api.drivethrurpg.com"))
            && original.origin() == next.origin()
    }
}
pub fn test_id(value: &str) -> Result<ProductId> {
    ProductId::new(value).or_else(|_| {
        product_url(value).ok_or_else(|| {
            Error::Config("Use a positive product ID or an approved product URL".into())
        })
    })
}
pub fn classify(status: u16, body: &[u8]) -> Option<Failure> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        let restricted = [
            "ageGate",
            "ageGateRequired",
            "requiresAgeVerification",
            "adultContentRestricted",
        ]
        .iter()
        .any(|k| value[k] == true);
        let auth = ["loginRequired", "authenticationRequired", "accessDenied"]
            .iter()
            .any(|k| value[k] == true);
        if restricted || auth {
            let mut f = Failure::new(
                if restricted {
                    FailureKind::Restricted
                } else {
                    FailureKind::Authentication
                },
                if restricted {
                    "Explicit individual product restriction"
                } else {
                    "Explicit authentication denial"
                },
            );
            f.status = Some(status);
            return Some(f);
        }
    }
    if (200..300).contains(&status)
        && body
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .is_some_and(|b| *b == b'{' || *b == b'[')
    {
        return None;
    }
    let (kind, reason) = if status == 401 {
        (
            FailureKind::Authentication,
            "HTTP authentication denial; review session and Test Access",
        )
    } else if let Some(gate) = access::body_gate(body) {
        gate
    } else if status == 403 {
        (
            FailureKind::Authentication,
            "HTTP access denial; review session and Test Access",
        )
    } else if status == 404 || status == 410 {
        (FailureKind::Missing, "Resource missing")
    } else if [408, 425, 429].contains(&status) || status >= 500 {
        (FailureKind::Transient, "Temporary HTTP failure")
    } else if !(200..300).contains(&status) {
        (FailureKind::Schema, "Unexpected HTTP response")
    } else {
        return None;
    };
    let mut f = Failure::new(kind, reason);
    f.status = Some(status);
    if status == 429 {
        f.retry_after_secs = 60;
    }
    Some(f)
}

/// Product API denials are product outcomes unless there is affirmative
/// evidence of session failure or a challenge. Robots/website responses use
/// the general classifier above; they cannot silently permit a crawl.
pub fn classify_product_api(status: u16, body: &[u8]) -> Option<Failure> {
    if status != 403 {
        return classify(status, body);
    }
    let mut gate = None;
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        if [
            "loginRequired",
            "authenticationRequired",
            "sessionExpired",
            "invalidSession",
        ]
        .iter()
        .any(|key| value[key] == true)
        {
            gate = Some((
                FailureKind::Authentication,
                "Explicit API authentication/session failure; review session and Test Access",
            ));
        } else {
            for key in ["message", "error", "detail"] {
                if let Some(message) = value[key].as_str()
                    && let Some(found) = access::product_forbidden_gate(message.as_bytes())
                {
                    gate = Some(found);
                    break;
                }
            }
        }
    }
    let (kind, reason) = gate
        .or_else(|| access::product_forbidden_gate(body))
        .unwrap_or((
            FailureKind::Restricted,
            "Product API returned HTTP 403; product unavailable or restricted, eligible next run",
        ));
    let mut failure = Failure::new(kind, reason);
    failure.status = Some(status);
    Some(failure)
}
