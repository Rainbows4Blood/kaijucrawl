//! Opt-in: two synthetic Decisions requests, no retries or catalogue access.
use kaijucrawl::{decisions::Client, osr};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cancel = CancellationToken::new();
    let cases = [
        (
            "The Ink Vault",
            "A fictional adventure with separate complete stat blocks for Old-School Essentials, Swords & Wizardry and Knave. All three game systems are expressly supported. No other rules versions are included.",
            vec!["swords_wizardry", "old_school_essentials", "other"],
        ),
        (
            "Neon Commuters",
            "A fictional modern urban narrative RPG using its own unrelated bespoke rules. This is not an OSR game and includes no compatibility with OSR systems. The store's OSR metadata tag was attached in error. An essay lists Cairn, Into the Odd, Shadowdark and Vagabond // Pulp Fantasy RPG as inspirations for the author's writing; these are comparisons only. This product requires none of those games and includes no compatible rules or stat blocks for them.",
            vec!["not_osr"],
        ),
    ];
    for (index, (title, description, expected)) in cases.into_iter().enumerate() {
        let evidence = json!({"title":title,"description":description,"rule_systems":["Old-School Revival (OSR)"]});
        let response = client.decide(&osr::request(evidence), &cancel).await?;
        let result = osr::answer(&response)?.ok_or("Synthetic book refused")?;
        let matched = result.members == expected;
        println!(
            "Synthetic OSR request {}: expected_match={matched}, members={:?}, input={}, cached={}, cache_write={}, output={}",
            index + 1,
            result.members,
            response.usage.input_tokens,
            response.usage.input_tokens_details.cached_tokens,
            response.usage.input_tokens_details.cache_write_tokens,
            response.usage.output_tokens
        );
        if !matched {
            return Err("Synthetic OSR classification did not match".into());
        }
    }
    Ok(())
}
