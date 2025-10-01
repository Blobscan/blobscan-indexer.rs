# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
