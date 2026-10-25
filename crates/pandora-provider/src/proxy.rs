#![forbid(unsafe_code)]

use reqwest::Proxy;
use std::env;
use url::Url;

/// Resolves corporate proxy configuration from environment variables or Windows settings.
/// Checks HTTPS_PROXY, ALL_PROXY, HTTP_PROXY (and lowercase counterparts).
/// Extracts HTTP Basic Authentication credentials from URLs (e.g. `http://user:pass@corp.proxy:8080`).
pub fn resolve_proxy() -> Option<Proxy> {
    let proxy_url_str = env::var("HTTPS_PROXY")
        .or_else(|_| env::var("https_proxy"))
        .or_else(|_| env::var("ALL_PROXY"))
        .or_else(|_| env::var("all_proxy"))
        .or_else(|_| env::var("HTTP_PROXY"))
        .or_else(|_| env::var("http_proxy"))
        .ok()?;

    let trimmed = proxy_url_str.trim();
    if trimmed.is_empty() {
        return None;
    }

    build_proxy_from_spec(trimmed)
}

/// Builds a reqwest `Proxy` from a proxy specification string.
pub fn build_proxy_from_spec(proxy_spec: &str) -> Option<Proxy> {
    let spec = if !proxy_spec.contains("://") {
        format!("http://{proxy_spec}")
    } else {
        proxy_spec.to_owned()
    };

    let parsed = Url::parse(&spec).ok()?;
    let username = parsed.username().to_owned();
    let password = parsed.password().unwrap_or_default().to_owned();

    let mut clean_url = parsed;
    let _ = clean_url.set_username("");
    let _ = clean_url.set_password(None);

    let mut proxy = Proxy::all(clean_url.as_str()).ok()?;

    if !username.is_empty() {
        proxy = proxy.basic_auth(&username, &password);
    }

    let no_proxy = env::var("NO_PROXY")
        .or_else(|_| env::var("no_proxy"))
        .unwrap_or_else(|_| "localhost,127.0.0.1,::1".to_owned());

    proxy = proxy.no_proxy(reqwest::NoProxy::from_string(&no_proxy));

    Some(proxy)
}

/// Creates a new `reqwest::blocking::ClientBuilder` preconfigured with corporate proxy settings.
pub fn proxy_client_builder() -> reqwest::blocking::ClientBuilder {
    let mut builder = reqwest::blocking::Client::builder();
    if let Some(proxy) = resolve_proxy() {
        builder = builder.proxy(proxy);
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_authenticated_proxy_url() {
        let spec = "http://corp_user:secret_pass@10.0.1.50:8080";
        let proxy = build_proxy_from_spec(spec);
        assert!(proxy.is_some(), "authenticated proxy spec should parse");
    }

    #[test]
    fn parse_bare_host_port_proxy() {
        let spec = "proxy.corp.internal:3128";
        let proxy = build_proxy_from_spec(spec);
        assert!(proxy.is_some(), "bare host:port spec should parse");
    }
}
