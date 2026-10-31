//! Opt-in: four synthetic Responses requests, one per configured tier. No retries/catalogue.
use kaijucrawl::{
    decisions::Client,
    earth_context::Kind,
    responses::{self, Model},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    client.check_credentials()?;
    let kind = Kind::ShadowrunDate;
    let mut bytes=kind.encode(&json!({"title":"Synthetic Night Train","description":"A fictional Shadowrun adventure set in the late 2050s, specifically spanning the years 2055 through 2059."}))?;
    for model in Model::ALL {
        let response = kind
            .extract(&client, bytes.clone(), &CancellationToken::new())
            .await?;
        let matched = matches!(&response.data,Some(kaijucrawl::earth_context::Data::Date{range}) if range.start.is_some_and(|d|d.to_string()=="2055-01-01") && range.end.is_some_and(|d|d.to_string()=="2059-12-31") && range.precision==kaijucrawl::earth_context::Precision::Range);
        println!(
            "Synthetic {}: expected_match={matched}, input={}, cached={}, cache_write={}, output={}",
            response.model,
            response.usage.input_tokens,
            response.usage.input_tokens_details.cached_tokens,
            response.usage.input_tokens_details.cache_write_tokens,
            response.usage.output_tokens
        );
        if !matched {
            return Err("Synthetic date result did not match".into());
        }
        assert_eq!(responses::escalate(&mut bytes)?, model.next());
    }
    Ok(())
}
