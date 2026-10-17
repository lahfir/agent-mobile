use agent_mobile_core::error::Failure;

const REFUSALS: &[(&[&str], &str, &str)] = &[
    (
        &["maximum number of installed apps using a free developer profile"],
        "the iPhone has no free app slot for this signing profile: a free (personal team) profile allows 3 apps per device",
        "on the iPhone, delete apps signed by your personal team that you no longer need (Settings > General > VPN & Device Management lists the developer), then retry",
    ),
    (
        &["Root install style is not supported"],
        "the iPhone refused to install the driver: this Xcode likely does not support the iPhone's iOS version",
        "update Xcode to a version that supports the iPhone's iOS (compare `xcodebuild -version` with Settings > General > About > iOS Version), then retry; if Xcode is current, free an app slot for your personal team (Settings > General > VPN & Device Management; a free profile allows 3 apps per device)",
    ),
    (
        &[
            "not been explicitly trusted",
            "certificate is not trusted",
            "Developer App Certificate",
        ],
        "the iPhone does not trust the developer certificate",
        "on the iPhone, open Settings > General > VPN & Device Management, trust the developer app, then retry",
    ),
    (
        &[
            "to Continue",
            "com.apple.dt.deviceprep",
            "device is locked",
            "passcode protected",
        ],
        "the iPhone is locked",
        "unlock the iPhone and keep it unlocked until the driver answers, then retry",
    ),
    (
        &[
            "Developer Mode disabled",
            "developer mode is disabled",
            "Developer Mode is disabled",
        ],
        "Developer Mode is off on the iPhone",
        "turn on Settings > Privacy & Security > Developer Mode, restart the iPhone, then retry",
    ),
    (
        &[
            "requires a development team",
            "No Account for Team",
            "No profiles for",
        ],
        "Xcode has no signing team for the driver",
        "sign in under Xcode > Settings > Accounts, open the driver's AgentMobileDriver.xcodeproj, choose your team under Signing & Capabilities for every target, then retry",
    ),
];

pub fn refusal(log_text: &str) -> Option<Failure> {
    REFUSALS
        .iter()
        .find(|(markers, _, _)| markers.iter().any(|marker| log_text.contains(marker)))
        .map(|(_, cause, next)| Failure::local(*cause, *next))
}

#[cfg(test)]
mod tests {
    use super::{REFUSALS, refusal};

    #[test]
    fn every_marker_maps_to_its_cause() {
        for (markers, cause, _) in REFUSALS {
            for marker in *markers {
                assert!(format!("{:?}", refusal(marker)).contains(cause));
            }
        }
    }

    #[test]
    fn earlier_free_profile_entry_wins() {
        assert!(format!(
            "{:?}",
            refusal(
                "maximum number of installed apps using a free developer profile; Root install style is not supported"
            )
        )
        .contains("no free app slot"));
    }

    #[test]
    fn unknown_log_has_no_refusal() {
        assert!(refusal("ordinary build output").is_none());
    }
}
