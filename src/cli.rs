use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use hickory_proto::rr::RecordType;

#[derive(Debug, Parser)]
#[command(name = "prop", version, about = "DNS propagation CLI")]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Check(CheckArgs),
    TraceParent(TraceParentArgs),
    Servers(ServersArgs),
    Config(ConfigArgs),
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    pub name: String,
    #[arg(long, value_enum, default_value_t = RecordTypeArg::A)]
    pub record_type: RecordTypeArg,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub region: Option<String>,
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
}

#[derive(Debug, Args)]
pub struct TraceParentArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ServersArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
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
