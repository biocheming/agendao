use regex::Regex;
use reqwest::Client;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::{Host, Url};

use crate::ToolError;

pub(crate) const MAX_WEB_RESPONSE_SIZE: usize = 5 * 1024 * 1024;
pub(crate) const DEFAULT_WEB_TIMEOUT_SECS: u64 = 30;
pub(crate) const MAX_WEB_TIMEOUT_SECS: u64 = 120;
pub(crate) const DEFAULT_WEB_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

pub(crate) fn build_web_client() -> Client {
    match Client::builder()
        .user_agent(DEFAULT_WEB_USER_AGENT)
        .timeout(std::time::Duration::from_secs(MAX_WEB_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "failed to build configured web client, using reqwest default client");
            Client::new()
        }
    }
}

pub(crate) fn ensure_http_url(url: &str) -> Result<(), ToolError> {
    let parsed = parse_http_url(url)?;
    validate_http_url_host(&parsed)?;
    Ok(())
}

pub(crate) async fn ensure_safe_http_url(url: &str) -> Result<Url, ToolError> {
    let parsed = parse_http_url(url)?;
    validate_http_url_host(&parsed)?;
    validate_resolved_host(&parsed).await?;
    Ok(parsed)
}

fn parse_http_url(url: &str) -> Result<Url, ToolError> {
    let parsed = Url::parse(url).map_err(|error| {
        ToolError::InvalidArguments(format!("invalid URL `{}`: {}", url, error))
    })?;

    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(ToolError::InvalidArguments(
                "URL must start with http:// or https://".to_string(),
            ));
        }
    }

    Ok(parsed)
}

fn validate_http_url_host(parsed: &Url) -> Result<(), ToolError> {
    let host = parsed.host().ok_or_else(|| {
        ToolError::InvalidArguments(format!("URL `{}` must include a host", parsed))
    })?;

    match host {
        Host::Ipv4(ip) if is_blocked_ip(IpAddr::V4(ip)) => Err(ToolError::PermissionDenied(
            format!("refusing to access local or private address `{}`", ip),
        )),
        Host::Ipv6(ip) if is_blocked_ip(IpAddr::V6(ip)) => Err(ToolError::PermissionDenied(
            format!("refusing to access local or private address `{}`", ip),
        )),
        Host::Domain(domain) if is_blocked_host(domain) => Err(ToolError::PermissionDenied(
            format!("refusing to access local host `{}`", domain),
        )),
        _ => Ok(()),
    }
}

async fn validate_resolved_host(parsed: &Url) -> Result<(), ToolError> {
    let Some(domain) = parsed.host_str() else {
        return Ok(());
    };
    if parsed
        .host()
        .is_some_and(|host| !matches!(host, Host::Domain(_)))
    {
        return Ok(());
    }

    let port = parsed.port_or_known_default().ok_or_else(|| {
        ToolError::InvalidArguments(format!("URL `{}` must include a valid port", parsed))
    })?;

    let resolved = tokio::net::lookup_host((domain, port))
        .await
        .map_err(|error| {
            ToolError::ExecutionError(format!("failed to resolve host `{}`: {}", domain, error))
        })?;

    for addr in resolved {
        if is_blocked_ip(addr.ip()) {
            return Err(ToolError::PermissionDenied(format!(
                "refusing to access host `{}` resolved to local or private address `{}`",
                domain,
                addr.ip()
            )));
        }
    }

    Ok(())
}

fn is_blocked_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("localhost.localdomain")
        || host.to_ascii_lowercase().ends_with(".localhost")
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_blocked_ipv4(ip),
        IpAddr::V6(ip) => is_blocked_ipv6(ip),
    }
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.octets()[0] == 0
        || ip.octets()[0] == 127
        || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local() || ip.is_unicast_link_local()
}

pub(crate) fn convert_html_to_markdown(html: &str) -> String {
    html2md::parse_html(html)
}

pub(crate) fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        if c == '<' {
            if i + 7 <= len {
                let tag: String = chars[i..i + 7].iter().collect();
                let tag_lower = tag.to_lowercase();
                if tag_lower.starts_with("<script") {
                    in_script = true;
                } else if tag_lower.starts_with("<style") {
                    in_style = true;
                }
            }
            in_tag = true;
            i += 1;
            continue;
        }

        if c == '>' {
            if in_script {
                if i >= 8 {
                    let end_tag: String = chars[i - 8..=i].iter().collect();
                    if end_tag.to_lowercase() == "</script>" {
                        in_script = false;
                    }
                }
            } else if in_style && i >= 7 {
                let end_tag: String = chars[i - 7..=i].iter().collect();
                if end_tag.to_lowercase() == "</style>" {
                    in_style = false;
                }
            }
            in_tag = false;
            i += 1;
            continue;
        }

        if !in_tag && !in_script && !in_style {
            if c == '&' {
                if i + 4 <= len {
                    let entity: String = chars[i..i + 4].iter().collect();
                    match entity.as_str() {
                        "&lt;" => {
                            result.push('<');
                            i += 4;
                            continue;
                        }
                        "&gt;" => {
                            result.push('>');
                            i += 4;
                            continue;
                        }
                        "&amp;" => {
                            result.push('&');
                            i += 5;
                            continue;
                        }
                        _ => {}
                    }
                }
                if i + 6 <= len {
                    let entity: String = chars[i..i + 6].iter().collect();
                    if entity == "&nbsp;" {
                        result.push(' ');
                        i += 6;
                        continue;
                    }
                }
            }
            result.push(c);
        }

        i += 1;
    }

    result
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn extract_title(html: &str) -> Option<String> {
    static TITLE_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = TITLE_RE.get_or_init(|| {
        Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("title regex should compile")
    });
    let captures = re.captures(html)?;
    let title = captures.get(1)?.as_str();
    let cleaned = strip_html(title).trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}
