use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;

use crate::cloudflare::{CloudflareError, DnsProvider, DnsRecord};
use crate::ip::{IpError, IpProvider};

pub struct Reconciler<I, D> {
    ip_provider: I,
    dns_provider: D,
    record_names: Vec<String>,
}

impl<I: IpProvider, D: DnsProvider> Reconciler<I, D> {
    pub fn new(ip_provider: I, dns_provider: D, record_names: Vec<String>) -> Self {
        Self {
            ip_provider,
            dns_provider,
            record_names,
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

        // Validate every applicable record before making the first change. This prevents
        // a bad later name from leaving an avoidable partial update across the set.
        let mut prepared_names = Vec::with_capacity(self.record_names.len());
        for name in &self.record_names {
            let records = self.dns_provider.list_records(name)?;
            let a_record = exactly_one(&records, name, "A")?;
            let a = prepare_record(a_record, name, IpAddr::V4(ipv4))?;
            let aaaa = match ipv6 {
                Some(address) => {
                    let record = exactly_one(&records, name, "AAAA")?;
                    Some(prepare_record(record, name, IpAddr::V6(address))?)
                }
                None => None,
            };
            prepared_names.push(PreparedName {
                name: name.clone(),
                a,
                aaaa,
            });
        }

        let mut record_reports = Vec::with_capacity(prepared_names.len());
        for prepared in prepared_names {
            let a_changed = self.reconcile_record(&prepared.name, &prepared.a)?;
            let aaaa_changed = prepared
                .aaaa
                .as_ref()
                .map(|record| self.reconcile_record(&prepared.name, record))
                .transpose()?;
            record_reports.push(RecordReport {
                name: prepared.name,
                a_changed,
                aaaa_changed,
            });
        }

        Ok(ReconcileReport {
            public_ipv4: ipv4,
            public_ipv6: ipv6,
            records: record_reports,
        })
    }

    fn reconcile_record(
        &self,
        name: &str,
        prepared: &PreparedRecord,
    ) -> Result<bool, ReconcileError> {
        if prepared.current == prepared.desired {
            eprintln!(
                "INFO {} record {name} is already current at {}",
                prepared.record.record_type, prepared.desired
            );
            return Ok(false);
        }

        self.dns_provider
            .update_record(&prepared.record.id, prepared.desired)?;
        eprintln!(
            "INFO updated {} record {name} from {} to {}",
            prepared.record.record_type, prepared.current, prepared.desired
        );
        Ok(true)
    }
}

struct PreparedName {
    name: String,
    a: PreparedRecord,
    aaaa: Option<PreparedRecord>,
}

struct PreparedRecord {
    record: DnsRecord,
    current: IpAddr,
    desired: IpAddr,
}

fn prepare_record(
    record: &DnsRecord,
    name: &str,
    desired: IpAddr,
) -> Result<PreparedRecord, ReconcileError> {
    let current = record.content.parse::<IpAddr>().map_err(|_| {
        ReconcileError::Topology(format!(
            "Cloudflare {} record {name} has invalid address content",
            record.record_type
        ))
    })?;
    Ok(PreparedRecord {
        record: record.clone(),
        current,
        desired,
    })
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
    pub records: Vec<RecordReport>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RecordReport {
    pub name: String,
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
        named_record(id, "home.example.com", record_type, content)
    }

    fn named_record(id: &str, name: &str, record_type: &str, content: &str) -> DnsRecord {
        DnsRecord {
            id: id.to_owned(),
            name: name.to_owned(),
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
        let reconciler = Reconciler::new(ips, dns, vec!["home.example.com".to_owned()]);

        let report = reconciler.reconcile().unwrap();
        assert!(report.records[0].a_changed);
        assert_eq!(report.records[0].aaaa_changed, Some(true));
        assert_eq!(reconciler.dns_provider.updates.borrow().len(), 2);
    }

    #[test]
    fn updates_multiple_names_with_one_discovered_address_pair() {
        let ips = FakeIps {
            ipv4: "198.51.100.10".parse().unwrap(),
            ipv6: Ok("2001:db8::10".parse().unwrap()),
        };
        let dns = FakeDns {
            records: vec![
                named_record("home-a", "home.example.com", "A", "192.0.2.1"),
                named_record("home-aaaa", "home.example.com", "AAAA", "2001:db8::1"),
                named_record("old-a", "old.example.com", "A", "192.0.2.2"),
                named_record("old-aaaa", "old.example.com", "AAAA", "2001:db8::2"),
            ],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(
            ips,
            dns,
            vec!["home.example.com".to_owned(), "old.example.com".to_owned()],
        );

        let report = reconciler.reconcile().unwrap();
        assert_eq!(report.records.len(), 2);
        assert_eq!(report.records[0].name, "home.example.com");
        assert_eq!(report.records[1].name, "old.example.com");
        assert!(report.records.iter().all(|record| record.a_changed));
        assert!(
            report
                .records
                .iter()
                .all(|record| record.aaaa_changed == Some(true))
        );
        assert_eq!(reconciler.dns_provider.updates.borrow().len(), 4);
    }

    #[test]
    fn validates_all_names_before_updating_any_record() {
        let ips = FakeIps {
            ipv4: "198.51.100.10".parse().unwrap(),
            ipv6: Ok("2001:db8::10".parse().unwrap()),
        };
        let dns = FakeDns {
            records: vec![
                named_record("home-a", "home.example.com", "A", "192.0.2.1"),
                named_record("home-aaaa", "home.example.com", "AAAA", "2001:db8::1"),
                named_record("old-a", "old.example.com", "A", "192.0.2.2"),
            ],
            ..FakeDns::default()
        };
        let reconciler = Reconciler::new(
            ips,
            dns,
            vec!["home.example.com".to_owned(), "old.example.com".to_owned()],
        );

        let error = reconciler.reconcile().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no AAAA record exists for old.example.com")
        );
        assert!(reconciler.dns_provider.updates.borrow().is_empty());
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
        let reconciler = Reconciler::new(ips, dns, vec!["home.example.com".to_owned()]);

        let report = reconciler.reconcile().unwrap();
        assert!(!report.records[0].a_changed);
        assert_eq!(report.records[0].aaaa_changed, None);
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
        let reconciler = Reconciler::new(ips, dns, vec!["home.example.com".to_owned()]);

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
        let reconciler = Reconciler::new(ips, dns, vec!["home.example.com".to_owned()]);

        let report = reconciler.reconcile().unwrap();
        assert_eq!(report.records[0].aaaa_changed, Some(false));
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
        let reconciler = Reconciler::new(ips, dns, vec!["home.example.com".to_owned()]);

        let error = reconciler.reconcile().unwrap_err();
        assert!(error.to_string().contains("multiple A records"));
        assert!(reconciler.dns_provider.updates.borrow().is_empty());
    }
}
