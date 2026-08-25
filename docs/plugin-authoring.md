# Writing a bddkit plugin

This is the complete contract. A plugin is written against this document, not against bddkit's source — nothing here requires reading the host.

## 1. What a plugin is

A plugin is a shared library (`cdylib`) that bddkit opens with `libloading` (`dlopen`) at startup and calls over a small C ABI. It adds two things to a run: **steps** that testers write in `.feature` files, and a **resource group** — a `resources.<group>` block in the config whose contents the host never interprets, exactly the way `resources.api` and `resources.db` work for the kinds that ship in the binary.

**There is no sandbox, and there will not be one.** A plugin runs inside the bddkit process, on the host's threads, with the host's full privileges: its file access, its network, its environment, its ability to `exit()`. `dlopen` runs the library's initialisers before bddkit has looked at a single byte of it. Installing a plugin is the same trust decision as installing any other binary on the machine, and the host's checks (ABI version, manifest name, group ownership) are consistency checks, not a security boundary.

The host and the plugin exchange **only JSON strings**. Rust has no stable ABI, so there is no shared type between them — the contract is a documented JSON schema plus the C representation of a pointer, and nothing else. That is what makes a plugin in another language possible in principle: everything in section 3 is JSON, and the Rust in this document is one way of producing it.

## 2. The crate shape

```toml
# Cargo.toml
# Its own workspace root on purpose: without this, a parent workspace builds
# the plugin implicitly and `cargo clippy --all-targets` in the parent sees it.
[workspace]

[package]
name = "bddkit-s3"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde_json = "1.0"
```

The build produces `libbddkit_s3.so` (Linux), `libbddkit_s3.dylib` (macOS) or `bddkit_s3.dll` (Windows) in `target/<profile>/`. That file's path is what goes into the lock file in section 7.

**`tests/fixtures/echo-plugin/` in the bddkit repository is a complete, working plugin — 230 lines, every export, every reply shape, the panic guard, the NUL handling.** Copy it and replace the body. It is also what the host's own test suite runs against, so it cannot rot: a change to the ABI that the fixture does not follow turns the suite red.

## 3. The ABI

`ABI_VERSION` is **1**. Every symbol is `extern "C"` and `#[unsafe(no_mangle)]`.

| Symbol | Signature | Required |
|---|---|---|
| `bddkit_abi_version` | `() -> u32` | yes |
| `bddkit_manifest` | `() -> *mut c_char` | yes |
| `bddkit_list_steps` | `() -> *mut c_char` | yes |
| `bddkit_validate_config` | `(*const c_char) -> *mut c_char` | yes |
| `bddkit_init_instance` | `(*const c_char) -> *mut c_char` | yes |
| `bddkit_dispatch` | `(u64, u32, *const c_char) -> *mut c_char` | yes |
| `bddkit_drop_instance` | `(u64) -> *mut c_char` | yes |
| `bddkit_reset_scenario` | `(u64) -> *mut c_char` | no |
| `bddkit_free_string` | `(*mut c_char)` | yes |

A missing required symbol fails the load with a message naming the symbol. **Returning NULL from any of the string-returning exports is an error**, reported as "plugin `<name>` returned nothing from `<call>`" — there is no reply shape that means "nothing to say"; say it with an envelope.

At load the host does, in order: `dlopen`; call `bddkit_abi_version` and refuse anything but `1`; resolve every required symbol, then `bddkit_reset_scenario` if it is exported; call `bddkit_manifest`; refuse a manifest whose `name` differs from the lock entry's name, or whose `groups` is empty, or whose `concurrency` is not `shared`; call `bddkit_list_steps`; refuse any step whose `group` the manifest does not claim. Nothing else is called until a scenario reaches a step.

### `bddkit_abi_version`

Returns `1`. This is the one call made before the version is known, so it is an unavoidable act of faith on both sides: export it with exactly this signature.

### `bddkit_manifest`

```json
{
  "name": "s3",
  "version": "1.2.0",
  "groups": ["s3"],
  "concurrency": "shared"
}
```

- `name` (string, required) — must be byte-identical to the `name` in the lock entry that pointed at this file, otherwise the load fails. It is the name every diagnostic uses.
- `version` (string, required) — required so a manifest without one is a load failure; the host does not otherwise compare it against anything.
- `groups` (array of strings, required, non-empty) — the `resources.<group>` sections this plugin serves. Two loaded plugins may not claim the same group, and section 8 lists the names that are reserved.
- `concurrency` (string, optional, default `"shared"`) — see section 6.

Unknown keys are ignored, so a newer plugin can add fields without breaking an older host.

### `bddkit_list_steps`

An **array** at the top level:

```json
[
  { "pattern": "^I upload file \"([^\"]+)\" to \"([^\"]+)\"$", "group": "s3", "kind": "action" },
  { "pattern": "^the bucket should contain \"([^\"]+)\"$",     "group": "s3", "kind": "assertion" }
]
```

- `pattern` (string, required) — a Rust `regex` crate pattern. Anchor it with `^…$`. Each capture group becomes one positional argument at dispatch, in order.
- `group` (string, required) — must be one the manifest claims.
- `kind` (string, required) — `"action"` or `"assertion"`. Only an assertion may answer `not_yet`, and only an assertion consumes an armed eventual-assertion modifier.

**The index of a step in this array is its identity.** It is the `u32` the host passes to `bddkit_dispatch`, so the array's order is part of your plugin's internal contract with itself — dispatch on the index, and keep the `match` arms and the array in one place so they cannot drift.

The `regex` crate has no backtracking and no lookahead. Patterns are matched against the **raw step text**, before any variable interpolation, so a pattern can never depend on a variable's value.

If your pattern also matches a bddkit built-in step, another plugin's step, or a macro, that step is **ambiguous**: the run stops before the first request with a message listing every definition that matched. It is not "first wins" — nothing is silently shadowed, but nothing works either. Make your patterns specific.

### `bddkit_validate_config`

Called once per declared instance at startup, before the first request, with the same request payload as `init_instance`. Nothing is connected yet — this is a schema check, not a reachability check. Rejecting here is what turns a typo in the config into exit code 2 instead of a failure halfway through the suite.

Request:

```json
{
  "group": "s3",
  "instance": "backups",
  "config": { "bucket": "acme-backups", "endpoint": "http://minio:9000" },
  "options": { "polling": { "timeout_secs": 30, "interval_ms": 100 } }
}
```

All four keys are always present. `config` is the instance's YAML body converted verbatim to JSON, minus the reserved `options` key. The host has no schema for it and never looks inside.

Reply — the **envelope**, shared with `drop_instance` and `reset_scenario`:

```json
{ "ok": true }
```

```json
{ "ok": false, "error": "resources.s3.backups requires a string \"bucket\"" }
```

`error` is optional; if it is missing on a failure the host substitutes "the plugin reported a failure with no message" rather than losing the failure. Always send one — the host prefixes it with `resources.<group>.<instance>: `, so write the part after the colon.

### `bddkit_init_instance`

Called **lazily**: the first time a scenario runs a step of that group with that instance selected. A declared instance nobody uses is validated but never initialised. This is where you open connections, spawn processes, authenticate.

Request: identical to `validate_config`.

Reply:

```json
{ "ok": true, "handle": 7 }
```

```json
{ "ok": false, "error": "cannot reach http://minio:9000: connection refused" }
```

`handle` is a `u64` you choose — an index into your own table, never a pointer. `ok: true` with no `handle` is an error. Handle `0` is legal.

**A failed init is not cached.** The step that triggered it fails, and the *next* step of that group tries again — so an expensive initialisation that keeps failing is re-attempted per step, not once per run. Fail fast, and put anything slow-but-optional behind the first dispatch that needs it.

**Every call must return a handle distinct from every other live instance of your plugin.** The host initialises without holding its lock, so two workers can call `init_instance` for the same instance at the same time; the loser's handle is then passed to `bddkit_drop_instance` while the winner's stays in use. A plugin that returns a singleton handle would have its live instance destroyed underneath it by that drop.

### `bddkit_dispatch`

`bddkit_dispatch(handle, step_index, request)`. `step_index` is the position in your `list_steps` array.

Request:

```json
{
  "args": ["report.pdf", "backups"],
  "docstring": null,
  "table": null,
  "artifacts_dir": "/tmp/bddkit-3n2k9a0f1x7q/000007",
  "debug": false,
  "options": { "polling": { "timeout_secs": 30, "interval_ms": 100 } }
}
```

All six keys are always present; `docstring` and `table` are `null` when the step carries neither.

- `args` — the regex capture groups in order, **already interpolated**: `<<variable>>`, `<<unique()>>` and `<<null>>` have been resolved before the payload was built. A plugin never sees raw step text and never sees bddkit's variable syntax.
- `docstring` — the step's doc string, interpolated, or `null`.
- `table` — the step's data table as an array of rows of strings, cells interpolated, or `null`. **Row 0 is the header row**; bddkit does not strip it, and gherkin guarantees every row has the same length as row 0.
- `artifacts_dir` — a fresh, unique directory path for this one dispatch, under `<temp dir>/bddkit-<run id>/<six-digit counter>`. **The host does not create it.** Call `create_dir_all` before writing, and write nothing if you have nothing to write — most dispatches never touch it.
- `debug` — true while the scenario is inside `I am in debug mode`. It resets at the scenario boundary.
- `options` — the polling options resolved for this instance, after the global → instance cascade and after any armed eventual-assertion modifier. Informational: **the retry loop is the host's**, see section 5.

Reply:

```json
{
  "status": "passed",
  "vars": { "uploaded_etag": "d41d8cd9" },
  "diagnostics": [],
  "error": null
}
```

```json
{
  "status": "fatal",
  "error": "403 Forbidden",
  "diagnostics": [
    { "title": "PUT /backups/report.pdf", "kind": "http", "content": "HTTP/1.1 403 Forbidden\n…", "path": null },
    { "title": "Screenshot",              "kind": "image", "content": null, "path": "/tmp/bddkit-3n2k9a0f1x7q/000007/fail.png" }
  ]
}
```

- `status` (required) — `"passed"`, `"not_yet"` or `"fatal"`. Anything else fails the parse and the step.
- `vars` (optional, default `{}`) — string → string, written into the scenario's variables so later steps can read `<<uploaded_etag>>`. **Kept only when `status` is `passed`**; on `not_yet` and `fatal` they are dropped, so an intermediate observation cannot leak into the scenario.
- `diagnostics` (optional, default `[]`) — evidence, rendered into the failure dump. **Kept only on the failure path**; a `passed` reply's diagnostics are discarded. There is currently no channel for evidence from a step that succeeded.
- `error` (optional, default `null`) — the failure message. See section 5.

A diagnostic is `{"title", "kind", "content", "path"}`: `title` and `kind` are required strings, `content` and `path` are optional. `kind` is free-form text — `text`, `json`, `http` and `image` are the conventional values, and an unrecognised one degrades to "print it as text" rather than failing the parse. The host renders each as `--- <title> (<kind>) ---`, then `content`, then `path`, appended to `error`.

### `bddkit_drop_instance`

`bddkit_drop_instance(handle)`, replying with an envelope. Called once per live instance after the worker pool drains, on **every** exit path including a failed run and `--fail-fast`. Close connections and kill child processes here: a plugin's external process must not outlive the run. A failure is printed as a warning and does not change the exit code.

### `bddkit_reset_scenario`

`bddkit_reset_scenario(handle)`, replying with an envelope. Optional — a plugin with no per-scenario state need not export it, and the host then treats every reset as `ok`.

Called at the scenario boundary for every instance that has been initialised, before the Background steps of the next scenario. This is the plugin's half of bddkit's state rule: HTTP state and the selected instance reset per scenario, variables do not. Clear anything the previous scenario left behind.

**A failed reset fails that scenario**, exactly as a failing Background step would — running a scenario against state the plugin could not clear is how a suite goes green for the wrong reason. Answer `{"ok":false,"error":…}` only when you mean it.

### `bddkit_free_string`

```rust
/// # Safety
/// `s` must be a pointer this library returned and has not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bddkit_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
```

The host copies every reply and immediately hands the original pointer back to **this** function. It never frees plugin memory itself, and you must not free host memory: the `*const c_char` request pointer belongs to the host and is valid only for the duration of the call.

## 4. Four rules only you can keep

The host cannot check any of these. Each one is undefined behaviour or a corrupted test report when broken.

**Wrap every export in `catch_unwind` — including the argument decoding.** Unwinding across an FFI boundary is undefined behaviour. The subtle half is *where* the guard starts:

```rust
fn guard(kind: &str, body: impl FnOnce() -> String) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(reply) => out(reply),
        Err(_) => out(match kind {
            "dispatch" => r#"{"status":"fatal","error":"the plugin panicked"}"#.to_string(),
            _ => r#"{"ok":false,"error":"the plugin panicked"}"#.to_string(),
        }),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn bddkit_validate_config(request: *const c_char) -> *mut c_char {
    guard("envelope", move || {
        let raw = input(request);   // INSIDE the guard, on purpose
        …
    })
}
```

Writing `guard("envelope", input(request), |raw| …)` would evaluate `input(request)` *before* entering the guard, and the export would then be exactly as panic-free as that one helper. The same applies to whatever produces the final string: in the fixture, `out` runs outside the guard and is safe there only because it provably cannot panic — `CString::new` returns a `Result` and the fallback literal has no interior NUL. Anything added to such a helper has to preserve that property.

Guard the trivial exports too. "Every export is guarded" is an invariant a reader can check in one pass; "this one happens to be safe" is a judgement every future edit has to make again.

**Free your own strings in `bddkit_free_string`.** A string allocated by your crate's allocator must be released by your crate's allocator. The host and the plugin can be built with different allocators, different `std` versions, even different compilers; freeing plugin memory with the host allocator is undefined behaviour.

**Never print to stdout.** bddkit composes a whole feature file's output into one string and writes it with a single `print!`, precisely so that parallel workers cannot interleave. A line your plugin prints lands in the middle of another file's failure dump, attributed to the wrong test. Return `diagnostics` instead — they end up inside the dump, attached to the step that produced them.

**stderr** is the escape hatch, and only for tracing while someone is debugging: bddkit's own debug steps use it, precisely because it is outside the per-file dump. It still interleaves between parallel workers, so gate it behind the request's `debug` flag and tell your users to set `concurrency: 1` when they turn it on. Nothing you write there ends up in the test report.

**`create_dir_all(artifacts_dir)` before writing into it.** The host allocates a fresh unique path per dispatch but does not create the directory, because most dispatches write nothing and a `mkdir` per step for the rare case is waste.

## 5. `passed | not_yet | fatal`

- **`passed`** — the step succeeded. `vars` are published; `diagnostics` are dropped.
- **`fatal`** — the step failed and retrying cannot help. The scenario fails with `error` plus the rendered diagnostics.
- **`not_yet`** — *one fresh observation says the condition is not met yet.* Only an assertion may answer this. An action that answers `not_yet` fails the scenario with "a plugin action answered not_yet, which only an assertion may do".

**Never sleep inside a plugin.** `not_yet` means "I looked, just now, and it was not there". The host owns the retry loop, the interval and the timeout — it is what the tester armed with `I expect the next assertion to pass within "30" seconds, checking every "500" milliseconds`, and what the `options.polling` cascade configures. A plugin that sleeps burns the host's budget from inside and makes the timeout mean nothing.

Every attempt is a full `bddkit_dispatch` with a fresh `artifacts_dir`. Re-observe: query again, fetch again, list the bucket again. Do not cache the first answer.

**Without an armed modifier there is no second attempt**, and `not_yet` is simply a failure. That is deliberate: an assertion is not implicitly eventual, the tester opts in per step.

**Send an `error` with every `not_yet` and every `fatal`.** It is optional in the schema, and the host substitutes "the plugin reported a failure with no message" rather than losing the failure — but that message is what the tester reads when the poll finally times out, so make it say what was observed: `"the bucket holds 2 objects, expected 3"`, not `"failed"`.

## 6. `concurrency`

bddkit runs feature files in parallel — `concurrency: 8` by default — and one instance of your plugin serves all of them.

- **`shared`** (the default) — you are promising that a handle is safe to call from several worker threads at the same time. In Rust that usually means a `Mutex` around your instance table and around anything mutable inside an instance.
- **`per_worker`** — declared in the ABI, and **refused by this host**: a plugin whose manifest says `per_worker` fails to load with "which this bddkit does not implement yet". This is on purpose. Treating it as `shared` would hand an instance you explicitly declared is *not* thread-safe to concurrent workers — a data race inside someone else's library, surfacing as a flaky suite. Refusing is louder and cheaper.

The value set is closed: an unknown `concurrency` string fails the manifest parse. A new scheduling mode arrives with an `ABI_VERSION` bump, not with a tolerant parse, so that a mode the host does not implement can never be silently degraded into one it does.

Until `per_worker` exists, a plugin that genuinely cannot be shared should say so in its README and its users should set `concurrency: 1`.

## 7. Installing a plugin

There is no `bddkit plugin install` yet — that is a later milestone. Write the lock file by hand:

```yaml
# .bddkit/plugins.yaml
plugin:
  - name: s3
    path: ./libbddkit_s3.so
```

- `plugin` is a list of `{name, path}`. Both are required; a missing `path` is a parse error naming the file.
- **`name` must equal the manifest's `name`.** A mismatch fails the load with both names in the message — it means the lock file points at the wrong file, and every later diagnostic would name the wrong plugin.
- `path` is absolute, or **relative to the lock file's own directory** — that is `.bddkit/`, not the project root. `./libbddkit_s3.so` in the example above resolves to `.bddkit/libbddkit_s3.so`.
- **`~` is not expanded.** A `~/plugins/libs3.so` reaches `dlopen` verbatim and fails with a confusing "no such file".
- Extra keys are ignored, so a file written by a future `plugin install` (`version`, `source`, `sha256`, `target`) still loads here.

Two files are read, and the project one overrides the user one **entry by entry, keyed by `name`**:

| Scope | Location |
|---|---|
| project | `<directory of the --config file>/.bddkit/plugins.yaml` |
| user | `$HOME/.config/bddkit/plugins.yaml` |

The project lock is anchored to the config file's directory, not the working directory, so `bddkit --config suites/cfg.yaml` finds the same plugins from anywhere. A missing lock file means no plugins, which is the normal case. An unset `HOME` means no user lock.

This file is **machine state and does not belong in the committed test config**: the config describes the system under test, a `.so` path describes one machine.

Then declare instances in the ordinary config, under the group name:

```yaml
paths: [features/]
resources:
  api: {}
  s3:
    backups:
      bucket: acme-backups
      endpoint: http://minio:9000
      options:
        polling: { timeout_secs: 30, interval_ms: 100 }
    archive:
      bucket: acme-archive
default_s3: backups
```

`options` is the one reserved key inside an instance body; the host takes it out, cascades it over the global `options`, and passes the rest through as `config`. `default_<group>` is inferred when the group declares exactly one instance, must be explicit when it declares several, and a `default_<group>` naming a group the config does not declare is a startup error (it is how `default_s4` for `default_s3` gets caught).

A `resources.<group>` block with no plugin serving that group exits 2 with "no installed plugin serves the group". A loaded plugin whose group the config never mentions is fine — it is simply never used.

Testers then select an instance with `I use "archive" s3`, which the host builds from the loaded group names. The selection resets to `default_<group>` at every scenario boundary, and macros may call plugin steps like any other step.

## 8. Footguns

**`<<null>>` arrives as a real NUL byte.** bddkit's SQL-NULL sentinel is `"\u{0}__bddkit_null__\u{0}"`. JSON escapes control characters, so the payload the host sends is clean ASCII — but after your `serde_json` parse, that argument is a Rust `String` **containing two literal NUL bytes**. `CString::new(arg)` on it returns `Err(NulError)`. Any string of yours that becomes a `CString` — a reply, a path, an argument to a C library — must handle it. The fixture's `out` shows the pattern: `CString::new(s).unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"NUL in reply\"}").expect("literal"))`. Deciding that `<<null>>` means something in *your* domain is your call; crashing on it is not.

**Your library is never unloaded.** bddkit deliberately leaks the mapping at process exit rather than `dlclose` it, because running a thread-local destructor or an `atexit` handler after the code page is unmapped is a segfault in someone's CI, and the process is exiting anyway. It also runs every plugin call on tokio's blocking-pool threads, which outlive the run. So: your `Drop` impls, thread-locals and `atexit` handlers may never run. Everything that must be released — connections, child processes, temp files — is released in `bddkit_drop_instance`, which *is* always called.

**Four group names are reserved.** `api`, `db`, `srp` and `connection` are refused with "which bddkit serves itself". The first three are the resource kinds in the binary; `connection` is the built-in step word for the `db` group (`I use "<conn>" connection`), and a plugin claiming it would generate a group-switch step that exactly overlaps the built-in one.

**A YAML tag inside an instance body does not parse.** `resources.<group>` is captured through a flattened map, and serde's buffering for flattened fields cannot represent a YAML tagged value:

```yaml
resources:
  s3:
    backups:
      key: !Foo bar        # error: untagged and internally tagged enums do not support enum input
```

Anchors and aliases are fine; tags are not. Design your instance config out of plain scalars, sequences and mappings.

**`list_steps` is array-rooted.** It has no place to grow a top-level field — a version marker, a capability list — without the host learning to accept both an array and an object. Everything a step needs goes in the step object, where unknown keys are already ignored.

**There is no SDK crate.** Your `Manifest`, `StepSpec` and `DispatchResult` are a hand-copied mirror of the host's types in `src/plugin/abi.rs`. Nothing makes them agree; this document is the only thing preventing the two from drifting. Two consequences: pin the bddkit version you tested against in your README, and prefer building JSON with `serde_json::json!` over deriving `Serialize` on a struct that only looks like the host's — a typo'd key name is then visible in the source instead of hidden behind a `#[serde(rename)]` you forgot.
