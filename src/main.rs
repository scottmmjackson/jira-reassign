use clap::{Parser, Subcommand};
use jira_reassign::{cmd_assign_me, cmd_list_fields, cmd_reassign_by_role, cmd_show, init_config};
use std::process::exit;

#[derive(Parser)]
#[command(
    name = "jira-reassign",
    version,
    about = "Reassign Jira tickets by role field (reviewer, responsible engineer, etc.)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a config file template in the OS config directory
    Init,
    /// List available Jira fields, optionally scoped to a project (e.g. `list-fields COMMON`)
    ListFields { project: Option<String> },
    /// Assign the issue's assignee to yourself, only if it's currently unassigned.
    /// `ticket` may be omitted if `branch_ticket_regex` is configured.
    AssignMe { ticket: Option<String> },
    /// Show who is currently assigned, and in what configured role(s).
    /// `ticket` may be omitted if `branch_ticket_regex` is configured.
    Show { ticket: Option<String> },
    /// `jira-reassign <field> [ticket]` — reassign the ticket to whoever currently holds
    /// that role field (from config), e.g. `jira-reassign reviewer COMMON-807` hands
    /// COMMON-807 to its current reviewer. `ticket` may be omitted if `branch_ticket_regex`
    /// is configured.
    #[command(external_subcommand)]
    Field(Vec<String>),
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => {
            init_config();
            Ok(())
        }
        Commands::ListFields { project } => cmd_list_fields(project),
        Commands::AssignMe { ticket } => cmd_assign_me(ticket.as_deref()),
        Commands::Show { ticket } => cmd_show(ticket.as_deref()),
        Commands::Field(args) => cmd_reassign_by_role(&args),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        exit(1);
    }
}
