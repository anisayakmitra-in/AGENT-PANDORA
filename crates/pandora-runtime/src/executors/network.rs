use crate::ConsumedPermit;
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ReceiptId, ResourceScope,
    Timestamp,
};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::redirect::Policy;
use serde::Serialize;
use std::fmt;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::{Host, Url};

const MAX_BROWSER_URL_BYTES: usize = 2048;
const MAX_BROWSER_BODY_BYTES: usize = 128 * 1024;
const BROWSER_TIMEOUT_SECONDS: u64 = 15;
static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkError {
    PermissionDenied,
    InvalidUrl,
    UnsupportedScheme,
    UnsafeHost,
    ResolutionFailed,
    RequestFailed,
    UnsupportedMediaType,
    ResponseTooLarge,
    ResponseReadFailed,
}

impl NetworkError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::InvalidUrl => "invalid_url",
            Self::UnsupportedScheme => "unsupported_scheme",
            Self::UnsafeHost => "unsafe_host",
            Self::ResolutionFailed => "resolution_failed",
            Self::RequestFailed => "request_failed",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::ResponseTooLarge => "response_too_large",
            Self::ResponseReadFailed => "response_read_failed",
        }
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PermissionDenied => "network request does not match the consumed permit",
            Self::InvalidUrl => "browser URL is invalid",
            Self::UnsupportedScheme => "browser URL scheme is unsupported",
            Self::UnsafeHost => "browser URL resolves outside the allowed network boundary",
            Self::ResolutionFailed => "browser host could not be resolved safely",
            Self::RequestFailed => "browser request failed",
            Self::UnsupportedMediaType => "browser response is not textual evidence",
            Self::ResponseTooLarge => "browser response exceeds the bounded evidence limit",
            Self::ResponseReadFailed => "browser response could not be read",
        })
    }
}

impl std::error::Error for NetworkError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserEvidence {
    url: String,
    status: u16,
    content_type: Option<String>,
    body: String,
    truncated: bool,
    lossy: bool,
}

impl BrowserEvidence {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub const fn status(&self) -> u16 {
        self.status
    }

    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    pub const fn lossy(&self) -> bool {
        self.lossy
    }
}

pub struct NetworkResult {
    result: Result<BrowserEvidence, NetworkError>,
    receipt: EffectReceipt,
}

impl NetworkResult {
    pub fn result(&self) -> Result<&BrowserEvidence, &NetworkError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<BrowserEvidence, NetworkError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        &self.receipt
    }
}

pub struct NetworkExecutor;

impl NetworkExecutor {
    pub const fn new() -> Self {
        Self
    }

    pub fn fetch(&self, permit: &ConsumedPermit, source: &str, now: Timestamp) -> NetworkResult {
        let result = self.fetch_inner(permit, source);
        let outcome = match &result {
            Ok(_) => EffectOutcome::Succeeded,
            Err(error) => EffectOutcome::Failed {
                code: error.code().to_owned(),
            },
        };
        NetworkResult {
            result,
            receipt: receipt_for(permit, now, outcome),
        }
    }

    fn fetch_inner(
        &self,
        permit: &ConsumedPermit,
        source: &str,
    ) -> Result<BrowserEvidence, NetworkError> {
        let parsed = parse_browser_url(source)?;
        if !request_matches(permit, source, &parsed) {
            return Err(NetworkError::PermissionDenied);
        }
        let resolution = resolve_pinned_target(&parsed)?;
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(BROWSER_TIMEOUT_SECONDS))
            .timeout(Duration::from_secs(BROWSER_TIMEOUT_SECONDS));
        if resolution.pin_dns {
            builder = builder.resolve(&resolution.host, resolution.address);
        }
        let client = builder.build().map_err(|_| NetworkError::RequestFailed)?;
        let response = client
            .get(parsed.clone())
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/json,text/plain;q=0.9,*/*;q=0.1",
            )
            .send()
            .map_err(|_| NetworkError::RequestFailed)?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.chars().take(128).collect::<String>());
        if content_type
            .as_deref()
            .is_some_and(|value| !is_textual_media_type(value))
        {
            return Err(NetworkError::UnsupportedMediaType);
        }
        let mut bytes = Vec::with_capacity(MAX_BROWSER_BODY_BYTES.min(16 * 1024));
        response
            .take((MAX_BROWSER_BODY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| NetworkError::ResponseReadFailed)?;
        let truncated = bytes.len() > MAX_BROWSER_BODY_BYTES;
        if truncated {
            bytes.truncate(MAX_BROWSER_BODY_BYTES);
        }
        if content_type.is_none() && std::str::from_utf8(&bytes).is_err() {
            return Err(NetworkError::UnsupportedMediaType);
        }
        let decoded = String::from_utf8_lossy(&bytes);
        let utf8_lossy = matches!(&decoded, std::borrow::Cow::Owned(_));
        let (body, controls_replaced) = sanitize_text(&decoded);
        if body.len() > MAX_BROWSER_BODY_BYTES {
            return Err(NetworkError::ResponseTooLarge);
        }
        Ok(BrowserEvidence {
            url: parsed.to_string(),
            status,
            content_type,
            body,
            truncated,
            lossy: utf8_lossy || controls_replaced,
        })
    }
}

impl Default for NetworkExecutor {
    fn default() -> Self {
        Self::new()
    }
}

struct PinnedResolution {
    host: String,
    address: SocketAddr,
    pin_dns: bool,
}

fn parse_browser_url(source: &str) -> Result<Url, NetworkError> {
    if source.is_empty()
        || source.len() > MAX_BROWSER_URL_BYTES
        || source.chars().any(char::is_control)
    {
        return Err(NetworkError::InvalidUrl);
    }
    let parsed = Url::parse(source).map_err(|_| NetworkError::InvalidUrl)?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(NetworkError::InvalidUrl);
    }
    let host = parsed.host().ok_or(NetworkError::InvalidUrl)?;
    let loopback = is_loopback_host(&host);
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
        return Err(NetworkError::UnsupportedScheme);
    }
    if parsed.port_or_known_default().is_none() {
        return Err(NetworkError::InvalidUrl);
    }
    Ok(parsed)
}

fn request_matches(permit: &ConsumedPermit, source: &str, parsed: &Url) -> bool {
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let Some(port) = parsed.port_or_known_default() else {
        return false;
    };
    let request = permit.request();
    request.capability() == Capability::NetworkConnect
        && request.operation() == Operation::Connect
        && request.payload_digest_matches(source.as_bytes())
        && matches!(request.target(), EffectTarget::Network { host: target, port: target_port } if target.eq_ignore_ascii_case(host) && *target_port == port)
        && matches!(request.resource_scope(), ResourceScope::Host { host: scope } if scope.eq_ignore_ascii_case(host))
}

fn resolve_pinned_target(parsed: &Url) -> Result<PinnedResolution, NetworkError> {
    let host = parsed.host().ok_or(NetworkError::InvalidUrl)?;
    let host_text = parsed
        .host_str()
        .ok_or(NetworkError::InvalidUrl)?
        .to_owned();
    let port = parsed
        .port_or_known_default()
        .ok_or(NetworkError::InvalidUrl)?;
    let expected_loopback = is_loopback_host(&host);
    let (addresses, pin_dns) = match host {
        Host::Domain(_) => (
            (host_text.as_str(), port)
                .to_socket_addrs()
                .map_err(|_| NetworkError::ResolutionFailed)?
                .collect::<Vec<_>>(),
            true,
        ),
        Host::Ipv4(address) => (vec![SocketAddr::new(IpAddr::V4(address), port)], false),
        Host::Ipv6(address) => (vec![SocketAddr::new(IpAddr::V6(address), port)], false),
    };
    if addresses.is_empty()
        || !addresses.iter().all(|address| {
            if expected_loopback {
                address.ip().is_loopback()
            } else {
                is_public_ip(address.ip())
            }
        })
    {
        return Err(NetworkError::UnsafeHost);
    }
    Ok(PinnedResolution {
        host: host_text,
        address: addresses[0],
        pin_dns,
    })
}

fn is_loopback_host(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(value) => value.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(value) => value.is_loopback(),
        Host::Ipv6(value) => value.is_loopback(),
    }
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !(first == 0
        || first == 10
        || first == 127
        || first >= 224
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 168)
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && (18..=19).contains(&second)))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    !address.is_loopback()
        && !address.is_unspecified()
        && !address.is_multicast()
        && (segments[0] & 0xfe00) != 0xfc00
        && (segments[0] & 0xffc0) != 0xfe80
        && (segments[0] & 0xffc0) != 0xfec0
}

fn is_textual_media_type(value: &str) -> bool {
    let media_type = value
        .split(';')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase();
    media_type.starts_with("text/")
        || matches!(
            media_type.as_str(),
            "application/json"
                | "application/ld+json"
                | "application/xml"
                | "application/xhtml+xml"
                | "application/javascript"
        )
        || media_type.ends_with("+json")
        || media_type.ends_with("+xml")
}

fn sanitize_text(value: &str) -> (String, bool) {
    let mut changed = false;
    let body = value
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                changed = true;
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect();
    (body, changed)
}

fn receipt_for(permit: &ConsumedPermit, now: Timestamp, outcome: EffectOutcome) -> EffectReceipt {
    let receipt_id = ReceiptId::new(format!(
        "receipt-network-{}",
        NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated receipt ID is valid");
    EffectReceipt::new(
        receipt_id,
        permit.permit().permit_id().clone(),
        permit.permit().request_digest().clone(),
        now,
        outcome,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Parliament, ReferenceMonitor};
    use pandora_types::{
        ExecutionId, GeneId, OperationRequest, PolicyContext, PrincipalId, SessionId,
    };
    use std::io::{Read as _, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn fetches_loopback_text_only_with_an_exact_payload_bound_permit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 17\r\nConnection: close\r\n\r\n<h1>Pandora</h1>\n",
                )
                .unwrap();
        });
        let source = format!("http://127.0.0.1:{}/docs", address.port());
        let permit = permit_for(&source, "127.0.0.1", address.port());

        let response =
            NetworkExecutor::new().fetch(&permit, &source, Timestamp::from_unix_seconds(10));

        let evidence = response.result().unwrap();
        assert_eq!(evidence.status(), 200);
        assert_eq!(evidence.body(), "<h1>Pandora</h1>\n");
        assert_eq!(evidence.content_type(), Some("text/html; charset=utf-8"));
        assert!(!evidence.truncated());
        assert!(!evidence.lossy());
        assert!(matches!(
            response.receipt().outcome(),
            EffectOutcome::Succeeded
        ));
        server.join().unwrap();
    }

    #[test]
    fn rejects_a_url_that_does_not_match_the_permit_payload() {
        let source = "http://127.0.0.1:5173/";
        let permit = permit_for(source, "127.0.0.1", 5173);
        let response = NetworkExecutor::new().fetch(
            &permit,
            "http://127.0.0.1:5174/",
            Timestamp::from_unix_seconds(10),
        );

        assert_eq!(
            response.result().unwrap_err(),
            &NetworkError::PermissionDenied
        );
        assert!(matches!(
            response.receipt().outcome(),
            EffectOutcome::Failed { .. }
        ));
    }

    #[test]
    fn rejects_public_cleartext_and_non_text_media_types() {
        assert_eq!(
            parse_browser_url("http://example.com/").unwrap_err(),
            NetworkError::UnsupportedScheme
        );
        assert!(!is_textual_media_type("application/octet-stream"));
        assert!(is_textual_media_type("application/problem+json"));
    }

    fn permit_for(source: &str, host: &str, port: u16) -> ConsumedPermit {
        let request = OperationRequest::new(
            ExecutionId::new("execution-network-1").unwrap(),
            SessionId::new("session-network-1").unwrap(),
            PrincipalId::new("principal-network-1").unwrap(),
            crate::test_support::execution_profile("network"),
            GeneId::new("browser.fetch").unwrap(),
            None,
            Capability::NetworkConnect,
            Operation::Connect,
            EffectTarget::network(host, port),
            ResourceScope::host(host),
        )
        .unwrap()
        .with_payload_digest(source.as_bytes())
        .unwrap();
        let context = PolicyContext::new(1, [Capability::NetworkConnect], []);
        let monitor = ReferenceMonitor::new_with_policy(context.clone(), 60);
        let decision = Parliament::new(1).decide(&request, &context);
        let now = Timestamp::from_unix_seconds(10);
        let permit = monitor.authorize(request.clone(), decision, now).unwrap();
        monitor.store().consume(permit, &request, now).unwrap()
    }
}
