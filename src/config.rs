use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dirs::config_dir;
use serde::{Deserialize, Serialize};

use crate::catalog::builtin_resolvers;

const APP_NAME: &str = "prop";
const CONFIG_FILE: &str = "config.toml";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolverConfig {
    pub id: String,
    pub name: String,
    pub address: IpAddr,
    pub region: String,
    pub country: String,
    pub provider: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub settings: SettingsConfig,
    #[serde(default)]
    pub servers: Vec<ResolverConfig>,
    #[serde(default)]
    pub overrides: Vec<ResolverOverride>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsConfig {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_output_format")]
    pub output: OutputFormat,
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
            concurrency: default_concurrency(),
            output: default_output_format(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    #[default]
    Table,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ResolverOverride {
    pub id: String,
    pub name: Option<String>,
    pub address: Option<IpAddr>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub provider: Option<String>,
    pub enabled: Option<bool>,
    pub tags: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: AppConfig,
}

pub fn default_config_path() -> PathBuf {
    if let Some(mut dir) = config_dir() {
        dir.push(APP_NAME);
        dir.push(CONFIG_FILE);
        dir
    } else {
        PathBuf::from(CONFIG_FILE)
    }
}

pub fn load_config(path: Option<&Path>) -> Result<LoadedConfig> {
    let resolved_path = path.map(Path::to_path_buf).unwrap_or_else(default_config_path);

    if !resolved_path.exists() {
        return Ok(LoadedConfig {
            path: resolved_path,
            config: AppConfig::default(),
        });
    }

    let contents = fs::read_to_string(&resolved_path)
        .with_context(|| format!("failed to read config file {}", resolved_path.display()))?;
    let config: AppConfig = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file {}", resolved_path.display()))?;
    validate_config(&config)?;

    Ok(LoadedConfig {
        path: resolved_path,
        config,
    })
}

pub fn validate_config(config: &AppConfig) -> Result<()> {
    let mut ids = BTreeSet::new();
    for resolver in &config.servers {
        if !ids.insert(resolver.id.clone()) {
            bail!("duplicate resolver id in servers: {}", resolver.id);
        }
    }

    let mut override_ids = BTreeSet::new();
    for resolver in &config.overrides {
        if !override_ids.insert(resolver.id.clone()) {
            bail!("duplicate resolver id in overrides: {}", resolver.id);
        }
    }

    Ok(())
}

pub fn merged_resolvers(config: &AppConfig) -> Result<Vec<ResolverConfig>> {
    validate_config(config)?;

    let mut merged = BTreeMap::new();
    for resolver in builtin_resolvers() {
        merged.insert(resolver.id.clone(), resolver);
    }

    for resolver in &config.servers {
        merged.insert(resolver.id.clone(), resolver.clone());
    }

    for override_config in &config.overrides {
        let Some(resolver) = merged.get_mut(&override_config.id) else {
            bail!("override references unknown resolver id: {}", override_config.id);
        };

        if let Some(name) = &override_config.name {
            resolver.name = name.clone();
        }
        if let Some(address) = override_config.address {
            resolver.address = address;
        }
        if let Some(region) = &override_config.region {
            resolver.region = region.clone();
        }
        if let Some(country) = &override_config.country {
            resolver.country = country.clone();
        }
        if let Some(provider) = &override_config.provider {
            resolver.provider = provider.clone();
        }
        if let Some(enabled) = override_config.enabled {
            resolver.enabled = enabled;
        }
        if let Some(tags) = &override_config.tags {
            resolver.tags = tags.clone();
        }
    }

    Ok(merged.into_values().collect())
}

fn default_true() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    2_500
}

fn default_concurrency() -> usize {
    32
}

fn default_output_format() -> OutputFormat {
    OutputFormat::Table
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn merges_builtin_override_and_custom_resolvers() {
        let config = AppConfig {
            settings: SettingsConfig::default(),
            servers: vec![ResolverConfig {
                id: "custom-apac".to_string(),
                name: "Custom APAC".to_string(),
                address: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
                region: "asia".to_string(),
                country: "SG".to_string(),
                provider: "Example".to_string(),
                enabled: true,
                tags: vec!["apac".to_string()],
            }],
            overrides: vec![ResolverOverride {
                id: "google-primary".to_string(),
                enabled: Some(false),
                tags: Some(vec!["disabled".to_string()]),
                ..ResolverOverride::default()
            }],
        };

        let merged = merged_resolvers(&config).expect("merge should work");
        let google = merged
            .iter()
            .find(|resolver| resolver.id == "google-primary")
            .expect("google primary should exist");
        let custom = merged
            .iter()
            .find(|resolver| resolver.id == "custom-apac")
            .expect("custom resolver should exist");

        assert!(!google.enabled);
        assert_eq!(google.tags, vec!["disabled"]);
        assert_eq!(custom.provider, "Example");
    }

    #[test]
    fn rejects_duplicate_server_ids() {
        let resolver = ResolverConfig {
            id: "dup".to_string(),
            name: "Duplicate".to_string(),
            address: IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            region: "global".to_string(),
            country: "US".to_string(),
            provider: "Test".to_string(),
            enabled: true,
            tags: vec![],
        };
        let config = AppConfig {
            settings: SettingsConfig::default(),
            servers: vec![resolver.clone(), resolver],
            overrides: vec![],
        };

        let error = validate_config(&config).expect_err("validation should fail");
        assert!(error.to_string().contains("duplicate resolver id"));
    }
}
