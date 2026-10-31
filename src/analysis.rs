//! Versioned analysis policy. This module has no storage, network or UI dependencies.
use serde::{Deserialize, Serialize};

pub const ALGORITHM: &str = "weighted-rating-1";
pub const SCALE: i64 = 1_000_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub eligible: i64,
    pub completed: i64,
    pub prerequisite_pending: i64,
    pub counts: std::collections::BTreeMap<String, i64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Overview {
    pub catalogue_total: i64,
    pub analyses: std::collections::BTreeMap<String, Coverage>,
}
pub const DISCOVERY_ALGORITHMS: [&str; 4] = [
    ALGORITHM,
    crate::classification::ALGORITHM,
    crate::solo::ALGORITHM,
    crate::players::ALGORITHM,
];
impl Overview {
    pub fn discovery_complete(&self) -> bool {
        DISCOVERY_ALGORITHMS.iter().all(|key| {
            self.analyses.get(*key).is_some_and(|coverage| {
                coverage.completed >= coverage.eligible && coverage.prerequisite_pending == 0
            })
        })
    }
    pub fn require_discovery_complete(&self) -> crate::domain::Result<()> {
        if self.discovery_complete() {
            Ok(())
        } else {
            Err(crate::domain::Error::Config(
                "Complete all Discovery analyses for the current catalogue before running system analyses".into(),
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisWorkspace {
    #[default]
    Discovery,
    Dnd,
    Osr,
    Shadowrun,
    Coc,
    WarhammerFantasy,
    #[serde(rename = "warhammer_40k")]
    Warhammer40k,
}
impl AnalysisWorkspace {
    pub fn name(self) -> &'static str {
        match self {
            Self::Discovery => "Discovery",
            Self::Dnd => "Dungeons & Dragons",
            Self::Osr => "OSR",
            Self::Shadowrun => "Shadowrun",
            Self::Coc => "Call of Cthulhu",
            Self::WarhammerFantasy => "Warhammer Fantasy",
            Self::Warhammer40k => "Warhammer 40k",
        }
    }
}

/// Transient orchestration context; each stage retains its own durable run/history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryProgress {
    /// The existing snapshot field also carries system sequences; old values mean Discovery.
    #[serde(default)]
    pub workspace: AnalysisWorkspace,
    pub rerun_all: bool,
    pub step: usize,
    pub completed_steps: usize,
    pub unsuccessful: i64,
}

/// Twenty baseline ratings at 3/5, blended with the saved average and count.
/// Fixed baselines keep subset runs comparable to whole-catalogue runs.
pub fn relevance(rating: Option<f64>, count: Option<i64>) -> Option<i64> {
    let (rating, count) = (rating?, count?);
    if !rating.is_finite()
        || !(1.0..=5.0).contains(&rating)
        || !(1..=1_000_000_000).contains(&count)
    {
        return None;
    }
    Some((((rating * count as f64 + 60.0) / (count as f64 + 20.0)) * SCALE as f64).round() as i64)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<DiscoveryProgress>,
    /// Live committed catalogue totals; rebuilt at run start, never persisted in run history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<Overview>,
    #[serde(default)]
    pub classification: Option<crate::classification::Progress>,
    pub id: Option<i64>,
    pub algorithm: String,
    pub state: String,
    pub scope: String,
    pub query: Option<serde_json::Value>,
    pub total: i64,
    pub processed: i64,
    pub scored: i64,
    pub skipped: i64,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub detail: String,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            discovery: None,
            overview: None,
            classification: None,
            id: None,
            algorithm: ALGORITHM.into(),
            state: "idle".into(),
            scope: "all".into(),
            query: None,
            total: 0,
            processed: 0,
            scored: 0,
            skipped: 0,
            started_at: None,
            finished_at: None,
            detail: String::new(),
        }
    }
}
impl Status {
    pub fn active(&self) -> bool {
        matches!(self.state.as_str(), "preparing" | "running" | "stopping")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Score {
    pub value: Option<f64>,
    pub algorithm: String,
    pub rating: Option<f64>,
    pub count: Option<i64>,
    pub calculated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_gate_requires_current_coverage_and_resolved_eligibility() {
        let mut overview = Overview::default();
        assert!(!overview.discovery_complete());
        for algorithm in DISCOVERY_ALGORITHMS {
            overview
                .analyses
                .insert(algorithm.into(), Coverage::default());
        }
        assert!(overview.discovery_complete());
        overview.catalogue_total = 1;
        for coverage in overview.analyses.values_mut() {
            coverage.eligible = 1;
            coverage.completed = 1;
            coverage.counts.insert("unknown".into(), 1);
        }
        assert!(overview.discovery_complete());
        for algorithm in DISCOVERY_ALGORITHMS {
            overview.analyses.get_mut(algorithm).unwrap().completed = 0;
            assert!(!overview.discovery_complete());
            overview.analyses.get_mut(algorithm).unwrap().completed = 1;
        }
        overview
            .analyses
            .get_mut(crate::players::ALGORITHM)
            .unwrap()
            .prerequisite_pending = 1;
        assert!(!overview.discovery_complete());
    }
    #[test]
    fn weighted_rating_uses_evidence_and_rejects_unusable_inputs() {
        assert_eq!(relevance(Some(4.7), Some(1000)), Some(4_666_667));
        assert_eq!(relevance(Some(5.0), Some(3)), Some(3_260_870));
        assert_eq!(relevance(Some(3.0), Some(100)), Some(3_000_000));
        assert!(relevance(Some(4.7), Some(1000)) > relevance(Some(5.0), Some(3)));
        for rating in [
            None,
            Some(0.0),
            Some(6.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(relevance(rating, Some(3)), None);
        }
        for count in [None, Some(0), Some(-1), Some(1_000_000_001)] {
            assert_eq!(relevance(Some(5.0), count), None);
        }
    }
}
