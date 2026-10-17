"""Declarative agent tasks: seed variants, device reset, and host-side checks.

A reset is a list of host commands and a check is one host command whose stdout
must equal `expected`. Both are formatted with the seed values plus `{device}`.
Android commands run through `adb shell`; iOS commands use `xcrun simctl`.
"""


def adb(script):
    return ["adb", "shell", script]


CONTACTS = ("C=content://com.android.contacts; "
            "content delete --uri \"$C/raw_contacts?caller_is_syncadapter=true\" --where \"display_name LIKE 'Bench %'\"; "
            "for n in 'Bench Seed One' 'Bench Seed Two'; do "
            "content insert --uri $C/raw_contacts --bind account_type:n: --bind account_name:n:; "
            "id=$(content query --uri $C/raw_contacts --projection _id --sort '_id DESC' | head -1 | sed 's/.*_id=//'); "
            "content insert --uri $C/data --bind raw_contact_id:i:$id --bind mimetype:s:vnd.android.cursor.item/name --bind data1:s:\"$n\"; "
            "content insert --uri $C/data --bind raw_contact_id:i:$id --bind mimetype:s:vnd.android.cursor.item/phone_v2 --bind data1:s:5550100 --bind data2:i:2; "
            "done; am force-stop com.google.android.contacts; input keyevent 3")
CONTACT_FIELD = ("content query --uri content://com.android.contacts/data --projection data1 "
                 "--where \"display_name='{name}' AND mimetype='vnd.android.cursor.item/{kind}'\" | sed -n 's/.*data1=//p'")
PHONE_DIGITS = " | tr -cd 0-9 | tail -c 7"
PHONE_FIELD = CONTACT_FIELD + PHONE_DIGITS
CALENDAR = ("K=content://com.android.calendar; "
            "content delete --uri \"$K/events?caller_is_syncadapter=true&account_name=bench&account_type=LOCAL\" --where \"title LIKE 'Bench Meeting%'\"; "
            "content query --uri $K/calendars --projection _id | grep -q Row || "
            "content insert --uri \"$K/calendars?caller_is_syncadapter=true&account_name=bench&account_type=LOCAL\" "
            "--bind account_name:s:bench --bind account_type:s:LOCAL --bind name:s:bench --bind calendar_displayName:s:Bench "
            "--bind calendar_color:i:-16776961 --bind calendar_access_level:i:700 --bind visible:i:1 --bind sync_events:i:1 "
            "--bind ownerAccount:s:bench --bind calendar_timezone:s:America/Los_Angeles; "
            "am force-stop com.google.android.calendar; input keyevent 3")
PHOTO = ("P=/sdcard/Pictures/{photo}; "
         "content delete --uri content://media/external/images/media --where \"_display_name='{photo}'\"; "
         "rm -f $P; screencap -p $P; am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d file://$P >/dev/null; "
         "for i in 1 2 3 4 5 6 7 8 9 10; do content query --uri content://media/external/images/media "
         "--where \"_display_name='{photo}'\" | grep -q Row && break; sleep 1; done; "
         "am force-stop com.google.android.apps.photos; input keyevent 3")
SETTINGS_HOME = ("settings put system screen_off_timeout 600000; input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; "
                 "am force-stop com.google.android.gms; am force-stop com.google.android.calendar; "
                 "am force-stop com.android.settings; am start -W -n com.android.settings/.Settings >/dev/null; sleep 1")
SETTINGS_PAGE = ("dumpsys activity top | awk '/ ACTIVITY /{{r=0}} /mResumed=true/{{r=1}} "
                 "r && /^ +{fragment}[{{]/{{f=1}} END{{print f ? \"yes\" : \"no\"}}'")

TASKS = [
    {"id": "dark-mode", "goal": "Change the device appearance to {value} mode.",
     "values": [{"value": "dark", "night": "yes", "from": "light", "from_night": "no"},
                {"value": "light", "night": "no", "from": "dark", "from_night": "yes"}],
     "reset": {"android": [adb("cmd uimode night {from_night}")], "ios": [["xcrun", "simctl", "ui", "{device}", "appearance", "{from}"]]},
     "checks": {"android": {"command": adb("cmd uimode night"), "expected": "Night mode: {night}"},
                "ios": {"command": ["xcrun", "simctl", "ui", "{device}", "appearance"], "expected": "{value}"}}},
    {"id": "screen-timeout", "goal": "Set the screen timeout to {value} milliseconds.", "values": [{"value": "60000"}, {"value": "120000"}],
     "reset": {"android": [adb("settings put system screen_off_timeout 30000")]},
     "checks": {"android": {"command": adb("settings get system screen_off_timeout"), "expected": "{value}"}}},
    {"id": "create-contact", "goal": "Create a contact named {name} with phone number {phone}.",
     "values": [{"name": "Bench Ada", "phone": "5550101", "kind": "phone_v2"},
                {"name": "Bench Grace", "phone": "5550102", "kind": "phone_v2"}],
     "reset": {"android": [adb(CONTACTS)]},
     "checks": {"android": {"command": adb(PHONE_FIELD), "expected": "{phone}"}}},
    {"id": "edit-contact", "goal": "Change the phone number for the seeded contact {name} to {phone}.",
     "values": [{"name": "Bench Seed One", "phone": "5550111", "kind": "phone_v2"},
                {"name": "Bench Seed Two", "phone": "5550112", "kind": "phone_v2"}],
     "reset": {"android": [adb(CONTACTS)]},
     "checks": {"android": {"command": adb(PHONE_FIELD), "expected": "{phone}"}}},
    {"id": "calendar-event", "goal": "Create a calendar event titled {title} at {time}.",
     "values": [{"title": "Bench Meeting A", "time": "2030-01-02T10:00"}, {"title": "Bench Meeting B", "time": "2030-01-02T11:00"}],
     "reset": {"android": [adb(CALENDAR)]},
     "checks": {"android": {"command": adb(
         "s=$(content query --uri content://com.android.calendar/events --projection dtstart --where \"title='{title}'\" "
         "| sed -n 's/.*dtstart=//p'); [ -n \"$s\" ] && date -d @${{s%???}} +%Y-%m-%dT%H:%M || echo none"), "expected": "{time}"}}},
    {"id": "alarm", "goal": "Set an alarm for {time}.", "values": [{"time": "07:10"}, {"time": "07:20"}],
     "reset": {"android": [adb("am force-stop com.google.android.deskclock; pm clear com.google.android.deskclock; "
                               "pm grant com.google.android.deskclock android.permission.POST_NOTIFICATIONS; input keyevent 3")]},
     "checks": {"android": {"command": adb(
         "dumpsys alarm | grep -A1 'Next alarm clock' | tail -1 | sed -n 's/.* = [0-9-]* \\([0-9:]*\\)\\.[0-9]* = .*/\\1/p'"),
         "expected": "{time}:00"}}},
    {"id": "contact-notes", "goal": "Add the note {note} to the seeded contact {name}.",
     "values": [{"name": "Bench Seed One", "note": "Bench note alpha", "kind": "note"},
                {"name": "Bench Seed Two", "note": "Bench note beta", "kind": "note"}],
     "reset": {"android": [adb(CONTACTS)]},
     "checks": {"android": {"command": adb(CONTACT_FIELD), "expected": "{note}"}}},
    {"id": "delete-photo", "goal": "Delete the seeded photo named {photo}.", "values": [{"photo": "bench-a.png"}, {"photo": "bench-b.png"}],
     "reset": {"android": [adb(PHOTO)]},
     "checks": {"android": {"command": adb(
         "q=$(content query --uri content://media/external/images/media --where \"_display_name='{photo}' AND is_trashed=0\") || exit $?; "
         "printf '%s\\n' \"$q\" | awk '/Row/ {{n++}} END {{print n+0}}'"), "expected": "0"}}},
    {"id": "deep-settings", "goal": "Open the {page} page in Settings.",
     "values": [{"page": "About", "fragment": "MyDeviceInfoFragment"}, {"page": "Storage", "fragment": "StorageDashboardFragment"}],
     "reset": {"android": [adb(SETTINGS_HOME)]},
     "checks": {"android": {"command": adb(SETTINGS_PAGE), "expected": "yes"}}},
    {"id": "settings-entry", "goal": "Find and open the Settings entry named {entry} below the fold.",
     "values": [{"entry": "Accessibility", "fragment": "AccessibilitySettings"}, {"entry": "Location", "fragment": "LocationSettings"}],
     "reset": {"android": [adb(SETTINGS_HOME)]},
     "checks": {"android": {"command": adb(SETTINGS_PAGE), "expected": "yes"}}},
]


def task_variant(task, seed):
    values = task["values"][seed % len(task["values"])]
    return task["goal"].format(**values), values
