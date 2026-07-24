//! Capture plugin selection. Offered as a checkbox list (with an advanced
//! free-text escape hatch); the wise plugin is auto-enabled whenever the wise
//! component is selected. Emitted as `plugins=` (native config.ini) or
//! `ARKIME__plugins=` (docker env).

/// The wise integration plugin, force-enabled when the wise component is on.
pub const WISE_PLUGIN: &str = "wise.so";

/// A small curated set of commonly enabled plugins. Anything else can be typed
/// in advanced mode.
pub const KNOWN_PLUGINS: [&str; 4] = [WISE_PLUGIN, "ja4plus.amd64.so", "entropy.so", "suricata.so"];

/// Merge a `;`-separated plugin list with the wise requirement, de-duplicating
/// while preserving order. `wise_required` forces `wise.so` to the front if
/// absent.
pub fn finalize(list: &str, wise_required: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut push_unique = |item: &str| {
        let item = item.trim();
        if !item.is_empty() && !out.iter().any(|e| e == item) {
            out.push(item.to_string());
        }
    };

    if wise_required {
        push_unique(WISE_PLUGIN);
    }
    for item in list.split(';') {
        push_unique(item);
    }
    out.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_joins_and_trims() {
        assert_eq!(
            finalize("ja4plus.amd64.so ; entropy.so", false),
            "ja4plus.amd64.so;entropy.so"
        );
    }

    #[test]
    fn finalize_forces_wise_when_required() {
        assert_eq!(finalize("entropy.so", true), "wise.so;entropy.so");
    }

    #[test]
    fn finalize_dedupes_wise() {
        assert_eq!(finalize("wise.so;entropy.so", true), "wise.so;entropy.so");
    }

    #[test]
    fn finalize_empty_stays_empty() {
        assert_eq!(finalize("", false), "");
    }
}
