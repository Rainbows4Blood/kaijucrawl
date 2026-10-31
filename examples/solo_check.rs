//! Explicit paid developer check: four fictional products, never catalogue data.
use kaijucrawl::{
    decisions,
    solo::{self, Category},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() {
    let result = async {
        let client=decisions::Client::new()?;let cancel=CancellationToken::new();let mut tokens=0;
        for (index,(expected,title,description)) in [
            (Category::SoloAdventure,"The Lantern Gate","A prepared solo adventure for one player, without a GM. Make choices through numbered scenes and explore a ruined city. You must own and use the separate Lanterns RPG core rulebook for character creation and combat; this scenario does not include those rules."),
            (Category::Gamebook,"The Silver Orchard","A complete self-contained solo gamebook. Choose routes through a branching narrative and resolve encounters with the simple rules included here. All character and combat rules are supplied. Play alone without a GM or any other rulebook; only a pencil and two dice are needed. The book also offers optional compatibility notes for the Lanterns RPG."),
            (Category::Neither,"The Two Travelers","A ready-to-play duet adventure requiring one player and a separate game master. The GM runs the mystery and plays its characters. Requires the Lanterns RPG core rules. There is no GM-free solo mode."),
            (Category::Neither,"The Lonely Oracle","A generic solo roleplaying toolkit with oracle tables, random prompts and advice for using any existing RPG alone. It contains no prepared adventure, playable story or self-contained narrative experience."),
        ].into_iter().enumerate() {
            let response=client.decide(&solo::request(json!({"title":title,"description":description})),&cancel).await?;
            let matched=solo::answer(&response)?.is_some_and(|a|a.0==expected);tokens+=response.usage.input_tokens;
            println!("Synthetic request {}: expected_match={matched}, input_tokens={}",index+1,response.usage.input_tokens);
            if !matched {return Err(decisions::Error::InvalidResponse);}
        }
        println!("Four synthetic requests passed; {tokens} reported input tokens.");Ok::<(),decisions::Error>(())
    }.await;
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
