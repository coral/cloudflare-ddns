# cf-ddns

## SLOP ALERT

## THIS IS 100% SLOP

`cf-ddns` is a small, single-purpose Cloudflare dynamic DNS client. It keeps one
or more existing A records aligned with the public IPv4 address seen from the
container. When the container also has working public IPv6 connectivity, it
updates the existing AAAA records for those names.

The process reconciles immediately at startup and every five minutes afterward.
It does not keep local state, create records, delete records, or alter record
metadata such as TTL, proxy status, comments, or tags.

## Cloudflare setup

Create an API token scoped to the target zone with **Zone / DNS / Edit**
permission. Copy the zone ID from the Cloudflare dashboard, then create the A
records and, if IPv6 should be managed, the corresponding AAAA records before
starting the client.

Only API token authentication is supported. A global API key is intentionally
not supported.

## Run in Docker

The workflow publishes multi-architecture images for AMD64 and ARM64 to
`ghcr.io/coral/cloudflare-ddns`. Pull the current image with:

```sh
docker pull ghcr.io/coral/cloudflare-ddns:latest
```

To build the same minimal image locally:

```sh
docker build -t cf-ddns .
```

Run it as a daemon:

```sh
docker run --detach \
  --name cf-ddns \
  --restart unless-stopped \
  --env CLOUDFLARE_API_TOKEN='replace-me' \
  --env CLOUDFLARE_ZONE_ID='0123456789abcdef0123456789abcdef' \
  --env CLOUDFLARE_RECORD_NAME='home.example.com' \
  ghcr.io/coral/cloudflare-ddns:latest
```

For a one-shot job, append `--once`:

```sh
docker run --rm \
  --env CLOUDFLARE_API_TOKEN='replace-me' \
  --env CLOUDFLARE_ZONE_ID='0123456789abcdef0123456789abcdef' \
  --env CLOUDFLARE_RECORD_NAME='home.example.com' \
  ghcr.io/coral/cloudflare-ddns:latest --once
```

Prefer injecting the token through an environment-backed secret. Supplying it
with `--api-token` can expose it in the host process list.

### Docker Compose

Copy [compose.example.yaml](compose.example.yaml) to `compose.yaml`, export the
three required variables, and start it:

```sh
export CLOUDFLARE_API_TOKEN='replace-me'
export CLOUDFLARE_ZONE_ID='0123456789abcdef0123456789abcdef'
export CLOUDFLARE_RECORD_NAME='home.example.com,old.example.com'
docker compose up -d
```

Use a version tag instead of `latest` when you want repeatable deployments.
Images pushed from `master` receive `latest`, `master`, and commit-SHA tags;
Git tags such as `v1.2.3` also publish `1.2.3` and `1.2` image tags.

The runtime stage is based on `scratch` and contains only the static executable.
It runs without root privileges, capabilities, or a writable filesystem in the
provided Compose configuration.

## Configuration

Command-line options override environment variables.

| CLI option | Environment variable | Default |
| --- | --- | --- |
| `--api-token TOKEN` | `CLOUDFLARE_API_TOKEN` | required |
| `--zone-id ID` | `CLOUDFLARE_ZONE_ID` | required |
| `--record-name FQDN` | `CLOUDFLARE_RECORD_NAME` | required; repeatable/CSV |
| `--interval SECONDS` | `CF_DDNS_INTERVAL_SECONDS` | `300` |
| `--ipv4-url URL` | `CF_DDNS_IPV4_URL` | Cloudflare trace |
| `--ipv6-url URL` | `CF_DDNS_IPV6_URL` | Cloudflare trace |
| `--once` | `CF_DDNS_ONCE` | `false` |

The default address source is
`https://www.cloudflare.com/cdn-cgi/trace`. Separate HTTP clients force IPv4
and IPv6 when calling it. Replacement endpoints must use HTTPS and may return
either a Cloudflare-style `ip=...` line or a bare IP address.

Record names must be fully qualified and written as ASCII or punycode. A final
dot is accepted and removed. To manage multiple names in the configured zone,
use a comma-separated environment value:

```sh
CLOUDFLARE_RECORD_NAME='home.coral.works,old.coral.works'
```

The equivalent CLI form repeats the option:

```sh
--record-name home.coral.works --record-name old.coral.works
```

Public addresses are discovered once per cycle and applied to every name. The
client validates that all required records exist before updating any of them.

If IPv6 discovery fails, the cycle still succeeds after reconciling the A records
and any existing AAAA records are left unchanged. If IPv6 is discovered but any
name lacks a single matching AAAA record, the client exits with an error rather
than creating or choosing a record.

Authentication failures and missing or duplicate required records cause daemon
mode to exit nonzero. Network failures, Cloudflare rate limits, and Cloudflare
5xx responses are retried on the next interval.

## Run directly

```sh
cargo run --release -- \
  --api-token 'replace-me' \
  --zone-id '0123456789abcdef0123456789abcdef' \
  --record-name 'home.example.com' \
  --record-name 'old.example.com' \
  --once
```

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
