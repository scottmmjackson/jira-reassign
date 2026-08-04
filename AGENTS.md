# jira-reassign — maintenance instructions

## What this is

A single-binary Rust CLI that reassigns a Jira ticket's *assignee* to whoever currently holds a
given "role" on that same ticket — a role is a name (e.g. `reviewer`, `responsible-engineer`)
mapped in the user's config file to a Jira custom field ID (e.g. `customfield_10000`).
`jira-reassign reviewer COMMON-807` reads COMMON-807's `reviewer` field and sets its *assignee*
to that person — it does NOT set the reviewer field to the caller. (`cmd_reassign_by_role` in
`src/lib.rs` is the one to check if this drifts; it was originally implemented backwards —
setting the role field to the current user instead of reading it — before being corrected.)
Any role in the config's `fields` map works as `jira-reassign <role> <ticket>` with no code
changes — this is handled by a clap `external_subcommand` catch-all in `src/main.rs`
(`Commands::Field(Vec<String>)`), not a fixed enum of role names. Don't add hardcoded
subcommands for new roles; that defeats the point.

## Layout

- `src/lib.rs` — config load/init, `JiraClient` (blocking `reqwest`, Basic Auth via
  email + API token), and the `cmd_*` functions that implement each CLI command.
- `src/main.rs` — clap `Parser`/`Subcommand` definitions and dispatch only. Business logic
  belongs in `lib.rs`, not here.
- `justfile`, `build/`, `.github/workflows/release.yml` — build/packaging/release tooling
  cribbed from [bbpipelinewait](https://github.com/scottmmjackson/bbpipelinewait). Keep the
  two repos' tooling in sync where it makes sense (e.g. if the release workflow changes there
  for a real bug fix, consider porting it here).

## Conventions

- Errors: `Box<dyn Error>` throughout, no `anyhow`. User-facing failures are formatted with
  `eprintln!` + `exit(1)` (see `load_config`) or bubbled up as `Err(String.into())` from `cmd_*`
  functions and printed once in `main`.
- HTTP: blocking `reqwest`, not async — this CLI only ever makes a couple of sequential Jira API
  calls per invocation, so there's no reason to pull in tokio.
- Config lives at the OS config dir via `directories::ProjectDirs::from("com",
  "scottmmjackson", "jira-reassign")`. Don't change the qualifier/org/app triplet — it would
  silently orphan existing users' config files.
- Jira API: uses `/rest/api/2/*` (not v3) for both reads and writes, since v2 accepts the same
  plain-text-ish issue update bodies without ADF wrapping, which keeps `set_user_field` simple.
- Jira API routing: `JiraClient::new` never calls `config.base_url` directly. It first resolves
  the site's cloud ID via an unauthenticated GET to `{base_url}/_edge/tenant_info`, then routes
  every subsequent request through `https://api.atlassian.com/ex/jira/{cloudId}/...`. This is
  required for Atlassian's newer *scoped* API tokens ("Create API token with scopes") — calling
  the site's own `*.atlassian.net` domain with one doesn't error, it just silently behaves as
  unauthenticated (list endpoints return empty results, e.g. `createmeta` returned `"projects":
  []` for a project that definitely existed). Don't "simplify" this back to hitting `base_url`
  directly even though it looks redundant for classic tokens — the gateway accepts both token
  kinds, and the bug this fixed was invisible without a scoped token to reproduce it against.
- `/rest/api/2/myself` is separately unreliable: some scoped tokens 401 with `"scope does not
  match"` even when they have full issue/field/project read+write access, because the token
  simply wasn't granted user-profile read scope. `resolve_current_user` handles this by
  preferring an optional `config.account_id` over calling `/myself`. Don't remove that fallback
  or assume `/myself` always works — it's the one endpoint that behaves differently by scope
  independent of everything else the tool needs. Note `resolve_current_user` is only used by
  `cmd_assign_me` — `cmd_reassign_by_role` never needs to know who's running the tool, since it
  reassigns to the role field's existing holder, not to the caller.

## Before committing

- `cargo build` and `cargo clippy` should both be clean (no warnings).
- There's no test suite yet (all commands hit live Jira). If you add one, prefer mocking the
  HTTP layer over hitting a real Jira instance in CI.

## Releasing

Bump `version` in `Cargo.toml`, then `just do-release` (or `do-release-build` /
`do-release-package` separately) — see `justfile` for the full pipeline (cross-compiled
binaries, `.deb`/`.rpm` via `nfpm`, GitHub release, Homebrew formula update).
