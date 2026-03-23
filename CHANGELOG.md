# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.6...v0.6.0) - 2026-03-23

### Fixed

- add retry logic for 404 responses when indexing blocks ([#128](https://github.com/Blobscan/blobscan-indexer.rs/pull/128))
- re-add reorg handling log + missing last processed block set
- handle beacon block header propagation delay on SSE head events ([#126](https://github.com/Blobscan/blobscan-indexer.rs/pull/126))

### Other

- *(deps)* bump rustls-webpki from 0.103.8 to 0.103.10 ([#125](https://github.com/Blobscan/blobscan-indexer.rs/pull/125))
- *(deps)* bump quinn-proto from 0.11.13 to 0.11.14 ([#124](https://github.com/Blobscan/blobscan-indexer.rs/pull/124))
- *(deps)* bump keccak from 0.1.5 to 0.1.6 ([#123](https://github.com/Blobscan/blobscan-indexer.rs/pull/123))

## [0.5.6](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.5...v0.5.6) - 2026-02-13

### Fixed

- *(docker)* bump debian docker image to trixie
- handle stale sse subscriptions + handle channel errors properly ([#121](https://github.com/Blobscan/blobscan-indexer.rs/pull/121))

## [0.5.5](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.4...v0.5.5) - 2026-02-13

### Other

- *(deps)* enable aws_lc_rs crypto provider feature ([#119](https://github.com/Blobscan/blobscan-indexer.rs/pull/119))

## [0.5.4](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.3...v0.5.4) - 2026-02-13

### Other

- *(deps)* bump time from 0.3.44 to 0.3.47 ([#117](https://github.com/Blobscan/blobscan-indexer.rs/pull/117))
- *(deps)* bump bytes from 1.11.0 to 1.11.1 ([#116](https://github.com/Blobscan/blobscan-indexer.rs/pull/116))
- *(deps)* bump jsonwebtoken from 8.3.0 to 10.3.0 ([#115](https://github.com/Blobscan/blobscan-indexer.rs/pull/115))

## [0.5.3](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.2...v0.5.3) - 2026-01-08

### Other

- update Cargo.lock dependencies

## [0.5.2](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.1...v0.5.2) - 2025-11-03

### Other

- *(ci)* use PAT in release-plz workflow to allow Docker release to trigger

## [0.5.1](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.5.0...v0.5.1) - 2025-11-01

### Added

- add backoff retry support to PUT requests ([#110](https://github.com/Blobscan/blobscan-indexer.rs/pull/110))

## [0.5.0](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.4.4...v0.5.0) - 2025-10-01

### Fixed

- on SSE disconnect fetch the current latest synced block instead of reusing the one passed to the task, preventing duplicate block reindexing ([#105](https://github.com/Blobscan/blobscan-indexer.rs/pull/105))

### Other

- *(deps)* bump tokio from 1.40.0 to 1.43.1 ([#109](https://github.com/Blobscan/blobscan-indexer.rs/pull/109))
- *(deps)* bump openssl from 0.10.66 to 0.10.73 ([#108](https://github.com/Blobscan/blobscan-indexer.rs/pull/108))
- *(deps)* bump tracing-subscriber from 0.3.18 to 0.3.20 ([#107](https://github.com/Blobscan/blobscan-indexer.rs/pull/107))
- show usage parameters and improve wording ([#96](https://github.com/Blobscan/blobscan-indexer.rs/pull/96))

## [0.4.4](https://github.com/Blobscan/blobscan-indexer.rs/compare/v0.4.3...v0.4.4) - 2025-09-26

### Added

- add `lib` file

### Other

- add aync trait macro
- restrict env and args usage to main entrypoint, separating them from library code
