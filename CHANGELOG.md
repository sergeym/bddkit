# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `bddkit steps list` — the step vocabulary as templates grouped by resource, with `--filter`, `-v`, `--json`, plugin steps under `--config`, and descriptions in `en`, `ru` and `lv` (`--lang` / `BDDKIT_LANG`)
- *(plugin)* an optional `description` per step in the `list_steps` reply

### Changed

- **Breaking:** running a suite is now `bddkit run --config …`; the flat `bddkit --config …` form is gone

## [0.1.1](https://github.com/sergeym/bddkit/compare/v0.1.0...v0.1.1) - 2026-08-27

### Added

- give each feature file a working directory
- *(plugin)* per_worker instances, one per feature file
- *(plugin)* load at startup, drop after the pool drains
- *(runner)* dispatch plugin steps off the executor
- *(plugin)* plugin steps, selected per scenario
- *(plugin)* eager config validation, lazy instances
- *(config)* opaque resource groups served by plugins
- *(plugin)* load a cdylib over a JSON ABI

### Other

- the plugin contract, and a plugin written against it
- *(plugin)* end-to-end acceptance for the plugin layer
- run the examples against a local Smocker mock server
