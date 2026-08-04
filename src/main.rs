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
    /// Assign the issue's assignee to yourself, only if it's currently unassigned
    AssignMe { ticket: String },
    /// Show who is currently assigned, and in what configured role(s)
    Show { ticket: String },
    /// `jira-reassign <field> <ticket>` — reassign the ticket to whoever currently holds
    /// that role field (from config), e.g. `jira-reassign reviewer COMMON-807` hands
    /// COMMON-807 to its current reviewer
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
        Commands::AssignMe { ticket } => cmd_assign_me(&ticket),
        Commands::Show { ticket } => cmd_show(&ticket),
        Commands::Field(args) => cmd_reassign_by_role(&args),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        exit(1);
    }
}
