# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/sergeym/bddkit/releases/tag/v0.1.0) - 2026-08-23

### Added

- add eventual (polling) assertions for HTTP and database checks
- add AES encryption step
- add Hawk request signing
- add SRP-6a client steps
- add parallel feature-file runner with @serial/@priority/--fail-fast
- add layered .env file loading and docker-compose interpolation operators
- make api resources optional, refresh examples
- add response debug steps with syntax-highlighted output
- [**breaking**] replace suites with global api/db resources, add tag-based scenario filtering
- add YAML macro system with scoped exports and startup validation
- add debug steps for sleep and variable inspection
- add extract-from-table step, deleted-row counters, and procedure/sequence support
- add DB pools with schema introspection cache, wire DB into World/runner
- add INSERT/WHERE/UPDATE/DELETE/EXISTS query builders
- add sqlx dependency, table reference parser, and column-value parsing
- add sequential runner, console report, and CLI entry point
- add feature discovery, Scenario Outline expansion, and static validation
- add HTTP request/response plumbing and World state dispatcher
- add step registry, JSON path reader, and inline JSON matchers
- add YAML config loading with environment variable expansion
- add scoped variable stack, unique value generator, and <<...>> interpolation
- add crate scaffold and Gherkin parsing entry point

### Other

- read fixed-size hex pairs with as_chunks
- automate releases with release-plz and cross-platform binaries
- add CLAUDE.md & AGENTS.md
- add README
- Initial commit
- build a shared Apis registry and add the api-switch step
- speed up DB integration tests
- add DB integration test harness
- add axum reference service and end-to-end acceptance tests
