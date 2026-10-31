//! Three explicit fictional edition regression checks, no catalogue access.
use kaijucrawl::{decisions, dnd};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() {
    let result=async{
  let client=decisions::Client::new()?;let cancel=CancellationToken::new();let mut tokens=0;
  for (index,(expected,description)) in [
   (vec!["3_5e"],"A Dungeons & Dragons supplement exclusively for the revised 3.5 edition rules. It explicitly does not support the original 3.0/3rd edition, nor any other D&D edition or retroclone."),
   (vec!["unknown"],"A third-party fantasy adventure explicitly designed for Dungeons & Dragons fifth edition. The saved description says only '5e compatible' and supplies no indication whether it targets the 2014 or 2024 rules; no other edition is mentioned."),
   (vec!["0e","bx","adnd_1e","adnd_2e","3e","3_5e","4e","5e_2014","5e_2024"],"A fictional nine-edition Dungeons & Dragons conversion compendium. Includes separate fully playable rules/stat blocks explicitly supporting each of: original D&D 0e; original D&D Basic/Expert (B/X); Advanced D&D 1st edition; Advanced D&D 2nd edition; D&D 3rd edition; D&D 3.5 edition; D&D 4th edition; D&D 5th edition 2014; and D&D 5th edition 2024. These are actual D&D editions, not retroclones. All nine versions are included.")
  ].into_iter().enumerate(){
   let r=client.decide(&dnd::editions_request(json!({"title":"Fictional edition example","description":description})),&cancel).await?;
   let a=dnd::editions_answer(&r)?.ok_or(decisions::Error::InvalidResponse)?;tokens+=r.usage.input_tokens;let matched=a.members==expected;
   println!("Edition regression {}: expected_match={matched}, selected={:?}, input_tokens={}",index+1,a.members,r.usage.input_tokens);
   if !matched{return Err(decisions::Error::InvalidResponse);}
  }
  println!("Three edition regressions passed; {tokens} input tokens.");Ok::<(),decisions::Error>(())
 }.await;
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
