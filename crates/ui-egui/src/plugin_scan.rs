use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

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
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
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
    serde_json::from_slice::<Result<Vec<T>, String>>(&bytes).map_err(|error| error.to_string())?
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
