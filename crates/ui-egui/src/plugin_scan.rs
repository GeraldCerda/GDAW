use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

#[derive(Clone, Default)]
pub struct ScanProgress {
    pub completed: usize,
    pub current: std::collections::BTreeMap<String, String>,
    pub failures: Vec<(String, String)>,
    pub formats: std::collections::BTreeMap<String, FormatProgress>,
    pub cached: usize,
}

#[derive(Clone, Default)]
pub struct FormatProgress {
    pub total: Option<usize>,
    pub completed: usize,
    pub finished: bool,
}

impl ScanProgress {
    pub fn fraction(&self) -> Option<f32> {
        if self.formats.is_empty() || self.formats.values().any(|format| format.total.is_none()) { return None; }
        let total = self.formats.values().map(|format| format.total.unwrap_or(0)).sum::<usize>();
        if total == 0 { return Some(if self.formats.values().all(|format| format.finished) { 1.0 } else { 0.0 }); }
        Some((self.completed as f32 / total as f32).clamp(0.0, 1.0))
    }
}

pub fn report_format(format: &str, total: usize, completed: usize, current: Option<&str>) {
    let mut state = progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = state.formats.entry(format.into()).or_default();
    entry.total = current.map(|_| total);
    entry.completed = completed.min(total);
    state.completed = state.formats.values().map(|format| format.completed).sum();
    match current {
        Some(path) if !path.is_empty() => { state.current.insert(format.into(), path.into()); }
        _ => { state.current.remove(format); }
    }
}

pub fn finish_format(format: &str) {
    let mut state = progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = state.formats.entry(format.into()).or_default();
    entry.finished = true;
    if entry.total.is_none() { entry.total = Some(entry.completed); }
    state.current.remove(format);
}

pub fn format_key(format: crate::plugin_browser::PluginFormat) -> &'static str {
    match format {
        crate::plugin_browser::PluginFormat::Au => "--gdaw-scan-au",
        crate::plugin_browser::PluginFormat::Vst3 => "--gdaw-scan-vst3",
        crate::plugin_browser::PluginFormat::Clap => "--gdaw-scan-clap",
        _ => "built-in",
    }
}

static FORCE_SCAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn progress_state() -> &'static std::sync::Mutex<ScanProgress> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<ScanProgress>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| std::sync::Mutex::new(ScanProgress::default()))
}

pub fn progress() -> ScanProgress {
    progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

pub fn begin(rescan: bool) {
    FORCE_SCAN.store(rescan, std::sync::atomic::Ordering::Release);
    let mut state = ScanProgress::default();
    for key in ["--gdaw-scan-au", "--gdaw-scan-vst3", "--gdaw-scan-clap"] { state.formats.insert(key.into(), FormatProgress::default()); }
    *progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = state;
}

pub fn cache_dir() -> Option<PathBuf> {
    let root = match std::env::var_os("SOUNDCRAFT_CONFIG_DIR") {
        Some(value) if value.is_empty() => return None,
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(std::env::var_os("HOME")?).join(if cfg!(target_os = "macos") { "Library/Application Support/GDAW" } else { ".cache/gdaw" }),
    };
    Some(root.join("PluginCache"))
}

fn cache_path(path: &Path, format: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    format.hash(&mut hash);
    std::env::consts::ARCH.hash(&mut hash);
    std::env::consts::OS.hash(&mut hash);
    for candidate in [Some(path.to_path_buf()), Some(path.join("Contents/Info.plist")), soundcraft_vst3_host::scan::bundle_binary(path), soundcraft_clap_host::scan::bundle_executable(path), std::env::current_exe().ok()] .into_iter().flatten() {
        if let Ok(metadata) = std::fs::metadata(candidate) {
            metadata.len().hash(&mut hash);
            metadata.modified().ok().hash(&mut hash);
        }
    }
    Some(cache_dir()?.join(format!("{:016x}.json", hash.finish())))
}

pub fn run_helper(args: &[String]) -> Option<i32> {
    let format = args.first()?.as_str();
    if !matches!(format, "--gdaw-scan-vst3" | "--gdaw-scan-clap") {
        return None;
    }
    let (Some(path), Some(output)) = (args.get(1), args.get(2)) else { return Some(2) };
    let path = Path::new(path);
    let response = match format {
        "--gdaw-scan-vst3" => serde_json::to_vec(&soundcraft_vst3_host::load_bundle(path).map_err(|error| error.to_string())),
        _ => serde_json::to_vec(&soundcraft_clap_host::load_bundle(path).map_err(|error| error.to_string())),
    };
    Some(match response {
        Ok(bytes) if bytes.len() <= 4_194_304 => {
            if std::fs::write(output, bytes).is_ok() { 0 } else { 1 }
        }
        _ => 1,
    })
}

pub fn install() {
    soundcraft_vst3_host::set_scan_progress(|total, completed, current| report_format("--gdaw-scan-vst3", total, completed, current));
    soundcraft_clap_host::set_scan_progress(|total, completed, current| report_format("--gdaw-scan-clap", total, completed, current));
    soundcraft_au_host::set_scan_progress(|total, completed, current| report_format("--gdaw-scan-au", total, completed, current));
    soundcraft_vst3_host::set_bundle_scanner(|path| {
        isolated(path, "--gdaw-scan-vst3").map_err(|error| soundcraft_vst3_host::Vst3Error::Load(path.display().to_string(), error))
    });
    soundcraft_clap_host::set_bundle_scanner(|path| {
        isolated(path, "--gdaw-scan-clap").map_err(|error| soundcraft_clap_host::ClapError::Load(path.display().to_string(), error))
    });
}

struct ScanDirectory(PathBuf);

impl Drop for ScanDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scan_directory() -> Result<ScanDirectory, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
    let root = std::env::temp_dir();
    for _ in 0..8 {
        let path = root.join(format!("gdaw-scan-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(ScanDirectory(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("cannot create a plugin scan directory".into())
}

fn isolated<T: DeserializeOwned>(path: &Path, format: &str) -> Result<Vec<T>, String> {
    {
        let mut state = progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.current.insert(format.to_string(), path.display().to_string());
    }
    let result: Result<Vec<T>, String> = isolated_inner(path, format);
    if let Err(error) = &result {
        if let Some(cache) = cache_path(path, format) {
            if let Some(parent) = cache.parent() {
                if std::fs::create_dir_all(parent).is_ok() {
                    if let Ok(bytes) = serde_json::to_vec(&Result::<Vec<serde_json::Value>, String>::Err(error.clone())) {
                        let _ = std::fs::write(cache, bytes);
                    }
                }
            }
        }
    }
    let mut state = progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.current.remove(format);
    if let Err(error) = &result {
        if state.failures.len() < 4096 { state.failures.push((path.display().to_string(), error.clone())); }
    }
    result
}

fn isolated_inner<T: DeserializeOwned>(path: &Path, format: &str) -> Result<Vec<T>, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let cache = cache_path(path, format);
    if !FORCE_SCAN.load(std::sync::atomic::Ordering::Acquire) {
        if let Some(file) = cache.as_ref().and_then(|path| std::fs::File::open(path).ok()) {
            let mut bytes = Vec::new();
            if file.take(4_194_305).read_to_end(&mut bytes).is_ok() && bytes.len() <= 4_194_304 {
                if let Ok(result) = serde_json::from_slice::<Result<Vec<T>, String>>(&bytes) {
                    let mut state = progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.cached = state.cached.saturating_add(1);
                    return result;
                }
            }
        }
    }
    let directory = scan_directory()?;
    let output = directory.0.join("descriptor.json");
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable).args([format]).arg(path).arg(&output)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("isolated plugin scanner exited with {status}; skipped"));
                }
                break;
            }
            Ok(None) if started.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("isolated plugin scanner timed out; skipped".into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
    let file = std::fs::File::open(output).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(4_194_305).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > 4_194_304 {
        return Err("plugin scan metadata is too large".into());
    }
    let result = serde_json::from_slice::<Result<Vec<T>, String>>(&bytes).map_err(|error| error.to_string())?;
    if let Some(cache) = cache {
        if let Some(parent) = cache.parent() {
            if std::fs::create_dir_all(parent).is_ok() {
                let staged = cache.with_extension(format!("{}.partial", std::process::id()));
                if std::fs::write(&staged, &bytes).is_ok() { let _ = std::fs::rename(staged, cache); }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_arguments_never_open_an_editor_or_scan_without_a_response_path() {
        assert_eq!(run_helper(&[]), None);
        assert_eq!(run_helper(&["--demo".into()]), None);
        assert_eq!(run_helper(&["--gdaw-scan-vst3".into()]), Some(2));
        assert_eq!(run_helper(&["--gdaw-scan-clap".into(), "plugin.clap".into()]), Some(2));
    }

    #[test]
    fn scan_directories_are_unique_and_removed_after_use() {
        let a = scan_directory().unwrap();
        let b = scan_directory().unwrap();
        let path = a.0.clone();
        assert_ne!(a.0, b.0);
        assert!(path.is_dir());
        drop(a);
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;

    #[test]
    fn unloadable_bundles_report_errors_as_metadata_without_starting_the_daw() {
        let directory = scan_directory().unwrap();
        for format in ["--gdaw-scan-vst3", "--gdaw-scan-clap"] {
            let output = directory.0.join("result.json");
            let absent = directory.0.join("missing-plugin");
            let args = vec![format.into(), absent.to_string_lossy().into_owned(), output.to_string_lossy().into_owned()];
            assert_eq!(run_helper(&args), Some(0));
            let response: Result<Vec<serde_json::Value>, String> = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
            assert!(response.is_err());
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CatalogCache {
    version: u32,
    platform: String,
    inventory: u64,
    plugins: Vec<crate::plugin_browser::BrowserPlugin>,
    vst3: Vec<soundcraft_vst3_host::Vst3Descriptor>,
    clap: Vec<soundcraft_clap_host::ClapDescriptor>,
    au: Vec<soundcraft_au_host::AuDescriptor>,
    #[serde(default)]
    failures: Vec<(String, String)>,
}

fn platform() -> String { format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH) }

fn inventory() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut paths = soundcraft_vst3_host::scan::find_bundles(&soundcraft_vst3_host::scan::default_search_paths());
    paths.extend(soundcraft_clap_host::scan::find_bundles(&soundcraft_clap_host::scan::default_search_paths()));
    let mut roots = vec![PathBuf::from("/Library/Audio/Plug-Ins/Components"), PathBuf::from("/System/Library/Components"), PathBuf::from("/System/Library/Audio/Plug-Ins/Components")];
    if let Some(home) = std::env::var_os("HOME") { roots.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/Components")); }
    if cfg!(target_os = "macos") {
        for root in roots {
            if let Ok(entries) = std::fs::read_dir(&root) { paths.extend(entries.flatten().take(16384).map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == "component"))); }
            paths.push(root);
        }
    }
    paths.sort(); paths.dedup();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for path in paths {
        path.hash(&mut hash);
        let mut candidates = vec![path.clone(), path.join("Contents/Info.plist")];
        candidates.extend(soundcraft_vst3_host::scan::bundle_binary(&path));
        candidates.extend(soundcraft_clap_host::scan::bundle_executable(&path));
        if path.extension().is_some_and(|extension| extension == "component") {
            if let Ok(entries) = std::fs::read_dir(path.join("Contents/MacOS")) { candidates.extend(entries.flatten().take(64).map(|entry| entry.path())); }
        }
        candidates.sort();
        for candidate in candidates {
            candidate.hash(&mut hash);
            if let Ok(metadata) = std::fs::metadata(candidate) { metadata.len().hash(&mut hash); metadata.modified().ok().hash(&mut hash); }
        }
    }
    hash.finish()
}

pub fn restore_catalog() -> Option<(Vec<crate::plugin_browser::BrowserPlugin>, bool)> {
    use std::io::Read;
    let file = std::fs::File::open(cache_dir()?.join("catalog-v2.json")).ok()?;
    let mut bytes = Vec::new();
    file.take(16_777_217).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 16_777_216 { return None; }
    let cached: CatalogCache = serde_json::from_slice(&bytes).ok()?;
    if cached.version != 2 || cached.platform != platform() { return None; }
    let unchanged = cached.inventory == inventory();
    progress_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner).failures = cached.failures;
    soundcraft_vst3_host::restore_catalog(cached.vst3);
    soundcraft_clap_host::restore_catalog(cached.clap);
    soundcraft_au_host::restore_catalog(cached.au);
    let mut plugins = crate::plugin_browser::built_in_catalog();
    plugins.extend(cached.plugins.into_iter().filter(|plugin| plugin.format != crate::plugin_browser::PluginFormat::BuiltIn));
    crate::plugin_browser::normalize(&mut plugins);
    Some((plugins, unchanged))
}

pub fn save_catalog(plugins: &[crate::plugin_browser::BrowserPlugin]) {
    let (Some(vst3), Some(clap), Some(au), Some(directory)) = (soundcraft_vst3_host::cached(), soundcraft_clap_host::cached(), soundcraft_au_host::cached(), cache_dir()) else { return };
    let cache = CatalogCache { version: 2, platform: platform(), inventory: inventory(), plugins: plugins.to_vec(), vst3, clap, au, failures: progress().failures };
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        if bytes.len() <= 16_777_216 && std::fs::create_dir_all(&directory).is_ok() {
            let path = directory.join("catalog-v2.json");
            let staged = directory.join(format!("catalog-v2.{}.partial", std::process::id()));
            if std::fs::write(&staged, bytes).is_ok() { let _ = std::fs::rename(staged, path); }
        }
    }
}

pub fn retry_failed_scans() {
    for (path, _) in progress().failures {
        for format in ["--gdaw-scan-vst3", "--gdaw-scan-clap"] {
            if let Some(cache) = cache_path(Path::new(&path), format) { let _ = std::fs::remove_file(cache); }
        }
    }
}

#[cfg(test)]
mod scan_progress_tests {
    use super::*;

    #[test]
    fn unknown_inventories_do_not_publish_a_misleading_fraction() {
        let mut progress = ScanProgress::default();
        progress.formats.insert("VST3".into(), FormatProgress { total: Some(20), completed: 10, finished: false });
        progress.formats.insert("AU".into(), FormatProgress::default());
        progress.completed = 10;
        assert_eq!(progress.fraction(), None);
        progress.formats.insert("AU".into(), FormatProgress { total: Some(20), completed: 0, finished: false });
        assert_eq!(progress.fraction(), Some(0.25));
    }

    #[test]
    fn an_empty_completed_inventory_finishes_without_dividing_by_zero() {
        let mut progress = ScanProgress::default();
        progress.formats.insert("CLAP".into(), FormatProgress { total: Some(0), completed: 0, finished: true });
        assert_eq!(progress.fraction(), Some(1.0));
    }
}
