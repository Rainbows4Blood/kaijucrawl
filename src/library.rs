//! Read-only 40k game browsing contracts. No arbitrary catalogue query surface.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Game {
    DarkHeresy,
    #[serde(rename = "dark_heresy_2e")]
    DarkHeresy2e,
    RogueTrader,
    OnlyWar,
    Deathwatch,
    BlackCrusade,
    ImperiumMaledictum,
    WrathGlory,
}
impl Game {
    pub fn key(self) -> &'static str {
        match self {
            Self::DarkHeresy => "dark_heresy",
            Self::DarkHeresy2e => "dark_heresy_2e",
            Self::RogueTrader => "rogue_trader",
            Self::OnlyWar => "only_war",
            Self::Deathwatch => "deathwatch",
            Self::BlackCrusade => "black_crusade",
            Self::ImperiumMaledictum => "imperium_maledictum",
            Self::WrathGlory => "wrath_glory",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    ReleaseDate,
    #[default]
    Name,
    BookType,
    InUniverseDate,
    InUniverseLocation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub game: Game,
    pub sort: Sort,
    pub descending: bool,
    pub key: Option<String>,
    pub target: String,
    pub product_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub game: Game,
    #[serde(default)]
    pub sort: Sort,
    #[serde(default)]
    pub descending: bool,
    pub after: Option<Cursor>,
}
#[derive(Debug, Serialize)]
pub struct Book {
    pub target: String,
    pub product_id: String,
    pub name: String,
    pub release_date: Option<String>,
    pub book_type: Option<String>,
    pub in_universe_date: Option<String>,
    pub locations: Vec<String>,
    pub date_analyzed: bool,
    pub locations_analyzed: bool,
}
#[derive(Debug, Serialize)]
pub struct Page {
    pub books: Vec<Book>,
    pub total: i64,
    pub next: Option<Cursor>,
}
#[derive(Debug, Serialize)]
pub struct Details {
    pub book: Book,
    pub description: String,
    pub description_truncated: bool,
    pub publisher: Option<String>,
    pub page_count: Option<i64>,
    pub authors: Vec<String>,
    pub rule_systems: Vec<String>,
    pub source_product_types: Vec<String>,
    pub games: Vec<String>,
    pub captured_at: String,
}
pub const PAGE_SIZE: usize = 50;
