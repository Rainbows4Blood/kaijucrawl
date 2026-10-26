# kaijucrawl

A Rust terminal application for building a resumable catalogue of DriveThruRPG product metadata.

**Status: specification stage.** This repository currently contains the implementation specification and agent guidance. There is no runnable crawler yet.

- [Product and technical specification](SPEC.md)
- [Instructions for coding agents](AGENTS.md)

The first release supports **DriveThruRPG only**. It uses PostgreSQL for catalogue and crawl state, and S3-compatible object storage for successful response archives. Starting `kaijucrawl` will open an interactive terminal dashboard and require storage setup when it has not been configured.

Each crawl rescans sitemaps, discovers new products, and retries outstanding work. Successfully collected products are skipped on subsequent crawls. A successful product must have both a title and a description. Saved responses can be reprocessed after parser updates without fetching the products again.

The architecture leaves room for additional storefronts. Their records will remain separate; cross-store merging belongs to a later processing stage.

The specification records intended behavior, not completed implementation or verified live-storefront compatibility. See [acceptance criteria](SPEC.md#18-acceptance-criteria) before treating a build as ready.

Licensed under the [MIT License](LICENSE).
