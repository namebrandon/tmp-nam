//! Tone3000 model variants (architecture × size) and the download allow-list.
//!
//! A1 models come in standard / lite / feather / nano sizes, one size per file. An
//! A2 model is one file holding its sizes (Full and Lite), switched on the unit, and
//! Tone3000 lists it with no size, so A2 is a single variant. The allow-list picks
//! which of these the sync may download; by default A2 plus A1 feather / nano (`t3k_sync.py`'s `ALLOWED_SIZES` for A1).
//! Size labels are a download filter, not proof a capture fits the ~725 µs callback.

use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Variant {
    pub id: &'static str,
    pub arch: u8,
    pub label: &'static str,
    /// Lower = larger / higher-fidelity network; the UI prefers A2, then the lowest rank.
    pub rank: u8,
}

/// The one A2 variant (all sizes in one file).
pub const A2: &str = "a2";

pub const VARIANTS: &[Variant] = &[
    Variant {
        id: A2,
        arch: 2,
        label: "Lite and Full, in one file",
        rank: 0,
    },
    Variant {
        id: "a1-standard",
        arch: 1,
        label: "Standard",
        rank: 0,
    },
    Variant {
        id: "a1-lite",
        arch: 1,
        label: "Lite",
        rank: 1,
    },
    Variant {
        id: "a1-feather",
        arch: 1,
        label: "Feather",
        rank: 2,
    },
    Variant {
        id: "a1-nano",
        arch: 1,
        label: "Nano",
        rank: 3,
    },
];

pub fn default_enabled() -> Vec<String> {
    [A2, "a1-feather", "a1-nano"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Map a Tone3000 model's `architecture_version` + `size` to a variant id.
pub fn classify(arch_version: &str, size: Option<&str>) -> Option<&'static str> {
    let arch = match arch_version.trim() {
        "1" | "1.0" => 1,
        "2" | "2.0" => 2,
        _ => return None,
    };
    let size = size.unwrap_or("").trim().to_ascii_lowercase();
    let id = match (arch, size.as_str()) {
        (1, "standard") => "a1-standard",
        (1, "lite") => "a1-lite",
        (1, "feather") => "a1-feather",
        (1, "nano") => "a1-nano",
        (2, _) => A2,
        _ => return None,
    };
    Some(id)
}

/// Allow-lists saved before A2 was one variant (`a2-full`, `a2-feather`,
/// `a2-nano`) become `a2` when any of them was allowed.
pub fn migrate(ids: Vec<String>) -> Vec<String> {
    let had_a2 = ids.iter().any(|i| i == A2 || i.starts_with("a2-"));
    let mut out: Vec<String> = ids.into_iter().filter(|i| !i.starts_with("a2")).collect();
    if had_a2 {
        out.insert(0, A2.to_string());
    }
    out
}

/// Device-safe IR name — a port of `t3k_sync.py`'s `safe_name` so names stay stable
/// between the on-device script and this app.
pub fn safe_name(title: Option<&str>, model_name: Option<&str>, size: Option<&str>) -> String {
    let base = model_name
        .filter(|s| !s.trim().is_empty())
        .or(title.filter(|s| !s.trim().is_empty()))
        .unwrap_or("t3k")
        .trim();
    let mapped: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut s = mapped.trim_matches(|c| "-._".contains(c)).to_string();
    if s.is_empty() {
        s = "t3k".into();
    }
    if !s.to_lowercase().ends_with(".nam") {
        s.push_str(".nam");
    }
    if let Some(size) = size.filter(|z| !z.is_empty()) {
        if !s.to_lowercase().contains(&size.to_lowercase()) {
            s = format!("{}-{}.nam", &s[..s.len() - 4], size);
        }
    }
    s.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_sizes() {
        assert_eq!(classify("2", None), Some(A2));
        assert_eq!(classify("2.0", Some("feather")), Some(A2));
        assert_eq!(classify("1", Some("Lite")), Some("a1-lite"));
        assert_eq!(classify("1", Some("custom")), None);
        assert_eq!(classify("3", Some("nano")), None);
    }

    #[test]
    fn migrate_folds_old_a2_sizes() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            migrate(v(&["a2-feather", "a2-nano", "a1-feather"])),
            v(&[A2, "a1-feather"])
        );
        assert_eq!(migrate(v(&["a1-nano"])), v(&["a1-nano"]));
        assert_eq!(migrate(v(&[A2, "a1-lite"])), v(&[A2, "a1-lite"]));
    }

    #[test]
    fn safe_name_matches_python() {
        assert_eq!(
            safe_name(Some("tone"), Some("Fender Twin!"), Some("feather")),
            "Fender-Twin.nam".replace(".nam", "-feather.nam")
        );
        assert_eq!(safe_name(Some("JCM 800"), None, None), "JCM-800.nam");
        assert_eq!(
            safe_name(None, Some("amp-nano.nam"), Some("nano")),
            "amp-nano.nam"
        );
        assert_eq!(safe_name(None, Some("!!!"), None), "t3k.nam");
    }
}
