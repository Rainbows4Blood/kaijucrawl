//! Opt-in, four synthetic requests only; no catalogue or storefront access.
use kaijucrawl::{
    decisions::Client,
    dnd_context::{Data, Kind},
    dnd_settings,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cancel = CancellationToken::new();
    let evidence = json!({"title":"The Lanterns of Zakhara","description":"An Al-Qadim adventure set in Zakhara on Toril, designed for player characters of levels 3–5. The story takes place in the fictional city of Stonebridge, in the fictional land of Aster, circa 1492 DR."});
    let result = client
        .decide(&dnd_settings::request(evidence.clone()), &cancel)
        .await?;
    let settings = dnd_settings::answer(&result)?.ok_or("Synthetic settings refused")?;
    let matched = settings.members.contains(&"al_qadim")
        && settings.members.contains(&"forgotten_realms")
        && settings.category == "identified";
    println!(
        "Synthetic settings labels: category={}, members={:?}",
        settings.category, settings.members
    );
    report("settings", matched, &result.usage);
    if !matched {
        return Err("Synthetic settings did not match".into());
    }
    for kind in Kind::ALL {
        let result = kind
            .extract(&client, kind.encode(&evidence)?, &cancel)
            .await?;
        let matched = match result.data.as_ref() {
            Some(Data::Levels {
                min_level,
                max_level,
            }) => *min_level == Some(3) && *max_level == Some(5),
            Some(Data::Locations { locations }) => ["Stonebridge", "Aster"]
                .iter()
                .all(|n| locations.iter().any(|v| v == n)),
            Some(Data::Date { in_universe_date }) => in_universe_date
                .as_ref()
                .is_some_and(|s| s.contains("1492") && s.contains("DR")),
            None => false,
        };
        report(kind.key(), matched, &result.usage);
        if !matched {
            return Err("Synthetic extraction did not match".into());
        }
    }
    Ok(())
}
fn report(kind: &str, matched: bool, usage: &kaijucrawl::decisions::Usage) {
    println!(
        "Synthetic {kind}: expected_match={matched}, input={}, cached={}, cache_write={}, output={}",
        usage.input_tokens,
        usage.input_tokens_details.cached_tokens,
        usage.input_tokens_details.cache_write_tokens,
        usage.output_tokens
    );
}
