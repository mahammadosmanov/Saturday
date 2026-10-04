use crate::owner_authorization::cli::{parse_native_mode, NativeMode};
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

const OUTPUT_NAME: &str = "native-main-thread.json";
const SAMPLE_INTERVAL: Duration = Duration::from_millis(10);
const SAMPLE_DURATION: Duration = Duration::from_secs(5);
const WARMUP_DURATION: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum MainThreadProbeError {
    #[error("main-thread probe arguments are invalid")]
    InvalidArguments,
    #[error("main-thread probe output path is unsafe")]
    UnsafeOutput,
    #[error("main-thread probe scheduling failed")]
    Scheduling,
    #[error("main-thread probe output failed")]
    Io(#[from] std::io::Error),
}

pub fn parse_probe_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Option<PathBuf>, MainThreadProbeError> {
    match parse_native_mode(arguments).map_err(|_| MainThreadProbeError::InvalidArguments)? {
        NativeMode::Ordinary => Ok(None),
        NativeMode::MeasureMainThread(output) => Ok(Some(output)),
        _ => Err(MainThreadProbeError::InvalidArguments),
    }
}

fn validate_output(path: &Path) -> Result<(), MainThreadProbeError> {
    if path.file_name() != Some(OsStr::new(OUTPUT_NAME)) || path.exists() || path.is_symlink() {
        return Err(MainThreadProbeError::UnsafeOutput);
    }
    let parent = path.parent().ok_or(MainThreadProbeError::UnsafeOutput)?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| MainThreadProbeError::UnsafeOutput)?;
    if canonical_parent != parent {
        return Err(MainThreadProbeError::UnsafeOutput);
    }
    let metadata = fs::symlink_metadata(parent).map_err(|_| MainThreadProbeError::UnsafeOutput)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(MainThreadProbeError::UnsafeOutput);
    }
    Ok(())
}

fn write_result(
    output_path: &Path,
    maximum_stall_us: u64,
    samples: u64,
    planned_samples: u64,
) -> Result<(), MainThreadProbeError> {
    validate_output(output_path)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output_path)?;
    writeln!(
        output,
        "{{\"maximum_main_thread_stall_us\":{maximum_stall_us},\"planned_samples\":{planned_samples},\"samples\":{samples}}}"
    )?;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

pub fn start_probe(
    app_handle: tauri::AppHandle,
    output: PathBuf,
) -> Result<(), MainThreadProbeError> {
    validate_output(&output)?;
    std::thread::Builder::new()
        .name("jarvis-main-thread-probe".to_owned())
        .spawn(move || {
            std::thread::sleep(WARMUP_DURATION);
            let maximum = Arc::new(AtomicU64::new(0));
            let samples = Arc::new(AtomicU64::new(0));
            let mut planned_samples = 0_u64;
            let deadline = Instant::now() + SAMPLE_DURATION;
            while Instant::now() < deadline {
                let scheduled = Instant::now();
                let maximum_for_callback = Arc::clone(&maximum);
                let samples_for_callback = Arc::clone(&samples);
                if app_handle
                    .run_on_main_thread(move || {
                        let elapsed =
                            scheduled.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
                        maximum_for_callback.fetch_max(elapsed, Ordering::SeqCst);
                        samples_for_callback.fetch_add(1, Ordering::SeqCst);
                    })
                    .is_err()
                {
                    app_handle.exit(78);
                    return;
                }
                planned_samples += 1;
                std::thread::sleep(SAMPLE_INTERVAL);
            }
            let drain_deadline = Instant::now() + Duration::from_secs(2);
            while samples.load(Ordering::SeqCst) < planned_samples
                && Instant::now() < drain_deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            let sample_count = samples.load(Ordering::SeqCst);
            let result = if sample_count == 0 || sample_count != planned_samples {
                Err(MainThreadProbeError::Scheduling)
            } else {
                write_result(
                    &output,
                    maximum.load(Ordering::SeqCst),
                    sample_count,
                    planned_samples,
                )
            };
            app_handle.exit(if result.is_ok() { 0 } else { 78 });
        })
        .map_err(MainThreadProbeError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn private_root() -> PathBuf {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "jarvis-main-thread-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn parses_only_the_exact_diagnostic_argument() {
        let output = PathBuf::from("/private/tmp/native-main-thread.json");
        assert_eq!(
            parse_probe_arguments([
                OsString::from("jarvis"),
                OsString::from("--measure-main-thread"),
                output.clone().into_os_string(),
            ])
            .unwrap(),
            Some(output)
        );
        assert!(parse_probe_arguments([
            OsString::from("jarvis"),
            OsString::from("--measure-main-thread")
        ])
        .is_err());
    }

    #[test]
    fn writes_exclusive_private_integer_only_result() {
        let root = private_root();
        let output = root.join(OUTPUT_NAME);
        write_result(&output, 99, 10, 10).unwrap();
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "{\"maximum_main_thread_stall_us\":99,\"planned_samples\":10,\"samples\":10}\n"
        );
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_result(&output, 1, 1, 1).is_err());
        fs::remove_file(output).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
