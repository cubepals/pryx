// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Store-time lease renewal. Every PUT compares the GET's ETag; neither a delayed reply nor a failed request extends local authority.
use super::guard::boot_ms;
use crate::http_client as http;
use crate::protocol::{STORE_LEASE_MARGIN_SECONDS, StoreLeaseGrant, StoreLeaseObject};
use hyper::{
    Method, StatusCode,
    header::{AGE, CACHE_CONTROL, CONTENT_TYPE, DATE, ETAG, IF_MATCH},
};
use std::time::{Duration, UNIX_EPOCH};

// HTTP Date is whole seconds. Fence two seconds early, reserving one second for its rounding and one for watchdog scheduling.
pub(crate) const MARGIN_MS: u64 = STORE_LEASE_MARGIN_SECONDS * 1000;
const CALL: Duration = Duration::from_secs(3);
const MAX_OBJECT: usize = 4096;
pub(crate) fn deadline(start: u64, expires: u64, store_now: u64) -> Result<u64, String> {
    let remaining = expires.checked_sub(store_now).filter(|n| *n > MARGIN_MS).ok_or("Store lease expired")?;
    start.checked_add(remaining - MARGIN_MS).ok_or_else(|| "Store lease deadline overflow".into())
}
pub(crate) async fn renew(
    id: &str,
    epoch: u64,
    node: &str,
    grant: &StoreLeaseGrant,
    write: bool,
) -> Result<u64, String> {
    let start = boot_ms().map_err(|_| "Boot clock unavailable")?;
    renew_at(start, id, epoch, node, grant, write).await
}
async fn renew_at(
    start: u64,
    id: &str,
    epoch: u64,
    node: &str,
    grant: &StoreLeaseGrant,
    write: bool,
) -> Result<u64, String> {
    let exchange = async {
        let tls = if grant.read_url.starts_with("https:") || grant.write_url.starts_with("https:") {
            Some(http::public_tls()?)
        } else {
            None
        };
        let response = http::send(
            Method::GET,
            &grant.read_url,
            &[(CACHE_CONTROL.as_str(), "no-cache, no-store".into())],
            http::empty(),
            tls.clone(),
            CALL,
        )
        .await
        .map_err(|_| "Store lease read failed")?;
        if response.status() != StatusCode::OK {
            return Err("Store lease is missing or refused".into());
        }
        if response
            .headers()
            .get(AGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|age| age > 0)
        {
            return Err("Cached lease authority refused".into());
        }
        let date = response.headers().get(DATE).and_then(|h| h.to_str().ok()).ok_or("Store lease has no Date")?;
        let now = httpdate::parse_http_date(date)
            .map_err(|_| "Store Date invalid")?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Store Date before epoch")?
            .as_millis() as u64;
        let etag =
            response.headers().get(ETAG).and_then(|h| h.to_str().ok()).ok_or("Store lease has no ETag")?.to_owned();
        let body = http::read_body(response, MAX_OBJECT).await.map_err(|_| "Store lease body failed")?;
        let mut object: StoreLeaseObject = serde_json::from_slice(&body).map_err(|_| "Store lease object invalid")?;
        if object.session != grant.session
            || object.version != 1
            || object.workload != id
            || object.node != node
            || object.epoch != epoch
            || object.generation != grant.generation
        {
            return Err("Store lease ownership changed".into());
        }
        let old =
            deadline(start, object.expires_at_ms.min(now.saturating_add(grant.seconds.saturating_mul(1000))), now)?;
        if !write {
            return Ok(old);
        }
        object.expires_at_ms = now.checked_add(grant.seconds.saturating_mul(1000)).ok_or("Store expiry overflow")?;
        let body = serde_json::to_vec(&object).map_err(|_| "Store lease serialization failed")?;
        let response = http::send(
            Method::PUT,
            &grant.write_url,
            &[(IF_MATCH.as_str(), etag), (CONTENT_TYPE.as_str(), "application/json".into())],
            http::full(body),
            tls,
            CALL,
        )
        .await
        .map_err(|_| "Store lease write failed")?;
        if !response.status().is_success() {
            return Err("Conditional store lease write refused".into());
        }
        let written_at = response
            .headers()
            .get(DATE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| httpdate::parse_http_date(v).ok())
            .and_then(|v| v.duration_since(UNIX_EPOCH).ok())
            .map(|v| v.as_millis() as u64)
            .ok_or("Store write has no valid Date")?;
        if written_at < now {
            return Err("Store clock moved backwards".into());
        }
        let deadline = deadline(
            start,
            object.expires_at_ms.min(written_at.saturating_add(grant.seconds.saturating_mul(1000))),
            written_at,
        )?;
        let _ = http::read_body(response, MAX_OBJECT).await;
        if boot_ms().map_err(|_| "Boot clock unavailable")? >= old.min(deadline) {
            return Err("Store lease reply arrived after authority ended".into());
        }
        Ok(deadline)
    };
    tokio::time::timeout(CALL, exchange).await.map_err(|_| "Store lease request timed out".to_owned())?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_is_anchored_before_network_and_rounded_early() {
        assert_eq!(deadline(100, 40_000, 10_000).unwrap(), 28_100);
        assert!(deadline(100, 11_000, 10_000).is_err());
        assert!(deadline(100, 9_000, 10_000).is_err());
        assert!(deadline(u64::MAX, 40_000, 10_000).is_err());
    }
}

#[cfg(test)]
mod exchange_tests {
    use super::*;
    use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Clone)]
    struct Store {
        object: Arc<Mutex<StoreLeaseObject>>,
        stale: Arc<AtomicBool>,
        cut: Arc<AtomicBool>,
    }
    async fn read(State(store): State<Store>) -> axum::response::Response {
        use axum::response::IntoResponse;
        if store.cut.load(Ordering::Relaxed) {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let now = std::time::SystemTime::now() + Duration::from_secs(600);
        ([(DATE, httpdate::fmt_http_date(now)), (ETAG, "\"etag\"".into())], Json(store.object.lock().unwrap().clone()))
            .into_response()
    }
    async fn write(
        State(store): State<Store>,
        headers: HeaderMap,
        Json(object): Json<StoreLeaseObject>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if store.stale.load(Ordering::Relaxed)
            || headers.get(IF_MATCH).and_then(|v| v.to_str().ok()) != Some("\"etag\"")
        {
            return StatusCode::PRECONDITION_FAILED.into_response();
        }
        *store.object.lock().unwrap() = object;
        ([(DATE, httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_mins(10)))], StatusCode::OK)
            .into_response()
    }
    #[tokio::test]
    async fn store_time_skew_cas_refusal_and_cut_never_grant_extra_time() {
        let now =
            (std::time::SystemTime::now() + Duration::from_secs(600)).duration_since(UNIX_EPOCH).unwrap().as_millis()
                as u64;
        let store = Store {
            object: Arc::new(Mutex::new(StoreLeaseObject {
                session: "session".into(),
                version: 1,
                workload: "one".into(),
                node: "node".into(),
                epoch: 1,
                generation: "generation".into(),
                expires_at_ms: now + 30_000,
            })),
            stale: Arc::new(AtomicBool::new(false)),
            cut: Arc::new(AtomicBool::new(false)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/lease?secret-do-not-log", listener.local_addr().unwrap());
        let router = Router::new().route("/lease", get(read).put(write)).with_state(store.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let grant = StoreLeaseGrant {
            session: "session".into(),
            read_url: url.clone(),
            write_url: url,
            generation: "generation".into(),
            seconds: 30,
            renew_seconds: 10,
        };
        let start = boot_ms().unwrap();
        let until = renew_at(start, "one", 1, "node", &grant, true).await.unwrap();
        assert!(until > start && until <= start + 30_000 - MARGIN_MS);
        assert!(!format!("{grant:?}").contains("secret-do-not-log"));
        store.object.lock().unwrap().session = "another-daemon".into();
        assert!(renew("one", 1, "node", &grant, false).await.is_err());
        store.object.lock().unwrap().session = "session".into();
        store.stale.store(true, Ordering::Relaxed);
        assert!(renew("one", 1, "node", &grant, true).await.is_err());
        store.cut.store(true, Ordering::Relaxed);
        assert!(renew("one", 1, "node", &grant, true).await.is_err());
        server.abort();
    }
}
