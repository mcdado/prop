mod catalog;
mod cli;
mod config;
mod dns;
mod output;

use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser};
use clap_complete::Generator;
use cli::{Cli, Commands};
use config::{OutputFormat, load_config, merged_resolvers};
use dns::{run_check, trace_parent};
use output::{print_check_table, print_json, print_parent_trace_table, print_servers_table};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    let load = || -> Result<_> {
        let loaded = load_config(cli.config.as_deref()).context("failed to load config")?;
        let resolvers =
            merged_resolvers(&loaded.config).context("failed to build resolver catalog")?;
        Ok((loaded, resolvers))
    };

    match cli.command {
        Commands::Check(args) => {
            let (loaded, resolvers) = load()?;
            let filtered = filter_resolvers(
                resolvers,
                args.region.as_deref(),
                args.provider.as_deref(),
                args.tag.as_deref(),
            );
            if filtered.is_empty() {
                bail!("no resolvers matched the supplied filters");
            }
            let report = run_check(
                &args.name,
                args.record_type.into(),
                filtered,
                &loaded.config.settings,
            )
            .await?;

            if args.json || loaded.config.settings.output == OutputFormat::Json {
                print_json(&report)?;
            } else {
                print_check_table(&report);
            }
        }
        Commands::TraceParent(args) => {
            let (loaded, _) = load()?;
            let report = trace_parent(&args.name, &loaded.config.settings).await?;
            if args.json || loaded.config.settings.output == OutputFormat::Json {
                print_json(&report)?;
            } else {
                print_parent_trace_table(&report);
            }
        }
        Commands::Servers(args) => {
            let (loaded, resolvers) = load()?;
            if args.json || loaded.config.settings.output == OutputFormat::Json {
                let visible = if args.all {
                    resolvers
                } else {
                    resolvers
                        .into_iter()
                        .filter(|resolver| resolver.enabled)
                        .collect()
                };
                print_json(&visible)?;
            } else {
                print_servers_table(&resolvers, args.all);
            }
        }
        Commands::Config(args) => {
            let (loaded, _) = load()?;
            println!("config_path={}", display_path(&loaded.path));
            if args.validate {
                println!("config_valid=true");
            }
        }
        Commands::Completions(args) => {
            let mut command = Cli::command().bin_name("prop");
            command.build();
            let shell = clap_complete::Shell::from(args.shell);
            shell
                .try_generate(&command, &mut std::io::stdout().lock())
                .context("failed to write shell completions")?;
        }
    }

    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn filter_resolvers(
    resolvers: Vec<config::ResolverConfig>,
    region: Option<&str>,
    provider: Option<&str>,
    tag: Option<&str>,
) -> Vec<config::ResolverConfig> {
    resolvers
        .into_iter()
        .filter(|resolver| resolver.enabled)
        .filter(|resolver| {
            region
                .map(|value| resolver.region.eq_ignore_ascii_case(value))
                .unwrap_or(true)
        })
        .filter(|resolver| {
            provider
                .map(|value| resolver.provider.eq_ignore_ascii_case(value))
                .unwrap_or(true)
        })
        .filter(|resolver| {
            tag.map(|value| {
                resolver
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(value))
            })
            .unwrap_or(true)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn filters_resolvers_by_region_provider_and_tag() {
        let resolvers = vec![
            config::ResolverConfig {
                id: "one".to_string(),
                name: "One".to_string(),
                address: IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                region: "global".to_string(),
                country: "US".to_string(),
                provider: "Cloudflare".to_string(),
                enabled: true,
                tags: vec!["public".to_string()],
            },
            config::ResolverConfig {
                id: "two".to_string(),
                name: "Two".to_string(),
                address: IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
                region: "asia".to_string(),
                country: "JP".to_string(),
                provider: "Google".to_string(),
                enabled: true,
                tags: vec!["regional".to_string()],
            },
        ];

        let filtered = filter_resolvers(resolvers, Some("asia"), Some("google"), Some("regional"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "two");
    }
}
