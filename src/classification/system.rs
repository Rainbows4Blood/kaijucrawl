//! Fixed-choice and multi-label systems share fenced storage.
use super::{Progress, answer};
use crate::{
    decisions,
    domain::{Error, Result},
    solo,
};

#[derive(Clone, Copy)]
pub enum System {
    BookRole,
    SoloPlay,
    Dnd,
    DndEditions,
    DndSettings,
    Osr,
    Warhammer(crate::warhammer::Family),
    EarthEditions(crate::earth_editions::Kind),
}
pub struct Selection {
    pub category: &'static str,
    pub confidence: f64,
    pub probabilities: serde_json::Value,
    pub members: Vec<&'static str>,
}
impl Selection {
    pub fn single(
        category: &'static str,
        confidence: f64,
        probabilities: serde_json::Value,
    ) -> Self {
        Self {
            category,
            confidence,
            probabilities,
            members: vec![category],
        }
    }
}
impl System {
    pub fn rule_system(self) -> Option<&'static str> {
        match self {
            Self::EarthEditions(kind) => Some(kind.scope()),
            _ => None,
        }
    }
    pub fn needs_dnd(self) -> bool {
        matches!(self, Self::DndEditions | Self::DndSettings)
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::BookRole => super::SYSTEM,
            Self::SoloPlay => solo::SYSTEM,
            Self::Dnd => crate::dnd::SYSTEM,
            Self::DndEditions => crate::dnd::EDITIONS_SYSTEM,
            Self::DndSettings => crate::dnd_settings::SYSTEM,
            Self::Osr => crate::osr::SYSTEM,
            Self::Warhammer(f) => f.key(),
            Self::EarthEditions(kind) => kind.system(),
        }
    }
    pub fn algorithm(self) -> &'static str {
        match self {
            Self::BookRole => super::ALGORITHM,
            Self::SoloPlay => solo::ALGORITHM,
            Self::Dnd => crate::dnd::ALGORITHM,
            Self::DndEditions => crate::dnd::EDITIONS_ALGORITHM,
            Self::DndSettings => crate::dnd_settings::ALGORITHM,
            Self::Osr => crate::osr::ALGORITHM,
            Self::Warhammer(f) => f.algorithm(),
            Self::EarthEditions(kind) => kind.algorithm(),
        }
    }
    pub fn from_algorithm(value: &str) -> Result<Self> {
        match value {
            super::ALGORITHM => Ok(Self::BookRole),
            solo::ALGORITHM => Ok(Self::SoloPlay),
            crate::dnd::ALGORITHM => Ok(Self::Dnd),
            crate::dnd::EDITIONS_ALGORITHM => Ok(Self::DndEditions),
            crate::dnd_settings::ALGORITHM => Ok(Self::DndSettings),
            crate::osr::ALGORITHM => Ok(Self::Osr),
            "shadowrun-editions-1" => {
                Ok(Self::EarthEditions(crate::earth_editions::Kind::Shadowrun))
            }
            "coc-editions-1" => Ok(Self::EarthEditions(crate::earth_editions::Kind::Coc)),
            a if crate::warhammer::Family::from_algorithm(a).is_some() => Ok(Self::Warhammer(
                crate::warhammer::Family::from_algorithm(a)
                    .ok_or_else(|| Error::Config("Unknown Warhammer family".into()))?,
            )),
            _ => Err(Error::Config("Unknown classification system".into())),
        }
    }
    pub fn answer(self, decision: &decisions::Decision) -> decisions::Result<Option<Selection>> {
        match self {
            Self::BookRole => {
                answer(decision).map(|a| a.map(|(c, p, v)| Selection::single(c.key(), p, v)))
            }
            Self::SoloPlay => {
                solo::answer(decision).map(|a| a.map(|(c, p, v)| Selection::single(c.key(), p, v)))
            }
            Self::Dnd => crate::dnd::answer(decision),
            Self::DndEditions => crate::dnd::editions_answer(decision),
            Self::DndSettings => crate::dnd_settings::answer(decision),
            Self::Osr => crate::osr::answer(decision),
            Self::Warhammer(f) => f.answer(decision),
            Self::EarthEditions(kind) => kind.answer(decision),
        }
    }
    pub fn count(self, category: &str, p: &mut Progress) -> Result<()> {
        match (self, category) {
            (Self::BookRole, "core_rulebook") => p.core_rulebook += 1,
            (Self::BookRole, "supplement") => p.supplement += 1,
            (Self::BookRole, "adventure") => p.adventure += 1,
            (Self::BookRole, "none") => p.none += 1,
            (Self::SoloPlay, "solo_adventure") => p.solo_adventure += 1,
            (Self::SoloPlay, "gamebook") => p.gamebook += 1,
            (Self::SoloPlay, "neither") => p.neither += 1,
            (Self::Dnd, "dnd_only") => p.dnd_only += 1,
            (Self::Dnd, "dnd_and_others") => p.dnd_and_others += 1,
            (Self::Dnd, "no") => p.dnd_no += 1,
            (Self::DndEditions | Self::EarthEditions(_), "identified") => p.editions_known += 1,
            (Self::DndEditions | Self::EarthEditions(_), "unknown") => p.editions_unknown += 1,
            (Self::Warhammer(_), "identified") => p.editions_known += 1,
            (Self::Warhammer(_), "unknown" | "uncertain" | "not_applicable") => {
                p.editions_unknown += 1
            }
            (Self::Osr, "identified") => p.osr_identified += 1,
            (Self::Osr, "not_osr") => p.osr_not_osr += 1,
            (Self::DndSettings, "identified") => p.settings_known += 1,
            (Self::DndSettings, "any") => p.settings_any += 1,
            (Self::DndSettings, "unknown") => p.settings_unknown += 1,
            _ => return Err(Error::Config("Category does not match its analysis".into())),
        }
        Ok(())
    }
}
