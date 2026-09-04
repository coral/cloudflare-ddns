use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use thiserror::Error;
use ureq::Agent;
use ureq::config::IpFamily;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub trait IpProvider {
    fn ipv4(&self) -> Result<Ipv4Addr, IpError>;
    fn ipv6(&self) -> Result<Ipv6Addr, IpError>;
}

pub struct PublicIpDiscovery {
    ipv4_agent: Agent,
    ipv6_agent: Agent,
    ipv4_url: String,
    ipv6_url: String,
    user_agent: String,
}

impl PublicIpDiscovery {
    pub fn new(ipv4_url: String, ipv6_url: String, version: &str) -> Self {
        Self {
            ipv4_agent: agent_for(IpFamily::Ipv4Only),
            ipv6_agent: agent_for(IpFamily::Ipv6Only),
            ipv4_url,
            ipv6_url,
            user_agent: format!("cf-ddns/{version}"),
        }
    }

    fn discover(&self, agent: &Agent, url: &str, family: IpFamily) -> Result<IpAddr, IpError> {
        let mut response = agent
            .get(url)
            .header("User-Agent", &self.user_agent)
            .header("Accept", "text/plain")
            .call()
            .map_err(|source| IpError::Request {
                url: url.to_owned(),
                source,
            })?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(IpError::HttpStatus {
                url: url.to_owned(),
                status,
            });
        }

        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|source| IpError::ReadBody {
                url: url.to_owned(),
                source,
            })?;
        let address = parse_address(&body)?;
        match (family, address) {
            (IpFamily::Ipv4Only, IpAddr::V4(_)) | (IpFamily::Ipv6Only, IpAddr::V6(_)) => {
                Ok(address)
            }
            (IpFamily::Ipv4Only, _) => Err(IpError::WrongFamily { expected: "IPv4" }),
            (IpFamily::Ipv6Only, _) => Err(IpError::WrongFamily { expected: "IPv6" }),
            (IpFamily::Any, _) => Ok(address),
        }
    }
}

impl IpProvider for PublicIpDiscovery {
    fn ipv4(&self) -> Result<Ipv4Addr, IpError> {
        match self.discover(&self.ipv4_agent, &self.ipv4_url, IpFamily::Ipv4Only)? {
            IpAddr::V4(address) => Ok(address),
            IpAddr::V6(_) => unreachable!("address family was checked by discover"),
        }
    }

    fn ipv6(&self) -> Result<Ipv6Addr, IpError> {
        match self.discover(&self.ipv6_agent, &self.ipv6_url, IpFamily::Ipv6Only)? {
            IpAddr::V6(address) => Ok(address),
            IpAddr::V4(_) => unreachable!("address family was checked by discover"),
        }
    }
}

fn agent_for(family: IpFamily) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .https_only(true)
        .ip_family(family)
        .build()
        .new_agent()
}

fn parse_address(body: &str) -> Result<IpAddr, IpError> {
    if let Ok(address) = body.trim().parse() {
        return Ok(address);
    }

    let mut addresses = body.lines().filter_map(|line| {
        line.strip_prefix("ip=")
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
    });
    let address = addresses.next().ok_or(IpError::MissingAddress)?;
    if addresses.next().is_some() {
        return Err(IpError::MultipleAddresses);
    }
    Ok(address)
}

#[derive(Debug, Error)]
pub enum IpError {
    #[error("public IP request to {url} failed: {source}")]
    Request { url: String, source: ureq::Error },
    #[error("public IP endpoint {url} returned HTTP {status}")]
    HttpStatus { url: String, status: u16 },
    #[error("could not read public IP response from {url}: {source}")]
    ReadBody { url: String, source: ureq::Error },
    #[error("public IP response did not contain an address")]
    MissingAddress,
    #[error("public IP response contained more than one address")]
    MultipleAddresses,
    #[error("public IP endpoint returned the wrong address family; expected {expected}")]
    WrongFamily { expected: &'static str },
}

impl IpError {
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::HttpStatus { status, .. } if (400..500).contains(status) && *status != 429
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cloudflare_trace() {
        let address = parse_address("fl=123\nip=2001:db8::1\ncolo=SJC\n").unwrap();
        assert_eq!(address, "2001:db8::1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn parses_bare_address() {
        let address = parse_address(" 192.0.2.10\n").unwrap();
        assert_eq!(address, "192.0.2.10".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn rejects_trace_without_address() {
        assert!(matches!(
            parse_address("fl=123\ncolo=SJC\n"),
            Err(IpError::MissingAddress)
        ));
    }

    #[test]
    fn classifies_client_status_as_permanent_except_rate_limit() {
        assert!(
            IpError::HttpStatus {
                url: "https://example.com".to_owned(),
                status: 404,
            }
            .is_permanent()
        );
        assert!(
            !IpError::HttpStatus {
                url: "https://example.com".to_owned(),
                status: 429,
            }
            .is_permanent()
        );
    }
}
