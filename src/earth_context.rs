//! Source-only Shadowrun / Call of Cthulhu extraction with typed calendar ranges.
use crate::{
    decisions::{self, Error, Result, Usage},
    responses,
};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
pub const MODEL: &str = responses::MODEL;
const INSTRUCTIONS: &str = include_str!("earth_context_instructions.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    ShadowrunLocation,
    ShadowrunDate,
    CocLocation,
    CocDate,
}
impl Kind {
    pub const ALL: [Self; 4] = [
        Self::ShadowrunLocation,
        Self::ShadowrunDate,
        Self::CocLocation,
        Self::CocDate,
    ];
    pub fn system(self) -> &'static str {
        match self {
            Self::ShadowrunLocation | Self::ShadowrunDate => "shadowrun",
            _ => "coc",
        }
    }
    pub fn field(self) -> &'static str {
        match self {
            Self::ShadowrunLocation | Self::CocLocation => "location",
            _ => "date",
        }
    }
    pub fn algorithm(self) -> &'static str {
        match self {
            Self::ShadowrunLocation => "shadowrun-location-1",
            Self::ShadowrunDate => "shadowrun-date-1",
            Self::CocLocation => "coc-location-1",
            Self::CocDate => "coc-date-1",
        }
    }
    pub fn from_algorithm(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.algorithm() == value)
    }
    pub fn encode(self, evidence: &Value) -> Result<Vec<u8>> {
        if !evidence["title"].is_string() || !evidence["description"].is_string() {
            return Err(Error::InvalidRequest(
                "Analysis needs saved source metadata and description",
            ));
        }
        let (task, properties, required) = if self.field() == "location" {
            (
                "Return only location: a nullable string. Combine multiple supported places into one concise semicolon-separated string. Do not output dates.",
                json!({"location":{"type":["string","null"],"minLength":1,"maxLength":1024}}),
                json!(["location"]),
            )
        } else {
            let date = json!({"type":["string","null"],"format":"date"});
            (
                "Return only date_start, date_end, precision and basis. Dates are inclusive Gregorian YYYY-MM-DD bounds. For a CoC era-only fallback choose basis modern or dark_ages, precision era, and null date bounds: the application supplies the fixed range. Otherwise use basis explicit with supported bounds, or basis unknown with null bounds and unknown precision.",
                json!({"date_start":date,"date_end":date,"precision":{"type":"string","enum":["day","month","year","decade","range","era","unknown"]},"basis":{"type":"string","enum":["explicit","modern","dark_ages","unknown"]}}),
                json!(["date_start", "date_end", "precision", "basis"]),
            )
        };
        responses::encode(
            evidence,
            self.algorithm(),
            self.field(),
            &format!("SYSTEM: {}. TASK: {task}\n\n{INSTRUCTIONS}", self.system()),
            json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            1024,
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
    pub fn decode(self, value: Value) -> Result<Response> {
        self.decode_response(responses::decode(value)?)
    }
    pub(crate) fn decode_response(self, r: responses::Response) -> Result<Response> {
        Ok(Response {
            data: r.value.map(|v| self.data(v)).transpose()?,
            usage: r.usage,
            model: r.model,
        })
    }
    pub fn data(self, value: Value) -> Result<Data> {
        let obj = value.as_object().ok_or(Error::InvalidResponse)?;
        let data = if self.field() == "location" {
            if obj.len() != 1 || !obj.contains_key("location") {
                return Err(Error::InvalidResponse);
            }
            let location = if value["location"].is_null() {
                None
            } else {
                let s = value["location"]
                    .as_str()
                    .ok_or(Error::InvalidResponse)?
                    .trim();
                if s.is_empty() || s.chars().count() > 1024 || s.chars().any(char::is_control) {
                    return Err(Error::InvalidResponse);
                }
                Some(s.into())
            };
            Data::Location { location }
        } else {
            if obj.len() != 4
                || !["date_start", "date_end", "precision", "basis"]
                    .iter()
                    .all(|k| obj.contains_key(*k))
            {
                return Err(Error::InvalidResponse);
            }
            let mut range = DateRange {
                start: parse_date(&value["date_start"])?,
                end: parse_date(&value["date_end"])?,
                precision: serde_json::from_value(value["precision"].clone())
                    .map_err(|_| Error::InvalidResponse)?,
                basis: serde_json::from_value(value["basis"].clone())
                    .map_err(|_| Error::InvalidResponse)?,
            };
            if matches!(range.basis, Basis::Modern | Basis::DarkAges) {
                if self.system() != "coc"
                    || range.start.is_some()
                    || range.end.is_some()
                    || range.precision != Precision::Era
                {
                    return Err(Error::InvalidResponse);
                }
                let (start, end) = era(range.basis).ok_or(Error::InvalidResponse)?;
                range.start = Some(start);
                range.end = Some(end);
            }
            Data::Date { range }
        };
        data.validate(self)?;
        Ok(data)
    }
}
fn parse_date(v: &Value) -> Result<Option<NaiveDate>> {
    if v.is_null() {
        return Ok(None);
    }
    let s = v.as_str().ok_or(Error::InvalidResponse)?;
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| Error::InvalidResponse)?;
    if s.len() != 10
        || !(1..=9999).contains(&date.year())
        || date.format("%Y-%m-%d").to_string() != s
    {
        return Err(Error::InvalidResponse);
    }
    Ok(Some(date))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    Day,
    Month,
    Year,
    Decade,
    Range,
    Era,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Explicit,
    Modern,
    DarkAges,
    Unknown,
}
fn era(basis: Basis) -> Option<(NaiveDate, NaiveDate)> {
    let (a, b) = match basis {
        Basis::Modern => (1980, 2030),
        Basis::DarkAges => (950, 1050),
        _ => return None,
    };
    Some((
        NaiveDate::from_ymd_opt(a, 1, 1)?,
        NaiveDate::from_ymd_opt(b, 12, 31)?,
    ))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRange {
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub precision: Precision,
    pub basis: Basis,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Data {
    Location { location: Option<String> },
    Date { range: DateRange },
}
impl Data {
    pub fn known(&self) -> bool {
        match self {
            Self::Location { location } => location.is_some(),
            Self::Date { range } => range.start.is_some() || range.end.is_some(),
        }
    }
    pub fn validate(&self, kind: Kind) -> Result<()> {
        match self {
            Self::Location { location } if kind.field() == "location" => {
                if let Some(s) = location
                    && (s.is_empty()
                        || s.trim() != s
                        || s.chars().count() > 1024
                        || s.chars().any(char::is_control))
                {
                    return Err(Error::InvalidResponse);
                }
            }
            Self::Date { range: r } if kind.field() == "date" => {
                if r.start
                    .into_iter()
                    .chain(r.end)
                    .any(|d| !(1..=9999).contains(&d.year()))
                    || matches!((r.start,r.end),(Some(a),Some(b)) if a>b)
                {
                    return Err(Error::InvalidResponse);
                }
                match r.basis {
                    Basis::Unknown if !self.known() && r.precision == Precision::Unknown => {}
                    Basis::Explicit
                        if self.known()
                            && !matches!(r.precision, Precision::Unknown | Precision::Era) =>
                    {
                        // Precision is descriptive, not extra certainty added by normalization.
                        if matches!(
                            r.precision,
                            Precision::Day | Precision::Month | Precision::Year | Precision::Decade
                        ) {
                            let (a, b) = r.start.zip(r.end).ok_or(Error::InvalidResponse)?;
                            let valid = match r.precision {
                                Precision::Day => a == b,
                                Precision::Month => {
                                    a.day() == 1
                                        && a.year() == b.year()
                                        && a.month() == b.month()
                                        && b.succ_opt().is_some_and(|d| d.month() != b.month())
                                }
                                Precision::Year => {
                                    a.month() == 1
                                        && a.day() == 1
                                        && b.month() == 12
                                        && b.day() == 31
                                        && a.year() == b.year()
                                }
                                Precision::Decade => {
                                    a.month() == 1
                                        && a.day() == 1
                                        && a.year() % 10 == 0
                                        && b.month() == 12
                                        && b.day() == 31
                                        && b.year() == a.year() + 9
                                }
                                _ => false,
                            };
                            if !valid {
                                return Err(Error::InvalidResponse);
                            }
                        }
                    }
                    Basis::Modern | Basis::DarkAges
                        if kind.system() == "coc"
                            && r.precision == Precision::Era
                            && r.start.zip(r.end) == era(r.basis) => {}
                    _ => return Err(Error::InvalidResponse),
                }
            }
            _ => return Err(Error::InvalidResponse),
        }
        Ok(())
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
