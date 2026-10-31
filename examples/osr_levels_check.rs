//! Opt-in: two fictional-book Responses requests, no retries or catalogue access.
use kaijucrawl::{decisions::Client, osr_levels, responses};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cases = [
        (
            "The Fictional Copper Vault",
            "This fictional adventure has separate rules versions: Old-School Essentials is for character levels 3–5; Dungeon Crawl Classics is a level-zero funnel; Cairn uses level-free characters with no numbered levels. The Swords & Wizardry version has no stated level range. Additional OSR versions: Basic Fantasy RPG for levels 5–7 and Knave for levels 1–2. These are separate versions, not conversions to be inferred.",
            vec![
                "old_school_essentials",
                "dungeon_crawl_classics",
                "cairn",
                "swords_wizardry",
                "other",
            ],
            vec![
                ("cairn", None, "not_applicable", None, None),
                ("dungeon_crawl_classics", None, "known", Some(0), Some(0)),
                ("old_school_essentials", None, "known", Some(3), Some(5)),
                (
                    "other",
                    Some("Basic Fantasy RPG"),
                    "known",
                    Some(5),
                    Some(7),
                ),
                ("other", Some("Knave"), "known", Some(1), Some(2)),
                ("swords_wizardry", None, "unknown", None, None),
            ],
        ),
        (
            "The Fictional Paper Tower",
            "Two separate versions of this fictional adventure: OSRIC for characters level 4 and up; Swords & Wizardry for characters up to level 7. The booklet has 6 pages and expects 3–5 players. Those numbers are not character levels. No other system is supported.",
            vec!["osric", "swords_wizardry"],
            vec![
                ("osric", None, "known", Some(4), None),
                ("swords_wizardry", None, "known", None, Some(7)),
            ],
        ),
    ];
    for (i, (title, description, scope, expected)) in cases.into_iter().enumerate() {
        let evidence = json!({"book":{"title":title,"description":description,"rule_systems":["Old-School Revival (OSR)"]},"requested_systems":scope});
        let bytes = osr_levels::encode(&evidence)?;
        let response = osr_levels::decode_response(
            responses::extract(&client, bytes, &CancellationToken::new()).await?,
            &osr_levels::systems(&evidence)?,
        )?;
        let data = response.data.ok_or("Synthetic response refused")?;
        let actual = data
            .ranges
            .iter()
            .map(|r| {
                (
                    r.system.as_str(),
                    r.game.as_deref(),
                    r.status.key(),
                    r.min_level,
                    r.max_level,
                )
            })
            .collect::<Vec<_>>();
        let matched = actual == expected;
        println!(
            "Synthetic OSR levels {}: expected_match={matched}, ranges={}, input={}, cached={}, cache_write={}, output={}",
            i + 1,
            data.ranges.len(),
            response.usage.input_tokens,
            response.usage.input_tokens_details.cached_tokens,
            response.usage.input_tokens_details.cache_write_tokens,
            response.usage.output_tokens
        );
        if !matched {
            return Err("Synthetic per-system levels did not match".into());
        }
    }
    Ok(())
}
