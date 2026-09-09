# jira-reassign

A small CLI for reassigning Jira tickets to whoever holds a given role — reviewer, responsible
engineer, or any other single-user-picker custom field your team uses — without clicking through
the Jira UI.

## Installation

### Homebrew (macOS/Linux)

```sh
brew tap scottmmjackson/sj
brew install jira-reassign
```

### Linux packages

`.deb` and `.rpm` packages for `amd64`/`arm64` are attached to each
[GitHub release](https://github.com/scottmmjackson/jira-reassign/releases).

### Manual download

Prebuilt archives for macOS (Intel/Apple Silicon), Linux (x86_64/aarch64), and Windows (x86_64)
are attached to each [GitHub release](https://github.com/scottmmjackson/jira-reassign/releases).

## Configuration

```sh
jira-reassign init
```

This writes a template to your OS config directory and prints the path. Fill it in:

```json
{
  "base_url": "https://your-domain.atlassian.net",
  "email": "you@yourcompany.com",
  "api_token": "your-api-token",
  "account_id": null,
  "branch_ticket_regex": null,
  "fields": {
    "reviewer": "customfield_10000",
    "responsible-engineer": "customfield_10001"
  }
}
```

- `email` is your Atlassian account email.
- `api_token` is an Atlassian API token. Create one at
  [id.atlassian.com](https://id.atlassian.com) under
  *Security → Create and manage API tokens*. All requests go through the
  `api.atlassian.com/ex/jira/{cloudId}` gateway (resolved automatically from `base_url`), so both
  classic tokens and newer *scoped* tokens work — just make sure a scoped token has Jira read/write
  scopes for issues, fields, and projects.
- `account_id` is optional, and only used by `assign-me`. It's needed if your API token is a
  scoped token **without** user-profile read access — some scoped tokens 401 on `/myself`
  ("scope does not match") even though issue/field reads and writes work fine. If `assign-me`
  fails with that error, set `account_id` to your own Jira accountId instead of leaving it
  `null`. The easiest way to find it: run `jira-reassign show <a-ticket-currently-assigned-to-you>`
  — your accountId is printed next to your name.
- `fields` maps a role name of your choosing to the Jira custom field ID that holds that role's
  current holder. Run `jira-reassign list-fields` (optionally with a project key) to find field
  IDs. You can add as many roles as you like — each becomes usable as
  `jira-reassign <role> <ticket>` immediately, no code changes required.
- `branch_ticket_regex` is optional and off by default: leave it `null` (or omit it) and you
  must always pass `<ticket>` explicitly. Set it to enable extracting the ticket ID from your
  current git branch name instead, so `jira-reassign reviewer`, `jira-reassign show`, and
  `jira-reassign assign-me` all work with the ticket omitted:
  - `""` (empty string) enables the feature using a sane default pattern — it matches a
    Jira-style key (letters, then digits) anywhere in the branch name, case-insensitively, e.g.
    `feature/COMMON-807-fix-thing` or `common-807-fix-thing` both resolve to `COMMON-807`.
  - Any non-empty string is used as a custom regex instead. The first capture group is used as
    the ticket ID (or the whole match, if the regex has no capture group).

## Usage

```sh
# Reassign COMMON-807 to whoever is currently set as its "reviewer"
jira-reassign reviewer COMMON-807

# Reassign COMMON-807 to whoever is currently set as its "responsible-engineer"
jira-reassign responsible-engineer COMMON-807

# Any role name configured in fields.* works the same way:
jira-reassign <role> <ticket>

# Assign yourself as the ticket's assignee, but only if it's currently unassigned
jira-reassign assign-me COMMON-807

# Show the ticket's assignee, and who holds each configured role
jira-reassign show COMMON-807

# With "branch_ticket_regex" configured, <ticket> can be omitted and is extracted from
# the current git branch name instead, e.g. on branch "feature/COMMON-807-fix-thing":
jira-reassign reviewer
jira-reassign show
jira-reassign assign-me

# List all fields visible on the Jira site
jira-reassign list-fields

# List fields visible on a specific project (helps you find the right customfield_ID)
jira-reassign list-fields COMMON
```

## License

No license has been assigned to this project yet; all rights reserved by the author.
