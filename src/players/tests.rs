use super::*;
use crate::decisions::tests::{http, server};

pub(crate) fn response(counts: Value) -> Value {
    json!({"status":"completed","model":MODEL,"error":null,"incomplete_details":null,
        "output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":counts.to_string()}]}],
        "usage":{"input_tokens":1600,"output_tokens":20,"total_tokens":1620,"input_tokens_details":{"cached_tokens":1200,"cache_write_tokens":0}}})
}
#[tokio::test]
async fn request_uses_luna_strict_nullable_integers_and_only_shared_prefix_caching() {
    let reply = response(json!({"min_players":3,"max_players":5}));
    let (client, received, task) = server(http(200, &reply.to_string(), "")).await;
    let evidence = json!({"title":"Synthetic mystery","description":"For 3–5 players plus a GM."});
    let result = extract(
        &client,
        encode(&evidence).unwrap(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        result.counts,
        Some(Counts {
            min_players: Some(3),
            max_players: Some(5)
        })
    );
    assert_eq!(result.usage.input_tokens_details.cached_tokens, 1200);
    let request = received.await.unwrap();
    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(
        headers
            .to_lowercase()
            .contains("authorization: bearer synthetic-test-key")
    );
    assert!(!headers.to_lowercase().contains("cookie:"));
    let wire: Value = serde_json::from_str(body).unwrap();
    assert_eq!(wire["model"], MODEL);
    assert_eq!(wire["store"], false);
    assert_eq!(wire["max_output_tokens"], 256);
    assert_eq!(wire["reasoning"]["effort"], "none");
    assert_eq!(wire["prompt_cache_options"]["mode"], "explicit");
    assert_eq!(wire["input"][0]["role"], "developer");
    assert_eq!(
        wire["input"][0]["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    assert!(
        wire["input"][1]["content"][0]
            .get("prompt_cache_breakpoint")
            .is_none()
    );
    assert_eq!(wire["input"][1]["content"][0]["text"], evidence.to_string());
    assert_eq!(wire["text"]["format"]["strict"], true);
    assert_eq!(
        wire["text"]["format"]["schema"]["required"],
        json!(["min_players", "max_players"])
    );
    let other: Value = serde_json::from_slice(
        &encode(&json!({"title":"Another book","description":"No count."})).unwrap(),
    )
    .unwrap();
    assert_eq!(wire["input"][0], other["input"][0]);
    assert_eq!(wire["text"], other["text"]);
    task.await.unwrap();
}
#[test]
fn null_partial_exact_and_refused_ranges_are_distinct_and_bad_results_are_rejected() {
    for (min, max) in [
        (None, None),
        (Some(3), None),
        (None, Some(6)),
        (Some(1), Some(1)),
        (Some(4), Some(8)),
    ] {
        let answer = decode(response(json!({"min_players":min,"max_players":max}))).unwrap();
        assert_eq!(
            answer.counts,
            Some(Counts {
                min_players: min,
                max_players: max
            })
        );
    }
    for value in [
        json!({"min_players":5,"max_players":3}),
        json!({"min_players":0,"max_players":null}),
        json!({"min_players":-2,"max_players":3}),
        json!({"min_players":2.5,"max_players":4}),
        json!({"min_players":"3","max_players":4}),
        json!({"min_players":3}),
        json!({"min_players":null,"max_players":null,"extra":1}),
    ] {
        assert_eq!(decode(response(value)).unwrap_err(), Error::InvalidResponse);
    }
    let mut refused = response(Value::Null);
    refused["output"][0]["content"] = json!([{"type":"refusal","refusal":"PRIVATE_PROVIDER_TEXT"}]);
    assert!(decode(refused).unwrap().counts.is_none());
    for status in ["failed", "incomplete", "cancelled", "queued", "in_progress"] {
        let mut r = response(json!({"min_players":null,"max_players":null}));
        r["status"] = json!(status);
        assert_eq!(decode(r).unwrap_err(), Error::InvalidResponse);
    }
    let mut r = response(json!({"min_players":2,"max_players":3}));
    r["usage"]["total_tokens"] = json!(0);
    assert_eq!(decode(r).unwrap_err(), Error::InvalidResponse);
}
#[test]
fn evidence_is_bounded_and_extra_metadata_never_enters_requests() {
    assert!(encode(&json!({"title":"Synthetic","description":"x".repeat(128*1024)})).is_err());
    assert!(
        encode(&json!({"title":"Synthetic","description":"Full text","raw_metadata":"PRIVATE"}))
            .is_err()
    );
    assert!(encode(&json!({"title":"Synthetic"})).is_err());
}
