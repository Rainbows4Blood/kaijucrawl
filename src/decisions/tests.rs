use super::*;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

fn response() -> Value {
    json!({"model":MODEL,"answers":[
        {"type":"predicate","name":"fantasy","probability":0.99},
        {"type":"choice","name":"kind","choice":"adventure","confidence":0.98,"probabilities":[
            {"value":"adventure","probability":0.98},{"value":"rulebook","probability":0.01},{"value":"other","probability":0.01}]},
        {"type":"score","name":"adventure_evidence","score":1.9,"confidence":0.9,"probabilities":[
            {"label":"none","value":0,"probability":0.0},{"label":"possible","value":1,"probability":0.1},{"label":"explicit","value":2,"probability":0.9}]}
    ],"usage":{"input_tokens":200,"input_tokens_details":{"cached_tokens":20,"cache_write_tokens":10},"output_tokens":0,"total_tokens":200}})
}
pub(crate) async fn server(
    reply: String,
) -> (
    Client,
    oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/decisions", listener.local_addr().unwrap());
    let (send, received) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let len = socket.read(&mut chunk).await.unwrap();
            if len == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..len]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        send.send(String::from_utf8(request).unwrap()).ok();
        if reply.is_empty() {
            std::future::pending::<()>().await;
        }
        let _ = socket.write_all(reply.as_bytes()).await;
        drop(socket);
        // Any redirect/retry/duplicate request would be a billing or credential leak bug.
        assert!(
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err()
        );
    });
    (Client::local(url), received, task)
}
pub(crate) fn http(status: u16, body: &str, headers: &str) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn posts_json_and_all_question_types_with_isolated_auth_and_reports_usage() {
    let (client, received, task) = server(http(200, &response().to_string(), "")).await;
    let d = client
        .decide(&diagnostic(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(d.answers.len(), 3);
    assert_eq!(d.usage.input_tokens, 200);
    let request = received.await.unwrap();
    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("POST /v1/decisions HTTP/1.1"));
    assert!(
        headers.to_lowercase().contains(
            "authorization: Bearer synthetic-test-key"
                .to_lowercase()
                .as_str()
        )
    );
    assert!(!headers.to_lowercase().contains("cookie:"));
    assert!(headers.contains(concat!("KaijuCrawl/", env!("CARGO_PKG_VERSION"))));
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["prompt_cache_options"], json!({"mode":"explicit"}));
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_json");
    assert!(
        body["input"][0]["content"][0]
            .get("prompt_cache_breakpoint")
            .is_none()
    );
    assert_eq!(body["questions"][1]["choices"][0]["value"], "adventure");
    task.await.unwrap();
}

#[test]
fn enforces_request_bounds_and_preserves_boolean_vs_string_choices() {
    let mut r = diagnostic();
    r.input = Input::Text("Synthetic text".into());
    let body: Value = serde_json::from_slice(&r.encode().unwrap()).unwrap();
    assert_eq!(body["input"], "Synthetic text");
    assert_eq!(body["prompt_cache_options"], json!({"mode":"explicit"}));
    r.questions = vec![Question::Choice {
        name: "typed".into(),
        instructions: "Select".into(),
        choices: vec![
            Choice {
                value: ChoiceValue::Boolean(true),
                description: String::new(),
            },
            Choice {
                value: ChoiceValue::Text("true".into()),
                description: String::new(),
            },
        ],
    }];
    assert!(r.encode().is_ok());
    if let Question::Choice { choices, .. } = &mut r.questions[0] {
        choices[1].value = ChoiceValue::Boolean(true);
    }
    assert!(r.encode().is_err());
    for questions in [
        vec![],
        vec![diagnostic().questions[0].clone(); 401],
        vec![diagnostic().questions[0].clone(); 2],
    ] {
        r = diagnostic();
        r.questions = questions;
        assert!(r.encode().is_err());
    }
    r = diagnostic();
    r.input = Input::Text("x".repeat(65537));
    assert!(r.encode().is_err());
    for data in [
        json!("text"),
        json!([1]),
        json!([]),
        json!({"values":vec![0;10001]}),
        json!({"huge":"x".repeat(REQUEST_LIMIT)}),
    ] {
        r.input = Input::Json(data);
        assert!(r.encode().is_err());
    }
    let mut nested = json!({});
    for _ in 0..32 {
        nested = json!({"nested":nested});
    }
    r.input = Input::Json(nested);
    assert!(r.encode().is_err());
    r = diagnostic();
    if let Question::Score { levels, .. } = &mut r.questions[2] {
        levels.truncate(1);
    }
    assert!(r.encode().is_err());
    r = diagnostic();
    if let Question::Predicate { instructions, .. } = &mut r.questions[0] {
        instructions.clear();
    }
    assert!(r.encode().is_err());
}

#[test]
fn refuses_mismatched_names_options_scores_and_usage_but_preserves_refusals() {
    let mut good = response();
    good["answers"][0] = json!({"type":"refusal","name":"fantasy"});
    let parsed: Decision = serde_json::from_value(good).unwrap();
    assert!(parsed.validate(&diagnostic()).is_ok());
    for (pointer, value) in [
        ("/answers/0/name", json!("wrong")),
        ("/answers/0/probability", json!(1.2)),
        ("/answers/1/choice", json!("not-an-option")),
        ("/answers/1/confidence", json!(-1)),
        ("/answers/1/probabilities/1/value", json!("adventure")),
        ("/answers/1/probabilities/2/probability", json!(0.9)),
        ("/answers/2/score", json!(0.0)),
        ("/answers/2/probabilities/2/value", json!(1)),
        ("/answers/2/probabilities/2/label", json!("invented")),
        ("/model", json!("different-model")),
        ("/usage/total_tokens", json!(201)),
        ("/usage/input_tokens_details/cached_tokens", json!(201)),
    ] {
        let mut value_json = response();
        *value_json.pointer_mut(pointer).unwrap() = value;
        let d: Decision = serde_json::from_value(value_json).unwrap();
        assert_eq!(
            d.validate(&diagnostic()),
            Err(Error::InvalidResponse),
            "{pointer}"
        );
    }
    let mut d: Decision = serde_json::from_value(response()).unwrap();
    d.answers.pop();
    assert!(d.validate(&diagnostic()).is_err());
    let mut d: Decision = serde_json::from_value(response()).unwrap();
    d.answers.swap(0, 1);
    assert!(d.validate(&diagnostic()).is_err());
}

#[tokio::test]
async fn errors_and_redirects_never_leak_bodies_or_retry() {
    for (status, expected) in [
        (
            401,
            Error::Authentication {
                retry_after_secs: Some(12),
            },
        ),
        (
            403,
            Error::Permission {
                retry_after_secs: Some(12),
            },
        ),
        (
            429,
            Error::RateLimit {
                retry_after_secs: Some(12),
            },
        ),
        (
            500,
            Error::Http {
                status: 500,
                retry_after_secs: Some(12),
            },
        ),
        (
            503,
            Error::Capacity {
                retry_after_secs: Some(12),
            },
        ),
        (
            302,
            Error::Http {
                status: 302,
                retry_after_secs: Some(12),
            },
        ),
    ] {
        let (client, received, task) = server(http(
            status,
            "SECRET-MARKER",
            "Retry-After: 12\r\nLocation: /credential-trap\r\n",
        ))
        .await;
        let e = client
            .decide(&diagnostic(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(e, expected);
        let retry = e.retry_hint().unwrap();
        assert_eq!(retry.status, Some(status));
        assert_eq!(retry.after, Some(Duration::from_secs(12)));
        assert!(!format!("{e:?} {e}").contains("SECRET-MARKER"));
        received.await.unwrap();
        task.await.unwrap();
    }
}

#[test]
fn retry_after_accepts_seconds_and_dates_without_early_rounding_or_body_content() {
    let now =
        std::time::UNIX_EPOCH + Duration::from_secs(1_800_000_000) + Duration::from_millis(500);
    assert_eq!(retry_after("12", now), Some(12));
    assert_eq!(retry_after(" 0 ", now), Some(0));
    let future = httpdate::fmt_http_date(now + Duration::from_secs(12));
    assert_eq!(retry_after(&future, now), Some(12));
    let past = httpdate::fmt_http_date(now - Duration::from_secs(12));
    assert_eq!(retry_after(&past, now), Some(0));
    for invalid in ["-1", "+1", "1.5", "PRIVATE_HEADER", "18446744073709551616"] {
        assert_eq!(retry_after(invalid, now), None);
    }
    assert_eq!(retry_after("18446744073709551615", now), Some(u64::MAX));
    for error in [
        Error::Cancelled,
        Error::MissingKey,
        Error::InvalidKey,
        Error::InvalidRequest("synthetic invalid input"),
    ] {
        assert!(error.retry_hint().is_none());
    }
    for error in [
        Error::Transport,
        Error::Timeout,
        Error::InvalidResponse,
        Error::ResponseTooLarge,
    ] {
        let retry = error.retry_hint().unwrap();
        assert_eq!(retry.status, None);
        assert_eq!(retry.after, None);
        assert!(!retry.reason.is_empty());
    }
    assert_eq!(
        Error::Capacity {
            retry_after_secs: Some(12)
        }
        .retry_hint(),
        Some(Retry {
            status: Some(503),
            reason: "Model capacity unavailable",
            after: Some(Duration::from_secs(12))
        })
    );
}

#[tokio::test]
async fn rejects_malformed_and_oversized_responses() {
    let huge = "x".repeat(RESPONSE_LIMIT + 1);
    for (reply, expected) in [
        (
            http(200, "not JSON SECRET-MARKER", ""),
            Error::InvalidResponse,
        ),
        (http(200, &huge, ""), Error::ResponseTooLarge),
        (
            format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{huge}\r\n0\r\n\r\n",
                huge.len()
            ),
            Error::ResponseTooLarge,
        ),
    ] {
        let (client, _, task) = server(reply).await;
        let e = client
            .decide(&diagnostic(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(e, expected);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_prevents_sends_and_interrupts_inflight_and_queued_requests() {
    let (client, received, task) = server(String::new()).await;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        client.decide(&diagnostic(), &cancelled).await.unwrap_err(),
        Error::Cancelled
    );
    let stop = CancellationToken::new();
    let stopper = stop.clone();
    let copy = client.clone();
    let request = tokio::spawn(async move { copy.decide(&diagnostic(), &stopper).await });
    received.await.unwrap();
    let waiting = CancellationToken::new();
    let waiting_copy = waiting.clone();
    let second = tokio::spawn(async move { client.decide(&diagnostic(), &waiting_copy).await });
    waiting.cancel();
    assert_eq!(second.await.unwrap().unwrap_err(), Error::Cancelled);
    stop.cancel();
    assert_eq!(request.await.unwrap().unwrap_err(), Error::Cancelled);
    task.abort();
}

#[tokio::test]
async fn key_validation_is_value_free_and_precedes_network() {
    let mut client = Client::local("http://127.0.0.1:1/v1/decisions".into());
    for (key, expected) in [
        ("", Error::MissingKey),
        (" \n", Error::MissingKey),
        ("synthetic\r\nsecret", Error::InvalidKey),
        ("synthetic key", Error::InvalidKey),
    ] {
        client.key = key.into();
        assert_eq!(
            client
                .decide(&diagnostic(), &CancellationToken::new())
                .await
                .unwrap_err(),
            expected
        );
    }
}

#[tokio::test]
async fn total_deadline_bounds_a_stalled_response_without_retrying() {
    let (mut client, received, task) = server(String::new()).await;
    client.deadline = Duration::from_millis(150);
    let result = client
        .decide(&diagnostic(), &CancellationToken::new())
        .await;
    assert_eq!(result.unwrap_err(), Error::Timeout);
    received.await.unwrap();
    task.abort();
}

#[tokio::test]
async fn desktop_close_cancels_check_and_waits_for_operation_without_opening_storage() {
    let (client, received, task) = server(String::new()).await;
    let temp = tempfile::tempdir().unwrap();
    let app = crate::controller::Controller::new(crate::paths::LocalPaths::at(temp.path()), false);
    app.install_test_decisions(client);
    let copy = app.clone();
    let check = tokio::spawn(async move { copy.test_decisions().await });
    received.await.unwrap();
    assert!(
        app.test_decisions()
            .await
            .unwrap_err()
            .contains("Another operation")
    );
    tokio::time::timeout(Duration::from_secs(2), app.shutdown())
        .await
        .unwrap();
    assert!(check.await.unwrap().unwrap_err().contains("cancelled"));
    assert!(app.test_decisions().await.unwrap_err().contains("closing"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    task.abort();
}
