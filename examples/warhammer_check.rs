//! Opt-in: six synthetic requests at most; no catalogue access or retries.
use kaijucrawl::{
    decisions::Client, dnd_context::Data, warhammer::Family, warhammer_context::Kind,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cancel = CancellationToken::new();
    for (f, description, expected) in [
        (
            Family::Fantasy,
            "This fictional Warhammer Fantasy Roleplay adventure includes separate stat blocks explicitly for WFRP 2nd and 4th editions. It is set in the Old World, but is not for the separate The Old World Roleplaying Game. No other game or edition is supported.",
            vec!["wfrp_2e", "wfrp_4e"],
        ),
        (
            Family::FortyK,
            "This fictional Warhammer 40,000 adventure explicitly supports the Rogue Trader roleplaying game and Dark Heresy Second Edition, with separate stat blocks. It does not support the original Dark Heresy RPG, any miniatures wargame or other game.",
            vec!["dark_heresy_2e", "rogue_trader"],
        ),
    ] {
        let r=client.decide(&f.request(json!({"title":"Fictional compatibility check","description":description,"rule_systems":["Warhammer"]})),&cancel).await?;
        let answer = f.answer(&r)?.ok_or("Synthetic refusal")?;
        let matched = answer.members == expected;
        println!(
            "{} games: expected_match={matched}; input={}; output={}; cached={}; cache_write={}",
            f.name(),
            r.usage.input_tokens,
            r.usage.output_tokens,
            r.usage.input_tokens_details.cached_tokens,
            r.usage.input_tokens_details.cache_write_tokens
        );
        if !matched {
            return Err("Synthetic game categories did not match".into());
        }
    }
    for (kind, description, expected) in [
        (
            Kind::FantasyLocations,
            "A fictional WFRP adventure set in Altdorf and the Reikwald. The publisher is based in London.",
            json!(["Altdorf", "Reikwald"]),
        ),
        (
            Kind::FantasyDate,
            "A fictional WFRP adventure set in autumn 2512 IC. Published in 2026, revised for WFRP 5th edition.",
            json!("autumn 2512 IC"),
        ),
        (
            Kind::FortyKLocations,
            "A fictional 40k RPG adventure takes place on Scintilla in the Calixis Sector. No other location is named.",
            json!(["Scintilla", "Calixis Sector"]),
        ),
        (
            Kind::FortyKDate,
            "A fictional 40k RPG adventure takes place during 123.456.M41–125.001.M41. Published in 2026.",
            json!("123.456.M41–125.001.M41"),
        ),
    ] {
        let evidence = json!({"title":"Fictional context check","description":description});
        let r = kind
            .extract(&client, kind.encode(&evidence)?, &cancel)
            .await?;
        let data = r.data.ok_or("Synthetic refusal")?;
        let matched = match data {
            Data::Locations { locations } => {
                let mut actual = locations;
                actual.sort();
                let mut expected: Vec<String> = serde_json::from_value(expected)?;
                expected.sort();
                actual == expected
            }
            Data::Date { in_universe_date } => in_universe_date.as_deref() == expected.as_str(),
            _ => false,
        };
        println!(
            "{}: expected_match={matched}; input={}; output={}; cached={}; cache_write={}",
            kind.algorithm(),
            r.usage.input_tokens,
            r.usage.output_tokens,
            r.usage.input_tokens_details.cached_tokens,
            r.usage.input_tokens_details.cache_write_tokens
        );
        if !matched {
            return Err("Synthetic context did not match".into());
        }
    }
    Ok(())
}
