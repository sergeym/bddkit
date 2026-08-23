# CLAUDE.md — bddkit

Agent orientation: the fast path into this codebase for an AI agent.

`bddkit`: Gherkin-based API acceptance-testing tool in Rust. Testers write `.feature` files (readable by non-devs); the tool validates every step up front, then runs each scenario against an HTTP API and the systems behind it. `README.md` is the user-facing description of the same tool — read it for the vocabulary and config shape as a tester sees them.

## Language convention (do not get this wrong)

- **Everything in the codebase is English: code comments, runtime error messages, logs, this file, all other documentation.** No exceptions.
- **Identifiers, Gherkin step text, commit messages: English** (unchanged).
- Matcher DSL tokens (`@variableType`, `@arrayLength`, `@regExp`) are the language surface — leave as-is.
- This is about the codebase only. Conversational replies to the operator follow the operator's language (mirror Russian/Ukrainian/etc. as they write) — that is a separate axis from what goes into the repo.

## Markdown convention

**Never soft-wrap prose in `.md` files.** One paragraph — or one list item — is one physical line, however long. The renderer (browser, IDE preview, GitHub) wraps it to the reader's width; a hard line break at column 80 only fights that and makes every later edit produce a reflow diff instead of a content diff. Applies to this file, `README.md`, and anything else Markdown. Hard breaks stay meaningful where they are semantic: separate lines for list items, table rows, and code blocks.

## Architectural invariants — breaking any of these breaks the design

1. **Step matching needs no variable values.** `Registry::find` runs on RAW step text. Interpolation happens AFTER `find`, BEFORE `dispatch`, and ONLY on captured args / docstrings / table cells — never on whole step text. This is what lets `validate::check` verify every step before the first request. If you ever need a variable's value to match a step, you have broken the design — stop.
2. **State scoping is asymmetric.** Variables (`VarStack`) live per feature-FILE and persist across scenarios within it. HTTP state (`HttpState`) resets per SCENARIO (`World::reset_scenario` → `HttpState::reset`), and that reset includes the **current API resource**, which returns to `Apis::default_name()` — a scenario never inherits the API another one switched to. This mirrors Behat/Imbo — confirmed from Imbo source, not guessed. Background steps re-run before every scenario.
3. **`contains` JSON is deliberately loose; `equals` is strict.** contains = object subset + order-independent array containment + extras allowed (Imbo semantics, on purpose). equals = exact keys, element-by-element, same length. Do not "tighten" contains — that's a separate function.
4. **The dispatch match is exhaustive with NO catch-all.** `steps::dispatch` names every `StepId`. The compiler enforces completeness; a `_ =>` arm would silently defeat that when new steps are added. Never add a catch-all.
5. **Uniqueness = run_id + atomic counter.** `<<unique()>>` gives per-run prefix (time+random, sortable, greppable) + a monotonic counter → intra-run uniqueness by construction. Do not draw randomness per-value.
6. **Worker-pool runner over chains, exit codes 0/1/2.** The scheduling unit is a **chain**: files sharing a `@serial(<name>)` tag run one after another, every other file is a chain of one. `runner::build_chains` groups and sorts them by `@priority(<N>)` (higher first, default 0, a file takes the max over its tags); `runner::run_all` spawns `concurrency` tokio tasks, at least one and never more than the chain count, over one shared cursor. Each file is run by a nested `tokio::spawn` the worker awaits immediately — that bounds concurrency to the pool and isolates a panic to one file. `--fail-fast` sets an atomic flag checked at exactly two points: before a worker claims the next chain, and before it starts the next file within a chain it has already claimed. There is no checkpoint between scenarios — a file that has started always runs to completion, so a result is never partial: it stops *starting* work, it never aborts a request in flight or truncates a file mid-run. Exit: 0 all-pass, 1 any scenario failed, 2 anything that failed before the first request (validation, config, resource construction, a malformed scheduling tag). Failure ALWAYS dumps the full HTTP exchange, and a file's whole output is composed by `report::render_file` and written with a single `print!` so workers cannot interleave.

## Gotchas that cost real debugging

- **`gen` is a reserved keyword in edition 2024.** Never name a variable/field/param `gen`; use `generator`. (`r#gen` compiles but diverges from the codebase — don't.)
- **reqwest 0.13 gates `.form()` behind the `form` feature.** Already enabled. Adding a feature to a pinned crate is allowed; bumping the version is not.
- **gherkin `Table` rows are guaranteed rectangular** ("each row is always the same length as the first row"). Table indexing after checking the header row cannot panic on parsed input.
- **Integration tests that spawn the stub AND block on `Command::output()` need a multi-thread runtime.** Use `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`. Single-threaded starves the spawned server → requests hang to timeout.
- **Two substitution syntaxes, different phases:** `<placeholder>` = Scenario Outline, substituted at parse time in `feature::substitute`. `<<variable>>` = runtime, substituted in `vars::interpolate`. `substitute` MUST leave `<<...>>` intact (a naive `replace("<key>",…)` corrupts `<<key>>` — it uses a single-pass regex `<<(\w+)>>|<(\w+)>` to protect double brackets).
- **On transport failure (`connection refused`/timeout), `HttpState::send` clears `self.last` first** so the failure dump never shows a stale prior exchange.
- **DB binds are text, Postgres casts.** Every value is bound as `$N::type` (type from schema introspection) and Postgres does the coercion — Rust never parses the value. Consequence: a value must be a valid literal for its column type or Postgres rejects it.
- **`<<null>>` is the only way to write a SQL NULL.** It interpolates to `vars::NULL_SENTINEL` (`\u{0}__api_bdd_null__\u{0}`, NUL-bytes can't occur in `.feature` text), which the DB layer maps to SQL NULL on bind. A literal empty string `""` binds as `''`, not NULL.
- **DB tests run the compiled binary as a subprocess and need a live Postgres.** `docker compose up -d` first, then run single-threaded (`--test-threads=1`) — the fixture schema `apibdd_it` is shared, so parallel tests race. Set `API_BDD_TEST_TIMINGS=1` and pass `--nocapture` to print a per-test timing table. `cargo test --bin bddkit` has no DB dependency and stays fast.
- **`@serial(<name>)` orders files, it does not share their state.** `World` (and its `VarStack`) is built per file inside `run_file`, so variables do NOT flow between files of a chain. This is the natural misreading of the tag — a tester who expects `companyId` to survive into the next file gets an «undefined variable» error.
- **`I delete all "T"` is a full table wipe and does not know about other workers.** Under `concurrency > 1` it removes rows a parallel file is using. Same for any `I have` / `I delete … where` on fixed literals instead of `<<unique()>>`. Isolation in this tool is by data uniqueness, never by transaction rollback — parallelism does not change that rule, it just makes breaking it visible. Put such files in one `@serial(<name>)` chain, or make their data unique.
- **Debug output is not part of the atomic per-file dump.** `I am in debug mode`, `Show …`, and `Print response …` write to **stderr** as they run, so with `concurrency > 1` two workers' debug output interleaves. Deliberate — routing every debug line through a per-file buffer would delay it past the step that produced it. Set `concurrency: 1` while debugging.
- **`--fail-fast` makes a run non-reproducible.** Which files got done depends on timing. That is inherent to «stop starting new work», not a defect.
- **`tokio::spawn` needs `Send + 'static`.** `execute_step` returns `Pin<Box<dyn Future + Send + 'a>>`; keep the `+ Send`. If a new step breaks it, the compiler names the value held across the `await` — scope the guard in a block. Do NOT reach for `tokio::sync::Mutex`: an async mutex held across a DB round-trip serialises the workers, which defeats the whole runner design.
- **`Config.concurrency` defaults to 8.** A suite written for a sequential runner runs 8-way parallel with no config change; set `concurrency: 1` to get the sequential behavior back until the suite is made parallel-safe.
- **SRP `hex-string` pads the client proof before hashing it into the server proof.** (`M1`/`M2` below are the SRP-6a proof messages, nothing to do with any release numbering.) The variant zero-pads the client proof `M1` to the hash's digest width before it goes into the server proof `M2`; getting that padding wrong — e.g. stripping the proof's leading zero the way some browser SRP clients do — silently produces a different `M2` about 1 login in 16, whenever the client proof's leading nibble happens to be zero.

## Module map (`src/`)

`main.rs` CLI+orchestration · `config.rs` YAML+${ENV}+resources · `options.rs` global/resource polling-option inheritance · `polling.rs` one-shot assertion polling adapter · `unique.rs` run_id/counter · `vars.rs` VarStack+interpolate (holds `NULL_SENTINEL`) · `http.rs` Apis/ApiResource+request state+Exchange · `hawk.rs` Hawk request signing · `world.rs` per-run state · `feature.rs` discover+parse+outline expansion+`TagFilter`+priority_of/serial_of (scheduling tags) · `macros.rs` YAML domain steps · `steps/` registry(`mod.rs`)+api/assert/vars/db/debug/srp step fns+dispatch · `db/` `reference`(`[connection:][schema.]table` refs)+`value`(one-liner/table pair parsing)+`plan`(INSERT/UPDATE/WHERE/EXISTS SQL builders)+`introspect`(schema→`TableSchema`)+`ops`(execute+set result vars)+`mod`(pools/`DbHandle`) · `json/` path reader + matcher · `srp/` SRP-6a verifier generation and proof computation (`hex_string`, `rfc5054`) · `validate.rs` pre-run static check · `runner.rs` RunContext (per-run shared state) + build_chains (@serial/@priority) + run_all (worker pool) + run_file · `report.rs` console+exit code.

## Resources and selection

APIs, databases and SRP configurations are global named resources (`resources.api` / `resources.db` / `resources.srp`, plus optional `default_api` / `default_db` / `default_srp`, inferred when a kind has exactly one resource). Any scenario reaches any of them by name — `I use "<name>" api`, `I use "<conn>" connection`. One `reqwest::Client` per API resource, built once into an `Arc<Apis>`; one Postgres pool per DB connection for the whole run (`Db::connect`).

Test selection lives on the CLI: a positional path argument overrides `cfg.paths`, and a repeatable `--tag` filters scenarios (feature-level tags are merged onto every scenario at load; an empty selection exits 2).

**Known gap, deliberate:** `resources.srp` accepts multiple named SRP resources, but only the default one is reachable — no step exists yet to select a non-default one by name.

## Macros (declarative domain steps)

YAML files listed under `macro_paths` declare domain steps (`I login as user "..."`) without a rebuild. Matching is transparent — a step is a built-in or a macro, the caller can't tell. Macros take scalar parameters, run in isolated frames that publish results through named/glob `exports`, and may nest up to depth 16. Conflicts and cycles are caught at startup validation (exit 2). Macro call tables/docstrings and loops are intentionally out of scope.

## Eventual assertions

`I expect the next assertion to pass ...` arms exactly one future assertion: inputs are interpolated once, a later modifier silently replaces an earlier one, and an unconsumed modifier is discarded at scenario end or reset. DB attempts query the current connection anew; an HTTP response assertion first checks the saved exchange, then replays its immutable original request at its original API after each mismatch, freshly signing Hawk on every replay. Polling options are inherited global → resource (`options.rs`).

## Adding a built-in step

1. Add a `StepId` variant + its regex to `BUILTIN_STEPS` in `steps/mod.rs`. Anchor with `^…$`. Keep the ambiguity test green — if two patterns overlap, fix the PATTERN, not the table order.
2. Write the step fn in `steps/{api,assert,vars,db,debug,srp}.rs`.
3. Add a dispatch arm (compiler forces this — no catch-all).

Verb convention: `I <action>` mutates, `should` asserts, `the … is` sets a request param. No verb does both directions.

## Database layer

DB steps live in `steps/db.rs`; SQL is built in `db/plan.rs` and run in `db/ops.rs`. The connection a scenario starts on comes from `default_db` (or is the single declared connection). Table refs are `[connection:][schema.]table` (current connection + `search_path` schema otherwise). Full Gherkin vocabulary (all in `BUILTIN_STEPS`):

- **Insert** — `I have "<table>" with "<pairs>"` (one-liner) · `I have "<table>" where:` (data table, columns as header) · `I have:` (multi-table wide form, first column = table). Schema introspection drives PK/NOT-NULL fill: value given → use it; identity/serial or column with a default → omit from INSERT, let Postgres generate; no-default `uuid` PK → UUIDv7 client-side; NOT-NULL timestamp with no value → `now()`; otherwise fail naming the column.
- **Update** — `I update "<table>" with "<pairs>" where "<cond>"`.
- **Delete** — `I delete "<table>" where "<cond>"` · `I delete all "<table>"` (no WHERE, full wipe).
- **Read into a var** — `I extract "<column>" from "<table>" with "<where>" as "<var>"`.
- **Assert row presence** — `I should have "<table>" with "<pairs>"` / `with:` · `I should not have "<table>" with "<pairs>"` / `with:`.
- **Routines / sequences** — `I call procedure "<name>" with "<args>"` · `I call function "<name>" with "<args>" as "<var>"` · `I get next value of sequence "<name>" as "<var>"`.
- **Connection / debug** — `I use "<conn>" connection` (its API counterpart is `I use "<name>" api`, a `steps/api.rs` step: it switches the target API resource, replaces the headers with that resource's `default_headers`, clears the pending query/body/form, and deliberately KEEPS `last`) · `I am in debug mode` / `I am not in debug mode` (debug prints the generated SQL + binds).

`<pairs>`/`<cond>`/`<where>` are one-liners: comma-separated `col: value`; a literal comma inside a value is escaped `\,`. `<<...>>` is runtime variable interpolation (`vars::interpolate`), and `<<null>>` yields a SQL NULL (see Gotchas). In a WHERE, a NULL value compiles to `col IS NULL` (no bind).

**Result variables** written back after a step (readable as `<<...>>`):
- `last_insert_*` — PK values from the last insert: `last_insert_id_<table>` for a single-column `id` PK, `last_insert_<table>_<col>` per column for other/composite PKs, plus a `_<index>` suffix in multi-row/`I have:` inserts.
- `updated_<table>` — rows affected by the last update on that table.
- `deleted_<table>` — rows affected by the last delete on that table.

**Known ceilings / limitations (intentional, not bugs — do NOT "fix" without a spec change):**
- **`I extract` cannot distinguish SQL NULL from empty string.** It runs `SELECT (col)::text … LIMIT 1` and coerces a NULL result to `""` (`unwrap_or_default`). A row that is NULL and a row that is `''` both yield `""` in the variable.
- **`I extract` reads only the first matching row** (`LIMIT 1`); the WHERE is not required to be unique and no ordering is imposed.
- **Isolation is by data uniqueness, not transaction rollback.** The service-under-test reads through its own connection and won't see uncommitted rows, so tests can't wrap work in a rollback — they rely on `<<unique()>>`-scoped data instead.
- **A connection is bound to one database (no `USE db`).** Many schemas can share one connection (`schema.table`), but a second database needs a second entry in `resources.db`.

## Dev workflow

- do not use `superpowers` for simple tasks and non-code changes.
- when changing Rust code, use `rust-best-practices` skill.
- TDD per task: failing test → RED → implement → GREEN → commit. Tests assert real behavior, never mock theater.
- Dependency VERSIONS are frozen (features may be added). Edition 2024, toolchain 1.97.
- `cargo test` (unit + `tests/acceptance.rs` e2e against the axum stub). `cargo clippy --all-targets` must be clean — no warnings. `HttpState::current` carries an `#[allow(dead_code)]` because only tests read it; that is correct, don't delete the method.
- Acceptance gate: never weaken a `.feature` or an assertion to force green. A red feature file means a real bug — fix the code or report it, don't patch the test.
- Load-bearing acceptance tests, each pinning an invariant that is easy to break silently: `one_scenario_can_call_two_different_apis` (resources), `two_feature_files_run_at_the_same_time` + `a_single_worker_cannot_release_the_barrier` + `two_files_in_one_serial_chain_never_run_together` (parallelism and per-file variable isolation, proven by a barrier stub rather than timing), `tests/features/srp_login.feature` (registration + authenticated login end to end).
- Subagent reports have miscounted tests and quoted non-existent text — verify claims against files, don't trust report prose.
