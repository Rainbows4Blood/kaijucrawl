//! Opt-in: at most four synthetic Decisions requests, no retries or catalogue access.
use kaijucrawl::{decisions::Client, earth_editions::Kind};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cases = [
        (
            Kind::Coc,
            "The Glass Lighthouse",
            "A fictional Call of Cthulhu scenario. Includes separate statistics for Call of Cthulhu 6th and 7th editions, supporting both rules editions explicitly.",
            vec!["6e", "7e"],
        ),
        (
            Kind::Coc,
            "Village Mystery, Third Printing",
            "The third edition of this fictional scenario is set in the Dark Ages. It is for Call of Cthulhu, but does not specify which edition of the parent game rules it uses. Third edition refers only to this scenario's revision, not the rules.",
            vec![],
        ),
        (
            Kind::Shadowrun,
            "The Glass Datavault",
            "A fictional Shadowrun adventure with two separate rules versions included: numbered Shadowrun 6th edition and original first-edition Shadowrun Anarchy. Anarchy 2.0 and other numbered Shadowrun editions are not supported.",
            vec!["6e", "anarchy"],
        ),
        (
            Kind::Shadowrun,
            "Neon Clockwork",
            "A fictional adventure written exclusively for Shadowrun Anarchy 2.0, the revised Anarchy rules. Not for original Anarchy or numbered Shadowrun editions 1 through 6. The story takes place in the Sixth World.",
            vec!["anarchy_2"],
        ),
    ];
    let cancel = CancellationToken::new();
    for (kind, title, description, expected) in cases {
        let evidence = json!({"title":title,"description":description,"rule_systems":[if kind==Kind::Coc {"Call of Cthulhu"} else {"Shadowrun"}]});
        let response = client.decide(&kind.request(evidence), &cancel).await?;
        let result = kind.answer(&response)?.ok_or("Synthetic book refused")?;
        let members: Vec<_> = result
            .members
            .into_iter()
            .filter(|v| *v != "unknown")
            .collect();
        let matched = members == expected;
        println!(
            "Synthetic {}: expected_match={matched}, members={members:?}, input={}, cached={}, cache_write={}, output={}",
            kind.scope(),
            response.usage.input_tokens,
            response.usage.input_tokens_details.cached_tokens,
            response.usage.input_tokens_details.cache_write_tokens,
            response.usage.output_tokens
        );
        if !matched {
            return Err("Synthetic edition result did not match".into());
        }
    }
    Ok(())
}
