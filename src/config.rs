use std::time::Duration;

use clap::Parser;
use thiserror::Error;
use ureq::http::Uri;

pub const DEFAULT_TRACE_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";

#[derive(Parser)]
#[command(
    version,
    about = "Keep Cloudflare DNS names pointed at this machine's public IP addresses"
)]
pub struct Cli {
    /// Cloudflare API token scoped to DNS Edit for the configured zone.
    #[arg(
        long,
        env = "CLOUDFLARE_API_TOKEN",
        value_name = "TOKEN",
        hide_env_values = true
    )]
    pub api_token: String,

    /// Cloudflare zone ID containing the record.
    #[arg(long, env = "CLOUDFLARE_ZONE_ID", value_name = "ID")]
    pub zone_id: String,

    /// Fully-qualified A/AAAA record name. Repeat or comma-separate for multiple names.
    #[arg(
        long = "record-name",
        env = "CLOUDFLARE_RECORD_NAME",
        value_name = "FQDN",
        value_delimiter = ',',
        action = clap::ArgAction::Append,
        required = true
    )]
    pub record_names: Vec<String>,

    /// Seconds between reconciliation attempts.
    #[arg(
        long,
        env = "CF_DDNS_INTERVAL_SECONDS",
        default_value_t = 300,
        value_name = "SECONDS"
    )]
    pub interval: u64,

    /// IPv4 public-address discovery endpoint.
    #[arg(
        long,
        env = "CF_DDNS_IPV4_URL",
        default_value = DEFAULT_TRACE_URL,
        value_name = "URL"
    )]
    pub ipv4_url: String,

    /// IPv6 public-address discovery endpoint.
    #[arg(
        long,
        env = "CF_DDNS_IPV6_URL",
        default_value = DEFAULT_TRACE_URL,
        value_name = "URL"
    )]
    pub ipv6_url: String,

    /// Reconcile once and exit instead of polling.
    #[arg(long, env = "CF_DDNS_ONCE", action = clap::ArgAction::SetTrue)]
    pub once: bool,
}

pub struct Config {
    pub api_token: String,
    pub zone_id: String,
    pub record_names: Vec<String>,
    pub interval: Duration,
    pub ipv4_url: String,
    pub ipv6_url: String,
    pub once: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("the API token must not be empty or contain control characters")]
    InvalidApiToken,
    #[error("the zone ID must contain exactly 32 hexadecimal characters")]
    InvalidZoneId,
    #[error("the record name must be a valid ASCII/punycode fully-qualified name")]
    InvalidRecordName,
    #[error("record name {0} was configured more than once")]
    DuplicateRecordName(String),
    #[error("the reconciliation interval must be greater than zero")]
    InvalidInterval,
    #[error("{field} must be an absolute HTTPS URL")]
    InvalidUrl { field: &'static str },
}

impl TryFrom<Cli> for Config {
    type Error = ConfigError;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        if cli.api_token.is_empty() || cli.api_token.chars().any(char::is_control) {
            return Err(ConfigError::InvalidApiToken);
        }
        if cli.zone_id.len() != 32 || !cli.zone_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ConfigError::InvalidZoneId);
        }

        let mut record_names = Vec::with_capacity(cli.record_names.len());
        for configured_name in cli.record_names {
            let record_name = configured_name.trim_end_matches('.').to_ascii_lowercase();
            if record_name.is_empty()
                || record_name.len() > 253
                || !record_name.is_ascii()
                || !record_name.contains('.')
                || !valid_record_name(&record_name)
                || configured_name.trim() != configured_name
            {
                return Err(ConfigError::InvalidRecordName);
            }
            if record_names.contains(&record_name) {
                return Err(ConfigError::DuplicateRecordName(record_name));
            }
            record_names.push(record_name);
        }
        if record_names.is_empty() {
            return Err(ConfigError::InvalidRecordName);
        }
        if cli.interval == 0 {
            return Err(ConfigError::InvalidInterval);
        }

        validate_https_url("IPv4 discovery URL", &cli.ipv4_url)?;
        validate_https_url("IPv6 discovery URL", &cli.ipv6_url)?;

        Ok(Self {
            api_token: cli.api_token,
            zone_id: cli.zone_id,
            record_names,
            interval: Duration::from_secs(cli.interval),
            ipv4_url: cli.ipv4_url,
            ipv6_url: cli.ipv6_url,
            once: cli.once,
        })
    }
}

fn valid_record_name(name: &str) -> bool {
    name.split('.').enumerate().all(|(index, label)| {
        !label.is_empty()
            && label.len() <= 63
            && ((index == 0 && label == "*")
                || label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    })
}

fn validate_https_url(field: &'static str, value: &str) -> Result<(), ConfigError> {
    let uri = value
        .parse::<Uri>()
        .map_err(|_| ConfigError::InvalidUrl { field })?;
    if uri.scheme_str() != Some("https") || uri.authority().is_none() {
        return Err(ConfigError::InvalidUrl { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> Cli {
        Cli {
            api_token: "token".to_owned(),
            zone_id: "0123456789abcdef0123456789abcdef".to_owned(),
            record_names: vec!["Home.Example.COM.".to_owned()],
            interval: 300,
            ipv4_url: DEFAULT_TRACE_URL.to_owned(),
            ipv6_url: DEFAULT_TRACE_URL.to_owned(),
            once: false,
        }
    }

    #[test]
    fn validates_and_normalizes_configuration() {
        let config = Config::try_from(cli()).unwrap();
        assert_eq!(config.record_names, ["home.example.com"]);
        assert_eq!(config.interval, Duration::from_secs(300));
    }

    #[test]
    fn rejects_zero_interval() {
        let mut cli = cli();
        cli.interval = 0;
        assert!(matches!(
            Config::try_from(cli),
            Err(ConfigError::InvalidInterval)
        ));
    }

    #[test]
    fn rejects_non_https_discovery_url() {
        let mut cli = cli();
        cli.ipv4_url = "http://example.com/ip".to_owned();
        assert!(matches!(
            Config::try_from(cli),
            Err(ConfigError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn rejects_malformed_zone_id() {
        let mut cli = cli();
        cli.zone_id = "not-a-zone-id".to_owned();
        assert!(matches!(
            Config::try_from(cli),
            Err(ConfigError::InvalidZoneId)
        ));
    }

    #[test]
    fn rejects_invalid_record_name_characters() {
        for record_name in ["home example.com", "home/example.com", "home.*.example.com"] {
            let mut cli = cli();
            cli.record_names = vec![record_name.to_owned()];
            assert!(matches!(
                Config::try_from(cli),
                Err(ConfigError::InvalidRecordName)
            ));
        }
    }

    #[test]
    fn accepts_wildcard_and_underscore_record_names() {
        for record_name in ["*.example.com", "_home.example.com"] {
            let mut cli = cli();
            cli.record_names = vec![record_name.to_owned()];
            assert!(Config::try_from(cli).is_ok());
        }
    }

    #[test]
    fn accepts_multiple_record_names() {
        let mut cli = cli();
        cli.record_names = vec!["Home.Example.COM.".to_owned(), "old.example.com".to_owned()];

        let config = Config::try_from(cli).unwrap();
        assert_eq!(config.record_names, ["home.example.com", "old.example.com"]);
    }

    #[test]
    fn rejects_duplicate_record_names_after_normalization() {
        let mut cli = cli();
        cli.record_names = vec![
            "Home.Example.COM.".to_owned(),
            "home.example.com".to_owned(),
        ];

        assert!(matches!(
            Config::try_from(cli),
            Err(ConfigError::DuplicateRecordName(name)) if name == "home.example.com"
        ));
    }

    #[test]
    fn parses_repeated_and_comma_separated_record_names() {
        let cli = Cli::try_parse_from([
            "cf-ddns",
            "--api-token",
            "token",
            "--zone-id",
            "0123456789abcdef0123456789abcdef",
            "--record-name",
            "home.example.com,old.example.com",
            "--record-name",
            "other.example.com",
        ])
        .unwrap();

        assert_eq!(
            cli.record_names,
            ["home.example.com", "old.example.com", "other.example.com"]
        );
    }
}
