use anyhow::Result;
use comfy_table::{Cell, ContentArrangement, Table, presets::UTF8_FULL};
use serde::Serialize;

use crate::config::ResolverConfig;
use crate::dns::{CheckReport, ParentTraceReport, QueryStatus};

pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn print_servers_table(resolvers: &[ResolverConfig], include_disabled: bool) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header([
        "ID", "Name", "Address", "Provider", "Region", "Enabled", "Tags",
    ]);

    for resolver in resolvers {
        if !include_disabled && !resolver.enabled {
            continue;
        }

        table.add_row([
            Cell::new(&resolver.id),
            Cell::new(&resolver.name),
            Cell::new(resolver.address),
            Cell::new(&resolver.provider),
            Cell::new(format!("{}/{}", resolver.region, resolver.country)),
            Cell::new(if resolver.enabled { "yes" } else { "no" }),
            Cell::new(resolver.tags.join(",")),
        ]);
    }

    println!("{table}");
}

pub fn print_check_table(report: &CheckReport) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header([
        "Resolver", "Address", "Provider", "Region", "Status", "RTT ms", "TTL", "Answers", "Error",
    ]);

    for result in &report.results {
        let status = match result.status {
            QueryStatus::Success => "ok",
            QueryStatus::NoRecords => "empty",
            QueryStatus::NxDomain => "nxdomain",
            QueryStatus::Error => "error",
        };
        let ttl = result
            .ttl
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let duration = result
            .duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());

        table.add_row([
            Cell::new(&result.resolver.name),
            Cell::new(result.resolver.address),
            Cell::new(&result.resolver.provider),
            Cell::new(format!(
                "{}/{}",
                result.resolver.region, result.resolver.country
            )),
            Cell::new(status),
            Cell::new(duration),
            Cell::new(ttl),
            Cell::new(result.answers.join(", ")),
            Cell::new(result.error.as_deref().unwrap_or("-")),
        ]);
    }

    println!(
        "Query: {} {} | Consensus answers: {}",
        report.name,
        report.record_type,
        report.consensus.join(", ")
    );
    println!("{table}");
}

pub fn print_parent_trace_table(report: &ParentTraceReport) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header([
        "Server", "Address", "Status", "RTT ms", "NS", "Glue", "Error",
    ]);

    for result in &report.results {
        let duration = result
            .duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let status = if result.error.is_some() {
            "error"
        } else {
            "ok"
        };
        let ns_values = if result.ns_records.is_empty() {
            "-".to_string()
        } else {
            result.ns_records.join(", ")
        };
        let glue_values = if result.glue_records.is_empty() {
            "-".to_string()
        } else {
            result.glue_records.join(", ")
        };

        table.add_row([
            Cell::new(&result.server_name),
            Cell::new(result.server_address),
            Cell::new(status),
            Cell::new(duration),
            Cell::new(ns_values),
            Cell::new(glue_values),
            Cell::new(result.error.as_deref().unwrap_or("-")),
        ]);
    }

    println!("TLD: {} | Domain: {}", report.tld, report.name);
    println!("{table}");
}
