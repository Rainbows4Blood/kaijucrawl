//! Explicit validation with four fictional descriptions. Never opens the catalogue.
use kaijucrawl::{
    decisions::Client,
    players::{self, Counts},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cases = [
        (
            "A fictional mystery for 3–5 players plus a game master.",
            Some(3),
            Some(5),
        ),
        (
            "A fictional adventure for characters of levels 3–5. Includes six optional pregenerated heroes.",
            None,
            None,
        ),
        (
            "A solo fictional choose-your-own-path adventure, played alone without a GM.",
            Some(1),
            Some(1),
        ),
        (
            "A fictional expedition for at least three players; no upper count is stated.",
            Some(3),
            None,
        ),
    ];
    for (i, (description, min, max)) in cases.into_iter().enumerate() {
        let response = players::extract(
            &client,
            players::encode(
                &json!({"title":"Fictional player-count validation","description":description}),
            )?,
            &CancellationToken::new(),
        )
        .await?;
        println!(
            "Synthetic request {}: expected_match={}, input={}, cached={}, cache_write={}, output={}",
            i + 1,
            response.counts
                == Some(Counts {
                    min_players: min,
                    max_players: max
                }),
            response.usage.input_tokens,
            response.usage.input_tokens_details.cached_tokens,
            response.usage.input_tokens_details.cache_write_tokens,
            response.usage.output_tokens
        );
        if response.counts
            != Some(Counts {
                min_players: min,
                max_players: max,
            })
        {
            return Err("Synthetic extraction did not match expected range".into());
        }
    }
    Ok(())
}
