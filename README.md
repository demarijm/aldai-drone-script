# unspace-dock

`unspace-dock` is a Rust CLI for the Unspace drone dock ingest agent. It is intended to run 24/7 on ARM64 Linux drone docks, manage a versioned config file, run as a hardened systemd service, and provide the command surface needed for video ingest work.

## Implemented foundation

- Clap command routing for `install`, `uninstall`, `watch`, `status`, `logs`, `update`, `healthcheck`, `config show`, and `config set`.
- Versioned JSON config at `/etc/unspace/config.json` by default.
- `UNSPACE_CONFIG_PATH` config path override.
- `UNSPACE_API_KEY` secret override.
- `UNSPACE_PID_FILE` PID-file override (useful for running `watch` outside systemd in dev/test).
- API key validation requiring a `ysk_` prefix.
- `config show` redacts the API key.
- `config set <key> <value>` validates and writes the config, then sends `SIGHUP` to `/run/unspace/unspace-dock.pid` when the service is running.
- `watch` writes the PID file, writes the local heartbeat file, catches `SIGHUP` for live config reload, and exits gracefully on `SIGTERM`/`SIGINT` (cleaning up the PID file).
- `install` writes the initial config from `--api-key` and `--dock-id`, creates the `unspace` system user and working directories, writes a hardened systemd unit, and starts the service.
- `healthcheck` validates the watch directory, heartbeat freshness, and API `/health` reachability.
- Tag-push CI builds a stripped ARM64 Linux musl binary for the dock hardware.

The previous Python prototype is still present under `src/drone_dock_agent/` for reference while the Rust CLI is built out.

## Python prototype parity

No. The Rust CLI is a deployment and service-management scaffold, not a 1:1 port of the Python prototype yet. The Rust implementation currently covers CLI routing, config validation/redaction, systemd install scaffolding, PID/heartbeat handling, SIGHUP config reload, and health checks.

The Python prototype still contains ingest-agent behavior that has not been ported to Rust yet:

- filesystem event watching and startup backfill
- video extension filtering and file stability checks
- presigned upload URL requests and S3 POST uploads
- ingest-mission API calls
- KMZ resolution and filename metadata parsing
- WPML heading extraction from KMZ archives
- mission name, `track_group`, and `track_label` generation

Keep `src/drone_dock_agent/` as the functional reference implementation until those features are ported.

## Install on a Pi (one-liner)

The full bring-up — download, verify, install service, start, healthcheck — is one command on the dock:

```bash
curl -fsSL https://YOUR-URL/install.sh | sudo bash -s -- \
  --api-key ysk_KEY --dock-id DOCK_42
```

Updates use the same one-liner with no flags (existing config is reused, the binary is replaced and the service restarted):

```bash
curl -fsSL https://YOUR-URL/install.sh | sudo bash
```

Host both `unspace-dock-linux-arm64` (and its `.sha256`) and `install.sh` from the same URL prefix. The installer (`scripts/install.sh` in this repo) defaults to `https://releases.unspace.com/...`; replace the `DEFAULT_RELEASE_URL` constant before publishing, or override at runtime with `--release-url URL` (or `UNSPACE_RELEASE_URL`).

The release CI job builds and uploads `unspace-dock-linux-arm64` + `unspace-dock-linux-arm64.sha256`; copy `scripts/install.sh` next to those two files and you're done.

## Build and test

```bash
cargo test
cargo build --release
# Release CI builds the dock target: aarch64-unknown-linux-musl
```

## Configuration

Default path: `/etc/unspace/config.json`

Override path:

```bash
export UNSPACE_CONFIG_PATH=/tmp/unspace.config.json
```

Override API key without storing it in JSON:

```bash
export UNSPACE_API_KEY=ysk_KEY
```

Example config:

```json
{
  "config_version": 1,
  "api_base_url": "https://api.unspace.com",
  "api_key": "",
  "dock_id": "DOCK_42",
  "watch_dir": "/var/lib/unspace/uploads",
  "heartbeat_file": "/var/lib/unspace/heartbeat",
  "heartbeat_interval_secs": 15,
  "heartbeat_max_age_secs": 120
}
```

## CLI examples

```bash
unspace-dock --version
unspace-dock install --api-key ysk_KEY --dock-id DOCK_42
unspace-dock config show
unspace-dock config set watch_dir /mnt/drone-footage
unspace-dock healthcheck
unspace-dock status
unspace-dock logs
```

The systemd unit installed by `unspace-dock install` is named `unspace-dock.service`. Inspect it directly with `systemctl status unspace-dock` / `journalctl -u unspace-dock -f` if you prefer not to go through the CLI wrappers.

## Notes

The config intentionally contains only fields used by the current scaffold: API/dock identity, watch directory, and local heartbeat/healthcheck settings. Upload pipeline fields such as video extension filters, mission naming, model options, queue limits, and polling intervals will be added when those features are implemented. The `update` command is intentionally scaffolded and returns a clear not-implemented error rather than silently doing partial work.
