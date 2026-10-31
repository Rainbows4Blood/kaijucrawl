//! Adventure levels are scoped to each compatible OSR system, never converted.
use crate::{
    decisions::{Error, Result, Usage},
    responses,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const ALGORITHM: &str = "osr-levels-1";
const INSTRUCTIONS: &str = include_str!("osr_levels_instructions.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Known,
    Unknown,
    NotApplicable,
}
impl State {
    pub fn key(self) -> &'static str {
        match self {
            Self::Known => "known",
            Self::Unknown => "unknown",
            Self::NotApplicable => "not_applicable",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub system: String,
    pub game: Option<String>,
    pub status: State,
    pub min_level: Option<i32>,
    pub max_level: Option<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub ranges: Vec<Range>,
}
impl Data {
    pub fn validate(&self, systems: &[String]) -> Result<()> {
        if systems.is_empty() || self.ranges.is_empty() || self.ranges.len() > 64 {
            return Err(Error::InvalidResponse);
        }
        let mut seen = BTreeSet::new();
        let mut covered = BTreeSet::new();
        for r in &self.ranges {
            if !systems.contains(&r.system)
                || r.min_level.is_some_and(|n| n < 0)
                || r.max_level.is_some_and(|n| n < 0)
                || matches!((r.min_level,r.max_level),(Some(a),Some(b)) if a>b)
                || ((r.status == State::Known) != (r.min_level.is_some() || r.max_level.is_some()))
                || (r.system != "other" && r.game.is_some())
                || r.game.as_ref().is_some_and(|s| {
                    s.trim() != s
                        || s.is_empty()
                        || s.chars().count() > 160
                        || s.chars().any(char::is_control)
                })
                || (r.system == "other" && r.game.is_none() && r.status != State::Unknown)
                || !seen.insert((
                    r.system.clone(),
                    r.game.as_deref().unwrap_or_default().to_lowercase(),
                ))
            {
                return Err(Error::InvalidResponse);
            }
            covered.insert(r.system.clone());
        }
        if covered != systems.iter().cloned().collect()
            || (self
                .ranges
                .iter()
                .any(|r| r.system == "other" && r.game.is_none())
                && self.ranges.iter().filter(|r| r.system == "other").count() != 1)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }
    pub fn count(&self, state: State) -> i64 {
        self.ranges.iter().filter(|r| r.status == state).count() as i64
    }
    pub fn known(&self) -> bool {
        self.count(State::Known) > 0
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

/// Routing labels constrain the answer; they are not evidence for a level range.
pub fn systems(evidence: &Value) -> Result<Vec<String>> {
    let values: Vec<String> = serde_json::from_value(evidence["requested_systems"].clone())
        .map_err(|_| Error::InvalidRequest("Missing OSR level system scope"))?;
    if values.is_empty()
        || values.len() > 15
        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
        || values
            .iter()
            .any(|s| !crate::osr::SYSTEMS.iter().any(|(key, _, _)| s == key))
        || !evidence["book"]["title"].is_string()
        || !evidence["book"]["description"].is_string()
    {
        return Err(Error::InvalidRequest("Invalid OSR level evidence or scope"));
    }
    Ok(values)
}
pub fn encode(evidence: &Value) -> Result<Vec<u8>> {
    systems(evidence)?;
    let number = json!({"type":["integer","null"],"minimum":0,"maximum":i32::MAX});
    let schema = json!({"type":"object","additionalProperties":false,"required":["ranges"],"properties":{
        "ranges":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"object","additionalProperties":false,
        "required":["system","game","status","min_level","max_level"],"properties":{
            "system":{"type":"string","enum":crate::osr::SYSTEMS.iter().map(|s|s.0).collect::<Vec<_>>()},
            "game":{"type":["string","null"],"minLength":1,"maxLength":160},
            "status":{"type":"string","enum":["known","unknown","not_applicable"]},"min_level":number,"max_level":number
        }}}
    }});
    responses::encode(
        evidence,
        ALGORITHM,
        "osr_levels",
        INSTRUCTIONS,
        schema,
        8192,
    )
}
pub fn decode_response(r: responses::Response, systems: &[String]) -> Result<Response> {
    let data = r
        .value
        .map(|v| -> Result<Data> {
            // Serde Option accepts missing fields, but the strict contract requires explicit nulls.
            let rows = v["ranges"].as_array().ok_or(Error::InvalidResponse)?;
            if rows.iter().any(|r| {
                r.as_object().is_none_or(|o| {
                    o.len() != 5
                        || ["system", "game", "status", "min_level", "max_level"]
                            .iter()
                            .any(|k| !o.contains_key(*k))
                })
            }) {
                return Err(Error::InvalidResponse);
            }
            let mut d: Data = serde_json::from_value(v).map_err(|_| Error::InvalidResponse)?;
            d.validate(systems)?;
            d.ranges
                .sort_by(|a, b| (&a.system, &a.game).cmp(&(&b.system, &b.game)));
            Ok(d)
        })
        .transpose()?;
    Ok(Response {
        model: r.model,
        data,
        usage: r.usage,
    })
}
