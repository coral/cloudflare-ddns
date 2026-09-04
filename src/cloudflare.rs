use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;
use ureq::{Agent, Body};

const API_BASE_URL: &str = "https://api.cloudflare.com/client/v4";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub trait DnsProvider {
    fn list_records(&self, name: &str) -> Result<Vec<DnsRecord>, CloudflareError>;
    fn update_record(&self, record_id: &str, address: IpAddr) -> Result<(), CloudflareError>;
}

pub struct CloudflareClient {
    agent: Agent,
    api_token: String,
    zone_id: String,
    api_base_url: String,
    user_agent: String,
}

impl CloudflareClient {
    pub fn new(api_token: String, zone_id: String, version: &str) -> Self {
        Self::with_base_url(api_token, zone_id, version, API_BASE_URL, true)
    }

    fn with_base_url(
        api_token: String,
        zone_id: String,
        version: &str,
        api_base_url: &str,
        https_only: bool,
    ) -> Self {
        let agent = Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .https_only(https_only)
            .build()
            .new_agent();
        Self {
            agent,
            api_token,
            zone_id,
            api_base_url: api_base_url.trim_end_matches('/').to_owned(),
            user_agent: format!("cf-ddns/{version}"),
        }
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.api_token)
    }

    fn decode_response<T: DeserializeOwned>(
        &self,
        mut response: ureq::http::Response<Body>,
    ) -> Result<T, CloudflareError> {
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().map_err(|source| {
            CloudflareError::retryable(format!("could not read Cloudflare response: {source}"))
        })?;

        if !(200..300).contains(&status) {
            let details = serde_json::from_str::<Envelope<Value>>(&body)
                .ok()
                .map(|envelope| format_issues(&envelope.errors))
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| "no error details returned".to_owned());
            let message = format!("Cloudflare returned HTTP {status}: {details}");
            return Err(if status == 429 || status >= 500 {
                CloudflareError::retryable(message)
            } else {
                CloudflareError::permanent(message)
            });
        }

        let envelope = serde_json::from_str::<Envelope<T>>(&body).map_err(|source| {
            CloudflareError::retryable(format!(
                "Cloudflare returned an invalid JSON response: {source}"
            ))
        })?;
        if !envelope.success {
            let details = format_issues(&envelope.errors);
            return Err(CloudflareError::permanent(if details.is_empty() {
                "Cloudflare reported an unsuccessful API request".to_owned()
            } else {
                format!("Cloudflare rejected the API request: {details}")
            }));
        }
        envelope.result.ok_or_else(|| {
            CloudflareError::retryable(
                "Cloudflare response indicated success but omitted its result".to_owned(),
            )
        })
    }
}

impl DnsProvider for CloudflareClient {
    fn list_records(&self, name: &str) -> Result<Vec<DnsRecord>, CloudflareError> {
        let url = format!("{}/zones/{}/dns_records", self.api_base_url, self.zone_id);
        let response = self
            .agent
            .get(&url)
            .query("name.exact", name)
            .query("per_page", "100")
            .header("Authorization", self.authorization())
            .header("Accept", "application/json")
            .header("User-Agent", &self.user_agent)
            .call()
            .map_err(|source| {
                CloudflareError::retryable(format!("Cloudflare request failed: {source}"))
            })?;
        self.decode_response(response)
    }

    fn update_record(&self, record_id: &str, address: IpAddr) -> Result<(), CloudflareError> {
        let url = format!(
            "{}/zones/{}/dns_records/{record_id}",
            self.api_base_url, self.zone_id
        );
        let response = self
            .agent
            .patch(&url)
            .header("Authorization", self.authorization())
            .header("Accept", "application/json")
            .header("User-Agent", &self.user_agent)
            .send_json(&UpdateRecord {
                content: address.to_string(),
            })
            .map_err(|source| {
                CloudflareError::retryable(format!("Cloudflare request failed: {source}"))
            })?;
        let result = self.decode_response::<Value>(response)?;
        if result.get("id").and_then(Value::as_str) != Some(record_id) {
            return Err(CloudflareError::retryable(
                "Cloudflare update response did not identify the updated record".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DnsRecord {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub record_type: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct UpdateRecord {
    content: String,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    success: bool,
    result: Option<T>,
    #[serde(default)]
    errors: Vec<ApiIssue>,
}

#[derive(Debug, Deserialize)]
struct ApiIssue {
    code: Option<u64>,
    #[serde(default)]
    message: String,
}

fn format_issues(issues: &[ApiIssue]) -> String {
    issues
        .iter()
        .map(|issue| match issue.code {
            Some(code) if !issue.message.is_empty() => format!("{code}: {}", issue.message),
            Some(code) => code.to_string(),
            None => issue.message.clone(),
        })
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureKind {
    Retryable,
    Permanent,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct CloudflareError {
    kind: FailureKind,
    message: String,
}

impl CloudflareError {
    pub fn retryable(message: String) -> Self {
        Self {
            kind: FailureKind::Retryable,
            message,
        }
    }

    pub fn permanent(message: String) -> Self {
        Self {
            kind: FailureKind::Permanent,
            message,
        }
    }

    pub fn is_permanent(&self) -> bool {
        self.kind == FailureKind::Permanent
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;

    use super::*;

    const ZONE_ID: &str = "0123456789abcdef0123456789abcdef";

    fn response(status: u16, reason: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn mock_server(responses: Vec<String>) -> (String, Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(header_end) =
                        request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if request.len() >= header_end + 4 + content_length {
                            break;
                        }
                    }
                }
                let _ = sender.send(String::from_utf8(request).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}"), receiver, handle)
    }

    fn client(base_url: &str) -> CloudflareClient {
        CloudflareClient::with_base_url(
            "secret-token".to_owned(),
            ZONE_ID.to_owned(),
            "test",
            base_url,
            false,
        )
    }

    #[test]
    fn lists_by_exact_name_and_patches_only_content() {
        let list_body = r#"{"success":true,"errors":[],"result":[{"id":"record-a","name":"home.example.com","type":"A","content":"192.0.2.1"}]}"#;
        let update_body = r#"{"success":true,"errors":[],"result":{"id":"record-a"}}"#;
        let (base_url, requests, handle) = mock_server(vec![
            response(200, "OK", list_body),
            response(200, "OK", update_body),
        ]);
        let client = client(&base_url);

        let records = client.list_records("home.example.com").unwrap();
        assert_eq!(records.len(), 1);
        client
            .update_record("record-a", "198.51.100.8".parse().unwrap())
            .unwrap();

        let list_request = requests.recv().unwrap();
        let update_request = requests.recv().unwrap();
        handle.join().unwrap();
        assert!(list_request.starts_with(&format!(
            "GET /zones/{ZONE_ID}/dns_records?name.exact=home.example.com&per_page=100 HTTP/1.1"
        )));
        assert!(
            list_request
                .to_ascii_lowercase()
                .contains("authorization: bearer secret-token")
        );
        assert!(update_request.starts_with(&format!(
            "PATCH /zones/{ZONE_ID}/dns_records/record-a HTTP/1.1"
        )));
        let (_, update_body) = update_request.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(update_body).unwrap(),
            serde_json::json!({"content": "198.51.100.8"})
        );
    }

    #[test]
    fn classifies_authentication_error_as_permanent() {
        let body = r#"{"success":false,"errors":[{"code":9109,"message":"Invalid access token"}],"result":null}"#;
        let (base_url, _, handle) = mock_server(vec![response(401, "Unauthorized", body)]);
        let error = client(&base_url)
            .list_records("home.example.com")
            .unwrap_err();
        handle.join().unwrap();
        assert!(error.is_permanent());
        assert!(error.to_string().contains("9109: Invalid access token"));
        assert!(!error.to_string().contains("secret-token"));
    }

    #[test]
    fn classifies_rate_limit_and_server_errors_as_retryable() {
        for (status, reason) in [(429, "Too Many Requests"), (503, "Unavailable")] {
            let body = r#"{"success":false,"errors":[],"result":null}"#;
            let (base_url, _, handle) = mock_server(vec![response(status, reason, body)]);
            let error = client(&base_url)
                .list_records("home.example.com")
                .unwrap_err();
            handle.join().unwrap();
            assert!(!error.is_permanent());
        }
    }
}
