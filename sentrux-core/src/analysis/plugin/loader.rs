//! Runtime plugin loader — discovers and loads language plugins from ~/.sentrux/plugins/.
//!
//! Each plugin directory contains:
//! - plugin.toml (manifest)
//! - grammars/<platform>.so|.dylib (compiled tree-sitter grammar)
//! - queries/tags.scm (tree-sitter queries)
//!
//! Loaded grammars are registered into the global LangRegistry alongside built-in languages.
//! Plugin languages take priority over built-in (allows user overrides).

use super::manifest::PluginManifest;
use super::profile::LanguageProfile;
use sha2::Digest;
use std::path::{Path, PathBuf};
use tree_sitter::Language;

/// Result of loading a single plugin.
#[derive(Debug)]
pub struct LoadedPlugin {
    /// Plugin name from manifest
    pub name: String,
    /// Display name
    pub display_name: String,
    /// Version
    pub version: String,
    /// File extensions
    pub extensions: Vec<String>,
    /// Loaded tree-sitter grammar
    pub grammar: Language,
    /// Compiled tree-sitter query source
    pub query_src: String,
    /// Layer 2: language profile (semantics + thresholds)
    pub profile: LanguageProfile,
}

/// Error loading a plugin (non-fatal — logged and skipped).
#[derive(Debug)]
pub struct PluginLoadError {
    pub plugin_dir: PathBuf,
    pub error: String,
}

/// Get the user's plugins directory path (~/.sentrux/plugins/).
pub fn plugins_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".sentrux").join("plugins"))
}

/// Get the bundled plugins directory (next to the executable).
/// Used for distribution archives where grammars ship alongside the binary.
pub fn bundled_plugins_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("plugins")))
        .filter(|d| d.is_dir())
}

/// Discover and load all plugins from BOTH directories:
///   1. Bundled: <exe_dir>/plugins/ (grammars shipped with distribution)
///   2. User:   ~/.sentrux/plugins/ (configs from embedded sync + user plugins)
///
/// For each language, the grammar .dylib is searched in both locations.
/// The user dir's plugin.toml/tags.scm takes priority (embedded sync keeps them current).
pub fn load_all_plugins() -> (Vec<LoadedPlugin>, Vec<PluginLoadError>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();

    let dir = match plugins_dir() {
        Some(d) if d.is_dir() => d,
        _ => return (loaded, errors),
    };

    // If bundled plugins exist, copy any missing grammars to user dir
    // This handles: fresh install from distribution archive
    if let Some(bundled) = bundled_plugins_dir() {
        copy_bundled_grammars(&bundled, &dir);
    }

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            crate::debug_log!("[plugin] Failed to read plugins dir: {}", e);
            return (loaded, errors);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_single_plugin(&path) {
            Ok(plugin) => {
                // Verbose per-plugin logging removed — registry logs the total count
                loaded.push(plugin);
            }
            Err(e) => {
                crate::debug_log!("[plugin] Failed to load {}: {}", path.display(), e);
                errors.push(PluginLoadError {
                    plugin_dir: path,
                    error: e,
                });
            }
        }
    }

    (loaded, errors)
}

fn parse_version_component(s: &str) -> Result<u64, String> {
    s.parse::<u64>()
        .map_err(|_| format!("invalid version component '{}'", s))
}

/// Parse a dotted version string (e.g. "0.5.7" or "1.0.0-rc1").
/// Build metadata (`+linux`) is stripped and malformed core components are
/// rejected instead of silently coerced to zero.
fn parse_version(s: &str) -> Result<(u64, u64, u64, &str), String> {
    let s = s.split_once('+').map(|(v, _)| v).unwrap_or(s);
    let (core, pre) = s.split_once('-').unwrap_or((s, ""));
    let mut nums = core.split('.');
    let major = parse_version_component(nums.next().unwrap_or("0"))?;
    let minor = parse_version_component(nums.next().unwrap_or("0"))?;
    let patch = parse_version_component(nums.next().unwrap_or("0"))?;
    if nums.next().is_some() {
        return Err(format!(
            "invalid version '{}': too many numeric components",
            s
        ));
    }
    Ok((major, minor, patch, pre))
}

/// Pre-release identifier, either numeric or alphanumeric.
#[derive(Debug, PartialEq)]
enum PrId {
    Num(u64),
    Str(String),
}

/// Parse a pre-release suffix into semver-like identifiers.
fn parse_pre(s: &str) -> Vec<PrId> {
    s.split('.').flat_map(parse_pre_segment).collect()
}

fn parse_pre_segment(seg: &str) -> Vec<PrId> {
    if seg.is_empty() {
        return Vec::new();
    }
    if seg.chars().all(|c| c.is_ascii_digit()) {
        return vec![PrId::Num(seg.parse().unwrap_or(u64::MAX))];
    }
    if let Some(i) = seg.find(|c: char| c.is_ascii_digit()) {
        let prefix = &seg[..i];
        let rest = &seg[i..];
        if !prefix.is_empty()
            && prefix.chars().all(|c| c.is_alphabetic() || c == '-')
            && rest.chars().all(|c| c.is_ascii_digit())
        {
            let num = rest.parse().unwrap_or(u64::MAX);
            return vec![PrId::Str(prefix.to_string()), PrId::Num(num)];
        }
    }
    vec![PrId::Str(seg.to_string())]
}

fn cmp_pre(current: &str, min: &str) -> std::cmp::Ordering {
    let mut c = parse_pre(current).into_iter();
    let mut m = parse_pre(min).into_iter();
    loop {
        match (c.next(), m.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(a), Some(b)) => match (a, b) {
                (PrId::Num(_), PrId::Str(_)) => return std::cmp::Ordering::Less,
                (PrId::Str(_), PrId::Num(_)) => return std::cmp::Ordering::Greater,
                (PrId::Num(x), PrId::Num(y)) => match x.cmp(&y) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                },
                (PrId::Str(x), PrId::Str(y)) => match x.cmp(&y) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                },
            },
        }
    }
}

/// Compare two dotted version strings.
///
/// Semver-style rules are used: a release is newer than any pre-release of the
/// same core; pre-releases are compared identifier by identifier, with numeric
/// identifiers sorting before alphanumeric ones and shorter prefixes sorting
/// before longer ones (`alpha` < `alpha.1`).
fn version_at_least(current: &str, min: &str) -> Result<bool, String> {
    let (c_maj, c_min, c_pat, c_pre) = parse_version(current)?;
    let (m_maj, m_min, m_pat, m_pre) = parse_version(min)?;
    let ordering = (c_maj, c_min, c_pat).cmp(&(m_maj, m_min, m_pat));
    let result = match ordering {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            if m_pre.is_empty() {
                c_pre.is_empty()
            } else if c_pre.is_empty() {
                true
            } else {
                matches!(
                    cmp_pre(c_pre, m_pre),
                    std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                )
            }
        }
    };
    Ok(result)
}

/// Load a single plugin from a directory.
fn load_single_plugin(plugin_dir: &Path) -> Result<LoadedPlugin, String> {
    // 1. Parse manifest
    let manifest = PluginManifest::load(plugin_dir)?;

    // 1a. Validate minimum sentrux version (using sentrux-core's package version,
    // which is kept in lock-step with the sentrux binary).
    if let Some(min) = &manifest.plugin.min_sentrux_version {
        let current = env!("CARGO_PKG_VERSION");
        match version_at_least(current, min) {
            Ok(true) => {}
            Ok(false) => {
                return Err(format!(
                    "Plugin '{}' requires sentrux >= {}, but this build is {}",
                    manifest.plugin.name, min, current
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Plugin '{}' has invalid min_sentrux_version '{}': {}",
                    manifest.plugin.name, min, e
                ));
            }
        }
    }

    // 2. Load query source
    let query_path = plugin_dir.join("queries").join("tags.scm");
    let query_src = std::fs::read_to_string(&query_path)
        .map_err(|e| format!("Failed to read {}: {}", query_path.display(), e))?;

    // 3. Validate query captures match declared capabilities
    manifest.validate_query_captures(&query_src)?;

    // 4. Load grammar binary
    let grammar_file = PluginManifest::grammar_filename();
    if grammar_file == "unsupported" {
        return Err("Unsupported platform for runtime grammar loading".into());
    }
    let grammar_path = plugin_dir.join("grammars").join(grammar_file);
    if !grammar_path.exists() {
        return Err(format!(
            "Grammar binary not found: {}. Build it for this platform.",
            grammar_path.display()
        ));
    }

    // 5. Verify checksum if provided
    verify_checksum(&manifest, &grammar_path, grammar_file)?;

    // 6. Load the grammar via dynamic library
    let symbol_name = manifest
        .grammar
        .symbol_name
        .as_deref()
        .unwrap_or(&manifest.plugin.name);
    let grammar = load_grammar_dynamic(&grammar_path, symbol_name)?;

    // 7. Verify ABI version
    #[allow(deprecated)]
    let abi = grammar.version();
    if abi < manifest.grammar.abi_version as usize {
        return Err(format!(
            "Grammar ABI version {} < required {}",
            abi, manifest.grammar.abi_version
        ));
    }

    // 8. Test-compile the query to catch errors early
    tree_sitter::Query::new(&grammar, &query_src)
        .map_err(|e| format!("Query compilation failed: {:?}", e))?;

    let profile = LanguageProfile {
        name: manifest.plugin.name.clone(),
        semantics: manifest.semantics,
        thresholds: manifest.thresholds,
        color_rgb: manifest.plugin.color_rgb.unwrap_or([80, 85, 90]),
    };

    Ok(LoadedPlugin {
        name: manifest.plugin.name,
        display_name: manifest.plugin.display_name,
        version: manifest.plugin.version,
        extensions: manifest.plugin.extensions,
        grammar,
        query_src,
        profile,
    })
}

/// Verify SHA256 checksum of grammar binary against manifest.
fn verify_checksum(
    manifest: &PluginManifest,
    grammar_path: &Path,
    platform_key: &str,
) -> Result<(), String> {
    // Strip extension to get platform key (e.g., "darwin-arm64.dylib" → "darwin-arm64")
    let key = platform_key
        .rsplit_once('.')
        .map_or(platform_key, |(k, _)| k);
    let expected = match manifest.checksums.get(key) {
        Some(hash) => hash,
        None => return Ok(()), // No checksum in manifest = skip verification
    };

    let bytes = std::fs::read(grammar_path)
        .map_err(|e| format!("Failed to read grammar for checksum: {}", e))?;

    let hash = sha2::Sha256::digest(&bytes);
    let actual = hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "Checksum mismatch for {}: expected {}, got {}",
            grammar_path.display(),
            expected,
            actual
        ))
    }
}

/// Load a tree-sitter Language from a dynamic library (.so/.dylib).
///
/// The library must export a function named `tree_sitter_<name>` that returns
/// a `*const TSLanguage` pointer. This is the standard tree-sitter convention.
///
/// # Safety
///
/// This function performs `dlopen`/equivalent and calls a C ABI symbol exported
/// by the plugin grammar. The caller must ensure:
///
/// 1. The library file is from a trusted source. Plugins are user-installed, so
///    `verify_checksum` should be used before loading to detect tampering.
/// 2. The exported symbol is named `tree_sitter_<lang_name>` and has the
///    tree-sitter C ABI: `extern "C" fn() -> *const TSLanguage` (returned as
///    `tree_sitter::Language` by the `tree_sitter` crate).
/// 3. The returned `Language` must not outlive the loaded library. We leak the
///    `Library` with `std::mem::forget` so it stays mapped for the process
///    lifetime; this is the same approach taken by Helix, Zed, and
///    nvim-treesitter.
fn load_grammar_dynamic(path: &Path, lang_name: &str) -> Result<Language, String> {
    // SAFETY: `Library::new` is unsafe because loading arbitrary shared
    // libraries can execute initializer code. We only load plugin grammar
    // libraries that have passed `verify_checksum` and are expected to export
    // the tree-sitter ABI.
    let lib = unsafe { libloading::Library::new(path) }
        .map_err(|e| format!("Failed to load {}: {}", path.display(), e))?;

    // tree-sitter convention: exported function is `tree_sitter_<lang_name>`.
    // SAFETY: `Library::get` is unsafe because the symbol name/type is not
    // validated by the loader. The type below matches the tree-sitter ABI.
    let func_name = format!("tree_sitter_{}", lang_name);
    let func: libloading::Symbol<unsafe extern "C" fn() -> Language> = unsafe {
        lib.get(func_name.as_bytes()).map_err(|e| {
            format!(
                "Symbol '{}' not found in {}: {}. The grammar must export tree_sitter_{}().",
                func_name,
                path.display(),
                e,
                lang_name
            )
        })?
    };

    // SAFETY: Calling the symbol is unsafe because the grammar library is
    // responsible for returning a valid TSLanguage pointer. We trust the plugin
    // after checksum verification and the tree-sitter ABI contract.
    let language = unsafe { func() };

    // Leak the library to keep it alive for the lifetime of the process.
    // tree-sitter Language holds pointers into the library's memory.
    std::mem::forget(lib);

    Ok(language)
}

/// Copy grammar .dylib files from bundled distribution to user plugins dir.
/// Only copies if the user dir doesn't already have the grammar.
/// This handles: user extracts distribution → first launch → grammars copied.
fn copy_bundled_grammars(bundled_dir: &Path, user_dir: &Path) {
    let grammar_file = PluginManifest::grammar_filename();
    let entries = match std::fs::read_dir(bundled_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bundled_grammar = path.join("grammars").join(grammar_file);
        let user_grammar = user_dir.join(&name).join("grammars").join(grammar_file);
        if bundled_grammar.exists() && !user_grammar.exists() {
            let _ = std::fs::create_dir_all(user_dir.join(&name).join("grammars"));
            if std::fs::copy(&bundled_grammar, &user_grammar).is_ok() {
                crate::debug_log!("[plugin] Copied bundled grammar: {}", name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugins_dir() {
        let dir = plugins_dir();
        assert!(dir.is_some());
        assert!(dir.unwrap().ends_with(".sentrux/plugins"));
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let (loaded, errors) = load_all_plugins();
        // Should not crash even if dir doesn't exist
        let _ = (loaded, errors);
    }

    /// Diagnostic: dump all node types for grammars that fail to load.
    /// Run: cargo test dump_failing_grammar_nodes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dump_failing_grammar_nodes() {
        let dir = plugins_dir().unwrap();
        // Only dump languages that are NOT currently loaded (to avoid test pollution)
        let failing: [&str; 0] = [];
        for name in &failing {
            let plugin_dir = dir.join(name);
            let grammar_file = PluginManifest::grammar_filename();
            let grammar_path = plugin_dir.join("grammars").join(grammar_file);
            if !grammar_path.exists() {
                println!("\nSKIP {} — no grammar", name);
                continue;
            }
            // Try loading with the plugin name, then with symbol_name from toml
            let symbol = if let Ok(manifest) = PluginManifest::load(&plugin_dir) {
                manifest.grammar.symbol_name.unwrap_or(name.to_string())
            } else {
                name.to_string()
            };
            match load_grammar_dynamic(&grammar_path, &symbol) {
                Ok(lang) => {
                    println!(
                        "\n=== {} ({} node types, symbol: tree_sitter_{}) ===",
                        name,
                        lang.node_kind_count(),
                        symbol
                    );
                    for id in 0..lang.node_kind_count() as u16 {
                        if lang.node_kind_is_named(id) {
                            let kind = lang.node_kind_for_id(id).unwrap_or("?");
                            // Also check fields
                            println!("  {}", kind);
                        }
                    }
                    // Dump field names
                    println!("  --- fields ---");
                    for id in 0..lang.field_count() as u16 {
                        if let Some(fname) = lang.field_name_for_id(id) {
                            println!("  field: {}", fname);
                        }
                    }
                }
                Err(e) => println!("\nFAIL {}: {}", name, e),
            }
        }
    }

    #[test]
    fn test_version_at_least_release_and_prerelease() {
        assert!(version_at_least("0.5.7", "0.5.6").unwrap());
        assert!(version_at_least("0.5.7", "0.5.7").unwrap());
        assert!(!version_at_least("0.5.6", "0.5.7").unwrap());
        // Release newer than pre-release of same core.
        assert!(version_at_least("0.5.7", "0.5.7-rc1").unwrap());
        assert!(!version_at_least("0.5.7-rc1", "0.5.7").unwrap());
        // Numeric pre-release ordering.
        assert!(version_at_least("0.5.7-rc10", "0.5.7-rc2").unwrap());
        assert!(!version_at_least("0.5.7-rc2", "0.5.7-rc10").unwrap());
        // Dotted pre-release ordering.
        assert!(version_at_least("1.0.0-alpha.2", "1.0.0-alpha.1").unwrap());
        assert!(version_at_least("1.0.0-alpha.1", "1.0.0-alpha").unwrap());
        assert!(!version_at_least("1.0.0-alpha", "1.0.0-alpha.1").unwrap());
        // Build metadata ignored in comparison, core parsed correctly.
        assert!(!version_at_least("0.5.7", "0.5.8+linux").unwrap());
        assert!(version_at_least("0.5.8+linux", "0.5.7").unwrap());
        // Malformed numeric core components are rejected.
        assert!(version_at_least("0.5.7", "0.5.x").is_err());
    }
}
