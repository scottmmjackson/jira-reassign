use base64::{Engine, engine::general_purpose::STANDARD};
use directories::ProjectDirs;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::process::exit;

const APP_QUALIFIER: &str = "com";
const APP_ORG: &str = "scottmmjackson";
const APP_NAME: &str = "jira-reassign";

/// Sane default for `branch_ticket_regex`: matches a Jira-style issue key (letters, then
/// `-`, then digits) anywhere in a branch name, e.g. `feature/COMMON-807-fix-thing` or
/// `common-807-fix-thing`. Case-insensitive since branch names are conventionally lowercase.
pub const DEFAULT_BRANCH_TICKET_REGEX: &str = r"(?i)\b([a-z][a-z0-9]+-[0-9]+)\b";

/// Config file schema. `fields` maps a role name (e.g. "reviewer",
/// "responsible-engineer") to the Jira custom field ID (e.g. "customfield_10101")
/// that holds that role's assignee. Use `list-fields` to discover field IDs.
///
/// `account_id` is optional. It's needed as a fallback for scoped Atlassian API tokens
/// that lack user-profile read access, since those can't call `/myself` to look up
/// "you" — see `resolve_current_user`.
///
/// `branch_ticket_regex` is optional and off by default: when the field is absent, the
/// ticket ID must always be passed explicitly on the command line. Setting it enables
/// extracting the ticket ID from the current git branch name whenever the ticket argument
/// is omitted. An empty string enables the feature using [`DEFAULT_BRANCH_TICKET_REGEX`];
/// a non-empty string is used as a custom regex instead. The first capture group is used
/// as the ticket ID (the whole match if the regex has no capture group).
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigFile {
    pub base_url: String,
    pub email: String,
    pub api_token: String, // pragma: allowlist-secret
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub branch_ticket_regex: Option<String>,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserRef {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct FieldInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub custom: bool,
}

fn config_path() -> std::path::PathBuf {
    ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .expect("Failed to determine config directory.")
        .config_dir()
        .join("config.json")
}

pub fn init_config() {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create config directory.");
    }
    if path.exists() {
        eprintln!("'{}' already exists, not overwriting it.", path.display());
        exit(1);
    }

    let mut fields = HashMap::new();
    fields.insert("reviewer".to_string(), "customfield_10000".to_string());
    fields.insert(
        "responsible-engineer".to_string(),
        "customfield_10001".to_string(),
    );

    let dummy = ConfigFile {
        base_url: "https://your-domain.atlassian.net".to_string(),
        email: "you@yourcompany.com".to_string(),
        api_token: "your-api-token".to_string(), // pragma: allowlist-secret
        account_id: None,
        branch_ticket_regex: None,
        fields,
    };

    fs::write(&path, serde_json::to_string_pretty(&dummy).unwrap())
        .expect("Failed to write config file.");
    println!("Wrote config template to {}", path.display());
    println!(
        "Edit it with your Jira base URL, Atlassian account email, API token, and field \
        mappings (run `jira-reassign list-fields` to find field IDs)."
    );
    println!(
        "If your API token is a *scoped* token without user-profile read access, also set \
        \"account_id\" — see the README for how to find yours."
    );
    println!(
        "To extract ticket IDs from your current git branch name instead of typing them \
        every time, add \"branch_ticket_regex\": \"\" (or a custom regex) — see the README."
    );
}

pub fn load_config() -> ConfigFile {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("Failed to parse config file at {}: {}", path.display(), e);
            exit(1);
        }),
        Err(_) => {
            eprintln!(
                "Config file not found at {}. Run `jira-reassign init` to create one.",
                path.display()
            );
            exit(1);
        }
    }
}

pub struct JiraClient {
    client: reqwest::blocking::Client,
    base_url: String,
    auth_header: String,
}

impl JiraClient {
    /// Builds a client that talks to the `api.atlassian.com/ex/jira/{cloudId}` gateway
    /// rather than `{base_url}` directly. Atlassian's newer *scoped* API tokens
    /// (created via "Create API token with scopes") are only honored through that
    /// gateway — calling the site's own `*.atlassian.net` domain with one silently
    /// behaves as unauthenticated (empty list results, or flat 401s) instead of
    /// erroring clearly. The gateway also accepts classic unscoped tokens, so this
    /// works for both token types.
    pub fn new(config: &ConfigFile) -> Result<Self, Box<dyn Error>> {
        let creds = format!("{}:{}", config.email, config.api_token);
        let auth_header = format!("Basic {}", STANDARD.encode(creds));
        let client = reqwest::blocking::Client::new();

        let site_base = config.base_url.trim_end_matches('/').to_string();
        let tenant_info: Value = client
            .get(format!("{}/_edge/tenant_info", site_base))
            .send()?
            .json()?;
        let cloud_id = tenant_info
            .get("cloudId")
            .and_then(|c| c.as_str())
            .ok_or("Could not determine Jira cloud ID from base_url (is it a valid Atlassian Cloud site URL?)")?;

        Ok(JiraClient {
            client,
            base_url: format!("https://api.atlassian.com/ex/jira/{}", cloud_id),
            auth_header,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, format!("{}{}", self.base_url, path))
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
    }

    fn parse_response(&self, resp: reqwest::blocking::Response) -> Result<Value, Box<dyn Error>> {
        let status = resp.status();
        let text = resp.text()?;
        if status.is_success() {
            if text.trim().is_empty() {
                Ok(Value::Null)
            } else {
                Ok(serde_json::from_str(&text)?)
            }
        } else {
            Err(format!(
                "Jira API error ({}): {}",
                status,
                extract_error_message(&text)
            )
            .into())
        }
    }
}

fn extract_error_message(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        let mut msgs = vec![];
        if let Some(arr) = v.get("errorMessages").and_then(|m| m.as_array()) {
            for m in arr {
                if let Some(s) = m.as_str() {
                    msgs.push(s.to_string());
                }
            }
        }
        if let Some(obj) = v.get("errors").and_then(|e| e.as_object()) {
            for (k, val) in obj {
                if let Some(s) = val.as_str() {
                    msgs.push(format!("{}: {}", k, s));
                }
            }
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            msgs.push(m.to_string());
        }
        if !msgs.is_empty() {
            return msgs.join("; ");
        }
    }
    text.to_string()
}

pub fn get_myself(client: &JiraClient) -> Result<UserRef, Box<dyn Error>> {
    let resp = client
        .request(reqwest::Method::GET, "/rest/api/2/myself")
        .send()?;
    let v = client.parse_response(resp)?;
    Ok(serde_json::from_value(v)?)
}

/// Resolves "you" for assignment purposes. Prefers `config.account_id` when set, since
/// scoped API tokens without user-profile read access can't call `/myself` (it fails
/// with a 401 "scope does not match" even though issue/field/project reads work fine).
pub fn resolve_current_user(
    client: &JiraClient,
    config: &ConfigFile,
) -> Result<UserRef, Box<dyn Error>> {
    if let Some(account_id) = &config.account_id {
        return Ok(UserRef {
            account_id: account_id.clone(),
            display_name: config.email.clone(),
        });
    }
    get_myself(client).map_err(|e| {
        format!(
            "{}\n\nIf your Jira API token is a *scoped* token without user-profile read \
            access, add \"account_id\": \"<your accountId>\" to your config instead of \
            relying on /myself. Find your accountId by running `jira-reassign show \
            <a-ticket-currently-assigned-to-you>` and copying the ID shown next to your name.",
            e
        )
        .into()
    })
}

/// Returns the name of the current git branch (e.g. via `git rev-parse --abbrev-ref HEAD`).
fn current_branch_name() -> Result<String, Box<dyn Error>> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|e| format!("Failed to run `git rev-parse --abbrev-ref HEAD`: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "`git rev-parse --abbrev-ref HEAD` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Applies `pattern` to `branch` and returns the extracted ticket ID (the first capture
/// group, or the whole match if the regex has none), uppercased since Jira issue keys are
/// conventionally uppercase but branch names are often lowercase.
fn extract_ticket_from_branch(branch: &str, pattern: &str) -> Result<String, Box<dyn Error>> {
    let re = Regex::new(pattern)
        .map_err(|e| format!("Invalid branch_ticket_regex '{}': {}", pattern, e))?;
    let caps = re.captures(branch).ok_or_else(|| {
        format!(
            "Could not extract a ticket ID from branch '{}' using pattern '{}'.",
            branch, pattern
        )
    })?;
    let matched = caps.get(1).or_else(|| caps.get(0)).unwrap().as_str();
    Ok(matched.to_uppercase())
}

/// Resolves the ticket ID to operate on: the explicit CLI argument when given, otherwise
/// (if `config.branch_ticket_regex` is set) extracted from the current git branch name.
pub fn resolve_ticket(
    config: &ConfigFile,
    ticket_arg: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    if let Some(t) = ticket_arg {
        return Ok(t.to_string());
    }
    let pattern = config.branch_ticket_regex.as_deref().ok_or(
        "No ticket specified. Either pass one explicitly, or set \"branch_ticket_regex\" in \
        your config to extract it from the current git branch name.",
    )?;
    let pattern = if pattern.is_empty() {
        DEFAULT_BRANCH_TICKET_REGEX
    } else {
        pattern
    };
    let branch = current_branch_name()?;
    extract_ticket_from_branch(&branch, pattern)
}

pub fn get_issue_fields(
    client: &JiraClient,
    ticket: &str,
    field_ids: &[&str],
) -> Result<Value, Box<dyn Error>> {
    let query = field_ids.join(",");
    let path = format!("/rest/api/2/issue/{}?fields={}", ticket, query);
    let resp = client.request(reqwest::Method::GET, &path).send()?;
    client.parse_response(resp)
}

pub fn extract_user(fields: &Value, field_id: &str) -> Option<UserRef> {
    fields.get(field_id).and_then(|v| {
        if v.is_null() {
            None
        } else {
            serde_json::from_value(v.clone()).ok()
        }
    })
}

pub fn set_user_field(
    client: &JiraClient,
    ticket: &str,
    field_id: &str,
    account_id: &str,
) -> Result<(), Box<dyn Error>> {
    let path = format!("/rest/api/2/issue/{}", ticket);
    let body = serde_json::json!({ "fields": { field_id: { "accountId": account_id } } });
    let resp = client
        .request(reqwest::Method::PUT, &path)
        .json(&body)
        .send()?;
    client.parse_response(resp)?;
    Ok(())
}

pub fn list_all_fields(client: &JiraClient) -> Result<Vec<FieldInfo>, Box<dyn Error>> {
    let resp = client
        .request(reqwest::Method::GET, "/rest/api/2/field")
        .send()?;
    let v = client.parse_response(resp)?;
    Ok(serde_json::from_value(v)?)
}

pub fn list_project_fields(
    client: &JiraClient,
    project: &str,
) -> Result<Vec<FieldInfo>, Box<dyn Error>> {
    let path = format!(
        "/rest/api/2/issue/createmeta?projectKeys={}&expand=projects.issuetypes.fields",
        project
    );
    let resp = client.request(reqwest::Method::GET, &path).send()?;
    let v = client.parse_response(resp)?;

    let mut seen: HashMap<String, FieldInfo> = HashMap::new();
    let projects = v
        .get("projects")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    for project_val in &projects {
        let issuetypes = project_val
            .get("issuetypes")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        for it in &issuetypes {
            if let Some(fields) = it.get("fields").and_then(|f| f.as_object()) {
                for (id, meta) in fields {
                    let name = meta
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or(id)
                        .to_string();
                    seen.entry(id.clone()).or_insert(FieldInfo {
                        id: id.clone(),
                        name,
                        custom: id.starts_with("customfield_"),
                    });
                }
            }
        }
    }

    if projects.is_empty() {
        return Err(format!(
            "No project '{}' found (or it has no issue types with visible fields).",
            project
        )
        .into());
    }

    Ok(seen.into_values().collect())
}

fn role_label(role: &str) -> String {
    role.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn cmd_list_fields(project: Option<String>) -> Result<(), Box<dyn Error>> {
    let config = load_config();
    let client = JiraClient::new(&config)?;
    let mut fields = match &project {
        Some(p) => list_project_fields(&client, p)?,
        None => list_all_fields(&client)?,
    };
    if fields.is_empty() {
        println!("No fields found.");
        return Ok(());
    }
    fields.sort_by_key(|f| f.name.to_lowercase());
    for f in &fields {
        println!(
            "{:<20} {}{}",
            f.id,
            f.name,
            if f.custom { "  (custom)" } else { "" }
        );
    }
    Ok(())
}

pub fn cmd_assign_me(ticket_arg: Option<&str>) -> Result<(), Box<dyn Error>> {
    let config = load_config();
    let ticket = resolve_ticket(&config, ticket_arg)?;
    let client = JiraClient::new(&config)?;

    let value = get_issue_fields(&client, &ticket, &["assignee"])?;
    let fields = value.get("fields").cloned().unwrap_or(Value::Null);
    if let Some(u) = extract_user(&fields, "assignee") {
        println!(
            "{} is already assigned to {} ({}); leaving as-is.",
            ticket, u.display_name, u.account_id
        );
        return Ok(());
    }

    let me = resolve_current_user(&client, &config)?;
    set_user_field(&client, &ticket, "assignee", &me.account_id)?;
    println!(
        "Assigned {} to {} ({}).",
        ticket, me.display_name, me.account_id
    );
    Ok(())
}

pub fn cmd_show(ticket_arg: Option<&str>) -> Result<(), Box<dyn Error>> {
    let config = load_config();
    let ticket = resolve_ticket(&config, ticket_arg)?;
    let client = JiraClient::new(&config)?;

    let mut role_field_ids: Vec<(String, String)> = config
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    role_field_ids.sort_by(|a, b| a.0.cmp(&b.0));

    let mut field_ids = vec!["summary".to_string(), "assignee".to_string()];
    field_ids.extend(role_field_ids.iter().map(|(_, fid)| fid.clone()));
    let field_ids_refs: Vec<&str> = field_ids.iter().map(|s| s.as_str()).collect();

    let value = get_issue_fields(&client, &ticket, &field_ids_refs)?;
    let fields = value.get("fields").cloned().unwrap_or(Value::Null);
    let summary = fields
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("(no summary)");
    let assignee = extract_user(&fields, "assignee");

    println!("{}: {}", ticket, summary);
    match &assignee {
        Some(u) => println!("Assignee: {} ({})", u.display_name, u.account_id),
        None => println!("Assignee: (unassigned)"),
    }

    let mut roles_held: Vec<String> = vec![];
    for (role, fid) in &role_field_ids {
        let holder = extract_user(&fields, fid);
        match &holder {
            Some(u) => println!(
                "{}: {} ({})",
                role_label(role),
                u.display_name,
                u.account_id
            ),
            None => println!("{}: (unassigned)", role_label(role)),
        }
        if let (Some(a), Some(h)) = (&assignee, &holder)
            && a.account_id == h.account_id
        {
            roles_held.push(role_label(role));
        }
    }

    if assignee.is_some() {
        if roles_held.is_empty() {
            println!("The assignee holds none of the configured roles.");
        } else {
            println!("The assignee is also: {}", roles_held.join(", "));
        }
    }

    Ok(())
}

/// Reassigns a ticket's assignee to whoever currently holds the given role field, e.g.
/// `jira-reassign reviewer COMMON-807` hands COMMON-807 to its current reviewer. If
/// `<ticket>` is omitted, it's extracted from the current git branch name when
/// `branch_ticket_regex` is configured.
pub fn cmd_reassign_by_role(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (role, ticket_arg) = match args {
        [role, ticket] => (role, Some(ticket.as_str())),
        [role] => (role, None),
        _ => {
            return Err("Usage: jira-reassign <field> [ticket]\n\
                Where <field> is one of the roles configured in your config file \
                (e.g. reviewer, responsible-engineer). This reassigns <ticket> to \
                whoever currently holds that role. <ticket> may be omitted if \
                \"branch_ticket_regex\" is configured."
                .into());
        }
    };

    let config = load_config();
    let ticket = resolve_ticket(&config, ticket_arg)?;
    let field_id = config.fields.get(role).cloned().ok_or_else(|| {
        format!(
            "Unknown field role '{}'. Configured roles: {}",
            role,
            config.fields.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;

    let client = JiraClient::new(&config)?;
    let value = get_issue_fields(&client, &ticket, &["assignee", &field_id])?;
    let fields = value.get("fields").cloned().unwrap_or(Value::Null);
    let holder = extract_user(&fields, &field_id).ok_or_else(|| {
        format!(
            "{} has no {} set; nothing to reassign to.",
            ticket,
            role_label(role)
        )
    })?;
    let assignee = extract_user(&fields, "assignee");

    if let Some(a) = &assignee
        && a.account_id == holder.account_id
    {
        println!(
            "{} is already assigned to {} ({}); leaving as-is.",
            ticket, a.display_name, a.account_id
        );
        return Ok(());
    }

    set_user_field(&client, &ticket, "assignee", &holder.account_id)?;
    println!(
        "Reassigned {} to {} ({}), the ticket's {}.",
        ticket,
        holder.display_name,
        holder.account_id,
        role_label(role)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_uppercase_ticket_from_branch() {
        assert_eq!(
            extract_ticket_from_branch("feature/COMMON-807-fix-thing", DEFAULT_BRANCH_TICKET_REGEX)
                .unwrap(),
            "COMMON-807"
        );
    }

    #[test]
    fn extracts_and_uppercases_lowercase_ticket_from_branch() {
        assert_eq!(
            extract_ticket_from_branch("common-807-fix-thing", DEFAULT_BRANCH_TICKET_REGEX)
                .unwrap(),
            "COMMON-807"
        );
    }

    #[test]
    fn fails_when_no_ticket_in_branch() {
        assert!(extract_ticket_from_branch("main", DEFAULT_BRANCH_TICKET_REGEX).is_err());
    }

    #[test]
    fn custom_pattern_is_used_when_non_empty() {
        assert_eq!(
            extract_ticket_from_branch("feat/XYZ-42-thing", r"XYZ-\d+").unwrap(),
            "XYZ-42"
        );
    }
}
