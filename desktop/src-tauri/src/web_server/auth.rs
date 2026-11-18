//! Token, Host and Origin checks for the headless server. Pure functions so every rule is tested
//! without sockets. The token is the only credential; it is never logged by the server.

use axum::http::{header, HeaderMap};
use sha2::{Digest, Sha256};

const MIN_TOKEN_LENGTH: usize = 16;

/// The shared secret. Compared through SHA-256 digests with a fold over every byte, so the time taken
/// does not depend on where a wrong guess first differs.
#[derive(Clone)]
pub(crate) struct Token(String);

impl Token {
    pub(crate) fn generate() -> Result<Token, String> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|error| format!("Could not read secure random bytes: {error}"))?;
        Ok(Token(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    pub(crate) fn from_secret(secret: &str) -> Result<Token, String> {
        let secret = secret.trim();
        if secret.len() < MIN_TOKEN_LENGTH {
            return Err(format!(
                "The server token must be at least {MIN_TOKEN_LENGTH} characters."
            ));
        }
        if !secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.~".contains(c))
        {
            return Err("The server token may only contain letters, digits and - _ . ~".into());
        }
        Ok(Token(secret.to_string()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn verify(&self, candidate: &str) -> bool {
        let expected = Sha256::digest(self.0.as_bytes());
        let offered = Sha256::digest(candidate.as_bytes());
        expected
            .iter()
            .zip(offered.iter())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

/// The credential a request carries: `Authorization: Bearer <token>` and nothing else. A cookie would be sent
/// to every port on the same host name, so the token never travels as one.
pub(crate) fn credential_from_headers(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    Some(token.trim().to_string())
}

fn host_without_port(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    host.split(':').next().unwrap_or("").to_string()
}

/// DNS-rebinding defence: only loopback names and explicitly allowed hosts may address the server.
pub(crate) fn host_allowed(host_header: &str, extra_hosts: &[String]) -> bool {
    let host = host_without_port(host_header);
    if host.is_empty() {
        return false;
    }
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1")
        || extra_hosts
            .iter()
            .any(|allowed| host_without_port(allowed) == host)
}

/// A browser that sends `Origin` must be talking to the host it loaded the page from.
pub(crate) fn origin_matches_host(origin: &str, host_header: &str) -> bool {
    let Some((_, authority)) = origin.split_once("://") else {
        return false;
    };
    authority
        .trim_end_matches('/')
        .eq_ignore_ascii_case(host_header.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn generated_tokens_are_long_unique_and_verify_only_themselves() {
        let first = Token::generate().unwrap();
        let second = Token::generate().unwrap();
        assert_eq!(first.expose().len(), 64);
        assert_ne!(first.expose(), second.expose());
        assert!(first.verify(first.expose()));
        assert!(!first.verify(second.expose()));
        assert!(!first.verify(""));
        assert!(!first.verify(&first.expose()[..63]));
    }

    #[test]
    fn supplied_secrets_must_be_long_and_header_safe() {
        assert!(Token::from_secret("short").is_err());
        assert!(Token::from_secret("has spaces in it but is long").is_err());
        assert!(Token::from_secret("semi;colon-in-secret-value").is_err());
        let token = Token::from_secret("  a-long-enough_secret.value~ok \n").unwrap();
        assert_eq!(token.expose(), "a-long-enough_secret.value~ok");
    }

    #[test]
    fn the_credential_is_the_bearer_header_only() {
        let mut headers = HeaderMap::new();
        assert_eq!(credential_from_headers(&headers), None);
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("hevc_session=abc123"),
        );
        assert_eq!(
            credential_from_headers(&headers),
            None,
            "a cookie is never a credential"
        );
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer tok-1"),
        );
        assert_eq!(credential_from_headers(&headers).as_deref(), Some("tok-1"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        );
        assert_eq!(credential_from_headers(&headers), None);
    }

    #[test]
    fn host_header_must_be_loopback_or_allowed() {
        assert!(host_allowed("localhost:8437", &[]));
        assert!(host_allowed("127.0.0.1", &[]));
        assert!(host_allowed("[::1]:8437", &[]));
        assert!(host_allowed("LOCALHOST", &[]));
        assert!(!host_allowed("evil.example:8437", &[]));
        assert!(!host_allowed("127.0.0.1.evil.example", &[]));
        assert!(!host_allowed("", &[]));
        let extra = vec!["studio.example.com".to_string()];
        assert!(host_allowed("studio.example.com", &extra));
        assert!(host_allowed("Studio.Example.com:443", &extra));
        assert!(!host_allowed("other.example.com", &extra));
    }

    #[test]
    fn origin_must_match_the_host_it_was_served_from() {
        assert!(origin_matches_host(
            "http://localhost:8437",
            "localhost:8437"
        ));
        assert!(origin_matches_host(
            "https://studio.example.com",
            "studio.example.com"
        ));
        assert!(!origin_matches_host(
            "http://evil.example",
            "localhost:8437"
        ));
        assert!(!origin_matches_host("null", "localhost:8437"));
    }
}
