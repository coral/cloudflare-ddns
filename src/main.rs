use std::process::ExitCode;
use std::thread;

use cf_ddns::cloudflare::CloudflareClient;
use cf_ddns::config::{Cli, Config};
use cf_ddns::ip::PublicIpDiscovery;
use cf_ddns::reconcile::Reconciler;
use clap::Parser;

fn main() -> ExitCode {
    let config = match Config::try_from(Cli::parse()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("ERROR invalid configuration: {error}");
            return ExitCode::from(2);
        }
    };

    let discovery = PublicIpDiscovery::new(
        config.ipv4_url.clone(),
        config.ipv6_url.clone(),
        env!("CARGO_PKG_VERSION"),
    );
    let cloudflare =
        CloudflareClient::new(config.api_token, config.zone_id, env!("CARGO_PKG_VERSION"));
    let reconciler = Reconciler::new(discovery, cloudflare, config.record_names);

    loop {
        match reconciler.reconcile() {
            Ok(report) => {
                let ipv6 = report
                    .public_ipv6
                    .map_or_else(|| "unavailable".to_owned(), |address| address.to_string());
                eprintln!(
                    "INFO reconciliation complete for {} name(s): IPv4={}, IPv6={ipv6}",
                    report.records.len(),
                    report.public_ipv4
                );
                if config.once {
                    return ExitCode::SUCCESS;
                }
            }
            Err(error) => {
                eprintln!("ERROR reconciliation failed: {error}");
                if config.once || error.is_permanent() {
                    return ExitCode::from(1);
                }
                eprintln!(
                    "WARN transient failure; retrying in {} seconds",
                    config.interval.as_secs()
                );
            }
        }

        thread::sleep(config.interval);
    }
}
