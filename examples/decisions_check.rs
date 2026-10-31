//! Explicit developer smoke test. Makes exactly one paid request with synthetic data.
//! Never opens the catalogue or prints credentials, inputs, or provider error bodies.
use kaijucrawl::decisions::{self, Answer};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let result = async {
        let client = decisions::Client::new()?;
        client
            .decide(&decisions::diagnostic(), &CancellationToken::new())
            .await
    }
    .await;
    match result {
        Ok(decision) => {
            let refusals = decision
                .answers
                .iter()
                .filter(|a| matches!(a, Answer::Refusal { .. }))
                .count();
            println!(
                "Decisions connected: {} typed answers, {refusals} refusals; {} input tokens ({} cached, {} cache write), {} output tokens.",
                decision.answers.len(),
                decision.usage.input_tokens,
                decision.usage.input_tokens_details.cached_tokens,
                decision.usage.input_tokens_details.cache_write_tokens,
                decision.usage.output_tokens
            );
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
