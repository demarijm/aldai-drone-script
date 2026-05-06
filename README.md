# drone-dock-agent

It watches a local folder, and for each new video file it:

1. Requests an upload target via API route `POST /api/v1/videos/upload-url`
2. Uploads the file to S3 via the returned pre-signed POST URL
3. Creates the mission, verifies/records the uploaded video, and triggers ML via `POST /api/v1/videos/ingest-mission`

The agent is intended to run with a yard API key (`ysk_...`). The yard is still supplied to `upload-url` because that route requires it, but the ingest step resolves the yard from the API key and does not send `yard_id` in the ingest body.

## Install

```bash
cd packages/drone-dock-agent
python -m venv .venv
source .venv/bin/activate
pip install -e .
```


## KMZ/WPML metadata support

The agent reads mission metadata from KMZ files using this naming convention:

`YardName_TrackGroup_FirstTrack_LastTrack.kmz`

Example: `Norris_GroupA_T01_T06.kmz`.

For each processed video, the agent will:

- Prefer a same-stem KMZ (e.g., `mission-001.mp4` + `mission-001.kmz`)
- Otherwise fall back to the first `.kmz` in the same directory
- Parse mission fields from the KMZ filename
- Open the KMZ and read WPML (`waylines.wpml`/`.wpml`) to extract drone heading
- Pass this metadata through the `/api/v1/videos/ingest-mission` payload metadata
- Send `track_group` from the KMZ track group and `track_label` from the first/last track designators when available

## Configuration (versioned JSON)

The agent now reads a versioned config file to keep deployment settings centralized and DRY.

1. Copy `config/dock-agent.config.v1.json` to your host (for example `/etc/unspace/drone-dock-agent.config.json`).
2. Fill in `api_base_url`, `yard_id`, `watch_directory`, and tuning values.
3. Provide your yard API key (`ysk_...`) either by:
   - setting `api_key` in JSON, or
   - setting env var `DRONE_DOCK_API_KEY`.
4. Optional health/liveness settings in JSON:
   - `heartbeat_file`
   - `heartbeat_interval_seconds`
   - `heartbeat_max_age_seconds`

Config schema version is enforced via `config_version` and currently supports `1`.

### Environment variables

- `DRONE_DOCK_CONFIG_PATH` (optional, default `./drone-dock-agent.config.json`)
- `DRONE_DOCK_API_KEY` (optional fallback if key omitted from JSON; must be a `ysk_` yard API key)

## Run

```bash
drone-dock-agent
```

## Notes for Raspberry Pi

- Run this process under `systemd` for automatic restart on boot.
- Ensure the watch directory and config/token are readable by the service user.
- Keep the Pi clock synced (NTP), since expiring auth tokens and pre-signed URL windows are time-sensitive.

## Systemd service (managed runtime + restart + logs + health checks)

Use the provided unit file:

- `systemd/drone-dock-agent.service`

This service includes:

- restart policy (`Restart=always`, `RestartSec=10`)
- journald logs (`journalctl -u drone-dock-agent -f`)
- environment variables (`DRONE_DOCK_CONFIG_PATH`, `PYTHONUNBUFFERED`)
- health checks using `drone-dock-agent --healthcheck` in `ExecStartPre` and `ExecStartPost`

Health check verifies:

- watch directory exists/readable
- API `/health` endpoint is reachable
- heartbeat file freshness when present

Install example:

```bash
sudo cp packages/drone-dock-agent/systemd/drone-dock-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable drone-dock-agent
sudo systemctl start drone-dock-agent
sudo systemctl status drone-dock-agent
```
