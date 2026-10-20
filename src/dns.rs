use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures::{StreamExt, stream};
use hickory_client::client::{AsyncClient, ClientHandle};
use hickory_client::udp::UdpClientStream;
use hickory_proto::iocompat::AsyncIoTokioAsStd;
use hickory_proto::op::Message;
#[cfg(test)]
use hickory_proto::op::{MessageType, OpCode, Query};
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
    pub consensus: Vec<String>,
    pub results: Vec<QueryReport>,
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

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    Success,
    NoRecords,
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
    match query_with_timeout(resolver.address, name.clone(), record_type, settings).await {
        Ok((message, duration_ms)) => {
            let mut answers = normalized_answers(&message.answers(), record_type);
            answers.sort();
            answers.dedup();

            let ttl = message.answers().iter().map(Record::ttl).min();
            let status = if answers.is_empty() {
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

async fn query_with_timeout(
    server: IpAddr,
    name: Name,
    record_type: RecordType,
    settings: &SettingsConfig,
) -> Result<(Message, u128)> {
    let deadline = Duration::from_millis(settings.timeout_ms);
    timeout(deadline, query_dns_server(server, name, record_type))
        .await
        .map_err(|_| anyhow!("query timed out after {}ms", settings.timeout_ms))?
}

async fn query_dns_server(
    server: IpAddr,
    name: Name,
    record_type: RecordType,
) -> Result<(Message, u128)> {
    let addr = SocketAddr::new(server, DNS_PORT);
    let stream = UdpClientStream::<tokio::net::UdpSocket>::new(addr);
    let (mut client, background) = AsyncClient::connect(stream)
        .await
        .with_context(|| format!("failed to connect to {addr}"))?;
    tokio::spawn(background);

    let start = Instant::now();
    let mut response: Message = client
        .query(name.clone(), DNSClass::IN, record_type)
        .await
        .with_context(|| format!("query to {addr} failed"))?
        .into();

    if response.truncated() {
        response = query_dns_server_tcp(server, name, record_type).await?;
    }

    Ok((response, start.elapsed().as_millis()))
}

async fn query_dns_server_tcp(
    server: IpAddr,
    name: Name,
    record_type: RecordType,
) -> Result<Message> {
    let addr = SocketAddr::new(server, DNS_PORT);
    let (stream, sender) =
        hickory_client::tcp::TcpClientStream::<AsyncIoTokioAsStd<tokio::net::TcpStream>>::new(addr);
    let client = AsyncClient::new(stream, sender, None);
    let (mut client, background) = client
        .await
        .with_context(|| format!("failed to connect over TCP to {addr}"))?;
    tokio::spawn(background);

    client
        .query(name, DNSClass::IN, record_type)
        .await
        .with_context(|| format!("tcp query to {addr} failed"))
        .map(Into::into)
}

async fn query_first_success(
    servers: &[RootServer],
    name: &Name,
    record_type: RecordType,
    settings: &SettingsConfig,
) -> Result<Message> {
    let mut last_error = None;
    for server in servers {
        match query_with_timeout(server.address, name.clone(), record_type, settings).await {
            Ok((response, _)) => return Ok(response),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("no root servers available")))
}

fn extract_tld_servers(message: &Message) -> Result<Vec<RootServer>> {
    let mut ns_names = vec![];
    let mut addresses = BTreeMap::new();

    for record in message.name_servers() {
        if let Some(RData::NS(ns)) = record.data() {
            ns_names.push(ns.to_utf8());
        }
    }

    for record in message.additionals() {
        if let Some(data) = record.data() {
            match data {
                RData::A(ipv4) => {
                    addresses.insert(record.name().to_utf8(), IpAddr::V4((*ipv4).into()));
                }
                RData::AAAA(ipv6) => {
                    addresses
                        .entry(record.name().to_utf8())
                        .or_insert(IpAddr::V6((*ipv6).into()));
                }
                _ => {}
            }
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
    match query_with_timeout(server.address, fqdn.clone(), RecordType::NS, settings).await {
        Ok((message, duration_ms)) => ParentTraceResult {
            server_name: server.name,
            server_address: server.address,
            duration_ms: Some(duration_ms),
            ns_records: normalized_answers(&message.answers(), RecordType::NS),
            glue_records: normalized_answers(&message.additionals(), RecordType::A)
                .into_iter()
                .chain(normalized_answers(&message.additionals(), RecordType::AAAA))
                .collect(),
            error: None,
        },
        Err(error) => ParentTraceResult {
            server_name: server.name,
            server_address: server.address,
            duration_ms: None,
            ns_records: vec![],
            glue_records: vec![],
            error: Some(error.to_string()),
        },
    }
}

fn ensure_fqdn(name: &str) -> Result<Name> {
    let fqdn = if name.ends_with('.') {
        name.to_string()
    } else {
        format!("{name}.")
    };
    Name::from_ascii(fqdn).context("invalid domain name")
}

fn build_consensus(results: &[QueryReport]) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();

    for result in results {
        for answer in &result.answers {
            *counts.entry(answer.clone()).or_default() += 1;
        }
    }

    let max = counts.values().copied().max().unwrap_or_default();
    counts
        .into_iter()
        .filter_map(|(answer, count)| (count == max).then_some(answer))
        .collect()
}

fn normalized_answers(records: &[Record], record_type: RecordType) -> Vec<String> {
    let mut answers = BTreeSet::new();

    for record in records {
        let Some(data) = record.data() else {
            continue;
        };

        let normalized = match (record_type, data) {
            (RecordType::A, RData::A(ipv4)) => Some(ipv4.to_string()),
            (RecordType::AAAA, RData::AAAA(ipv6)) => Some(ipv6.to_string()),
            (RecordType::CNAME, RData::CNAME(name)) => Some(name.to_utf8()),
            (RecordType::NS, RData::NS(name)) => Some(name.to_utf8()),
            (RecordType::MX, RData::MX(mx)) => {
                Some(format!("{} {}", mx.preference(), mx.exchange().to_utf8()))
            }
            (RecordType::TXT, RData::TXT(txt)) => Some(
                txt.txt_data()
                    .iter()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };

        if let Some(value) = normalized {
            answers.insert(value);
        }
    }

    answers.into_iter().collect()
}

#[cfg(test)]
pub fn build_query_message(name: &Name, record_type: RecordType) -> Message {
    let mut message = Message::new();
    message
        .set_id(0)
        .set_message_type(MessageType::Query)
        .set_op_code(OpCode::Query)
        .add_query(Query::query(name.clone(), record_type))
        .set_recursion_desired(false);
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::Name;
    use hickory_proto::rr::rdata::{A, CNAME};

    #[tokio::test]
    async fn rejects_invalid_settings_before_querying() {
        for settings in [
            SettingsConfig {
                concurrency: 0,
                ..SettingsConfig::default()
            },
            SettingsConfig {
                timeout_ms: 0,
                ..SettingsConfig::default()
            },
        ] {
            assert!(
                run_check("example.com", RecordType::A, vec![], &settings)
                    .await
                    .is_err()
            );
            assert!(trace_parent("example.com", &settings).await.is_err());
        }
    }

    #[tokio::test]
    async fn rejects_an_empty_enabled_resolver_selection() {
        let disabled = ResolverConfig {
            id: "disabled".into(),
            name: "Disabled".into(),
            address: "127.0.0.1".parse().unwrap(),
            region: "local".into(),
            country: "XX".into(),
            provider: "Test".into(),
            enabled: false,
            tags: vec![],
        };
        let error = run_check(
            "example.com",
            RecordType::A,
            vec![disabled],
            &SettingsConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("no enabled resolvers"));
    }

    #[test]
    fn normalizes_a_and_cname_answers() {
        let name = Name::from_ascii("www.example.com.").expect("valid name");
        let a_record = Record::from_rdata(name.clone(), 300, RData::A(A::new(203, 0, 113, 20)));
        let cname_record = Record::from_rdata(
            name,
            300,
            RData::CNAME(CNAME(
                Name::from_ascii("edge.example.net.").expect("valid cname"),
            )),
        );

        assert_eq!(
            normalized_answers(&[a_record], RecordType::A),
            vec!["203.0.113.20".to_string()]
        );
        assert_eq!(
            normalized_answers(&[cname_record], RecordType::CNAME),
            vec!["edge.example.net.".to_string()]
        );
    }

    #[test]
    fn builds_query_message_without_recursion() {
        let name = Name::from_ascii("example.com.").expect("valid name");
        let message = build_query_message(&name, RecordType::NS);

        assert_eq!(message.queries().len(), 1);
        assert!(!message.recursion_desired());
    }
}
