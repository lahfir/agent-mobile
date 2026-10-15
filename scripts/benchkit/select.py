"""Device selection and benchmark-target resolution: pure functions over
`devices` rows, no subprocesses except the shared `sh` helper's callers."""
import subprocess


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()


def select_device(devices, selector):
    exact_key = [d for d in devices if d.get("key") == selector]
    if len(exact_key) == 1:
        return exact_key[0]
    stable = [d for d in devices if selector in (d.get("id"), d.get("udid"))]
    if len(stable) == 1:
        return stable[0]
    names = [d for d in devices if d.get("name") == selector]
    return names[0] if len(names) == 1 else None


def canonical_bench_selection(selected, selector):
    """(device_key, pinned): canonical `key` when the row carries one,
    else the raw selector so a remembered display name still resolves."""
    key = selected.get("key")
    return (key, True) if key else (selector, selector is not None)


def android_avd_name(selected):
    """AVD name for an Android row, or None when only a bare serial is known
    (the cold boot then resolves it live via `emu avd name`)."""
    for field in ("id", "udid"):
        value = (selected or {}).get(field) or ""
        if value.startswith("avd:"):
            return value[len("avd:"):]
    return None


def bench_target(selected, selector):
    """(platform, match, boot) for a selected row.

    `match` re-selects the row for metadata; `boot` is the cold-boot handle
    (simulator UDID, or AVD name / None for Android). A legacy row with no
    platform stays iOS and derives its UDID from `udid`.
    """
    if not selected:
        raise SystemExit(f"no device matching {selector!r}; pass --device <name> (see `agent-mobile devices`)")
    platform = selected.get("platform") or "ios"
    if platform == "ios":
        udid = selected.get("id") or selected.get("udid")
        if not udid:
            raise SystemExit(f"no simulator UDID for {selector!r}; pass --device <name> (see `agent-mobile devices`)")
        return ("ios", udid, udid)
    if platform == "android":
        return ("android", selected.get("key") or selector, android_avd_name(selected))
    raise SystemExit(f"benchmark device {selector!r} is {platform}; ios and android only")
