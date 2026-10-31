//! Concrete online analyses share one bounded, cancellable coordinator.
use crate::{
    analysis::Status,
    classification, decisions,
    domain::{Error, Result},
    players,
    storage::{classification::Candidate, sqlite::Catalogue},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub(in crate::controller) enum Kind {
    BookRole,
    SoloPlay,
    Dnd,
    DndEditions,
    DndSettings,
    Osr,
    OsrLevels,
    Warhammer(crate::warhammer::Family),
    WarhammerContext(crate::warhammer_context::Kind),
    EarthEditions(crate::earth_editions::Kind),
    DndContext(crate::dnd_context::Kind),
    EarthContext(crate::earth_context::Kind),
    Players,
}
pub(super) enum Request {
    Decision(decisions::Request),
    WarhammerDecision(crate::warhammer::Family, decisions::Request),
    Players(Vec<u8>),
    Context(crate::dnd_context::Kind, Vec<u8>),
    Warhammer(crate::warhammer_context::Kind, Vec<u8>),
    Earth(crate::earth_context::Kind, Vec<u8>),
    OsrLevels(Vec<String>, Vec<u8>),
}
pub(super) enum Response {
    Decision(decisions::Decision),
    Players(players::Response),
    Context(crate::dnd_context::Response),
    Earth(crate::earth_context::Response),
    OsrLevels(crate::osr_levels::Response),
}
impl Kind {
    pub fn requires_discovery(self) -> bool {
        !matches!(self, Self::BookRole | Self::SoloPlay | Self::Players)
    }
    pub fn algorithm(self) -> &'static str {
        match self {
            Self::BookRole => classification::ALGORITHM,
            Self::SoloPlay => crate::solo::ALGORITHM,
            Self::Dnd => crate::dnd::ALGORITHM,
            Self::DndEditions => crate::dnd::EDITIONS_ALGORITHM,
            Self::DndSettings => crate::dnd_settings::ALGORITHM,
            Self::Osr => crate::osr::ALGORITHM,
            Self::OsrLevels => crate::osr_levels::ALGORITHM,
            Self::Warhammer(f) => f.algorithm(),
            Self::WarhammerContext(k) => k.algorithm(),
            Self::EarthEditions(kind) => kind.algorithm(),
            Self::DndContext(kind) => kind.algorithm(),
            Self::EarthContext(kind) => kind.algorithm(),
            Self::Players => players::ALGORITHM,
        }
    }
    pub async fn begin(self, db: &Catalogue, again: bool) -> Result<Status> {
        match self {
            Self::BookRole => db.begin_classification(again).await,
            Self::SoloPlay => db.begin_solo_classification(again).await,
            Self::Dnd => db.begin_dnd(false, again).await,
            Self::DndEditions => db.begin_dnd(true, again).await,
            Self::DndSettings => db.begin_settings(again).await,
            Self::Osr => db.begin_osr(again).await,
            Self::OsrLevels => db.begin_osr_levels(again).await,
            Self::Warhammer(f) => db.begin_warhammer(f, again).await,
            Self::WarhammerContext(k) => db.begin_warhammer_context(k, again).await,
            Self::EarthEditions(kind) => db.begin_earth_editions(kind, again).await,
            Self::DndContext(kind) => db.begin_context(kind, again).await,
            Self::EarthContext(kind) => db.begin_earth(kind, again).await,
            Self::Players => db.begin_players(again).await,
        }
    }
    pub async fn next(self, db: &Catalogue, s: &Status, after: i64) -> Result<Option<Candidate>> {
        match self {
            Self::BookRole
            | Self::SoloPlay
            | Self::Dnd
            | Self::DndEditions
            | Self::DndSettings
            | Self::Osr
            | Self::Warhammer(_)
            | Self::EarthEditions(_) => db.classification_next_after(s, after).await,
            Self::Players => db.players_next_after(s, after).await,
            Self::OsrLevels => db.osr_levels_next_after(s, after).await,
            Self::DndContext(_) => db.context_next_after(s, after).await,
            Self::EarthContext(_) => db.earth_next_after(s, after).await,
            Self::WarhammerContext(_) => db.warhammer_context_next_after(s, after).await,
        }
    }
    pub(super) fn request(self, evidence: serde_json::Value) -> decisions::Result<Request> {
        match self {
            Self::BookRole
            | Self::SoloPlay
            | Self::Dnd
            | Self::DndEditions
            | Self::DndSettings
            | Self::Osr
            | Self::Warhammer(_)
            | Self::EarthEditions(_) => {
                let r = match self {
                    Self::BookRole => classification::request(evidence),
                    Self::Dnd => crate::dnd::request(evidence),
                    Self::DndEditions => crate::dnd::editions_request(evidence),
                    Self::DndSettings => crate::dnd_settings::request(evidence),
                    Self::Osr => crate::osr::request(evidence),
                    Self::Warhammer(f) => f.request(evidence),
                    Self::EarthEditions(kind) => kind.request(evidence),
                    _ => crate::solo::request(evidence),
                };
                r.validate()?;
                if let Self::Warhammer(family) = self {
                    Ok(Request::WarhammerDecision(family, r))
                } else {
                    Ok(Request::Decision(r))
                }
            }
            Self::Players => players::encode(&evidence).map(Request::Players),
            Self::WarhammerContext(k) => k.encode(&evidence).map(|b| Request::Warhammer(k, b)),
            Self::OsrLevels => Ok(Request::OsrLevels(
                crate::osr_levels::systems(&evidence)?,
                crate::osr_levels::encode(&evidence)?,
            )),
            Self::DndContext(kind) => kind.encode(&evidence).map(|b| Request::Context(kind, b)),
            Self::EarthContext(kind) => kind.encode(&evidence).map(|b| Request::Earth(kind, b)),
        }
    }
    pub(super) async fn commit(
        self,
        db: &Catalogue,
        s: &mut Status,
        b: &Candidate,
        response: Option<&Response>,
        failure: Option<&str>,
    ) -> Result<()> {
        match (self, response) {
            (
                Self::BookRole
                | Self::SoloPlay
                | Self::Dnd
                | Self::DndEditions
                | Self::DndSettings
                | Self::Osr
                | Self::Warhammer(_)
                | Self::EarthEditions(_),
                Some(Response::Decision(r)),
            ) => db.classification_commit(s, b, Some(r), failure).await,
            (Self::Players, Some(Response::Players(r))) => {
                db.players_commit(s, b, Some(r), failure).await
            }
            (
                Self::BookRole
                | Self::SoloPlay
                | Self::Dnd
                | Self::DndEditions
                | Self::DndSettings
                | Self::Osr
                | Self::Warhammer(_)
                | Self::EarthEditions(_),
                None,
            ) => db.classification_commit(s, b, None, failure).await,
            (Self::WarhammerContext(_), Some(Response::Context(r))) => {
                db.warhammer_context_commit(s, b, Some(r), failure).await
            }
            (Self::WarhammerContext(_), None) => {
                db.warhammer_context_commit(s, b, None, failure).await
            }
            (Self::Players, None) => db.players_commit(s, b, None, failure).await,
            (Self::OsrLevels, Some(Response::OsrLevels(r))) => {
                db.osr_levels_commit(s, b, Some(r), failure).await
            }
            (Self::OsrLevels, None) => db.osr_levels_commit(s, b, None, failure).await,
            (Self::DndContext(_), Some(Response::Context(r))) => {
                db.context_commit(s, b, Some(r), failure).await
            }
            (Self::DndContext(_), None) => db.context_commit(s, b, None, failure).await,
            (Self::EarthContext(_), Some(Response::Earth(r))) => {
                db.earth_commit(s, b, Some(r), failure).await
            }
            (Self::EarthContext(_), None) => db.earth_commit(s, b, None, failure).await,
            _ => Err(Error::Config(
                "Analysis result does not match its run".into(),
            )),
        }
    }
}
impl Request {
    pub fn model(&self) -> decisions::Result<Option<crate::responses::Model>> {
        match self {
            Self::Decision(_) | Self::WarhammerDecision(_, _) => Ok(None),
            Self::Players(bytes)
            | Self::Warhammer(_, bytes)
            | Self::Context(_, bytes)
            | Self::Earth(_, bytes)
            | Self::OsrLevels(_, bytes) => crate::responses::request_model(bytes).map(Some),
        }
    }
    pub fn escalate(&mut self) -> decisions::Result<Option<crate::responses::Model>> {
        match self {
            Self::Decision(_) | Self::WarhammerDecision(_, _) => Ok(None),
            Self::Players(bytes)
            | Self::Warhammer(_, bytes)
            | Self::Context(_, bytes)
            | Self::Earth(_, bytes)
            | Self::OsrLevels(_, bytes) => crate::responses::escalate(bytes),
        }
    }
    pub async fn send(
        &self,
        client: &decisions::Client,
        cancel: &CancellationToken,
    ) -> std::result::Result<Response, crate::responses::Failure> {
        match self {
            Self::WarhammerDecision(family, r) => {
                let decision = client.decide(r, cancel).await?;
                if let Err(error) = family.answer(&decision) {
                    return Err(crate::responses::Failure {
                        error,
                        usage: Some(decision.usage),
                    });
                }
                Ok(Response::Decision(decision))
            }
            Self::Decision(r) => client
                .decide(r, cancel)
                .await
                .map(Response::Decision)
                .map_err(Into::into),
            Self::Players(bytes) => {
                crate::responses::attempt(client, bytes.clone(), cancel, players::from_response)
                    .await
                    .map(Response::Players)
            }
            Self::Warhammer(kind, bytes) => {
                crate::responses::attempt(client, bytes.clone(), cancel, |r| {
                    kind.decode_response(r)
                })
                .await
                .map(Response::Context)
            }
            Self::Context(kind, bytes) => {
                crate::responses::attempt(client, bytes.clone(), cancel, |r| {
                    kind.decode_response(r)
                })
                .await
                .map(Response::Context)
            }
            Self::OsrLevels(systems, bytes) => {
                crate::responses::attempt(client, bytes.clone(), cancel, |r| {
                    crate::osr_levels::decode_response(r, systems)
                })
                .await
                .map(Response::OsrLevels)
            }
            Self::Earth(kind, bytes) => {
                crate::responses::attempt(client, bytes.clone(), cancel, |r| {
                    kind.decode_response(r)
                })
                .await
                .map(Response::Earth)
            }
        }
    }
}
