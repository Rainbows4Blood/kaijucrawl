use crate::domain::FailureKind;
use scraper::{ElementRef, Html, node::Node};

// These are gate messages, not keywords to search for in arbitrary page source.
fn message_kind(text: &str, product_forbidden: bool) -> Option<FailureKind> {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let label = normalized.trim_matches(|c: char| c.is_whitespace() || ".!…:;".contains(c));
    if product_forbidden && label == "access denied" {
        // An ordinary product 403 is not evidence that the whole session failed.
        // Continue inspecting HTML so an explicit challenge still takes priority.
        return None;
    }
    if [
        "just a moment",
        "attention required! | cloudflare",
        "verify you are human",
        "verifying you are human",
        "checking your browser",
        "captcha challenge",
        "authentication required",
        "login required",
        "sign in required",
        "access denied",
        "session expired",
        "your session has expired",
        "invalid session",
    ]
    .contains(&label)
    {
        Some(FailureKind::Authentication)
    } else if !product_forbidden
        && [
            "age verification required",
            "content restricted",
            "product restricted",
            "adult content is disabled",
        ]
        .contains(&label)
    {
        Some(FailureKind::Restricted)
    } else {
        None
    }
}

fn excluded(element: ElementRef<'_>) -> bool {
    let value = element.value();
    matches!(value.name(), "script" | "style" | "template" | "noscript")
        || value.attr("hidden").is_some()
        || value.attr("inert").is_some()
        || value
            .attr("aria-hidden")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || value.attr("style").is_some_and(|style| {
            style
                .split(';')
                .filter_map(|rule| rule.split_once(':'))
                .any(|(name, value)| {
                    let value = value.split('!').next().unwrap_or(value).trim();
                    (name.trim().eq_ignore_ascii_case("display")
                        && value.eq_ignore_ascii_case("none"))
                        || (name.trim().eq_ignore_ascii_case("visibility")
                            && value.eq_ignore_ascii_case("hidden"))
                })
        })
}

fn gate(kind: FailureKind, evidence: &'static str) -> (FailureKind, &'static str) {
    if kind == FailureKind::Restricted {
        (
            kind,
            "Explicit individual product restriction in page message",
        )
    } else {
        (kind, evidence)
    }
}

pub(super) fn body_gate(body: &[u8]) -> Option<(FailureKind, &'static str)> {
    classify_body_gate(body, false)
}

pub(super) fn product_forbidden_gate(body: &[u8]) -> Option<(FailureKind, &'static str)> {
    classify_body_gate(body, true)
}

fn classify_body_gate(body: &[u8], product_forbidden: bool) -> Option<(FailureKind, &'static str)> {
    let source = String::from_utf8_lossy(&body[..body.len().min(65536)]);
    if !source
        .trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t'])
        .starts_with('<')
    {
        // A short, complete plain-text denial is evidence; words in robots rules,
        // XML, or prose are not. Do not mistake a truncated prefix for a message.
        return (body.len() <= 512).then(|| message_kind(&source, product_forbidden)).flatten().map(|kind|
            gate(kind, "Explicit authentication/challenge message in text response; review session and Test Access")
        );
    }
    let document = Html::parse_document(&source);
    let mut excluded_depth = 0;
    let mut heading: Option<(&str, String)> = None;
    let mut notice = String::new();
    let mut notice_too_long = body.len() > 65536;
    // Iterative traversal avoids recursion and repeated ancestor scans. Only
    // text labels count; scripts, attributes and comments are not denial text.
    for edge in document.tree.root().traverse() {
        match edge {
            ego_tree::iter::Edge::Open(node) => {
                if let Some(element) = ElementRef::wrap(node) {
                    if excluded_depth > 0 || excluded(element) {
                        excluded_depth += 1;
                        continue;
                    }
                    let value = element.value();
                    if matches!(value.name(), "title" | "h1" | "h2") {
                        heading = Some((value.name(), String::new()));
                    }
                    if value.name() == "form"
                        && value
                            .attr("action")
                            .is_some_and(|a| a.starts_with("/cdn-cgi/challenge-platform/"))
                        && value
                            .attr("id")
                            .is_some_and(|id| id == "challenge-form" || id.starts_with("cf-chl-"))
                    {
                        return Some((
                            FailureKind::Authentication,
                            "Explicit HTML challenge form; review session and Test Access",
                        ));
                    }
                } else if excluded_depth == 0
                    && let Node::Text(text) = node.value()
                {
                    if let Some((_, label)) = &mut heading {
                        label.push_str(text);
                    } else if !notice_too_long {
                        if notice.len() + text.len() < 512 {
                            notice.push_str(text);
                            notice.push(' ');
                        } else {
                            notice_too_long = true;
                        }
                    }
                }
            }
            ego_tree::iter::Edge::Close(node) => {
                if let Some(element) = ElementRef::wrap(node) {
                    if excluded_depth > 0 {
                        excluded_depth -= 1;
                    } else if heading
                        .as_ref()
                        .is_some_and(|(tag, _)| *tag == element.value().name())
                        && let Some((tag, label)) = heading.take()
                    {
                        if let Some(kind) = message_kind(&label, product_forbidden) {
                            return Some(gate(
                                kind,
                                if tag == "title" {
                                    "Explicit authentication/challenge page title; review session and Test Access"
                                } else {
                                    "Explicit authentication/challenge page heading; review session and Test Access"
                                },
                            ));
                        }
                        // Ordinary headings also count toward the page's
                        // notice, preventing a sidebar from becoming the
                        // apparent entire response when the page has content.
                        if notice.len() + label.len() < 512 {
                            notice.push_str(&label);
                            notice.push(' ');
                        } else {
                            notice_too_long = true;
                        }
                    }
                }
            }
        }
    }
    (!notice_too_long)
        .then(|| message_kind(&notice, product_forbidden))
        .flatten()
        .map(|kind| {
            gate(
                kind,
                "Explicit authentication/challenge page message; review session and Test Access",
            )
        })
}
