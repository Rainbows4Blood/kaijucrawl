//! Six explicit paid, fictional checks; no catalogue or storefront access.
use kaijucrawl::{decisions, dnd};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() {
    let result=async{
        let client=decisions::Client::new()?;let cancel=CancellationToken::new();let mut count=0;let mut tokens=0;
        for (expected,description) in [
            ("no","A fantasy adventure exclusively for Old-School Essentials, a B/X retroclone. Uses that game's rules and statistics. It supplies no original Dungeons & Dragons or Advanced Dungeons & Dragons support. The noisy storefront taxonomy says D&D."),
            ("dnd_only","A third-party Dungeons & Dragons adventure. Includes separate complete stat blocks and rules support for both the 2014 and 2024 fifth-edition D&D rules. It supports no other game system."),
            ("dnd_and_others","A scenario sold with two separate versions: one explicitly for actual Dungeons & Dragons 5th edition (2014 rules), and one for Pathfinder Second Edition. Both versions supply full game-specific statistics and rules references."),
        ]{
            let response=client.decide(&dnd::request(json!({"title":"Fictional compatibility example","description":description})),&cancel).await?;
            count+=1;tokens+=response.usage.input_tokens;let matched=dnd::answer(&response)?.is_some_and(|a|a.category==expected);
            println!("Synthetic request {count}: expected_match={matched}, input_tokens={}",response.usage.input_tokens);if !matched{return Err(decisions::Error::InvalidResponse);}
        }
        for (expected,description) in [
            (vec!["0e","bx","adnd_1e","adnd_2e","3e","3_5e","4e","5e_2014","5e_2024"],"A fictional nine-edition Dungeons & Dragons conversion compendium. Includes separate fully playable rules/stat blocks explicitly supporting each of: original D&D 0e; original D&D Basic/Expert (B/X); Advanced D&D 1st edition; Advanced D&D 2nd edition; D&D 3rd edition; D&D 3.5 edition; D&D 4th edition; D&D 5th edition 2014; and D&D 5th edition 2024. These are actual D&D editions, not retroclones. All nine versions are included."),
            (vec!["unknown"],"A third-party fantasy adventure explicitly designed for Dungeons & Dragons fifth edition. The saved description says only '5e compatible' and supplies no indication whether it targets the 2014 or 2024 rules; no other edition is mentioned."),
            (vec!["3_5e"],"A Dungeons & Dragons supplement exclusively for the revised 3.5 edition rules. It explicitly does not support the original 3.0/3rd edition, nor any other D&D edition or retroclone."),
        ]{
            let response=client.decide(&dnd::editions_request(json!({"title":"Fictional edition example","description":description})),&cancel).await?;
            count+=1;tokens+=response.usage.input_tokens;let matched=dnd::editions_answer(&response)?.is_some_and(|a|a.members==expected);
            println!("Synthetic request {count}: expected_match={matched}, input_tokens={}",response.usage.input_tokens);if !matched{return Err(decisions::Error::InvalidResponse);}
        }
        println!("Six synthetic requests passed; {tokens} reported input tokens.");Ok::<(),decisions::Error>(())
    }.await;
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
