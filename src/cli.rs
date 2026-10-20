use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use hickory_proto::rr::RecordType;

#[derive(Debug, Parser)]
#[command(
    name = "prop",
    version,
    about = "DNS propagation CLI",
    flatten_help = true,
    after_help = "Examples:\n  prop check example.com\n  prop check example.com --record-type txt --json\n  prop check example.com --region asia --provider Google\n  prop trace-parent example.com\n  prop servers --all\n  prop config --validate\n  prop completions zsh"
)]
pub struct Cli {
    /// Path to a custom TOML configuration file
    #[arg(long, global = true, value_hint = clap::ValueHint::FilePath)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Check DNS propagation across public resolvers
    Check(CheckArgs),
    /// Query the parent TLD nameservers for a domain's delegation
    TraceParent(TraceParentArgs),
    /// List configured DNS resolvers
    Servers(ServersArgs),
    /// Show the configuration path and optionally validate the configuration
    Config(ConfigArgs),
    /// Print a shell completion script to stdout
    Completions(CompletionsArgs),
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

impl From<CompletionShell> for clap_complete::Shell {
    fn from(value: CompletionShell) -> Self {
        match value {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Fish => Self::Fish,
            CompletionShell::Powershell => Self::PowerShell,
        }
    }
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Domain name to query
    pub name: String,
    /// DNS record type to query
    #[arg(long, value_enum, default_value_t = RecordTypeArg::A)]
    pub record_type: RecordTypeArg,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
    /// Only query resolvers in this region (case-insensitive)
    #[arg(long)]
    pub region: Option<String>,
    /// Only query resolvers from this provider (case-insensitive)
    #[arg(long)]
    pub provider: Option<String>,
    /// Only query resolvers with this tag (case-insensitive)
    #[arg(long)]
    pub tag: Option<String>,
}

#[derive(Debug, Args)]
pub struct TraceParentArgs {
    /// Domain name whose parent delegation should be queried
    pub name: String,
    /// Print the parent trace as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ServersArgs {
    /// Print the resolver list as JSON
    #[arg(long)]
    pub json: bool,
    /// Include disabled resolvers
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Validate the configuration and resolver catalog
    #[arg(long)]
    pub validate: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum RecordTypeArg {
    A,
    Aaaa,
    Cname,
    Mx,
    Ns,
    Txt,
}

impl From<RecordTypeArg> for RecordType {
    fn from(value: RecordTypeArg) -> Self {
        match value {
            RecordTypeArg::A => RecordType::A,
            RecordTypeArg::Aaaa => RecordType::AAAA,
            RecordTypeArg::Cname => RecordType::CNAME,
            RecordTypeArg::Mx => RecordType::MX,
            RecordTypeArg::Ns => RecordType::NS,
            RecordTypeArg::Txt => RecordType::TXT,
        }
    }
}
