# Running the examples

Three suites, three configs. Both need `cargo build --release` first (or use `cargo run --release --` in place of `./target/release/bddkit`).

## HTTP suite — `examples/api.yaml`

Talks to a local [Smocker](https://github.com/smocker-dev/smocker) instance, so the suite runs offline and its responses are fixed by a file in this repo.

```bash
docker compose up -d smocker                      # from the repo root
./target/release/bddkit run --config examples/api.yaml
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
./target/release/bddkit run --config examples/db.yaml
```

`db-features/db.feature` walks through every DB step: inserts (one-liner, table, multi-table), update, delete, extraction into a variable, row-presence assertions, procedures, functions, sequences, and a second connection. It is deliberately Postgres-only — see the portability table below for what that costs.

## Database suite on MySQL and MariaDB — `examples/db-mysql.yaml`

```bash
docker compose up -d mysql mariadb            # schema from examples/db/init-mysql.sql
./target/release/bddkit run --config examples/db-mysql.yaml
```

The config points at MySQL on `:3307`; MariaDB is the same config on `:3308`, and `db-features-mysql/db.feature` passes against both unchanged. That is the file's purpose — it uses only the vocabulary that is portable: the three insert forms (one-liner, data table, multi-table wide form), update, delete, `I extract`, both row-presence assertions, and a connection switch.

The init script only runs on a container's first start with an empty data directory. If the containers already exist, `docker compose down -v && docker compose up -d` to apply it.

### What does not port

| Feature | Postgres | MySQL | MariaDB | Why |
|---|---|---|---|---|
| `I get next value of sequence` | yes | no | yes | MySQL has no sequences at all; MariaDB does, so the step works there — but a file using it is no longer engine-neutral |
| `search_path` in `resources.db` | yes | no | no | A schema *is* a database here, so there is no session-level search path. bddkit refuses it at startup instead of ignoring it — name the database in the DSN, or qualify a step's table as `database.table` |
| `binary(16)` UUID columns | n/a | no | no | `UUID_TO_BIN(uuid, swap_flag)` reorders bytes depending on a flag that `information_schema` does not record, so bddkit cannot know which layout the service under test reads. It refuses rather than write disagreeing bytes. Use `char(36)` — as `examples/db/init-mysql.sql` does — and bddkit fills it with a client-side UUIDv7. Every other `binary`/`varbinary` column is refused too: this layer binds and compares as text, so a WHERE against one would match nothing |
| The *text* an `I extract` yields | — | — | — | Portable as a step, not as a value. Every engine renders its own types: `now()` is `2026-08-29 15:11:50.884052+00` on Postgres and `2026-08-29 15:11:50` on both others, a boolean is `true` vs `1`, and a bare `numeric` is `0` where `decimal(12,2)` is `0.00`. So `variable "x" should be equal to "..."` on an extracted timestamp or boolean is engine-specific even though the steps around it are not — the example only extracts from `varchar` columns for that reason |
| `I call procedure` / `I call function` | yes | yes | yes | The *steps* are portable; the routine *bodies* are not. The Postgres example's are `LANGUAGE sql` with `nextval()` and `\|\|`, which is why they are absent here rather than translated |

Everything else in the DB vocabulary behaves identically on all three.

## Narrowing a run

```bash
# one file (a positional path overrides the config's `paths`)
./target/release/bddkit run --config examples/api.yaml examples/features/methods.feature

# one tag (repeatable; feature-level tags apply to every scenario in the file)
./target/release/bddkit run --config examples/api.yaml --tag json
```

Exit codes: `0` everything passed, `1` a scenario failed, `2` the run never started (bad config, unknown step, no scenario matched the filter).
