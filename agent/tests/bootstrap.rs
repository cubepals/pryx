// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The real bootstrap executable must preserve per-port host addresses. Kernel process-tier
//! restrictions have separate fail-closed tests; this test isolates the actual listener setup.

use pryx_agent::auth::SigningKey;
use std::{
    net::{Ipv4Addr, TcpListener, UdpSocket},
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Supervisor(Child);
impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn agent_and_spare_listeners_are_unreachable_on_non_configured_interfaces() {
    // RFC 5737 TEST-NET-1 chooses an outbound route without sending a UDP datagram.
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    route.connect("192.0.2.1:9").unwrap();
    let outside = route.local_addr().unwrap().ip();
    assert!(!outside.is_loopback() && !outside.is_unspecified());
    let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let agent_port = first.local_addr().unwrap().port();
    let spare_port = second.local_addr().unwrap().port();
    let addresses =
        serde_json::json!({agent_port.to_string(): ["127.0.0.1"], spare_port.to_string(): ["127.0.0.2"]}).to_string();
    let data = tempfile::tempdir().unwrap();
    let (_, encoded) = SigningKey::generate().unwrap();
    drop((first, second));
    let mut child = Supervisor(
        Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
            .args([
                "--sandbox",
                "one",
                "--epoch",
                "1",
                "--data",
                data.path().to_str().unwrap(),
                "--domain",
                "pryx.test",
                "--port",
                &agent_port.to_string(),
                "--spare-ports",
                &spare_port.to_string(),
                "--secret-env",
                "TEST_BOOTSTRAP_SECRET",
                "--host-bindings",
                &addresses,
                "--keep-alive",
            ])
            .env("TEST_BOOTSTRAP_SECRET", encoded)
            .env_remove(pryx_agent::process_policy::POLICY_ENV)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, agent_port)).await.is_ok() {
            break;
        }
        assert!(child.0.try_wait().unwrap().is_none(), "bootstrap exited before listening");
        assert!(tokio::time::Instant::now() < deadline, "bootstrap did not listen");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(tokio::net::TcpStream::connect((outside, agent_port)).await.is_err());
    assert!(tokio::net::TcpStream::connect(("127.0.0.2", agent_port)).await.is_err());
    assert!(tokio::net::TcpStream::connect(("127.0.0.2", spare_port)).await.is_ok());
    assert!(tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, spare_port)).await.is_err());
    assert!(tokio::net::TcpStream::connect((outside, spare_port)).await.is_err());
}
