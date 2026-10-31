//! Explicit paid developer check: four fictional books, no catalogue access.
use kaijucrawl::{
    classification::{self, Category},
    decisions,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() {
    let result=async {
        let client=decisions::Client::new()?;let cancel=CancellationToken::new();let mut tokens=0;let mut matches=0;
        for (expected,title,description,misleading_type) in [
            (Category::CoreRulebook,"Lanterns: Player's Rules","The essential player rulebook for the fictional Lanterns tabletop roleplaying game. Contains all character creation, action resolution and combat rules needed to play. The GM uses a separate essential referee book. A short introductory scenario is included.","Adventures"),
            (Category::Supplement,"Lanterns: A Hundred Paths","An optional expansion for the Lanterns game. Offers new character professions, talents, spells and alternative game-master rules. Requires the existing core rulebook. Includes no playable adventure or story.","Core Rules"),
            (Category::Adventure,"The Clockwork Abbey","A complete ready-to-play mystery adventure for four Lanterns characters. Includes a prepared story, locations, NPC motivations, scenes, encounters and possible endings. The two new spells support this adventure; this is not a core rulebook.","Rule Supplements"),
            (Category::None,"Portraits of the Distant Isles","A digital pack of 30 character portrait illustrations for personal use. Contains only PNG image files. No game rules, player options, scenario, story or adventure text.","Adventures"),
        ] {
            let decision=client.decide(&classification::request(json!({"title":title,"description":description,"source_product_types":[misleading_type],"languages":["English"]})),&cancel).await?;
            tokens+=decision.usage.input_tokens;
            let actual=classification::answer(&decision)?.map(|v|v.0);
            matches+=usize::from(actual==Some(expected));
            println!("Synthetic {}: {}",expected.key(),actual.map_or("refused",Category::key));
        }
        println!("Four requests; {matches}/4 expected primary roles; {tokens} reported input tokens.");
        if matches!=4 {return Err(decisions::Error::InvalidResponse);}Ok::<(),decisions::Error>(())
    }.await;
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
