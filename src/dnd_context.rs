//! Conservative D&D source extraction, independent of all other enrichment outputs.
use crate::{
    decisions::{self, Error, Result, Usage},
    responses,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
pub const MODEL: &str = responses::MODEL;
const INSTRUCTIONS: &str = include_str!("dnd_context_instructions.txt");
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Levels,
    Locations,
    Date,
}
impl Kind {
    pub const ALL: [Self; 3] = [Self::Levels, Self::Locations, Self::Date];
    pub fn key(self) -> &'static str {
        match self {
            Self::Levels => "levels",
            Self::Locations => "locations",
            Self::Date => "date",
        }
    }
    pub fn algorithm(self) -> &'static str {
        match self {
            Self::Levels => "dnd-levels-1",
            Self::Locations => "dnd-locations-1",
            Self::Date => "dnd-date-1",
        }
    }
    pub fn from_algorithm(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.algorithm() == value)
    }
    fn schema(self) -> Value {
        let number = json!({"type":["integer","null"],"minimum":0,"maximum":i32::MAX});
        let (properties, required) = match self {
            Self::Levels => (
                json!({"min_level":number,"max_level":number}),
                json!(["min_level", "max_level"]),
            ),
            Self::Locations => (
                json!({"locations":{"type":"array","items":{"type":"string","minLength":1,"maxLength":160},"maxItems":64}}),
                json!(["locations"]),
            ),
            Self::Date => (
                json!({"in_universe_date":{"type":["string","null"],"minLength":1,"maxLength":1024}}),
                json!(["in_universe_date"]),
            ),
        };
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    }
    pub fn encode(self, evidence: &Value) -> Result<Vec<u8>> {
        if !evidence.is_object()
            || !evidence["title"].is_string()
            || !evidence["description"].is_string()
        {
            return Err(Error::InvalidRequest(
                "D&D analysis needs saved source metadata and description",
            ));
        }
        let task = match self {
            Self::Levels => {
                "TASK: Return only min_level and max_level, nonnegative nullable integers describing intended player-character levels. Do not output locations or dates."
            }
            Self::Locations => {
                "TASK: Return only locations, an array of unique in-universe place names (up to 64, each at most 160 characters). Return [] without evidence. Do not output character names, levels or dates."
            }
            Self::Date => {
                "TASK: Return only in_universe_date, a nullable string of at most 1024 characters preserving the source's in-world calendar wording, qualifiers or date range. Do not output real publication dates, locations or levels."
            }
        };
        let tokens = match self {
            Self::Levels => 256,
            Self::Locations => 4096,
            Self::Date => 1024,
        };
        responses::encode(
            evidence,
            self.algorithm(),
            self.key(),
            &format!("{task}\n\n{INSTRUCTIONS}"),
            self.schema(),
            tokens,
        )
    }
    pub async fn extract(
        self,
        client: &decisions::Client,
        bytes: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Response> {
        self.decode_response(responses::extract(client, bytes, cancel).await?)
    }
    pub fn decode(self, body: Value) -> Result<Response> {
        self.decode_response(responses::decode(body)?)
    }
    pub(crate) fn decode_response(self, r: responses::Response) -> Result<Response> {
        Ok(Response {
            data: r.value.map(|v| self.data(v)).transpose()?,
            usage: r.usage,
            model: r.model,
        })
    }
    pub fn data(self, value: Value) -> Result<Data> {
        let object = value.as_object().ok_or(Error::InvalidResponse)?;
        let data = match self {
            Self::Levels => {
                if object.len() != 2
                    || !object.contains_key("min_level")
                    || !object.contains_key("max_level")
                {
                    return Err(Error::InvalidResponse);
                }
                fn number(v: &Value) -> Result<Option<i32>> {
                    if v.is_null() {
                        Ok(None)
                    } else {
                        v.as_i64()
                            .and_then(|n| i32::try_from(n).ok())
                            .filter(|n| *n >= 0)
                            .map(Some)
                            .ok_or(Error::InvalidResponse)
                    }
                }
                let min_level = number(&value["min_level"])?;
                let max_level = number(&value["max_level"])?;
                if matches!((min_level,max_level),(Some(a),Some(b)) if a>b) {
                    return Err(Error::InvalidResponse);
                }
                Data::Levels {
                    min_level,
                    max_level,
                }
            }
            Self::Locations => {
                if object.len() != 1 {
                    return Err(Error::InvalidResponse);
                }
                let values = value["locations"]
                    .as_array()
                    .ok_or(Error::InvalidResponse)?;
                if values.len() > 64 {
                    return Err(Error::InvalidResponse);
                }
                let mut locations = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for v in values {
                    let s = text(v, 160)?;
                    if seen.insert(s.to_lowercase()) {
                        locations.push(s);
                    }
                }
                Data::Locations { locations }
            }
            Self::Date => {
                if object.len() != 1 || !object.contains_key("in_universe_date") {
                    return Err(Error::InvalidResponse);
                }
                Data::Date {
                    in_universe_date: if value["in_universe_date"].is_null() {
                        None
                    } else {
                        Some(text(&value["in_universe_date"], 1024)?)
                    },
                }
            }
        };
        Ok(data)
    }
}
fn text(value: &Value, limit: usize) -> Result<String> {
    let s = value.as_str().ok_or(Error::InvalidResponse)?.trim();
    if s.is_empty() || s.chars().count() > limit || s.chars().any(char::is_control) {
        return Err(Error::InvalidResponse);
    }
    Ok(s.into())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Data {
    Levels {
        min_level: Option<i32>,
        max_level: Option<i32>,
    },
    Locations {
        locations: Vec<String>,
    },
    Date {
        in_universe_date: Option<String>,
    },
}
impl Data {
    pub fn validate(&self) -> Result<()> {
        let mut value = serde_json::to_value(self).map_err(|_| Error::InvalidResponse)?;
        value
            .as_object_mut()
            .ok_or(Error::InvalidResponse)?
            .remove("kind");
        if self.kind().data(value)? != *self {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::Levels { .. } => Kind::Levels,
            Self::Locations { .. } => Kind::Locations,
            Self::Date { .. } => Kind::Date,
        }
    }
    pub fn known(&self) -> bool {
        match self {
            Self::Levels {
                min_level,
                max_level,
            } => min_level.is_some() || max_level.is_some(),
            Self::Locations { locations } => !locations.is_empty(),
            Self::Date { in_universe_date } => in_universe_date.is_some(),
        }
    }
}
pub struct Response {
    pub model: String,
    pub data: Option<Data>,
    pub usage: Usage,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Saved {
    pub data: Data,
    pub algorithm: String,
    pub model: String,
    pub input_hash: String,
    pub calculated_at: String,
}
