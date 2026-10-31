//! Shared strict, cached Responses transport for explicit extraction analyses.
use crate::decisions::{self, Error, Result, Usage};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
pub const MODEL: &str = "gpt-6-luna";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Luna,
    Terra,
    Sol,
    Astra,
}
impl Model {
    pub const ALL: [Self; 4] = [Self::Luna, Self::Terra, Self::Sol, Self::Astra];
    pub fn name(self) -> &'static str {
        match self {
            Self::Luna => MODEL,
            Self::Terra => "gpt-6.1-terra",
            Self::Sol => "gpt-6.1-sol",
            Self::Astra => "gpt-6-astra",
        }
    }
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Luna => Some(Self::Terra),
            Self::Terra => Some(Self::Sol),
            Self::Sol => Some(Self::Astra),
            Self::Astra => None,
        }
    }
    pub fn reasoning(self) -> &'static str {
        if matches!(self, Self::Sol | Self::Astra) {
            "low"
        } else {
            "none"
        }
    }
    fn deadline(self) -> std::time::Duration {
        std::time::Duration::from_secs(if matches!(self, Self::Sol | Self::Astra) {
            120
        } else {
            30
        })
    }
    pub fn accepts(self, value: &str) -> bool {
        value == self.name() || value.starts_with(&format!("{}-", self.name()))
    }
}
/// Safe error metadata, never provider bodies or generated text.
#[derive(Debug)]
pub struct Failure {
    pub error: Error,
    pub usage: Option<Usage>,
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self { error, usage: None }
    }
}
pub fn request_model(bytes: &[u8]) -> Result<Model> {
    let body: Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::InvalidRequest("Invalid structured request"))?;
    Model::ALL
        .into_iter()
        .find(|m| body["model"] == m.name())
        .ok_or(Error::InvalidRequest("Unknown structured model"))
}
/// Change only the model and its required reasoning budget; retain source/schema/cache prefix.
pub fn escalate(bytes: &mut Vec<u8>) -> Result<Option<Model>> {
    let Some(model) = request_model(bytes)?.next() else {
        return Ok(None);
    };
    let mut body: Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::InvalidRequest("Invalid structured request"))?;
    body["model"] = json!(model.name());
    body["reasoning"]["effort"] = json!(model.reasoning());
    if model.reasoning() != "none" {
        body["max_output_tokens"] =
            json!(body["max_output_tokens"].as_u64().unwrap_or(0).max(8192));
    }
    *bytes = serde_json::to_vec(&body)
        .map_err(|_| Error::InvalidRequest("Cannot encode structured retry"))?;
    Ok(Some(model))
}
/// One dispatch only. The coordinator owns escalation, retries, accounting and cancellation.
pub(crate) async fn attempt<T>(
    client: &decisions::Client,
    bytes: Vec<u8>,
    cancel: &CancellationToken,
    decode_value: impl FnOnce(Response) -> Result<T>,
) -> std::result::Result<T, Failure> {
    let model = request_model(&bytes)?;
    let body = client
        .post_json_timed(ENDPOINT, bytes, cancel, model.deadline())
        .await?;
    let usage = read_usage(&body).ok();
    decode_for(body, model)
        .and_then(decode_value)
        .map_err(|error| Failure { error, usage })
}
const ENDPOINT: &str = "https://api.openai.com/v1/responses";
pub struct Response {
    pub model: String,
    pub value: Option<Value>,
    pub usage: Usage,
}
pub fn encode(
    evidence: &Value,
    algorithm: &str,
    name: &str,
    instructions: &str,
    schema: Value,
    tokens: u32,
) -> Result<Vec<u8>> {
    let body = json!({"model":MODEL,"store":false,"stream":false,"reasoning":{"effort":"none"},"max_output_tokens":tokens,
      "prompt_cache_key":algorithm,"prompt_cache_options":{"mode":"explicit","ttl":"30m"},
      "input":[{"role":"developer","content":[{"type":"input_text","text":instructions,"prompt_cache_breakpoint":{"mode":"explicit"}}]},
               {"role":"user","content":[{"type":"input_text","text":evidence.to_string()}]}],
      "text":{"format":{"type":"json_schema","name":name,"strict":true,"schema":schema}}});
    let bytes =
        serde_json::to_vec(&body).map_err(|_| Error::InvalidRequest("Cannot encode analysis"))?;
    if bytes.len() > 128 * 1024 {
        return Err(Error::InvalidRequest("Encoded input exceeds 128 KiB"));
    }
    Ok(bytes)
}
pub async fn extract(
    client: &decisions::Client,
    bytes: Vec<u8>,
    cancel: &CancellationToken,
) -> Result<Response> {
    attempt(client, bytes, cancel, Ok)
        .await
        .map_err(|e| e.error)
}
pub fn decode(body: Value) -> Result<Response> {
    decode_for(body, Model::Luna)
}
pub fn decode_for(body: Value, model: Model) -> Result<Response> {
    if body["status"] != "completed"
        || !body["error"].is_null()
        || !body["incomplete_details"].is_null()
        || !body["model"].as_str().is_some_and(|m| model.accepts(m))
    {
        return Err(Error::InvalidResponse);
    }
    let usage = read_usage(&body)?;
    let items = body["output"].as_array().ok_or(Error::InvalidResponse)?;
    let mut answer = None;
    for item in items {
        if item["type"] == "reasoning" {
            continue;
        }
        if item["type"] != "message" || item["role"] != "assistant" || item["status"] != "completed"
        {
            return Err(Error::InvalidResponse);
        }
        for part in item["content"].as_array().ok_or(Error::InvalidResponse)? {
            if answer.is_some() {
                return Err(Error::InvalidResponse);
            }
            answer = Some(match part["type"].as_str() {
                Some("refusal") => None,
                Some("output_text") => {
                    let value: Value =
                        serde_json::from_str(part["text"].as_str().ok_or(Error::InvalidResponse)?)
                            .map_err(|_| Error::InvalidResponse)?;
                    Some(value)
                }
                _ => return Err(Error::InvalidResponse),
            });
        }
    }
    Ok(Response {
        value: answer.ok_or(Error::InvalidResponse)?,
        model: body["model"].as_str().ok_or(Error::InvalidResponse)?.into(),
        usage,
    })
}

fn read_usage(body: &Value) -> Result<Usage> {
    let u = &body["usage"];
    let input = u["input_tokens"].as_u64().ok_or(Error::InvalidResponse)?;
    let output = u["output_tokens"].as_u64().ok_or(Error::InvalidResponse)?;
    let total = u["total_tokens"].as_u64().ok_or(Error::InvalidResponse)?;
    let cached = u["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .ok_or(Error::InvalidResponse)?;
    // Some provider responses omit cache-write usage when no write occurred.
    let writes = match u["input_tokens_details"].get("cache_write_tokens") {
        None => 0,
        Some(v) => v.as_u64().ok_or(Error::InvalidResponse)?,
    };
    if input.checked_add(output) != Some(total) || cached.saturating_add(writes) > input {
        return Err(Error::InvalidResponse);
    }
    Ok(Usage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: total,
        input_tokens_details: decisions::InputTokens {
            cached_tokens: cached,
            cache_write_tokens: writes,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_contracts_keep_source_schema_and_cache_when_escalating() {
        let evidence = json!({"title":"Synthetic","description":"Full saved evidence"});
        let mut requests = vec![crate::players::encode(&evidence).unwrap()];
        requests.extend(
            crate::dnd_context::Kind::ALL
                .into_iter()
                .map(|k| k.encode(&evidence).unwrap()),
        );
        requests.extend(
            crate::earth_context::Kind::ALL
                .into_iter()
                .map(|k| k.encode(&evidence).unwrap()),
        );
        assert_eq!(requests.len(), 8);
        for mut bytes in requests {
            let original: Value = serde_json::from_slice(&bytes).unwrap();
            for model in Model::ALL {
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(request_model(&bytes).unwrap(), model);
                assert_eq!(body["reasoning"]["effort"], model.reasoning());
                for field in [
                    "input",
                    "text",
                    "store",
                    "stream",
                    "prompt_cache_key",
                    "prompt_cache_options",
                ] {
                    assert_eq!(body[field], original[field]);
                }
                assert_eq!(
                    body["max_output_tokens"],
                    if model.reasoning() == "low" {
                        json!(8192)
                    } else {
                        original["max_output_tokens"].clone()
                    }
                );
                assert_eq!(escalate(&mut bytes).unwrap(), model.next());
            }
            assert_eq!(request_model(&bytes).unwrap(), Model::Astra);
        }
    }
    #[test]
    fn envelopes_must_match_requested_model_and_keep_actual_model_identity() {
        for model in Model::ALL {
            let mut value =
                crate::players::tests::response(json!({"min_players":null,"max_players":null}));
            value["model"] = json!(format!("{}-2026-10-01", model.name()));
            let r = decode_for(value.clone(), model).unwrap();
            assert_eq!(r.model, value["model"].as_str().unwrap());
            value["model"] = json!("unrequested-model");
            assert!(decode_for(value, model).is_err());
        }
    }
    #[tokio::test]
    async fn semantic_rejection_preserves_available_usage_without_output_text() {
        use crate::decisions::tests::{http, server};
        let value = crate::players::tests::response(json!({"min_players":7,"max_players":2}));
        let (client, _, task) = server(http(200, &value.to_string(), "")).await;
        let bytes =
            crate::players::encode(&json!({"title":"Synthetic","description":"Full"})).unwrap();
        let error = attempt(
            &client,
            bytes,
            &CancellationToken::new(),
            crate::players::from_response,
        )
        .await
        .unwrap_err();
        assert_eq!(error.error, Error::InvalidResponse);
        assert_eq!(error.usage.unwrap().input_tokens, 1600);
        task.await.unwrap();
    }
}
