// SPDX-License-Identifier: FSL-1.1-ALv2
//! The resolver of a hostname-egress bridge. The host firewall redirects every UDP and TCP port 53
//! packet from the bridge here, and the container's embedded resolver forwards here too, so a name
//! reaches the host's resolver only when the policy allows it; any other query is answered REFUSED.
//! Each query's single question is checked before the maintained DNS client validates its
//! upstream transaction. Guest TCP framing does not require an upstream TCP-only resolver.
//!
//! Not a cache or a recursive resolver: the host's own resolver (`/etc/resolv.conf`) answers, and
//! the HTTP/TLS gateways still resolve and check every address they dial.
use super::*;
use hickory_net::{
    proto::{
        op::{DnsRequest, DnsRequestOptions, Header, Message, MessageType, OpCode, Query, ResponseCode},
        rr::Name,
        serialize::binary::{BinDecodable, BinDecoder, EncodedSize},
    },
    runtime::TokioRuntimeProvider,
    tcp::TcpClientStream,
    udp::UdpClientStream,
    xfer::{DnsHandle, FirstAnswer},
};
use std::{io, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

/// DNS's port, for UDP and TCP (RFC 1035 §4.2).
pub(crate) const PORT: u16 = 53;
const HEADER: usize = Header::LEN;
const FORMERR: ResponseCode = ResponseCode::FormErr;
const SERVFAIL: ResponseCode = ResponseCode::ServFail;
const NOTIMP: ResponseCode = ResponseCode::NotImp;
const REFUSED: ResponseCode = ResponseCode::Refused;
const MAX_NAME: usize = Name::MAX_LENGTH;
/// The largest message TCP's two-byte length prefix can frame (RFC 1035 §4.2.2).
const MAX_MESSAGE: usize = u16::MAX as usize;
const UPSTREAM: Duration = Duration::from_secs(5);
const RESOLV_CONF: &str = "/etc/resolv.conf";

/// A query's one question: its name, lowercased and dot-joined, and where the question ends.
#[derive(Debug, PartialEq)]
pub(super) struct Question {
    pub(super) name: String,
    end: usize,
}

/// Why a message gets no forwarding: the RCODE to answer with and the question to echo, or no
/// answer at all (not a query, or too short to carry an id).
#[derive(Debug, PartialEq)]
pub(super) enum Refusal {
    Answer(ResponseCode, Option<usize>),
    Silent,
}

/// The single question of a standard query. Labels are hostname characters only (letters, digits,
/// `-` and `_`), so the name the policy matches is the name the upstream resolver is asked.
pub(super) fn question(message: &[u8]) -> Result<Question, Refusal> {
    if message.len() < HEADER {
        return Err(Refusal::Silent);
    }
    let header = Header::read(&mut BinDecoder::new(message)).map_err(|_| Refusal::Answer(FORMERR, None))?;
    if header.message_type != MessageType::Query {
        return Err(Refusal::Silent);
    }
    if header.op_code != OpCode::Query {
        return Err(Refusal::Answer(NOTIMP, None));
    }
    if header.counts.queries != 1 {
        return Err(Refusal::Answer(FORMERR, None));
    }
    let mut at = HEADER;
    let mut labels = Vec::new();
    loop {
        let length = usize::from(*message.get(at).ok_or(Refusal::Answer(FORMERR, None))?);
        at += 1;
        if length == 0 {
            break;
        }
        // A pointer or an extended label type: never needed in a query's question.
        if length & 0xc0 != 0 || at - HEADER + length > MAX_NAME {
            return Err(Refusal::Answer(FORMERR, None));
        }
        let label = message.get(at..at + length).ok_or(Refusal::Answer(FORMERR, None))?;
        at += length;
        if !label.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) {
            return Err(Refusal::Answer(REFUSED, None));
        }
        labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
    }
    let end = at + 4;
    if message.len() < end {
        return Err(Refusal::Answer(FORMERR, None));
    }
    Ok(Question { name: labels.join("."), end })
}

/// An answer carrying only `rcode`, the query's id, opcode and RD bit, and its question if known.
fn refusal(message: &[u8], rcode: ResponseCode, end: Option<usize>) -> Option<Vec<u8>> {
    let header = Header::read(&mut BinDecoder::new(message)).ok()?;
    let mut response = Message::response(header.id, header.op_code);
    response.metadata.recursion_desired = header.recursion_desired;
    response.metadata.response_code = rcode;
    if let Some(end) = end {
        response.queries.push(Query::read(&mut BinDecoder::new(&message[HEADER..end])).ok()?);
    }
    response.to_vec().ok()
}

/// What the sandbox is told for `message`: the host resolver's answer for an allowed name, REFUSED
/// for any other, SERVFAIL when the host resolver doesn't answer, nothing for a non-query.
pub(super) async fn answer(message: &[u8], policy: &Policy) -> Option<Vec<u8>> {
    match question(message) {
        Ok(question) if gateway::allowed(&question.name, policy) => match forward(message).await {
            Ok(answer) => Some(answer),
            Err(_) => refusal(message, SERVFAIL, Some(question.end)),
        },
        Ok(question) => refusal(message, REFUSED, Some(question.end)),
        Err(Refusal::Answer(rcode, end)) => refusal(message, rcode, end),
        Err(Refusal::Silent) => None,
    }
}

/// The nameservers `/etc/resolv.conf`'s text names, in order.
fn nameservers_in(text: &str) -> impl Iterator<Item = IpAddr> + '_ {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|rest| rest.trim().parse::<IpAddr>().ok())
}

/// Every nameserver the host's resolver configuration names now.
pub(crate) fn nameservers() -> Vec<IpAddr> {
    std::fs::read_to_string(RESOLV_CONF).map(|text| nameservers_in(&text).collect()).unwrap_or_default()
}

/// The host's first nameserver, read for each query so a changed `/etc/resolv.conf` is followed.
async fn upstream() -> io::Result<SocketAddr> {
    let text = tokio::fs::read_to_string(RESOLV_CONF).await?;
    nameservers_in(&text)
        .next()
        .map(|ip| SocketAddr::new(ip, PORT))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no nameserver"))
}

async fn forward(message: &[u8]) -> io::Result<Vec<u8>> {
    forward_to(message, upstream().await?).await
}

async fn forward_to(message: &[u8], server: SocketAddr) -> io::Result<Vec<u8>> {
    tokio::time::timeout(UPSTREAM, async {
        let query = Message::from_vec(message).map_err(io::Error::other)?;
        let id = query.metadata.id;
        let request = DnsRequest::new(query, DnsRequestOptions::default());
        let provider = TokioRuntimeProvider::new();
        let udp = UdpClientStream::builder(server, provider.clone())
            .with_timeout(Some(UPSTREAM))
            .with_max_retries(0)
            .exchange();
        let mut response = udp.send(request.clone()).first_answer().await.map_err(io::Error::other)?;
        if response.metadata.truncation {
            let tcp =
                TcpClientStream::exchange(server, None, UPSTREAM, Some(1), provider).await.map_err(io::Error::other)?;
            response = tcp.send(request).first_answer().await.map_err(io::Error::other)?;
        }
        let mut response = response.into_message();
        response.metadata.id = id;
        response.to_vec().map_err(io::Error::other)
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

async fn read_framed(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let length = usize::from(stream.read_u16().await?);
    let mut message = vec![0; length];
    stream.read_exact(&mut message).await?;
    Ok(message)
}

async fn write_framed(stream: &mut TcpStream, message: &[u8]) -> io::Result<()> {
    let length = u16::try_from(message.len()).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut framed = length.to_be_bytes().to_vec();
    framed.extend_from_slice(message);
    stream.write_all(&framed).await
}

/// Answers the bridge's queries on `udp` and `tcp` until `cancel`, from `subnet` only.
pub(in crate::runtime) fn serve(
    udp: UdpSocket,
    tcp: TcpListener,
    subnet: ipnet::IpNet,
    policy: Policy,
    capacity: Arc<Semaphore>,
    cancel: CancellationToken,
) {
    let udp = Arc::new(udp);
    let (udp_policy, udp_capacity, udp_cancel) = (policy.clone(), capacity.clone(), cancel.clone());
    tokio::spawn(async move {
        let mut buffer = vec![0; MAX_MESSAGE];
        loop {
            let received = tokio::select! {()=udp_cancel.cancelled()=>break,result=udp.recv_from(&mut buffer)=>result};
            let Ok((n, peer)) = received else { continue };
            if !subnet.contains(&peer.ip()) {
                continue;
            }
            let Ok(permit) = udp_capacity.clone().try_acquire_owned() else { continue };
            let (udp, policy, message) = (udp.clone(), udp_policy.clone(), buffer[..n].to_vec());
            tokio::spawn(async move {
                let _permit = permit;
                if let Some(answer) = answer(&message, &policy).await {
                    let _ = udp.send_to(&answer, peer).await;
                }
            });
        }
    });
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {()=cancel.cancelled()=>break,result=tcp.accept()=>result};
            let Ok((mut stream, peer)) = accepted else { break };
            if !subnet.contains(&peer.ip()) {
                continue;
            }
            let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
            let (policy, cancel) = (policy.clone(), cancel.clone());
            tokio::spawn(async move {
                let _permit = permit;
                let work = async {
                    while let Ok(Ok(message)) = tokio::time::timeout(UPSTREAM, read_framed(&mut stream)).await {
                        let Some(answer) = answer(&message, &policy).await else { break };
                        if write_framed(&mut stream, &answer).await.is_err() {
                            break;
                        }
                    }
                };
                tokio::select! {()=cancel.cancelled()=>{},_=tokio::time::timeout(Duration::from_secs(600),work)=>{}}
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_net::proto::rr::{RData, Record, rdata::A};

    /// A standard recursive query for `name`, type A, class IN, with an EDNS OPT record after it.
    fn query(name: &str) -> Vec<u8> {
        let mut message = Message::query().metadata.id.to_be_bytes().to_vec();
        message.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 1]);
        for label in name.split('.').filter(|l| !l.is_empty()) {
            message.push(label.len() as u8);
            message.extend_from_slice(label.as_bytes());
        }
        message.extend_from_slice(&[0, 0, 1, 0, 1]);
        message.extend_from_slice(&[0, 0, 41, 4, 0, 0, 0, 0, 0, 0, 0]);
        message
    }

    fn policy() -> Policy {
        Policy::Names(EgressPolicy {
            allow: vec!["example.com".into(), "*.example.org".into()],
            deny: vec!["x.example.org".into()],
        })
    }

    #[test]
    fn reads_the_question_name_lowercased_and_where_it_ends() {
        let message = query("WWW.Example.COM");
        let question = question(&message).unwrap();
        assert_eq!(question.name, "www.example.com");
        assert_eq!(question.end, HEADER + 17 + 4);
        assert_eq!(super::question(&query("_dns._udp.example.com")).unwrap().name, "_dns._udp.example.com");
    }

    #[test]
    fn malformed_and_non_query_messages_are_refused_without_a_panic() {
        let message = query("example.com");
        for at in 0..HEADER {
            assert_eq!(question(&message[..at]), Err(Refusal::Silent));
        }
        for at in HEADER..question(&message).unwrap().end {
            assert_eq!(question(&message[..at]), Err(Refusal::Answer(FORMERR, None)), "cut at {at}");
        }
        let mut response = message.clone();
        response[2] |= 0x80;
        assert_eq!(question(&response), Err(Refusal::Silent));
        let mut notify = message.clone();
        notify[2] = 4 << 3;
        assert_eq!(question(&notify), Err(Refusal::Answer(NOTIMP, None)));
        let mut two = message.clone();
        two[5] = 2;
        assert_eq!(question(&two), Err(Refusal::Answer(FORMERR, None)));
        let mut pointer = message.clone();
        pointer[HEADER] = 0xc0;
        assert_eq!(question(&pointer), Err(Refusal::Answer(FORMERR, None)));
        let mut dotted = message.clone();
        dotted[HEADER + 1 + 3] = b'.';
        assert_eq!(question(&dotted), Err(Refusal::Answer(REFUSED, None)), "a dot inside a label");
        let long = query(&vec!["a".repeat(63); 5].join("."));
        assert_eq!(question(&long), Err(Refusal::Answer(FORMERR, None)), "past 255 bytes");
        for length in 0..512 {
            let _ = question(&vec![0; length]);
            let _ = question(&vec![0x3f; length]);
        }
    }

    #[tokio::test]
    async fn names_outside_the_policy_get_refused_with_their_question() {
        for name in ["exfil.attacker.test", "example.com.attacker.test", "x.example.org", "example.org"] {
            let message = query(name);
            let answer = answer(&message, &policy()).await.unwrap();
            let end = question(&message).unwrap().end;
            assert_eq!(&answer[..2], &message[..2], "same id");
            assert_eq!(answer[2], 0x81, "a response, recursion desired kept");
            assert_eq!(answer[3], REFUSED.low(), "{name}");
            assert_eq!(&answer[4..12], &[0, 1, 0, 0, 0, 0, 0, 0]);
            assert_eq!(&answer[HEADER..], &message[HEADER..end]);
        }
        let mut response = query("exfil.attacker.test");
        response[2] |= 0x80;
        assert_eq!(answer(&response, &policy()).await, None, "a response is never answered");
    }

    #[test]
    fn the_policy_decides_on_the_parsed_name() {
        let allowed = |name: &str| gateway::allowed(&question(&query(name)).unwrap().name, &policy());
        assert!(allowed("Example.com"));
        assert!(allowed("www.example.org"));
        assert!(!allowed("x.example.org"));
        assert!(!allowed("data.example.com"));
        assert!(!allowed("example.com.evil.test"));
    }

    fn upstream_response(bytes: &[u8], truncated: bool, code: ResponseCode) -> Vec<u8> {
        let query = Message::from_vec(bytes).unwrap();
        let mut response = Message::response(query.metadata.id, query.metadata.op_code);
        response.metadata.recursion_available = true;
        response.metadata.truncation = truncated;
        response.metadata.response_code = code;
        response.queries = query.queries;
        if code == ResponseCode::NoError && !truncated {
            response.answers.push(Record::from_rdata(
                response.queries[0].name.clone(),
                0,
                RData::A(A(std::net::Ipv4Addr::LOCALHOST)),
            ));
        }
        response.to_vec().unwrap()
    }

    #[tokio::test]
    async fn udp_upstream_preserves_negative_answers_and_ids() {
        let udp = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = udp.local_addr().unwrap();
        let tcp = TcpListener::bind(address).await.unwrap();
        let server = tokio::spawn(async move {
            let mut bytes = vec![0; MAX_MESSAGE];
            let (size, peer) = udp.recv_from(&mut bytes).await.unwrap();
            let response = upstream_response(&bytes[..size], false, ResponseCode::NXDomain);
            udp.send_to(&response, peer).await.unwrap();
        });
        let request = query("allowed.pryx.invalid");
        let response = forward_to(&request, address).await.unwrap();
        let response = Message::from_vec(&response).unwrap();
        assert_eq!(response.metadata.id, Message::from_vec(&request).unwrap().metadata.id);
        assert_eq!(response.metadata.response_code, ResponseCode::NXDomain);
        assert_eq!(response.queries, Message::from_vec(&request).unwrap().queries);
        server.await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(10), tcp.accept()).await.is_err());
    }

    #[tokio::test]
    async fn a_truncated_udp_answer_uses_one_framed_tcp_exchange() {
        let udp = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = udp.local_addr().unwrap();
        let tcp = TcpListener::bind(address).await.unwrap();
        let server = tokio::spawn(async move {
            let mut bytes = vec![0; MAX_MESSAGE];
            let (size, peer) = udp.recv_from(&mut bytes).await.unwrap();
            let response = upstream_response(&bytes[..size], true, ResponseCode::NoError);
            udp.send_to(&response, peer).await.unwrap();
            let (mut stream, _) = tcp.accept().await.unwrap();
            let query = read_framed(&mut stream).await.unwrap();
            let response = upstream_response(&query, false, ResponseCode::NoError);
            write_framed(&mut stream, &response).await.unwrap();
        });
        let request = query("allowed.pryx.invalid");
        let response = forward_to(&request, address).await.unwrap();
        let response = Message::from_vec(&response).unwrap();
        assert!(!response.metadata.truncation);
        assert_eq!(response.metadata.id, Message::from_vec(&request).unwrap().metadata.id);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert_eq!(response.answers.len(), 1);
        assert_eq!(response.answers[0].data, RData::A(A(std::net::Ipv4Addr::LOCALHOST)));
        assert_eq!(response.queries, Message::from_vec(&request).unwrap().queries);
        server.await.unwrap();
    }
}
