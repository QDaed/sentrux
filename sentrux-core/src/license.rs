//! License tier system with Ed25519 key validation.
//!
//! The `Tier` enum is the universal currency for feature gating.
//! License keys are Ed25519-signed JSON files stored at ~/.sentrux/license.key.
//! Validation is fully offline — no server call needed.
//!
//! ## Key format
//! ```json
//! {
//!   "user": "github:yjing",
//!   "email": "yjing@sentrux.dev",
//!   "tier": "pro",
//!   "issued": "2026-03-18",
//!   "expires": "2026-04-18",
//!   "id": "lic_a1b2c3d4e5f6",
//!   "sig": "base64_ed25519_signature"
//! }
//! ```

use serde::{Deserialize, Serialize};

/// Ed25519 public key for license verification (embedded at compile time).
/// The corresponding private key is kept offline — never in any repository.
const LICENSE_PUBLIC_KEY: [u8; 32] = [
    51, 80, 192, 124, 169, 177, 177, 37, 40, 185, 99, 192, 167, 42, 157, 250, 1, 110, 189, 234,
    236, 9, 143, 61, 221, 122, 243, 48, 251, 237, 154, 119,
];

/// License tier determining feature access.
///
/// Ordered by privilege: Free < Pro < Team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Free = 0,
    Pro = 1,
    Team = 2,
}

impl Tier {
    /// Check if this tier grants access to features requiring `required` tier.
    #[inline]
    pub fn can_access(self, required: Self) -> bool {
        self >= required
    }

    #[inline]
    pub fn is_pro(self) -> bool {
        self >= Self::Pro
    }

    #[inline]
    pub fn is_team(self) -> bool {
        self >= Self::Team
    }

    /// Detail list limit for this tier (used by health, test_gaps, etc.)
    pub const fn detail_limit(self) -> usize {
        match self {
            Self::Free => 0,
            Self::Pro | Self::Team => usize::MAX,
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Free => write!(f, "Free"),
            Self::Pro => write!(f, "Pro"),
            Self::Team => write!(f, "Team"),
        }
    }
}

// ── Global tier state ──

static TIER: std::sync::OnceLock<Tier> = std::sync::OnceLock::new();

/// Set the tier (called at startup after license validation).
pub fn set_tier(tier: Tier) {
    let _ = TIER.set(tier);
}

/// Get the current tier.
pub fn current_tier() -> Tier {
    *TIER.get().unwrap_or(&Tier::Free)
}

// ── License key types ──

/// Parsed license key (before signature verification).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseKey {
    pub user: String,
    pub email: String,
    pub tier: Tier,
    pub issued: String,
    pub expires: String,
    pub id: String,
    pub sig: String,
}

/// Validated license (after signature + expiry checks passed).
#[derive(Debug, Clone)]
pub struct ValidatedLicense {
    pub user: String,
    pub email: String,
    pub tier: Tier,
    pub id: String,
    pub expires: String,
}

// ── License validation ──

/// Validate a license key JSON string.
/// Returns None if signature is invalid, key is expired, or JSON is malformed.
/// Fully offline — no network calls.
pub fn validate_license(key_json: &str) -> Option<ValidatedLicense> {
    // Parse
    let key: LicenseKey = serde_json::from_str(key_json).ok()?;

    // Decode signature
    use base64::Engine;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&key.sig)
        .ok()?;
    if sig_bytes.len() != 64 {
        return None;
    }

    // Build the message that was signed (all fields except sig).
    // Use serde's lowercase serialization for tier (matches signing side).
    let tier_str = serde_json::to_string(&key.tier).unwrap_or_default();
    let tier_str = tier_str.trim_matches('"');
    let message = format!(
        "{}|{}|{}|{}|{}|{}",
        key.user, key.email, tier_str, key.issued, key.expires, key.id,
    );

    // Verify Ed25519 signature
    use ed25519_dalek::{Signature, VerifyingKey};
    let verifying_key = VerifyingKey::from_bytes(&LICENSE_PUBLIC_KEY).ok()?;
    let signature = Signature::from_bytes(sig_bytes.as_slice().try_into().ok()?);
    use ed25519_dalek::Verifier;
    verifying_key.verify(message.as_bytes(), &signature).ok()?;

    // Check expiry
    let today = chrono_today();
    if key.expires < today {
        return None; // expired
    }

    Some(ValidatedLicense {
        user: key.user,
        email: key.email,
        tier: key.tier,
        id: key.id,
        expires: key.expires,
    })
}

/// Get today's date as YYYY-MM-DD string (no chrono dependency — use simple system time).
fn chrono_today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Convert epoch seconds to YYYY-MM-DD
    let days = now / 86400;
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let months_days: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &d) in months_days.iter().enumerate() {
        if remaining < d {
            m = i;
            break;
        }
        remaining -= d;
    }
    format!("{:04}-{:02}-{:02}", y, m + 1, remaining + 1)
}

const fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Search paths for license.key, in priority order.
/// Handles sudo (where ~ becomes /root instead of /home/user).
fn license_search_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();

    // 1. Current user's home (~/.sentrux/license.key)
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".sentrux").join("license.key"));
    }

    // 2. Original user's home when running via sudo
    //    sudo sets $SUDO_USER to the real user who invoked sudo
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        // Linux: /home/<user>, macOS: /Users/<user>
        #[cfg(target_os = "macos")]
        paths.push(std::path::PathBuf::from(format!(
            "/Users/{}/.sentrux/license.key",
            sudo_user
        )));
        #[cfg(not(target_os = "macos"))]
        paths.push(std::path::PathBuf::from(format!(
            "/home/{}/.sentrux/license.key",
            sudo_user
        )));
    }

    // 3. System-wide location (for shared/server installs)
    #[cfg(unix)]
    paths.push(std::path::PathBuf::from("/etc/sentrux/license.key"));

    paths
}

/// Load and validate license from disk.
/// Tries multiple paths: user home, sudo user home, system-wide.
/// Returns the validated tier, or Free if no valid license found.
pub fn load_license_from_disk() -> Tier {
    for path in license_search_paths() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(license) = validate_license(&content) {
                crate::debug_log!(
                    "[license] Valid: {} ({}), expires {} [{}]",
                    license.user,
                    license.tier,
                    license.expires,
                    path.display()
                );
                return license.tier;
            }
            crate::debug_log!("[license] Invalid or expired at {}", path.display());
        }
    }
    Tier::Free
}

// ── Pro plugin loading ──

/// Search paths for pro.dylib, same order as license.key.
fn pro_dylib_search_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();

    let dylib_name = if cfg!(target_os = "macos") {
        "pro.dylib"
    } else if cfg!(target_os = "windows") {
        "pro.dll"
    } else {
        "pro.so"
    };

    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".sentrux").join("pro").join(dylib_name));
    }

    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        #[cfg(target_os = "macos")]
        paths.push(std::path::PathBuf::from(format!(
            "/Users/{}/.sentrux/pro/{}",
            sudo_user, dylib_name
        )));
        #[cfg(not(target_os = "macos"))]
        paths.push(std::path::PathBuf::from(format!(
            "/home/{}/.sentrux/pro/{}",
            sudo_user, dylib_name
        )));
    }

    paths
}

/// Stored Pro plugin library — kept alive for the process lifetime
/// so that function pointers from the dylib remain valid.
static PRO_LIB: std::sync::OnceLock<libloading::Library> = std::sync::OnceLock::new();

/// Try to load the Pro plugin dylib.
/// Returns true if Pro plugin was loaded successfully.
fn try_load_pro_dylib(license: &ValidatedLicense) -> bool {
    for path in pro_dylib_search_paths() {
        if !path.exists() {
            continue;
        }
        crate::debug_log!("[pro] Found pro plugin at {}", path.display());

        // Read watermark from dylib and verify it matches the license
        match read_watermark(&path) {
            Some(watermark_id) if watermark_id == license.id => {
                crate::debug_log!("[pro] Watermark matches license {}", license.id);
            }
            Some(watermark_id) => {
                crate::debug_log!(
                    "[pro] Watermark mismatch: dylib={}, license={}",
                    watermark_id,
                    license.id
                );
                continue; // dylib belongs to a different user
            }
            None => {
                // No watermark found — accept (dev builds, or watermark not yet implemented)
                crate::debug_log!("[pro] No watermark found, accepting dylib");
            }
        }

        // Load the library
        match unsafe { libloading::Library::new(&path) } {
            Ok(lib) => {
                // Call the init function
                let init_result: Result<libloading::Symbol<unsafe extern "C" fn()>, _> =
                    unsafe { lib.get(b"sentrux_pro_init") };
                match init_result {
                    Ok(init_fn) => {
                        unsafe { init_fn() };
                        // Store library so it stays loaded for the process lifetime
                        let _ = PRO_LIB.set(lib);
                        crate::debug_log!("[pro] Plugin loaded and initialized");
                        return true;
                    }
                    Err(e) => {
                        crate::debug_log!("[pro] Failed to find sentrux_pro_init: {}", e);
                    }
                }
            }
            Err(e) => {
                crate::debug_log!("[pro] Failed to load dylib: {}", e);
            }
        }
    }
    false
}

/// Read the 64-byte watermark from a pro dylib.
/// The watermark is a known byte pattern: 16 bytes marker + 32 bytes license_id + 16 bytes HMAC.
/// Returns the license_id string if found.
fn read_watermark(path: &std::path::Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    // Search for the watermark marker: "SENTRUX_WM_V1\0\0\0" (16 bytes)
    let marker = b"SENTRUX_WM_V1\0\0\0";
    let pos = data.windows(marker.len()).position(|w| w == marker)?;
    let id_start = pos + marker.len();
    let id_end = id_start + 32;
    if id_end > data.len() {
        return None;
    }
    // License ID is stored as UTF-8 bytes, null-padded
    let id_bytes = &data[id_start..id_end];
    let id = std::str::from_utf8(id_bytes).ok()?;
    let id = id.trim_end_matches('\0');
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Initialize the license system + Pro plugin. Call once at startup.
///
/// 1. Reads license.key from disk (tries user home, sudo home, system-wide)
/// 2. Validates Ed25519 signature + expiry
/// 3. If valid Pro/Team: tries to load pro.dylib
/// 4. Sets the global tier
pub fn init() {
    // Load and validate license
    let mut tier = Tier::Free;
    let mut license: Option<ValidatedLicense> = None;

    for path in license_search_paths() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(lic) = validate_license(&content) {
                crate::debug_log!(
                    "[license] Valid: {} ({}), expires {} [{}]",
                    lic.user,
                    lic.tier,
                    lic.expires,
                    path.display()
                );
                tier = lic.tier;
                license = Some(lic);
                break;
            }
            crate::debug_log!("[license] Invalid or expired at {}", path.display());
        }
    }

    // Try to load Pro plugin if licensed
    if tier.is_pro() {
        if let Some(ref lic) = license {
            if try_load_pro_dylib(lic) {
                crate::debug_log!("[pro] Pro plugin active");
            } else {
                crate::debug_log!("[pro] No pro plugin found — Pro license active but running with built-in features only");
            }
        }
    }

    set_tier(tier);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering() {
        assert!(Tier::Free < Tier::Pro);
        assert!(Tier::Pro < Tier::Team);
    }

    #[test]
    fn can_access_logic() {
        assert!(Tier::Pro.can_access(Tier::Free));
        assert!(Tier::Pro.can_access(Tier::Pro));
        assert!(!Tier::Pro.can_access(Tier::Team));
        assert!(Tier::Team.can_access(Tier::Team));
        assert!(!Tier::Free.can_access(Tier::Pro));
    }

    #[test]
    fn detail_limits() {
        assert_eq!(Tier::Free.detail_limit(), 0);
        assert_eq!(Tier::Pro.detail_limit(), usize::MAX);
        assert_eq!(Tier::Team.detail_limit(), usize::MAX);
    }

    #[test]
    fn display() {
        assert_eq!(Tier::Free.to_string(), "Free");
        assert_eq!(Tier::Pro.to_string(), "Pro");
        assert_eq!(Tier::Team.to_string(), "Team");
    }

    #[test]
    fn free_tier_default() {
        assert_eq!(current_tier(), Tier::Free);
    }

    #[test]
    fn chrono_today_format() {
        let today = chrono_today();
        // Should be YYYY-MM-DD format
        assert_eq!(today.len(), 10);
        assert_eq!(today.as_bytes()[4], b'-');
        assert_eq!(today.as_bytes()[7], b'-');
    }

    #[test]
    fn invalid_json_returns_none() {
        assert!(validate_license("not json").is_none());
    }

    #[test]
    fn missing_sig_returns_none() {
        let json = r#"{"user":"test","email":"t@t","tier":"pro","issued":"2026-01-01","expires":"2099-01-01","id":"x","sig":"bad"}"#;
        assert!(validate_license(json).is_none());
    }

    #[test]
    fn expired_key_returns_none() {
        // Even with a "valid" signature (which it won't be), expiry should fail
        let json = r#"{"user":"test","email":"t@t","tier":"pro","issued":"2020-01-01","expires":"2020-01-02","id":"x","sig":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
        assert!(validate_license(json).is_none());
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};

        // Use the actual keypair
        let private_bytes = base64::engine::general_purpose::STANDARD
            .decode("LacfYd2UWM+nEEeGQaR7VOYLZrSrXQZkqKU5eapZyTw=")
            .unwrap();
        let signing_key = SigningKey::from_bytes(private_bytes.as_slice().try_into().unwrap());

        let user = "github:test_user";
        let email = "test@sentrux.dev";
        let tier = "pro";
        let issued = "2026-03-18";
        let expires = "2099-12-31"; // far future
        let id = "lic_test_roundtrip";

        let message = format!("{}|{}|{}|{}|{}|{}", user, email, tier, issued, expires, id);
        let signature = signing_key.sign(message.as_bytes());
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        let key_json = serde_json::json!({
            "user": user,
            "email": email,
            "tier": tier,
            "issued": issued,
            "expires": expires,
            "id": id,
            "sig": sig_b64,
        })
        .to_string();

        let result = validate_license(&key_json);
        assert!(result.is_some(), "Roundtrip sign+verify failed");
        let license = result.unwrap();
        assert_eq!(license.user, user);
        assert_eq!(license.tier, Tier::Pro);
        assert_eq!(license.id, id);
    }
}
