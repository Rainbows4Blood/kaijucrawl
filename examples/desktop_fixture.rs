//! Native UI smoke fixture. All files are disposable; no production settings are read.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let paths = kaijucrawl::paths::LocalPaths::at(temp.path());
    let config = kaijucrawl::config::Config {
        selected: false,
        ..Default::default()
    };
    config.save_at(&paths)?;
    tokio::runtime::Runtime::new()?.block_on(seed(&paths))?;
    kaijucrawl::desktop::run_at(paths, false)
}

async fn seed(paths: &kaijucrawl::paths::LocalPaths) -> kaijucrawl::domain::Result<()> {
    use kaijucrawl::{
        config::RuntimeSettings,
        domain::Intent,
        storage::{archive::Archives, sqlite::Catalogue},
        targets::{drivethru as d, sitemap::Entry},
    };
    use serde_json::json;
    let db = Catalogue::connect(paths).await?;
    let archives = Archives::new(paths);
    let run = db.new_run(&RuntimeSettings::default(), "crawl").await?;
    for (n, title) in [
        "Atlas of the Wandering Isles",
        "Beneath the Copper Moon",
        "Citadels and Colossi",
        "Field Guide to Impossible Creatures",
        "The Last Lighthouse",
        "Voyages Beyond the Known Sea",
    ]
    .into_iter()
    .enumerate()
    {
        let id = (42 + n).to_string();
        db.discover(
            run,
            d::ROOT,
            vec![Entry {
                location: d::canonical(&id),
                lastmod: None,
                sitemap: false,
            }],
        )
        .await?;
        let body = serde_json::to_vec(
            &json!({"id":id,"name":title,"description":"A complete synthetic setting of drifting islands, ancient ruins and curious travellers.\n\nIncludes locations, encounters and tools for a fantasy campaign. This book is a local test fixture.","publisher":{"name":"Synthetic Press"},"authors":["Alex Rowan"],"price":"12.000000000000000001","currency":"USD","pagecount":128,"ruleSystems":["Custom system"],"genres":["Fantasy"],"languages":["English"],"options":[{"optionValueName":"PDF"}],"rating":"4.5","ratingCount":12,"isAi":false}),
        )?;
        let product = kaijucrawl::parser::extract::parse(&body, &id, &d::detail(&id))?;
        let (archive, compressed) = archives.prepare(db.catalogue, d::KEY, &id, &body)?;
        let intent = Intent {
            id: uuid::Uuid::new_v4(),
            target: d::KEY.into(),
            product_id: id.clone(),
            fetched_at: chrono::Utc::now(),
            api_url: d::detail(&id),
            http: json!({"status":200}),
            archive,
        };
        db.intent(&intent).await?;
        archives.write_verified(&intent.archive, compressed).await?;
        db.commit_captures(vec![(intent, product)]).await?;
    }
    db.release().await?;
    db.pool.close().await;
    Ok(())
}
