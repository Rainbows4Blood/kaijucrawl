//! Opt-in: at most six synthetic requests, no retries, catalogue or storefront access.
use kaijucrawl::{
    decisions::Client,
    earth_context::{Data, Kind},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let cases = [
        (
            Kind::ShadowrunLocation,
            "A Shadowrun adventure set in Seattle.",
            "Seattle",
            None,
        ),
        (
            Kind::CocLocation,
            "A Call of Cthulhu investigation set in Arkham, Massachusetts.",
            "Arkham",
            None,
        ),
        (
            Kind::ShadowrunDate,
            "A Shadowrun adventure set in the year 2084.",
            "2084-01-01",
            Some("2084-12-31"),
        ),
        (
            Kind::CocDate,
            "A Call of Cthulhu investigation set in the 1920s.",
            "1920-01-01",
            Some("1929-12-31"),
        ),
        (
            Kind::CocDate,
            "A Call of Cthulhu investigation set in the modern day.",
            "1980-01-01",
            Some("2030-12-31"),
        ),
        (
            Kind::CocDate,
            "A Call of Cthulhu investigation set in the Dark Ages.",
            "0950-01-01",
            Some("1050-12-31"),
        ),
    ];
    for (i, (kind, description, expected, end)) in cases.into_iter().enumerate() {
        let bytes=kind.encode(&json!({"title":"The Silent Lantern","description":description,"rule_systems":[if kind.system()=="coc"{"Call of Cthulhu"}else{"Shadowrun"}]}))?;
        let response = kind
            .extract(&client, bytes, &CancellationToken::new())
            .await?;
        let matched = match response.data {
            Some(Data::Location { location: Some(s) }) => s.contains(expected),
            Some(Data::Date { range }) => {
                range.start.is_some_and(|d| d.to_string() == expected)
                    && range.end.map(|d| d.to_string()).as_deref() == end
            }
            _ => false,
        };
        println!(
            "Synthetic request {}: matched={}, input={}, cached={}, cache_write={}, output={}",
            i + 1,
            matched,
            response.usage.input_tokens,
            response.usage.input_tokens_details.cached_tokens,
            response.usage.input_tokens_details.cache_write_tokens,
            response.usage.output_tokens
        );
        if !matched {
            return Err("Synthetic result did not match expected extraction".into());
        }
    }
    Ok(())
}
