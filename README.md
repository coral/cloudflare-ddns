# cf-ddns

## SLOP ALERT

## THIS IS 100% SLOP

`cf-ddns` is a small, single-purpose Cloudflare dynamic DNS client. It keeps one
existing A record aligned with the public IPv4 address seen from the container.
When the container also has working public IPv6 connectivity, it updates the
existing AAAA record for the same name.

The process reconciles immediately at startup and every five minutes afterward.
It does not keep local state, create records, delete records, or alter record
metadata such as TTL, proxy status, comments, or tags.

## Cloudflare setup

Create an API token scoped to the target zone with **Zone / DNS / Edit**
permission. Copy the zone ID from the Cloudflare dashboard, then create the A
record and, if IPv6 should be managed, the AAAA record before starting the
client.

Only API token authentication is supported. A global API key is intentionally
not supported.

## Run in Docker

Build the image:

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
  cf-ddns
```

For a one-shot job, append `--once`:

```sh
docker run --rm \
  --env CLOUDFLARE_API_TOKEN='replace-me' \
  --env CLOUDFLARE_ZONE_ID='0123456789abcdef0123456789abcdef' \
  --env CLOUDFLARE_RECORD_NAME='home.example.com' \
  cf-ddns --once
```

Prefer injecting the token through an environment-backed secret. Supplying it
with `--api-token` can expose it in the host process list.

## Configuration

Command-line options override environment variables.

| CLI option | Environment variable | Default |
| --- | --- | --- |
| `--api-token TOKEN` | `CLOUDFLARE_API_TOKEN` | required |
| `--zone-id ID` | `CLOUDFLARE_ZONE_ID` | required |
| `--record-name FQDN` | `CLOUDFLARE_RECORD_NAME` | required |
| `--interval SECONDS` | `CF_DDNS_INTERVAL_SECONDS` | `300` |
| `--ipv4-url URL` | `CF_DDNS_IPV4_URL` | Cloudflare trace |
| `--ipv6-url URL` | `CF_DDNS_IPV6_URL` | Cloudflare trace |
| `--once` | `CF_DDNS_ONCE` | `false` |

The default address source is
`https://www.cloudflare.com/cdn-cgi/trace`. Separate HTTP clients force IPv4
and IPv6 when calling it. Replacement endpoints must use HTTPS and may return
either a Cloudflare-style `ip=...` line or a bare IP address.

Record names must be fully qualified and written as ASCII or punycode. A final
dot is accepted and removed.

If IPv6 discovery fails, the cycle still succeeds after reconciling the A record
and any existing AAAA record is left unchanged. If IPv6 is discovered but there
is no single matching AAAA record, the client exits with an error rather than
creating or choosing a record.

Authentication failures and missing or duplicate required records cause daemon
mode to exit nonzero. Network failures, Cloudflare rate limits, and Cloudflare
5xx responses are retried on the next interval.

## Run directly

```sh
cargo run --release -- \
  --api-token 'replace-me' \
  --zone-id '0123456789abcdef0123456789abcdef' \
  --record-name 'home.example.com' \
  --once
```

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
