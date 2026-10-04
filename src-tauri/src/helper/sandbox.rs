use crate::helper::manifest::FrozenCandidate;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

pub const DENY_NETWORK_PROFILE: &str =
    "(version 1)\n(allow default)\n(deny network*)\n(deny process-fork)\n";
#[cfg(feature = "fixture-helper")]
const DESCENDANT_TEST_PROFILE: &str = "(version 1)\n(allow default)\n(deny network*)\n";
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureMode {
    Normal,
    CancelDelay,
    CancelLate,
    CancelIgnore,
    Malformed,
    Crash,
    StderrFlood,
    StderrOverflow,
    StderrContinuous,
    OutputFlood,
    ValidBurst,
    ResponseFlood,
    RssSpike,
    #[cfg(feature = "fixture-helper")]
    ProcessFlood,
    #[cfg(feature = "fixture-helper")]
    ProcessPulse,
    ForkAttempt,
    NetworkAttempt(u16),
    #[cfg(feature = "fixture-helper")]
    Descendant,
}

impl FixtureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::CancelDelay => "cancel-delay",
            Self::CancelLate => "cancel-late",
            Self::CancelIgnore => "cancel-ignore",
            Self::Malformed => "malformed",
            Self::Crash => "crash",
            Self::StderrFlood => "stderr-flood",
            Self::StderrOverflow => "stderr-overflow",
            Self::StderrContinuous => "stderr-continuous",
            Self::OutputFlood => "output-flood",
            Self::ValidBurst => "valid-burst",
            Self::ResponseFlood => "response-flood",
            Self::RssSpike => "rss-spike",
            #[cfg(feature = "fixture-helper")]
            Self::ProcessFlood => "process-flood",
            #[cfg(feature = "fixture-helper")]
            Self::ProcessPulse => "process-pulse",
            Self::ForkAttempt => "fork-attempt",
            Self::NetworkAttempt(_) => "network-attempt",
            #[cfg(feature = "fixture-helper")]
            Self::Descendant => "descendant",
        }
    }
}

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("fixed sandbox executable is unavailable or unsafe")]
    InvalidSandboxExecutable,
    #[error("runtime directory is invalid")]
    InvalidRuntimeDirectory,
    #[error("candidate executable changed after freezing")]
    CandidateChanged,
    #[error("sandbox inspection failed: {0}")]
    Io(#[from] std::io::Error),
}

pub trait LaunchPolicy {
    fn build_command(
        &self,
        candidate: &FrozenCandidate,
        runtime_root: &Path,
        mode: FixtureMode,
    ) -> Result<Command, SandboxError>;
}

#[derive(Debug, Clone)]
pub struct MacOsDenyNetwork {
    program: PathBuf,
}

impl MacOsDenyNetwork {
    pub fn new() -> Result<Self, SandboxError> {
        let path = PathBuf::from(SANDBOX_EXEC);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.uid() != 0 {
            return Err(SandboxError::InvalidSandboxExecutable);
        }
        Ok(Self { program: path })
    }

    pub fn program(&self) -> &Path {
        &self.program
    }
}

impl LaunchPolicy for MacOsDenyNetwork {
    fn build_command(
        &self,
        candidate: &FrozenCandidate,
        runtime_root: &Path,
        mode: FixtureMode,
    ) -> Result<Command, SandboxError> {
        candidate
            .revalidate_executable()
            .map_err(|_| SandboxError::CandidateChanged)?;
        let metadata = fs::symlink_metadata(runtime_root)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(SandboxError::InvalidRuntimeDirectory);
        }
        let runtime_root = runtime_root.canonicalize()?;
        let home = runtime_root.join("home");
        let temporary = runtime_root.join("tmp");
        for path in [&home, &temporary] {
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
            {
                return Err(SandboxError::InvalidRuntimeDirectory);
            }
        }

        let mut command = Command::new(&self.program);
        #[cfg(feature = "fixture-helper")]
        let profile = match mode {
            FixtureMode::Descendant | FixtureMode::ProcessFlood | FixtureMode::ProcessPulse => {
                DESCENDANT_TEST_PROFILE
            }
            _ => DENY_NETWORK_PROFILE,
        };
        #[cfg(not(feature = "fixture-helper"))]
        let profile = DENY_NETWORK_PROFILE;
        command
            .arg("-p")
            .arg(profile)
            .arg(candidate.executable())
            .arg("--mode")
            .arg(mode.as_str());
        if let FixtureMode::NetworkAttempt(port) = mode {
            command.arg("--port").arg(port.to_string());
        }
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .env("HOME", home)
            .env("TMPDIR", temporary);
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_profile_is_static_and_denies_every_network_operation() {
        assert_eq!(
            DENY_NETWORK_PROFILE,
            "(version 1)\n(allow default)\n(deny network*)\n(deny process-fork)\n"
        );
        assert!(!DENY_NETWORK_PROFILE.contains("network-outbound"));
        assert!(DENY_NETWORK_PROFILE.contains("deny process-fork"));
    }

    #[test]
    fn validates_fixed_system_sandbox_executable() {
        let policy = MacOsDenyNetwork::new().unwrap();
        assert_eq!(
            policy.program(),
            std::path::Path::new("/usr/bin/sandbox-exec")
        );
    }
}
