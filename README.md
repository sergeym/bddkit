# bddkit

Acceptance testing for backend services — the API and everything behind it —
written in Gherkin and run by a single Rust binary.

A backend test rarely ends at the HTTP response. The interesting question is
usually what happened *behind* it: did the row change, did the balance move,
did the state settle a second later. bddkit treats every system a scenario
touches as a **named resource** — declared once in the config, reachable from
any scenario — so seeding a row, calling the API, and asserting the row
changed are three steps in one vocabulary instead of three tools with glue
between them.

```gherkin
Scenario: registering a company charges the account
  Given I have "accounts" with "email: <<unique(email)>>, balance: 100"
  And the request body is:
    """
    {"account_id": "<<last_insert_id_accounts>>", "name": "Acme"}
    """
  When I request "/api/v1/companies" using HTTP POST
  Then the response code is 201
  And I expect the next assertion to pass within "10" seconds
  And I should have "accounts" with "balance: 75"
```

## What it solves

**Extending the vocabulary shouldn't require a rebuild.** Domain steps like
`I login as user "..."` are declared in YAML and loaded at startup, so
whoever writes the scenarios can also extend the language they're written in.

**A typo shouldn't cost you three minutes of run time.** Unknown steps,
ambiguous patterns, macro cycles, undeclared connections — all found in one
pass before the first request. Exit code `2` means "run not started", so CI
can tell a broken suite from a failing one. Pattern collisions fail loudly
instead of silently shadowing each other.

**Checking the database shouldn't mean leaving the feature file.** DB steps
introspect the real schema: primary keys fill themselves the way the column
declares them, values coerce to real column types, and a missing `NOT NULL`
column fails by name at the step rather than as a driver error. Hawk signing,
SRP-6a, and AES are steps too, so a login handshake stays declarative.

**Async systems shouldn't need sleep loops.** Prefix any assertion with
`I expect the next assertion to pass within "10" seconds` and it retries —
re-sending the request or re-running the query — until it holds.

**Test data shouldn't collide, and should be removable afterwards.** Every
`<<unique()>>` value in a run shares one prefix drawn once per process, so
uniqueness is guaranteed rather than probable, and cleanup afterwards is one
`DELETE FROM t WHERE col LIKE '%<run_id>%'`.

**A failure should explain itself.** Every failed step prints the full last
exchange — method, URL, headers, bodies, status — with no debug flag and no
re-run. JSON mismatches point at the path that differs.

## Quick start

```bash
docker compose up -d smocker   # local mock API the example talks to
cargo build --release
./target/release/bddkit --config examples/api.yaml
```

```console
run m4k2p9x7q3b1
  ✓ examples/features/methods.feature — scenarios: 8
  ✓ examples/features/json_matchers.feature — scenarios: 4
  ✓ examples/features/variables.feature — scenarios: 5
  ✓ examples/features/macros.feature — scenarios: 6
  ✓ examples/features/content_types.feature — scenarios: 4
  ✓ examples/features/eventual.feature — scenarios: 1

run m4k2p9x7q3b1
files: 6, scenarios: 28, failed: 0
```

The example suite talks to a local [Smocker](https://github.com/smocker-dev/smocker)
instance seeded from `examples/mocks/api-server.yaml`, so it runs offline and
its responses are fixed by a file in this repo. Smocker's web UI is on
<http://localhost:8081>.

The database examples are a second suite with its own config and its own
container:

```bash
docker compose up -d db
./target/release/bddkit --config examples/db.yaml
```

`examples/README.md` covers both suites, what each feature file demonstrates,
and how to narrow a run to one file or one tag.

Flags: `--config` (required), positional paths to override the config's,
`--tag` (repeatable), `--env` to pick a `.env.<name>` layer, `--fail-fast`.
Exit codes: `0` passed, `1` a scenario failed, `2` the run never started.

## Config

```yaml
concurrency: 8              # files in flight; a file is one tokio task
macro_paths: [macros/]
paths: [features/]

resources:
  api:
    review:
      base_url: http://review.local
      default_headers: { X-Client: bddkit }
  db:
    default:
      dsn: postgres://${DB_USER}:${DB_PASS}@db:5432/review
      search_path: [app, public]

options:
  polling: { timeout_secs: 5, interval_ms: 100 }   # defaults for eventual assertions
```

`${VAR}` expands docker-compose style (`:-`, `:?`, `:+` and friends) from the
real environment and a `.env` / `.env.local` / `.env.<APP_ENV>` layer stack
next to the config. With one resource of a kind its `default_*` is inferred;
with several it must be explicit. `options` set at the root cascades into
every resource and can be overridden per resource.

Variables live for the whole **feature file**, so a later scenario can read
what an earlier one produced. Request state and the current connection reset
per **scenario**. Files run in parallel; `@serial(name)` chains files that
contend, `@priority(N)` moves one up the queue.

## Where to look next

| For | Look at |
|---|---|
| Every step, authoritative | `BUILTIN_STEPS` in `src/steps/mod.rs` |
| How to run the examples | `examples/README.md` |
| A runnable HTTP example | `examples/api.yaml`, `examples/features/` |
| Every HTTP method, 404 included | `examples/features/methods.feature` |
| JSON matchers and paths | `examples/features/json_matchers.feature` |
| Variables: set, extract, reuse | `examples/features/variables.feature` |
| Macros, nesting, Scenario Outline | `examples/features/macros.feature`, `examples/macros/posts.yaml` |
| Non-JSON responses, form login | `examples/features/content_types.feature` |
| Polling an assertion until it passes | `examples/features/eventual.feature` |
| The mock API behind all of it | `examples/mocks/api-server.yaml` |
| Every DB step, worked through | `examples/db-features/db.feature` |
| SRP handshake, Hawk signing | `tests/features/` |
| Config schema | `src/config.rs` |

## Design notes

**Why not cucumber-rs.** Three axes diverge: steps register at compile time
via proc-macros (here they load from YAML at run time), `World` is recreated
per scenario (here variables are file-scoped), and concurrency is a flat pool
over scenarios (here it's chains of files). Working around all three leaves
only its run loop while breaking its own step diagnostics — which is the part
worth strengthening. The `gherkin` crate underneath it is reused directly.

**Why no transactional rollback.** The service under test reads over its own
connection and would never see uncommitted rows, so rolling back per scenario
would break any test where a step writes and the API reads. Isolation comes
from unique data instead.

**Where this is going.** `api`, `db`, and `srp` are the resource kinds that
ship, not the ceiling — a key under `resources:` is meant to be a capability
group, so reaching an object store or a mailbox becomes the same move as
reaching a second database. The seams exist (options cascade per instance,
`I use "<name>" <kind>` is one step shape, dispatch returns
`passed | not yet | fatal` so eventual assertions work without knowing what
they retry). What's missing is loading steps and resource kinds from outside
the binary.

## Development

```bash
cargo test
```

Some tests need PostgreSQL; `docker-compose.yml` brings one up and
`examples/db/init.sql` creates the schema. The same file brings up
[Smocker](https://github.com/smocker-dev/smocker) as the HTTP example's mock
API — it seeds `examples/mocks/api-server.yaml` at startup and serves it on
`localhost:8080` (web UI on `localhost:8081`).

## License

Apache-2.0. See [LICENSE](LICENSE).
