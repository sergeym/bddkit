# Running the examples

Two suites, two configs. Both need `cargo build --release` first (or use `cargo run --release --` in place of `./target/release/bddkit`).

## HTTP suite — `examples/api.yaml`

Talks to a local [Smocker](https://github.com/smocker-dev/smocker) instance, so the suite runs offline and its responses are fixed by a file in this repo.

```bash
docker compose up -d smocker                      # from the repo root
./target/release/bddkit --config examples/api.yaml
```

The mocks live in `examples/mocks/api-server.yaml` and are seeded into Smocker at startup — after editing them, `docker compose restart smocker` reloads. Smocker's web UI (mocks, request history) is on <http://localhost:8081>; the mocked API itself is on <http://localhost:8080>.

| File | What it demonstrates |
|---|---|
| `features/methods.feature` | GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS, `Location`/`Allow` headers, a 404 asserted like any other answer |
| `features/json_matchers.feature` | `@variableType`, `@regExp`, `@arrayLength`, paths into arrays, `contains` vs `equals` |
| `features/variables.feature` | `set variable`, extraction from JSON and cookies, reuse across steps and scenarios, `<<unique()>>` |
| `features/macros.feature` | YAML-declared steps from `macros/posts.yaml`, macro calling a macro, Scenario Outline |
| `features/content_types.feature` | HTML/XML/plain-text responses, a form login |
| `features/eventual.feature` | An assertion polled until it passes |

`eventual.feature` is only interesting on a fresh mock session: `/jobs/1` answers "pending" twice and then "done", and Smocker keeps that counter until the container restarts. Run `docker compose restart smocker` to see the polling actually retry — on a session where the job is already done, the assertion just passes on its first attempt.

## Database suite — `examples/db.yaml`

```bash
docker compose up -d db                          # Postgres on :5433, schema from examples/db/init.sql
./target/release/bddkit --config examples/db.yaml
```

`db-features/db.feature` walks through every DB step: inserts (one-liner, table, multi-table), update, delete, extraction into a variable, row-presence assertions, procedures, functions, sequences, and a second connection.

## Narrowing a run

```bash
# one file (a positional path overrides the config's `paths`)
./target/release/bddkit --config examples/api.yaml examples/features/methods.feature

# one tag (repeatable; feature-level tags apply to every scenario in the file)
./target/release/bddkit --config examples/api.yaml --tag json
```

Exit codes: `0` everything passed, `1` a scenario failed, `2` the run never started (bad config, unknown step, no scenario matched the filter).
