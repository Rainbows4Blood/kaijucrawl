//! Adventure player-count extraction using a fixed Responses structured-output contract.
use crate::decisions::{self, Error, Result, Usage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

pub const ALGORITHM: &str = "adventure-players-1";
pub const MODEL: &str = "gpt-6-luna";
pub const INSTRUCTIONS: &str = include_str!("players_instructions.txt");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    pub min_players: Option<i32>,
    pub max_players: Option<i32>,
}
impl Counts {
    pub fn validate(&self) -> Result<()> {
        if self.min_players.is_some_and(|n| n < 1)
            || self.max_players.is_some_and(|n| n < 1)
            || matches!((self.min_players,self.max_players), (Some(a),Some(b)) if a>b)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }
    pub fn known(&self) -> bool {
        self.min_players.is_some() || self.max_players.is_some()
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Saved {
    #[serde(flatten)]
    pub counts: Counts,
    pub algorithm: String,
    pub model: String,
    pub input_hash: String,
    pub calculated_at: String,
}
#[derive(Debug)]
pub struct Response {
    pub model: String,
    /// A refusal is not an unknown range and must not overwrite a prior valid result.
    pub counts: Option<Counts>,
    pub usage: Usage,
}
pub fn encode(evidence: &Value) -> Result<Vec<u8>> {
    if !evidence.as_object().is_some_and(|o| o.len() == 2)
        || !evidence["title"].is_string()
        || !evidence["description"].is_string()
    {
        return Err(Error::InvalidRequest(
            "Player analysis needs a saved title and description",
        ));
    }
    let number = json!({"type":["integer","null"],"minimum":1,"maximum":i32::MAX});
    crate::responses::encode(
        evidence,
        ALGORITHM,
        "adventure_players",
        INSTRUCTIONS,
        json!({"type":"object","properties":{"min_players":number,"max_players":number},"required":["min_players","max_players"],"additionalProperties":false}),
        256,
    )
}
pub async fn extract(
    client: &decisions::Client,
    bytes: Vec<u8>,
    cancel: &CancellationToken,
) -> Result<Response> {
    from_response(crate::responses::extract(client, bytes, cancel).await?)
}
pub fn decode(body: Value) -> Result<Response> {
    from_response(crate::responses::decode(body)?)
}
pub(crate) fn from_response(response: crate::responses::Response) -> Result<Response> {
    let counts = response
        .value
        .map(|value| {
            if value.get("min_players").is_none() || value.get("max_players").is_none() {
                return Err(Error::InvalidResponse);
            }
            let counts: Counts =
                serde_json::from_value(value).map_err(|_| Error::InvalidResponse)?;
            counts.validate()?;
            Ok(counts)
        })
        .transpose()?;
    Ok(Response {
        counts,
        usage: response.usage,
        model: response.model,
    })
}

#[cfg(test)]
pub(crate) mod tests;
