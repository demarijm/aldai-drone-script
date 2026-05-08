"""Command line installer and operations helpers for the Unspace drone dock agent."""

from __future__ import annotations

import argparse
import grp
import json
import os
import pwd
import shutil
import subprocess
from pathlib import Path
from typing import Any

CONFIG_VERSION = 1
DEFAULT_API_BASE_URL = "https://api.unspace.com"
DEFAULT_CONFIG_PATH = Path("/etc/unspace/drone-dock-agent.config.json")
DEFAULT_HEARTBEAT_FILE = Path("/tmp/drone-dock-agent.heartbeat")
DEFAULT_INSTALL_DIR = Path("/opt/unspace/drone-dock-agent")
DEFAULT_SERVICE_GROUP = "unspace"
DEFAULT_SERVICE_NAME = "drone-dock-agent"
DEFAULT_SERVICE_USER = "unspace"
DEFAULT_WATCH_DIRECTORY = Path("/var/lib/drone-dock/uploads")
SYSTEMD_DIR = Path("/etc/systemd/system")

SERVICE_TEMPLATE = """[Unit]
Description=Unspace Drone Dock Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={service_user}
Group={service_group}
WorkingDirectory={install_dir}
Environment=DRONE_DOCK_CONFIG_PATH={config_path}
Environment=PYTHONUNBUFFERED=1
ExecStartPre={venv_dir}/bin/drone-dock-agent --healthcheck
ExecStart={venv_dir}/bin/drone-dock-agent
ExecStartPost=/bin/bash -lc '{venv_dir}/bin/drone-dock-agent --healthcheck'
Restart=always
RestartSec=10
StartLimitIntervalSec=300
StartLimitBurst=5
TimeoutStopSec=30
KillSignal=SIGTERM
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=true
ReadWritePaths=/tmp {watch_directory}

# Logs (journalctl -u {service_name})
StandardOutput=journal
StandardError=journal
SyslogIdentifier={service_name}

[Install]
WantedBy=multi-user.target
"""


def main(argv: list[str] | None = None) -> None:
    parser = _build_parser()
    args = parser.parse_args(argv)
    args.func(args)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="unspace", description="Install and manage the Unspace drone dock agent.")
    subparsers = parser.add_subparsers(required=True)

    install_parser = subparsers.add_parser("install", help="Configure, enable, and start the drone dock agent.")
    install_parser.add_argument("--api-key", required=True, help="Yard API key. Must start with ysk_.")
    install_parser.add_argument("--yard-id", required=True, help="Unspace yard identifier for upload URL creation.")
    install_parser.add_argument("--dock-id", required=True, help="Dock identifier to include in config and ingest metadata.")
    install_parser.add_argument("--api-base-url", default=DEFAULT_API_BASE_URL, help="Unspace API base URL.")
    install_parser.add_argument(
        "--watch-directory",
        type=Path,
        default=DEFAULT_WATCH_DIRECTORY,
        help="Directory to watch for drone videos.",
    )
    install_parser.add_argument(
        "--install-dir",
        type=Path,
        default=DEFAULT_INSTALL_DIR,
        help="Directory that contains the installed agent virtualenv.",
    )
    install_parser.add_argument("--config-path", type=Path, default=DEFAULT_CONFIG_PATH, help="Config file to write.")
    install_parser.add_argument("--service-name", default=DEFAULT_SERVICE_NAME, help="systemd service name.")
    install_parser.add_argument("--service-user", default=DEFAULT_SERVICE_USER, help="User that runs the systemd service.")
    install_parser.add_argument("--service-group", default=DEFAULT_SERVICE_GROUP, help="Group that runs the systemd service.")
    install_parser.add_argument(
        "--startup-test-upload",
        choices=("auto", "true", "false"),
        default="auto",
        help="Whether to run the one-second MP4 upload smoke test at agent startup.",
    )
    install_parser.add_argument(
        "--require-startup-test",
        action="store_true",
        help="Fail startup if the upload smoke test fails.",
    )
    install_parser.add_argument(
        "--no-start",
        action="store_true",
        help="Write config and service files without starting systemd.",
    )
    install_parser.set_defaults(func=install)

    status_parser = subparsers.add_parser("status", help="Show the drone dock agent systemd status.")
    status_parser.add_argument("--service-name", default=DEFAULT_SERVICE_NAME, help="systemd service name.")
    status_parser.set_defaults(func=status)

    return parser


def install(args: argparse.Namespace) -> None:
    _require_root()
    if not args.api_key.startswith("ysk_"):
        raise SystemExit("--api-key must be a yard API key starting with ysk_")

    install_dir = args.install_dir.expanduser().resolve()
    venv_dir = install_dir / ".venv"
    config_path = args.config_path.expanduser().resolve()
    watch_directory = args.watch_directory.expanduser().resolve()

    _ensure_service_account(args.service_user, args.service_group)
    _ensure_directory(install_dir, args.service_user, args.service_group, mode=0o755)
    _ensure_directory(watch_directory, args.service_user, args.service_group, mode=0o775)
    _write_config(
        config_path=config_path,
        config=_build_agent_config(args=args, watch_directory=watch_directory),
        service_group=args.service_group,
    )
    _write_service_file(
        service_path=SYSTEMD_DIR / f"{args.service_name}.service",
        service_text=_render_service(
            service_name=args.service_name,
            service_user=args.service_user,
            service_group=args.service_group,
            install_dir=install_dir,
            venv_dir=venv_dir,
            config_path=config_path,
            watch_directory=watch_directory,
        ),
    )

    _run(["systemctl", "daemon-reload"])
    _run(["systemctl", "enable", args.service_name])
    if not args.no_start:
        _run(["systemctl", "restart", args.service_name])
        _run(["systemctl", "--no-pager", "--lines", "20", "status", args.service_name], check=False)

    print(f"Configured {args.service_name}.")
    print(f"Config: {config_path}")
    print(f"Watch directory: {watch_directory}")


def status(args: argparse.Namespace) -> None:
    _run(["systemctl", "--no-pager", "--lines", "50", "status", args.service_name], check=False)


def _build_agent_config(args: argparse.Namespace, watch_directory: Path) -> dict[str, Any]:
    return {
        "config_version": CONFIG_VERSION,
        "api_base_url": args.api_base_url.rstrip("/"),
        "yard_id": args.yard_id,
        "dock_id": args.dock_id,
        "watch_directory": str(watch_directory),
        "mission_name_prefix": f"Hextronics Dock {args.dock_id}",
        "include_extensions": [".mp4", ".mov", ".mkv", ".avi"],
        "stable_check_interval_seconds": 1.0,
        "stable_required_checks": 3,
        "model_type": "coupler_genie",
        "annotated_video": True,
        "multi_track": False,
        "heartbeat_file": str(DEFAULT_HEARTBEAT_FILE),
        "heartbeat_interval_seconds": 15,
        "heartbeat_max_age_seconds": 120,
        "api_key": args.api_key,
        "startup_test_upload_enabled": _parse_bool_option(args.startup_test_upload),
        "startup_test_upload_required": args.require_startup_test,
    }


def _parse_bool_option(value: str) -> bool | str:
    if value == "auto":
        return value
    return value == "true"


def _render_service(
    *,
    service_name: str,
    service_user: str,
    service_group: str,
    install_dir: Path,
    venv_dir: Path,
    config_path: Path,
    watch_directory: Path,
) -> str:
    return SERVICE_TEMPLATE.format(
        service_name=service_name,
        service_user=service_user,
        service_group=service_group,
        install_dir=install_dir,
        venv_dir=venv_dir,
        config_path=config_path,
        watch_directory=watch_directory,
    )


def _write_config(*, config_path: Path, config: dict[str, Any], service_group: str) -> None:
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    os.chmod(config_path, 0o640)
    _chown(config_path, "root", service_group)


def _write_service_file(*, service_path: Path, service_text: str) -> None:
    service_path.write_text(service_text, encoding="utf-8")
    os.chmod(service_path, 0o644)


def _ensure_service_account(user: str, group: str) -> None:
    if not _group_exists(group):
        _run(["groupadd", "--system", group])
    if not _user_exists(user):
        _run([
            "useradd",
            "--system",
            "--gid",
            group,
            "--home-dir",
            "/opt/unspace",
            "--shell",
            "/usr/sbin/nologin",
            user,
        ])


def _ensure_directory(path: Path, user: str, group: str, *, mode: int) -> None:
    path.mkdir(parents=True, exist_ok=True)
    os.chmod(path, mode)
    _chown(path, user, group)


def _user_exists(user: str) -> bool:
    if shutil.which("id") is None:
        return False
    result = subprocess.run(
        ["id", "-u", user],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def _group_exists(group: str) -> bool:
    if shutil.which("getent") is None:
        return False
    result = subprocess.run(
        ["getent", "group", group],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def _chown(path: Path, user: str, group: str) -> None:
    uid = pwd.getpwnam(user).pw_uid if user != "root" else 0
    gid = _gid_for_group(group)
    os.chown(path, uid, gid)


def _gid_for_group(group: str) -> int:
    return grp.getgrnam(group).gr_gid


def _require_root() -> None:
    if os.geteuid() != 0:
        raise SystemExit("Run this command with sudo: sudo unspace install ...")


def _run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    print("+ " + " ".join(command))
    return subprocess.run(command, text=True, check=check)


if __name__ == "__main__":
    main()
