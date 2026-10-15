"""Shared paths, app maps, and per-platform scenario constants."""
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OUT = ROOT / "bench-results"
STATE = Path.home() / ".agent-mobile" / "state.json"
REPORT_TEMPLATE = ROOT / "scripts" / "bench.html"
SETTINGS = "com.apple.Preferences"
APPS_IOS = {"Settings": SETTINGS, "Calendar": "com.apple.mobilecal", "Contacts": "com.apple.MobileAddressBook",
            "Photos": "com.apple.mobileslideshow", "Maps": "com.apple.Maps"}
APPS_ANDROID = {"Settings": "com.android.settings", "Clock": "com.google.android.deskclock",
                "Maps": "com.google.android.apps.maps", "Phone": "com.google.android.dialer",
                "Files": "com.google.android.documentsui"}
APPS = {"ios": APPS_IOS, "android": APPS_ANDROID}
TEXT = "the quick brown fox jumps over the lazy dog " * 5

# Per-platform scenario constants: (role, name) targets in each platform's
# Settings tree. Android names were read off a live api37 snapshot.
# `open` is the tap that reveals the search field (None: already on screen).
CONF = {
    "ios": {"settings": SETTINGS, "tap": ("button", "General"), "gesture": ("text", "Settings"),
            "pinch": "collectionview", "type": ("searchfield", None),
            "drill": [("button", "General"), ("button", "About")],
            "stype": ("searchfield", None, "Bluetooth", "Bluetooth"), "stale": ("button", "General"),
            "open": None, "resnap": False},
    "android": {"settings": "com.android.settings", "tap": ("text", "Apps"),
                "gesture": ("text", "Search Settings"), "pinch": "collectionview",
                "type": ("textfield", None), "drill": [("text", "Apps")],
                "stype": ("textfield", None, "Bluetooth", "Bluetooth"),
                "stale": ("text", "Apps"), "open": ("text", "Search Settings"), "resnap": True},
}
CONF_KEYS = ("settings", "tap", "gesture", "pinch", "type", "drill", "stype", "stale", "open", "resnap")
