use super::*;
use hickory_proto::op::MessageType;
use hickory_proto::rr::Name;
use hickory_proto::rr::rdata::{A, CNAME};
use hickory_proto::rr::rdata::{AAAA, NS};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

fn test_resolver() -> ResolverConfig {
    ResolverConfig {
        id: "mock".into(),
        name: "Mock DNS".into(),
        address: "127.0.0.1".parse().unwrap(),
        region: "local".into(),
        country: "XX".into(),
        provider: "Test".into(),
        enabled: true,
        tags: vec![],
    }
}

fn test_settings() -> SettingsConfig {
    SettingsConfig {
        timeout_ms: 1000,
        ..SettingsConfig::default()
    }
}

fn response_to(query: &Message, mut response: Message) -> Message {
    response
        .set_id(query.id())
        .set_message_type(MessageType::Response)
        .set_recursion_desired(query.recursion_desired());
    response.add_queries(query.queries().iter().cloned());
    response
}

async fn serve_udp(response: Message) -> (SocketAddr, tokio::task::JoinHandle<Message>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        let (len, peer) = timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let query = Message::from_vec(&buffer[..len]).unwrap();
        let response = response_to(&query, response);
        socket
            .send_to(&response.to_vec().unwrap(), peer)
            .await
            .unwrap();
        query
    });
    (addr, task)
}

#[tokio::test]
async fn sends_the_requested_recursion_flag_on_the_wire() {
    for recursive in [false, true] {
        let (addr, server) = serve_udp(Message::new()).await;
        query_endpoint_with_timeout(
            addr,
            Name::from_ascii("example.com.").unwrap(),
            RecordType::NS,
            recursive,
            &test_settings(),
        )
        .await
        .unwrap();
        let query = server.await.unwrap();
        assert_eq!(query.recursion_desired(), recursive);
        assert_eq!(query.queries().len(), 1);
        assert_eq!(
            query.queries()[0].name(),
            &Name::from_ascii("example.com.").unwrap()
        );
        assert_eq!(query.queries()[0].query_type(), RecordType::NS);
    }
}

#[tokio::test]
async fn reads_parent_delegation_and_only_its_glue_from_a_referral() {
    let name = Name::from_ascii("example.com.").unwrap();
    let ns = Name::from_ascii("ns.example.net.").unwrap();
    let mut response = Message::new();
    response.add_name_server(Record::from_rdata(
        name.clone(),
        300,
        RData::NS(NS(ns.clone())),
    ));
    response.add_additional(Record::from_rdata(
        ns.clone(),
        300,
        RData::A(A::new(192, 0, 2, 1)),
    ));
    response.add_additional(Record::from_rdata(
        ns,
        300,
        RData::AAAA(AAAA("2001:db8::1".parse().unwrap())),
    ));
    response.add_additional(Record::from_rdata(
        Name::from_ascii("unrelated.net.").unwrap(),
        300,
        RData::A(A::new(192, 0, 2, 99)),
    ));
    let (addr, server) = serve_udp(response).await;
    let response =
        query_endpoint_with_timeout(addr, name, RecordType::NS, false, &test_settings()).await;
    let report = parent_report(
        RootServer {
            name: "mock-tld".into(),
            address: addr.ip(),
        },
        response,
    );
    assert!(!server.await.unwrap().recursion_desired());
    assert_eq!(report.ns_records, vec!["ns.example.net."]);
    assert_eq!(report.glue_records, vec!["192.0.2.1", "2001:db8::1"]);
    assert!(report.error.is_none());
}

#[test]
fn accepts_answer_ns_records_and_rejects_empty_or_negative_parent_responses() {
    for code in [ResponseCode::NoError, ResponseCode::NXDomain] {
        let mut message = Message::new();
        message.set_response_code(code);
        let report = parent_report(
            RootServer {
                name: "mock".into(),
                address: test_resolver().address,
            },
            Ok((message, 1)),
        );
        assert!(report.error.is_some());
        assert!(report.ns_records.is_empty());
    }
    let mut message = Message::new();
    message.add_answer(Record::from_rdata(
        Name::from_ascii("example.com.").unwrap(),
        300,
        RData::NS(NS(Name::from_ascii("ns.example.net.").unwrap())),
    ));
    let report = parent_report(
        RootServer {
            name: "mock".into(),
            address: test_resolver().address,
        },
        Ok((message, 1)),
    );
    assert_eq!(report.ns_records, vec!["ns.example.net."]);
    assert!(report.error.is_none());
}

#[tokio::test]
async fn distinguishes_nodata_nxdomain_and_dns_failures() {
    for (code, expected) in [
        (ResponseCode::NoError, QueryStatus::NoRecords),
        (ResponseCode::NXDomain, QueryStatus::NxDomain),
        (ResponseCode::ServFail, QueryStatus::Error),
        (ResponseCode::Refused, QueryStatus::Error),
    ] {
        let mut message = Message::new();
        message.set_response_code(code);
        let (addr, server) = serve_udp(message).await;
        let response = query_endpoint_with_timeout(
            addr,
            Name::from_ascii("example.com.").unwrap(),
            RecordType::A,
            true,
            &test_settings(),
        )
        .await;
        let report = resolver_report(&test_resolver(), RecordType::A, response);
        server.await.unwrap();
        assert_eq!(report.status, expected, "rcode={code}");
        assert!(report.answers.is_empty());
        if expected == QueryStatus::Error {
            assert!(report.error.unwrap().contains(&code.to_string()));
        } else {
            assert!(report.error.is_none());
            let expected_json = if expected == QueryStatus::NxDomain {
                "nxdomain"
            } else {
                "no_records"
            };
            assert_eq!(serde_json::to_value(report.status).unwrap(), expected_json);
        }
    }
}

#[tokio::test]
async fn uses_tcp_after_udp_truncation_with_the_same_query_options() {
    for (recursive, code) in [
        (true, ResponseCode::NoError),
        (false, ResponseCode::NoError),
        (false, ResponseCode::Refused),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let udp = UdpSocket::bind(addr).await.unwrap();
        let server = tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            let (len, peer) = timeout(Duration::from_secs(3), udp.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            let udp_query = Message::from_vec(&buffer[..len]).unwrap();
            let mut truncated = Message::new();
            truncated.set_truncated(true);
            udp.send_to(&response_to(&udp_query, truncated).to_vec().unwrap(), peer)
                .await
                .unwrap();

            let (mut socket, _) = timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let tcp_query = timeout(Duration::from_secs(3), async {
                let len = socket.read_u16().await.unwrap();
                let mut bytes = vec![0; len as usize];
                socket.read_exact(&mut bytes).await.unwrap();
                Message::from_vec(&bytes).unwrap()
            })
            .await
            .unwrap();
            let mut message = Message::new();
            message.set_response_code(code);
            if code == ResponseCode::NoError {
                message.add_answer(Record::from_rdata(
                    tcp_query.queries()[0].name().clone(),
                    300,
                    RData::A(A::new(192, 0, 2, 10)),
                ));
            }
            let bytes = response_to(&tcp_query, message).to_vec().unwrap();
            socket.write_u16(bytes.len() as u16).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
            (udp_query, tcp_query)
        });
        let response = query_endpoint_with_timeout(
            addr,
            Name::from_ascii("example.com.").unwrap(),
            RecordType::A,
            recursive,
            &test_settings(),
        )
        .await;
        let (udp_query, tcp_query) = server.await.unwrap();
        assert_eq!(udp_query.recursion_desired(), recursive);
        assert_eq!(tcp_query.recursion_desired(), recursive);
        assert_eq!(udp_query.queries(), tcp_query.queries());
        if code == ResponseCode::NoError {
            let report = resolver_report(&test_resolver(), RecordType::A, response);
            assert_eq!(report.status, QueryStatus::Success);
            assert_eq!(report.answers, vec!["192.0.2.10"]);
            assert_eq!(report.ttl, Some(300));
        } else {
            assert!(
                response
                    .unwrap_err()
                    .to_string()
                    .contains(&code.to_string())
            );
        }
    }
}

#[tokio::test]
async fn times_out_a_silent_dns_server() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let settings = SettingsConfig {
        timeout_ms: 30,
        ..test_settings()
    };
    let response = query_endpoint_with_timeout(
        socket.local_addr().unwrap(),
        Name::from_ascii("example.com.").unwrap(),
        RecordType::A,
        true,
        &settings,
    )
    .await;
    let report = resolver_report(&test_resolver(), RecordType::A, response);
    assert_eq!(report.status, QueryStatus::Error);
    assert!(report.error.unwrap().contains("timed out after 30ms"));
}

#[test]
fn discovers_tld_authorities_from_root_referral_glue() {
    let ns = Name::from_ascii("a.gtld-servers.net.").unwrap();
    let mut message = Message::new();
    message.add_name_server(Record::from_rdata(
        Name::from_ascii("com.").unwrap(),
        300,
        RData::NS(NS(ns.clone())),
    ));
    message.add_additional(Record::from_rdata(ns, 300, RData::A(A::new(192, 0, 2, 1))));
    let servers = extract_tld_servers(&message).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "a.gtld-servers.net.");
    assert_eq!(servers[0].address, "192.0.2.1".parse::<IpAddr>().unwrap());
    assert!(extract_tld_servers(&Message::new()).is_err());
}

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
