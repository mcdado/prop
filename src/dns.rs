use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures::{StreamExt, stream};
use hickory_net::NetError;
use hickory_net::client::Client;
use hickory_net::runtime::TokioRuntimeProvider;
use hickory_net::tcp::TcpClientStream;
use hickory_net::udp::UdpClientStream;
use hickory_net::xfer::{DnsHandle, FirstAnswer};
use hickory_proto::op::{DnsRequestOptions, Message, Query, ResponseCode};
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use serde::Serialize;
use tokio::time::timeout;

use crate::config::{ResolverConfig, SettingsConfig};

const DNS_PORT: u16 = 53;
const ROOT_SERVERS: [(&str, &str); 13] = [
    ("a.root-servers.net", "198.41.0.4"),
    ("b.root-servers.net", "170.247.170.2"),
    ("c.root-servers.net", "192.33.4.12"),
    ("d.root-servers.net", "199.7.91.13"),
    ("e.root-servers.net", "192.203.230.10"),
    ("f.root-servers.net", "192.5.5.241"),
    ("g.root-servers.net", "192.112.36.4"),
    ("h.root-servers.net", "198.97.190.53"),
    ("i.root-servers.net", "192.36.148.17"),
    ("j.root-servers.net", "192.58.128.30"),
    ("k.root-servers.net", "193.0.14.129"),
    ("l.root-servers.net", "199.7.83.42"),
    ("m.root-servers.net", "202.12.27.33"),
];

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub name: String,
    pub record_type: String,
    #[serde(flatten)]
    pub consensus: ConsensusReport,
    pub results: Vec<QueryReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConsensusReport {
    #[serde(rename = "consensus")]
    pub answers: Vec<String>,
    #[serde(rename = "consensus_status")]
    pub status: ConsensusStatus,
    pub consensus_response_status: Option<QueryStatus>,
    pub agreeing_resolvers: usize,
    pub responding_resolvers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsensusStatus {
    Unanimous,
    Majority,
    NoMajority,
    NoResponses,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryReport {
    pub resolver: ResolverConfig,
    pub status: QueryStatus,
    pub duration_ms: Option<u128>,
    pub ttl: Option<u32>,
    pub answers: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    Success,
    NoRecords,
    #[serde(rename = "nxdomain")]
    NxDomain,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParentTraceReport {
    pub name: String,
    pub tld: String,
    pub root_servers: Vec<RootServer>,
    pub tld_servers: Vec<RootServer>,
    pub results: Vec<ParentTraceResult>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RootServer {
    pub name: String,
    pub address: IpAddr,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParentTraceResult {
    pub server_name: String,
    pub server_address: IpAddr,
    pub duration_ms: Option<u128>,
    pub ns_records: Vec<String>,
    pub glue_records: Vec<String>,
    pub error: Option<String>,
}

pub async fn run_check(
    name: &str,
    record_type: RecordType,
    resolvers: Vec<ResolverConfig>,
    settings: &SettingsConfig,
) -> Result<CheckReport> {
    settings.validate()?;
    let fqdn = ensure_fqdn(name)?;
    let settings = Arc::new(settings.clone());

    let resolvers: Vec<_> = resolvers.into_iter().filter(|r| r.enabled).collect();
    if resolvers.is_empty() {
        bail!("no enabled resolvers matched the supplied filters");
    }

    let mut results: Vec<QueryReport> = stream::iter(resolvers)
        .map(|resolver| {
            let fqdn = fqdn.clone();
            let settings = Arc::clone(&settings);
            async move { query_resolver(&resolver, &fqdn, record_type, &settings).await }
        })
        .buffer_unordered(settings.concurrency)
        .collect()
        .await;

    results.sort_by(|left, right| left.resolver.id.cmp(&right.resolver.id));

    let consensus = build_consensus(&results);

    Ok(CheckReport {
        name: fqdn.to_utf8(),
        record_type: record_type.to_string(),
        consensus,
        results,
    })
}

pub async fn trace_parent(name: &str, settings: &SettingsConfig) -> Result<ParentTraceReport> {
    settings.validate()?;
    let fqdn = ensure_fqdn(name)?;
    let trimmed = fqdn.to_utf8();
    let tld = trimmed
        .trim_end_matches('.')
        .rsplit('.')
        .next()
        .ok_or_else(|| anyhow!("domain name must contain at least one label"))?
        .to_string();

    let root_servers = ROOT_SERVERS
        .iter()
        .map(|(server_name, address)| RootServer {
            name: (*server_name).to_string(),
            address: address.parse().expect("root server IPs are valid"),
        })
        .collect::<Vec<_>>();

    let root_response = query_first_success(
        &root_servers,
        &Name::from_ascii(format!("{tld}."))?,
        RecordType::NS,
        settings,
    )
    .await
    .with_context(|| format!("failed to resolve TLD authorities for {tld}"))?;

    let tld_servers = extract_tld_servers(&root_response)?;
    let tld_query_name = fqdn.clone();

    let results: Vec<ParentTraceResult> = stream::iter(tld_servers.clone())
        .map(|server| {
            let settings = settings.clone();
            let tld_query_name = tld_query_name.clone();
            async move { query_parent_server(server, &tld_query_name, &settings).await }
        })
        .buffer_unordered(settings.concurrency)
        .collect()
        .await;

    Ok(ParentTraceReport {
        name: fqdn.to_utf8(),
        tld,
        root_servers,
        tld_servers,
        results,
    })
}

async fn query_resolver(
    resolver: &ResolverConfig,
    name: &Name,
    record_type: RecordType,
    settings: &SettingsConfig,
) -> QueryReport {
    let response = query_endpoint_with_timeout(
        SocketAddr::new(resolver.address, DNS_PORT),
        name.clone(),
        record_type,
        true,
        settings,
    )
    .await;
    resolver_report(resolver, record_type, response)
}

fn resolver_report(
    resolver: &ResolverConfig,
    record_type: RecordType,
    response: Result<(Message, u128)>,
) -> QueryReport {
    match response {
        Ok((message, duration_ms)) => {
            let nonexistent = message.response_code == ResponseCode::NXDomain;
            let answers = if nonexistent {
                vec![]
            } else {
                normalized_answers(&message.answers, record_type)
            };
            let ttl = if nonexistent {
                None
            } else {
                message.answers.iter().map(|record| record.ttl).min()
            };
            let status = if nonexistent {
                QueryStatus::NxDomain
            } else if answers.is_empty() {
                QueryStatus::NoRecords
            } else {
                QueryStatus::Success
            };

            QueryReport {
                resolver: resolver.clone(),
                status,
                duration_ms: Some(duration_ms),
                ttl,
                answers,
                error: None,
            }
        }
        Err(error) => QueryReport {
            resolver: resolver.clone(),
            status: QueryStatus::Error,
            duration_ms: None,
            ttl: None,
            answers: vec![],
            error: Some(error.to_string()),
        },
    }
}

async fn query_endpoint_with_timeout(
    addr: SocketAddr,
    name: Name,
    record_type: RecordType,
    recursion_desired: bool,
    settings: &SettingsConfig,
) -> Result<(Message, u128)> {
    let deadline = Duration::from_millis(settings.timeout_ms);
    match timeout(
        deadline,
        query_dns_server(addr, name, record_type, recursion_desired, deadline),
    )
    .await
    {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) if !matches!(error.downcast_ref::<NetError>(), Some(NetError::Timeout)) => {
            Err(error)
        }
        // The transport's timer can expire before the outer total-query timer.
        _ => bail!("query timed out after {}ms", settings.timeout_ms),
    }
}

async fn query_dns_server(
    addr: SocketAddr,
    name: Name,
    record_type: RecordType,
    recursion_desired: bool,
    deadline: Duration,
) -> Result<(Message, u128)> {
    let stream = UdpClientStream::builder(addr, TokioRuntimeProvider::new())
        .with_timeout(Some(deadline))
        .build();
    let (client, background) = Client::from_sender(stream);
    tokio::spawn(background);

    let start = Instant::now();
    let mut response = send_query(&client, name.clone(), record_type, recursion_desired)
        .await
        .with_context(|| format!("query to {addr} failed"))?;

    if response.truncation {
        response =
            query_dns_server_tcp(addr, name, record_type, recursion_desired, deadline).await?;
    }

    if !matches!(
        response.response_code,
        ResponseCode::NoError | ResponseCode::NXDomain
    ) {
        bail!("DNS server {addr} returned {}", response.response_code);
    }

    Ok((response, start.elapsed().as_millis()))
}

async fn query_dns_server_tcp(
    addr: SocketAddr,
    name: Name,
    record_type: RecordType,
    recursion_desired: bool,
    deadline: Duration,
) -> Result<Message> {
    let (stream, sender) =
        TcpClientStream::new(addr, None, Some(deadline), TokioRuntimeProvider::new());
    let stream = stream
        .await
        .with_context(|| format!("failed to connect over TCP to {addr}"))?;
    let (client, background) =
        Client::<TokioRuntimeProvider>::with_timeout(stream, sender, deadline);
    tokio::spawn(background);

    send_query(&client, name, record_type, recursion_desired)
        .await
        .with_context(|| format!("tcp query to {addr} failed"))
}

async fn send_query(
    client: &Client<TokioRuntimeProvider>,
    name: Name,
    record_type: RecordType,
    recursion_desired: bool,
) -> Result<Message> {
    let mut query = Query::query(name, record_type);
    query.set_query_class(DNSClass::IN);
    let mut options = DnsRequestOptions::default();
    options.recursion_desired = recursion_desired;
    options.use_edns = client.is_using_edns();
    Ok(client.lookup(query, options).first_answer().await?.into())
}

async fn query_first_success(
    servers: &[RootServer],
    name: &Name,
    record_type: RecordType,
    settings: &SettingsConfig,
) -> Result<Message> {
    let mut last_error = None;
    for server in servers {
        match query_endpoint_with_timeout(
            SocketAddr::new(server.address, DNS_PORT),
            name.clone(),
            record_type,
            false,
            settings,
        )
        .await
        {
            Ok((response, _)) => {
                if response.response_code != ResponseCode::NoError {
                    bail!("root server returned {} for {name}", response.response_code);
                }
                return Ok(response);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("no root servers available")))
}

fn extract_tld_servers(message: &Message) -> Result<Vec<RootServer>> {
    let mut ns_names = vec![];
    let mut addresses = BTreeMap::new();

    for record in &message.authorities {
        if let RData::NS(ns) = &record.data {
            ns_names.push(ns.to_utf8());
        }
    }

    for record in &message.additionals {
        match &record.data {
            RData::A(ipv4) => {
                addresses.insert(record.name.to_utf8(), IpAddr::V4((*ipv4).into()));
            }
            RData::AAAA(ipv6) => {
                addresses
                    .entry(record.name.to_utf8())
                    .or_insert(IpAddr::V6((*ipv6).into()));
            }
            _ => {}
        }
    }

    let mut servers = vec![];
    for ns_name in ns_names {
        if let Some(address) = addresses.get(&ns_name) {
            servers.push(RootServer {
                name: ns_name.clone(),
                address: *address,
            });
        }
    }

    if servers.is_empty() {
        bail!("root response did not include glue for TLD authorities")
    }

    Ok(servers)
}

async fn query_parent_server(
    server: RootServer,
    fqdn: &Name,
    settings: &SettingsConfig,
) -> ParentTraceResult {
    let response = query_endpoint_with_timeout(
        SocketAddr::new(server.address, DNS_PORT),
        fqdn.clone(),
        RecordType::NS,
        false,
        settings,
    )
    .await;
    parent_report(server, response)
}

fn parent_report(server: RootServer, response: Result<(Message, u128)>) -> ParentTraceResult {
    let mut report = ParentTraceResult {
        server_name: server.name,
        server_address: server.address,
        duration_ms: None,
        ns_records: vec![],
        glue_records: vec![],
        error: None,
    };
    match response {
        Ok((message, duration_ms)) => {
            report.duration_ms = Some(duration_ms);
            if message.response_code != ResponseCode::NoError {
                report.error = Some(format!("DNS response {}", message.response_code));
                return report;
            }
            let ns_records: Vec<_> = message
                .authorities
                .iter()
                .chain(&message.answers)
                .filter(|record| record.record_type() == RecordType::NS)
                .cloned()
                .collect();
            report.ns_records = normalized_answers(&ns_records, RecordType::NS);
            if report.ns_records.is_empty() {
                report.error = Some("response did not include a parent delegation".into());
                return report;
            }
            let glue: Vec<_> = message
                .additionals
                .iter()
                .filter(|record| {
                    report
                        .ns_records
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&record.name.to_utf8()))
                })
                .cloned()
                .collect();
            report.glue_records = normalized_answers(&glue, RecordType::A)
                .into_iter()
                .chain(normalized_answers(&glue, RecordType::AAAA))
                .collect();
        }
        Err(error) => report.error = Some(error.to_string()),
    }
    report
}

fn ensure_fqdn(name: &str) -> Result<Name> {
    let fqdn = if name.ends_with('.') {
        name.to_string()
    } else {
        format!("{name}.")
    };
    Name::from_ascii(fqdn).context("invalid domain name")
}

fn build_consensus(results: &[QueryReport]) -> ConsensusReport {
    let mut counts = BTreeMap::<(QueryStatus, Vec<String>), usize>::new();
    let mut responding_resolvers = 0;
    for result in results
        .iter()
        .filter(|result| result.status != QueryStatus::Error)
    {
        let mut answers = result.answers.clone();
        answers.sort();
        answers.dedup();
        *counts.entry((result.status, answers)).or_default() += 1;
        responding_resolvers += 1;
    }

    // Vote on complete responses, keeping NXDOMAIN distinct from an empty NOERROR answer.
    let winner = counts
        .into_iter()
        .find(|(_, count)| *count > responding_resolvers / 2);
    if let Some(((response_status, answers), agreeing_resolvers)) = winner {
        ConsensusReport {
            answers,
            status: if agreeing_resolvers == responding_resolvers {
                ConsensusStatus::Unanimous
            } else {
                ConsensusStatus::Majority
            },
            consensus_response_status: Some(response_status),
            agreeing_resolvers,
            responding_resolvers,
        }
    } else {
        ConsensusReport {
            answers: vec![],
            status: if responding_resolvers == 0 {
                ConsensusStatus::NoResponses
            } else {
                ConsensusStatus::NoMajority
            },
            consensus_response_status: None,
            agreeing_resolvers: 0,
            responding_resolvers,
        }
    }
}

fn normalized_answers(records: &[Record], record_type: RecordType) -> Vec<String> {
    let mut answers = BTreeSet::new();

    for record in records {
        let data = &record.data;

        let normalized = match (record_type, data) {
            (RecordType::A, RData::A(ipv4)) => Some(ipv4.to_string()),
            (RecordType::AAAA, RData::AAAA(ipv6)) => Some(ipv6.to_string()),
            (RecordType::CNAME, RData::CNAME(name)) => Some(name.to_lowercase().to_utf8()),
            (RecordType::NS, RData::NS(name)) => Some(name.to_lowercase().to_utf8()),
            (RecordType::MX, RData::MX(mx)) => Some(format!(
                "{} {}",
                mx.preference,
                mx.exchange.to_lowercase().to_utf8()
            )),
            (RecordType::TXT, RData::TXT(txt)) => {
                let bytes: Vec<u8> = txt
                    .txt_data
                    .iter()
                    .flat_map(|chunk| chunk.iter().copied())
                    .collect();
                Some(String::from_utf8_lossy(&bytes).into_owned())
            }
            _ => None,
        };

        if let Some(value) = normalized {
            answers.insert(value);
        }
    }

    answers.into_iter().collect()
}

#[cfg(test)]
mod tests;
