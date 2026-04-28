# drone-dock-agent

`drone-dock-agent` is a Raspberry Pi-friendly watcher process for a Hextronics drone dock.

It watches a local folder, and for each new video file it:

1. Creates a mission via API route `POST /api/v1/missions/yards/{yardId}/missions`
2. Requests an upload target via API route `POST /api/v1/videos/upload-url`
3. Uploads the file to S3 via the returned pre-signed POST URL
4. Confirms upload via API route `POST /api/v1/videos/confirm`
5. Associates the uploaded video to the mission via `PATCH /api/v1/missions/{missionId}/video`
6. Triggers inference via `POST /api/v1/missions/{missionId}/trigger-ml-processing`

This keeps the drone dock flow aligned with the same backend routes used by the app.

Implementation note: runtime logic is intentionally consolidated in `src/drone_dock_agent/agent.py` (single file), while deployment settings live in the separate JSON config file.

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
- Attach this metadata in `/api/v1/videos/confirm` payload metadata

## Configuration (versioned JSON)

The agent now reads a versioned config file to keep deployment settings centralized and DRY.

1. Copy `config/dock-agent.config.v1.json` to your host (for example `/etc/unspace/drone-dock-agent.config.json`).
2. Fill in `api_base_url`, `yard_id`, `watch_directory`, and tuning values.
3. Provide auth token either by:
   - setting `api_bearer_token` in JSON, or
   - setting env var `DRONE_DOCK_API_BEARER_TOKEN`.
4. Optional health/liveness settings in JSON:
   - `heartbeat_file`
   - `heartbeat_interval_seconds`
   - `heartbeat_max_age_seconds`

Config schema version is enforced via `config_version` and currently supports `1`.

### Environment variables

- `DRONE_DOCK_CONFIG_PATH` (optional, default `./drone-dock-agent.config.json`)
- `DRONE_DOCK_API_BEARER_TOKEN` (optional fallback if token omitted from JSON)

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
