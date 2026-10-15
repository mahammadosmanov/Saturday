use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeMode {
    Ordinary,
    VerifySignedHelper,
    MeasureMainThread(PathBuf),
    Stage1OwnerKeyProvision,
    Stage1OwnerKeyProve,
    Stage1OwnerAuthorizationIssue,
    Stage1OwnerKeyDelete,
    VoiceOwnerBootstrap,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NativeModeError {
    #[error("native mode arguments are invalid")]
    InvalidArguments,
}

pub fn parse_native_mode(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<NativeMode, NativeModeError> {
    let mut arguments = arguments.into_iter();
    let _program = arguments.next().ok_or(NativeModeError::InvalidArguments)?;
    let Some(flag) = arguments.next() else {
        return Ok(NativeMode::Ordinary);
    };

    let mode = if flag == OsStr::new("--verify-signed-helper") {
        NativeMode::VerifySignedHelper
    } else if flag == OsStr::new("--measure-main-thread") {
        NativeMode::MeasureMainThread(PathBuf::from(
            arguments.next().ok_or(NativeModeError::InvalidArguments)?,
        ))
    } else if flag == OsStr::new("--stage1-owner-key-provision") {
        NativeMode::Stage1OwnerKeyProvision
    } else if flag == OsStr::new("--stage1-owner-key-prove") {
        NativeMode::Stage1OwnerKeyProve
    } else if flag == OsStr::new("--stage1-owner-authorization-issue") {
        NativeMode::Stage1OwnerAuthorizationIssue
    } else if flag == OsStr::new("--voice-owner-bootstrap") {
        NativeMode::VoiceOwnerBootstrap
    } else if flag == OsStr::new("--stage1-owner-key-delete") {
        NativeMode::Stage1OwnerKeyDelete
    } else {
        return Err(NativeModeError::InvalidArguments);
    };

    if arguments.next().is_some() {
        return Err(NativeModeError::InvalidArguments);
    }
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<const N: usize>(arguments: [&str; N]) -> Result<NativeMode, NativeModeError> {
        parse_native_mode(arguments.into_iter().map(OsString::from))
    }

    #[test]
    fn parses_only_exact_native_mode_forms() {
        assert_eq!(parse(["saturday"]).unwrap(), NativeMode::Ordinary);
        assert_eq!(
            parse(["saturday", "--verify-signed-helper"]).unwrap(),
            NativeMode::VerifySignedHelper
        );
        assert_eq!(
            parse([
                "saturday",
                "--measure-main-thread",
                "/private/tmp/native-main-thread.json",
            ])
            .unwrap(),
            NativeMode::MeasureMainThread(PathBuf::from("/private/tmp/native-main-thread.json"))
        );
        assert_eq!(
            parse(["saturday", "--stage1-owner-key-provision"]).unwrap(),
            NativeMode::Stage1OwnerKeyProvision
        );
        assert_eq!(
            parse(["saturday", "--stage1-owner-key-prove"]).unwrap(),
            NativeMode::Stage1OwnerKeyProve
        );
        assert_eq!(
            parse(["saturday", "--stage1-owner-authorization-issue"]).unwrap(),
            NativeMode::Stage1OwnerAuthorizationIssue
        );
        assert_eq!(
            parse(["saturday", "--stage1-owner-key-delete"]).unwrap(),
            NativeMode::Stage1OwnerKeyDelete
        );
    }

    #[test]
    fn parses_closed_voice_owner_bootstrap_flag_and_rejects_parameters() {
        assert_eq!(
            parse(["saturday", "--voice-owner-bootstrap"]).unwrap(),
            NativeMode::VoiceOwnerBootstrap
        );
        assert!(parse([
            "saturday",
            "--voice-owner-bootstrap",
            "/arbitrary/model/path",
        ])
        .is_err());
    }

    #[test]
    fn rejects_missing_mixed_repeated_unknown_and_extra_arguments() {
        for arguments in [
            vec![],
            vec!["saturday", "--stage1-owner-key-provision", "extra"],
            vec![
                "saturday",
                "--stage1-owner-key-prove",
                "--verify-signed-helper",
            ],
            vec![
                "saturday",
                "--stage1-owner-key-delete",
                "--stage1-owner-key-delete",
            ],
            vec!["saturday", "--measure-main-thread"],
            vec![
                "saturday",
                "--measure-main-thread",
                "/private/tmp/native-main-thread.json",
                "extra",
            ],
            vec!["saturday", "--verify-signed-helper", "extra"],
            vec!["saturday", "--unknown"],
        ] {
            let result = parse_native_mode(arguments.into_iter().map(OsString::from));
            assert!(result.is_err(), "accepted invalid arguments");
        }
    }
}
