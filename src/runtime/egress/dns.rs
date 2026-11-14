// SPDX-License-Identifier: FSL-1.1-ALv2
//! The resolver of a hostname-egress bridge. The host firewall redirects every UDP and TCP port 53
//! packet from the bridge here, and the container's embedded resolver forwards here too, so a name
//! reaches the host's resolver only when the policy allows it; any other query is answered REFUSED.
//! Each query's single question is read by hand and the message is forwarded unchanged.
//!
//! Not a cache or a recursive resolver: the host's own resolver (`/etc/resolv.conf`) answers, and
//! the HTTP/TLS gateways still resolve and check every address they dial.
use super::*;
use std::{io, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

/// DNS's port, for UDP and TCP (RFC 1035 §4.2).
pub(in crate::runtime) const PORT: u16 = 53;
/// The fixed header's length (RFC 1035 §4.1.1).
const HEADER: usize = 12;
/// RCODEs of the header (RFC 1035 §4.1.1): no DNS crate is in the tree to export them.
const FORMERR: u8 = 1;
const SERVFAIL: u8 = 2;
const NOTIMP: u8 = 4;
const REFUSED: u8 = 5;
/// A name's longest wire form (RFC 1035 §2.3.4).
const MAX_NAME: usize = 255;
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
    Answer(u8, Option<usize>),
    Silent,
}

/// The single question of a standard query. Labels are hostname characters only (letters, digits,
/// `-` and `_`), so the name the policy matches is the name the upstream resolver is asked.
pub(super) fn question(message: &[u8]) -> Result<Question, Refusal> {
    if message.len() < HEADER || message[2] & 0x80 != 0 {
        return Err(Refusal::Silent);
    }
    if (message[2] >> 3) & 0x0f != 0 {
        return Err(Refusal::Answer(NOTIMP, None));
    }
    if u16::from_be_bytes([message[4], message[5]]) != 1 {
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
fn refusal(message: &[u8], rcode: u8, end: Option<usize>) -> Option<Vec<u8>> {
    if message.len() < HEADER {
        return None;
    }
    let mut answer = vec![message[0], message[1], 0x80 | (message[2] & 0x79), rcode, 0, 0, 0, 0, 0, 0, 0, 0];
    if let Some(end) = end {
        answer[5] = 1;
        answer.extend_from_slice(&message[HEADER..end]);
    }
    Some(answer)
}

/// What the sandbox is told for `message`: the host resolver's answer for an allowed name, REFUSED
/// for any other, SERVFAIL when the host resolver doesn't answer, nothing for a non-query.
pub(super) async fn answer(message: &[u8], policy: &EgressPolicy, tcp: bool) -> Option<Vec<u8>> {
    match question(message) {
        Ok(question) if gateway::allowed(&question.name, policy) => match forward(message, tcp).await {
            Ok(answer) => Some(answer),
            Err(_) => refusal(message, SERVFAIL, Some(question.end)),
        },
        Ok(question) => refusal(message, REFUSED, Some(question.end)),
        Err(Refusal::Answer(rcode, end)) => refusal(message, rcode, end),
        Err(Refusal::Silent) => None,
    }
}

/// The host's first nameserver, read for each query so a changed `/etc/resolv.conf` is followed.
async fn upstream() -> io::Result<SocketAddr> {
    let text = tokio::fs::read_to_string(RESOLV_CONF).await?;
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .find_map(|rest| rest.trim().parse::<IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, PORT))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no nameserver"))
}

async fn forward(message: &[u8], tcp: bool) -> io::Result<Vec<u8>> {
    let server = upstream().await?;
    tokio::time::timeout(UPSTREAM, async {
        if tcp {
            let mut stream = TcpStream::connect(server).await?;
            write_framed(&mut stream, message).await?;
            return read_framed(&mut stream).await;
        }
        let local: SocketAddr = if server.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
        let socket = UdpSocket::bind(local).await?;
        socket.connect(server).await?;
        socket.send(message).await?;
        let mut buffer = vec![0; MAX_MESSAGE];
        loop {
            let n = socket.recv(&mut buffer).await?;
            if n >= 2 && buffer[..2] == message[..2] {
                buffer.truncate(n);
                return Ok(buffer);
            }
        }
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
    policy: EgressPolicy,
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
                if let Some(answer) = answer(&message, &policy, false).await {
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
                        let Some(answer) = answer(&message, &policy, true).await else { break };
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

    /// A standard recursive query for `name`, type A, class IN, with an EDNS OPT record after it.
    fn query(name: &str) -> Vec<u8> {
        let mut message = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 1];
        for label in name.split('.').filter(|l| !l.is_empty()) {
            message.push(label.len() as u8);
            message.extend_from_slice(label.as_bytes());
        }
        message.extend_from_slice(&[0, 0, 1, 0, 1]);
        message.extend_from_slice(&[0, 0, 41, 4, 0, 0, 0, 0, 0, 0, 0]);
        message
    }

    fn policy() -> EgressPolicy {
        EgressPolicy { allow: vec!["example.com".into(), "*.example.org".into()], deny: vec!["x.example.org".into()] }
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
            let answer = answer(&message, &policy(), false).await.unwrap();
            let end = question(&message).unwrap().end;
            assert_eq!(&answer[..2], &message[..2], "same id");
            assert_eq!(answer[2], 0x81, "a response, recursion desired kept");
            assert_eq!(answer[3], REFUSED, "{name}");
            assert_eq!(&answer[4..12], &[0, 1, 0, 0, 0, 0, 0, 0]);
            assert_eq!(&answer[HEADER..], &message[HEADER..end]);
        }
        let mut response = query("exfil.attacker.test");
        response[2] |= 0x80;
        assert_eq!(answer(&response, &policy(), true).await, None, "a response is never answered");
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
}
