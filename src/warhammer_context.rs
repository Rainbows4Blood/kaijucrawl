//! Calendar-preserving dates and bounded place lists, using the shared strict parser.
use crate::{
    decisions::{self, Error, Result},
    dnd_context::{self, Data, Response},
    responses,
    warhammer::Family,
};
use serde_json::{Value, json};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    FantasyLocations,
    FantasyDate,
    FortyKLocations,
    FortyKDate,
}
impl Kind {
    pub const ALL: [Self; 4] = [
        Self::FantasyLocations,
        Self::FantasyDate,
        Self::FortyKLocations,
        Self::FortyKDate,
    ];
    pub fn family(self) -> Family {
        match self {
            Self::FantasyLocations | Self::FantasyDate => Family::Fantasy,
            _ => Family::FortyK,
        }
    }
    pub fn field(self) -> &'static str {
        match self {
            Self::FantasyLocations | Self::FortyKLocations => "locations",
            _ => "date",
        }
    }
    pub fn algorithm(self) -> &'static str {
        match self {
            Self::FantasyLocations => "warhammer-fantasy-locations-1",
            Self::FantasyDate => "warhammer-fantasy-date-1",
            Self::FortyKLocations => "warhammer-40k-locations-1",
            Self::FortyKDate => "warhammer-40k-date-1",
        }
    }
    pub fn from_algorithm(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.algorithm() == s)
    }
    pub fn parser(self) -> dnd_context::Kind {
        if self.field() == "locations" {
            dnd_context::Kind::Locations
        } else {
            dnd_context::Kind::Date
        }
    }
    pub fn data(self, value: Value) -> Result<Data> {
        self.parser().data(value)
    }
    pub fn decode_response(self, r: responses::Response) -> Result<Response> {
        self.parser().decode_response(r)
    }
    pub fn encode(self, evidence: &Value) -> Result<Vec<u8>> {
        if !evidence["title"].is_string() || !evidence["description"].is_string() {
            return Err(Error::InvalidRequest(
                "Warhammer extraction needs original metadata and description",
            ));
        }
        let task = if self.field() == "locations" {
            "Return only locations, a unique array of in-universe place names, at most 64 entries of at most 160 characters each. Preserve supported realms, planets, sectors, regions, settlements and other named places. Return [] when none is established. Do not output people, factions, game titles or dates."
        } else {
            "Return only in_universe_date, a nullable string of at most 1024 characters. Preserve the source's in-world calendar, era, millennium notation, uncertainty and ranges exactly in meaning. Return null if the playable/subject timeframe is not established. Never convert to a Gregorian date or supply a game's default era."
        };
        let schema = if self.field() == "locations" {
            json!({"type":"object","required":["locations"],"additionalProperties":false,"properties":{"locations":{"type":"array","maxItems":64,"items":{"type":"string","minLength":1,"maxLength":160}}}})
        } else {
            json!({"type":"object","required":["in_universe_date"],"additionalProperties":false,"properties":{"in_universe_date":{"type":["string","null"],"minLength":1,"maxLength":1024}}})
        };
        responses::encode(
            evidence,
            self.algorithm(),
            self.field(),
            &format!(
                "{task}\nFamily: {}.\n{}",
                self.family().name(),
                include_str!("warhammer_context_instructions.txt")
            ),
            schema,
            if self.field() == "locations" {
                4096
            } else {
                1024
            },
        )
    }
    pub async fn extract(
        self,
        client: &decisions::Client,
        bytes: Vec<u8>,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Response> {
        self.decode_response(responses::extract(client, bytes, cancel).await?)
    }
}
