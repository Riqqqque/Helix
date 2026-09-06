#!/usr/bin/env python3
"""Read-only, secret-free native server discovery for administrators."""

import argparse
import json
import sys
import uuid
from pathlib import Path

MAX_JSON_BYTES = 1024 * 1024


def read_json(path):
    if path.is_symlink():
        raise ValueError(f"Refusing symlink: {path.name}")
    with path.open("rb") as source:
        data = source.read(MAX_JSON_BYTES + 1)
    if len(data) > MAX_JSON_BYTES:
        raise ValueError(f"JSON exceeds 1 MiB: {path.name}")
    value = json.loads(data)
    if not isinstance(value, dict):
        raise ValueError(f"Expected JSON object: {path.name}")
    return value


def inventory(config):
    native = read_json(config).get("native")
    if not isinstance(native, dict):
        raise ValueError("Native hosting is not configured")
    state = Path(native["state_root"])
    root = Path(native["instance_root"])
    if not state.is_absolute() or not root.is_absolute():
        raise ValueError("Native registry and instance paths must be absolute")
    servers = []
    # Fail closed on unreadable or malformed definitions, rather than suggesting
    # that a partial inventory is complete or choosing a different server.
    for path in sorted(state.iterdir()):
        if path.suffix != ".json":
            continue
        item = read_json(path)
        instance_id = str(uuid.UUID(item["id"]))
        if (item["id"] != instance_id or path.stem != instance_id
                or item["schema_version"] != 1
                or item["container_name"] != f"helix-game-{instance_id}"):
            raise ValueError(f"Invalid server identity: {path.name}")
        name, slug = item["name"], item["instance_name"]
        if not isinstance(name, str) or not isinstance(slug, str) or not name or not slug:
            raise ValueError(f"Missing server name: {path.name}")
        data = root / instance_id
        kind = item.get("kind", "minecraft")
        # Explicit allowlist: manifests also contain RCON credentials and URLs.
        servers.append({
            "id": instance_id, "name": name, "instance_name": slug,
            "game": kind, "software": item["software"],
            "version": item["minecraft_version"], "build": item["build"],
            "container": item["container_name"], "game_port": item["game_port"],
            "data_path": str(data), "data_exists": data.is_dir(),
            "manifest_path": str(path),
            "plugins_path": str(data / "plugins") if kind == "minecraft" else None,
            "mods_path": str(data / "mods") if kind == "minecraft" else None,
        })
        if len(servers) > 512:
            raise ValueError("Registry exceeds the supported 512 servers")
    return servers


def select(servers, instance_id=None, name=None):
    if instance_id is None and name is None:
        return servers
    matches = [server for server in servers if (
        server["id"] == instance_id if instance_id is not None
        else name in (server["name"], server["instance_name"]))]
    if len(matches) != 1:
        raise ValueError("Expected exactly one exact match; list servers and use the full --id")
    return matches


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=Path("/etc/helix/privd.json"))
    selector = parser.add_mutually_exclusive_group()
    selector.add_argument("--id", help="Full, exact instance UUID (no prefix matching)")
    selector.add_argument("--name", help="Exact display name or instance name; duplicates fail")
    parser.add_argument("--json", action="store_true", help="Versioned JSON for scripts")
    args = parser.parse_args()
    try:
        servers = select(inventory(args.config), args.id, args.name)
    except PermissionError:
        print("Registry access denied. Run with authorized sudo; do not relax registry permissions.", file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        # Never print raw JSON decoder messages, which may contain secret text.
        message = "Invalid registry JSON" if isinstance(error, json.JSONDecodeError) else str(error)
        print(f"Discovery failed: {message}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps({"schema_version": 1, "servers": servers}, indent=2))
    else:
        for server in servers:
            print(f"{json.dumps(server['name'], ensure_ascii=True)}  {server['id']}")
            print(f"  {server['game']} / {server['software']} / {server['version']}  port {server['game_port']}")
            print(f"  container: {server['container']}")
            print(f"  data: {json.dumps(server['data_path'])}")
        if not servers:
            print("No registered native servers.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
