use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;

use crate::cloudflare::{CloudflareError, DnsProvider, DnsRecord};
use crate::ip::{IpError, IpProvider};

pub struct Reconciler<I, D> {
    ip_provider: I,
    dns_provider: D,
    record_name: String,
}

impl<I: IpProvider, D: DnsProvider> Reconciler<I, D> {
    pub fn new(ip_provider: I, dns_provider: D, record_name: String) -> Self {
        Self {
            ip_provider,
            dns_provider,
            record_name,
        }
    }

    pub fn reconcile(&self) -> Result<ReconcileReport, ReconcileError> {
        let ipv4 = self.ip_provider.ipv4().map_err(ReconcileError::Ipv4)?;
        eprintln!("INFO detected public IPv4 address {ipv4}");

        let ipv6 = match self.ip_provider.ipv6() {
            Ok(address) => {
                eprintln!("INFO detected public IPv6 address {address}");
                Some(address)
            }
            Err(error) => {
                eprintln!("INFO public IPv6 unavailable; leaving AAAA unchanged: {error}");
                None
            }
        };

        let records = self.dns_provider.list_records(&self.record_name)?;
        let a_record = exactly_one(&records, &self.record_name, "A")?;
        let aaaa_record = match ipv6 {
            Some(_) => Some(exactly_one(&records, &self.record_name, "AAAA")?),
            None => None,
        };

        // Validate the complete applicable record topology before making any changes.
        let a_changed = self.reconcile_record(a_record, IpAddr::V4(ipv4))?;
        let aaaa_changed = match (ipv6, aaaa_record) {
            (Some(address), Some(record)) => {
                Some(self.reconcile_record(record, IpAddr::V6(address))?)
            }
            _ => None,
        };

        Ok(ReconcileReport {
            public_ipv4: ipv4,
            public_ipv6: ipv6,
            a_changed,
            aaaa_changed,
        })
    }

    fn reconcile_record(
        &self,
        record: &DnsRecord,
        desired: IpAddr,
    ) -> Result<bool, ReconcileError> {
        let current = record.content.parse::<IpAddr>().map_err(|_| {
            ReconcileError::Topology(format!(
                "Cloudflare {} record {} has invalid address content",
                record.record_type, self.record_name
            ))
        })?;
        if current == desired {
            eprintln!(
                "INFO {} record {} is already current at {desired}",
                record.record_type, self.record_name
            );
            return Ok(false);
        }

        self.dns_provider.update_record(&record.id, desired)?;
        eprintln!(
            "INFO updated {} record {} from {current} to {desired}",
            record.record_type, self.record_name
        );
        Ok(true)
    }
}

fn exactly_one<'a>(
    records: &'a [DnsRecord],
    name: &str,
    record_type: &str,
) -> Result<&'a DnsRecord, ReconcileError> {
    let matching = records
        .iter()
        .filter(|record| {
            record.record_type == record_type
                && record.name.trim_end_matches('.').eq_ignore_ascii_case(name)
        })
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [record] => Ok(*record),
        [] => Err(ReconcileError::Topology(format!(
            "no {record_type} record exists for {name}; records are never created automatically"
        ))),
        _ => Err(ReconcileError::Topology(format!(
            "multiple {record_type} records exist for {name}; refusing to choose one"
        ))),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReconcileReport {
    pub public_ipv4: Ipv4Addr,
    pub public_ipv6: Option<Ipv6Addr>,
    pub a_changed: bool,
    pub aaaa_changed: Option<bool>,
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("could not determine required IPv4 address: {0}")]
    Ipv4(IpError),
    #[error(transparent)]
    Cloudflare(#[from] CloudflareError),
    #[error("invalid DNS record configuration: {0}")]
    Topology(String),
}

impl ReconcileError {
    pub fn is_permanent(&self) -> bool {
        match self {
            Self::Ipv4(error) => error.is_permanent(),
            Self::Cloudflare(error) => error.is_permanent(),
            Self::Topology(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct FakeIps {
        ipv4: Ipv4Addr,
        ipv6: Result<Ipv6Addr, IpError>,
    }

    impl IpProvider for FakeIps {
        fn ipv4(&self) -> Result<Ipv4Addr, IpError> {
            Ok(self.ipv4)
        }

        fn ipv6(&self) -> Result<Ipv6Addr, IpError> {
            self.ipv6
                .as_ref()
                .copied()
                .map_err(|_| IpError::MissingAddress)
        }
    }

    #[derive(Default)]
    struct FakeDns {
        records: Vec<DnsRecord>,
        updates: RefCell<Vec<(String, IpAddr)>>,
    }

    impl DnsProvider for FakeDns {
        fn list_records(&self, _name: &str) -> Result<Vec<DnsRecord>, CloudflareError> {
            Ok(self.records.clone())
        }

        fn update_record(&self, record_id: &str, address: IpAddr) -> Result<(), CloudflareError> {
            self.updates
                .borrow_mut()
                .push((record_id.to_owned(), address));
            Ok(())
        }
    }

    fn record(id: &str, record_type: &str, content: &str) -> DnsRecord {
        DnsRecord {
            id: id.to_owned(),
            name: "home.example.com".to_owned(),
            record_type: record_type.to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn updates_changed_dual_stack_records() {
        let ips = FakeIps {
            ipv4: "198.51.100.10".parse().unwrap(),
            ipv6: Ok("2001:db8::10".parse().unwrap()),
        };
        let dns = FakeDns {
            records: vec![
                record("a", "A", "192.0.2.1"),
                record("aaaa", "AAAA", "2001:db8::1"),
            ],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(ips, dns, "home.example.com".to_owned());

        let report = reconciler.reconcile().unwrap();
        assert!(report.a_changed);
        assert_eq!(report.aaaa_changed, Some(true));
        assert_eq!(reconciler.dns_provider.updates.borrow().len(), 2);
    }

    #[test]
    fn skips_aaaa_without_public_ipv6() {
        let ips = FakeIps {
            ipv4: "192.0.2.1".parse().unwrap(),
            ipv6: Err(IpError::MissingAddress),
        };
        let dns = FakeDns {
            records: vec![
                record("a", "A", "192.0.2.1"),
                record("aaaa", "AAAA", "2001:db8::1"),
            ],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(ips, dns, "home.example.com".to_owned());

        let report = reconciler.reconcile().unwrap();
        assert!(!report.a_changed);
        assert_eq!(report.aaaa_changed, None);
        assert!(reconciler.dns_provider.updates.borrow().is_empty());
    }

    #[test]
    fn missing_aaaa_is_permanent_when_ipv6_was_detected() {
        let ips = FakeIps {
            ipv4: "198.51.100.10".parse().unwrap(),
            ipv6: Ok("2001:db8::1".parse().unwrap()),
        };
        let dns = FakeDns {
            records: vec![record("a", "A", "192.0.2.1")],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(ips, dns, "home.example.com".to_owned());

        let error = reconciler.reconcile().unwrap_err();
        assert!(error.is_permanent());
        assert!(error.to_string().contains("no AAAA record"));
        assert!(reconciler.dns_provider.updates.borrow().is_empty());
    }

    #[test]
    fn compares_ipv6_addresses_semantically() {
        let ips = FakeIps {
            ipv4: "192.0.2.1".parse().unwrap(),
            ipv6: Ok("2001:db8::1".parse().unwrap()),
        };
        let dns = FakeDns {
            records: vec![
                record("a", "A", "192.0.2.1"),
                record("aaaa", "AAAA", "2001:0db8:0:0:0:0:0:1"),
            ],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(ips, dns, "home.example.com".to_owned());

        let report = reconciler.reconcile().unwrap();
        assert_eq!(report.aaaa_changed, Some(false));
        assert!(reconciler.dns_provider.updates.borrow().is_empty());
    }

    #[test]
    fn duplicate_a_records_fail_safely() {
        let ips = FakeIps {
            ipv4: "192.0.2.1".parse().unwrap(),
            ipv6: Err(IpError::MissingAddress),
        };
        let dns = FakeDns {
            records: vec![
                record("a1", "A", "192.0.2.1"),
                record("a2", "A", "192.0.2.2"),
            ],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(ips, dns, "home.example.com".to_owned());

        let error = reconciler.reconcile().unwrap_err();
        assert!(error.to_string().contains("multiple A records"));
        assert!(reconciler.dns_provider.updates.borrow().is_empty());
    }
}
