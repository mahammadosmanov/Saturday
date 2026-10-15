use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("process identifier is invalid")]
    InvalidProcess,
    #[error("expected process identity is invalid")]
    InvalidExpectedIdentity,
    #[error("current process has no accepted team identity")]
    TeamUnavailable,
    #[error("peer process identity is unavailable")]
    IdentityUnavailable,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_process_matches_current_team(peer_pid: libc::pid_t) -> libc::c_int;
    fn saturday_process_matches_identity(
        peer_pid: libc::pid_t,
        expected_identifier: *const std::ffi::c_char,
        expected_team_sha256: *const u8,
        expected_requirement_sha256: *const u8,
    ) -> libc::c_int;
}

pub fn process_matches_current_team(peer_pid: i32) -> Result<bool, IdentityError> {
    if peer_pid <= 0 {
        return Err(IdentityError::InvalidProcess);
    }

    #[cfg(target_os = "macos")]
    {
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        if unsafe { libc::kill(peer_pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return Err(IdentityError::IdentityUnavailable);
        }
    }

    #[cfg(target_os = "macos")]
    {
        // SAFETY: the C bridge accepts a positive pid value, owns all objects it creates,
        // and returns only an integer result code.
        match unsafe { saturday_process_matches_current_team(peer_pid) } {
            1 => Ok(true),
            0 => Ok(false),
            -2 => Err(IdentityError::TeamUnavailable),
            _ => Err(IdentityError::IdentityUnavailable),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = peer_pid;
        Err(IdentityError::IdentityUnavailable)
    }
}

fn decode_sha256(value: &str) -> Result<[u8; 32], IdentityError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(IdentityError::InvalidExpectedIdentity);
    }
    let mut digest = [0u8; 32];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for (index, pair) in pairs.iter().enumerate() {
        let high = (pair[0] as char)
            .to_digit(16)
            .ok_or(IdentityError::InvalidExpectedIdentity)?;
        let low = (pair[1] as char)
            .to_digit(16)
            .ok_or(IdentityError::InvalidExpectedIdentity)?;
        digest[index] = ((high << 4) | low) as u8;
    }
    Ok(digest)
}

pub fn process_matches_identity(
    peer_pid: i32,
    expected_identifier: &str,
    expected_team_id_sha256: &str,
    expected_requirement_sha256: &str,
) -> Result<bool, IdentityError> {
    if peer_pid <= 0 {
        return Err(IdentityError::InvalidProcess);
    }
    if expected_identifier.is_empty() || expected_identifier.len() > 256 {
        return Err(IdentityError::InvalidExpectedIdentity);
    }
    let expected_identifier = std::ffi::CString::new(expected_identifier)
        .map_err(|_| IdentityError::InvalidExpectedIdentity)?;
    let team_digest = decode_sha256(expected_team_id_sha256)?;
    let requirement_digest = decode_sha256(expected_requirement_sha256)?;

    #[cfg(target_os = "macos")]
    {
        if unsafe { libc::kill(peer_pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return Err(IdentityError::IdentityUnavailable);
        }
        match unsafe {
            saturday_process_matches_identity(
                peer_pid,
                expected_identifier.as_ptr(),
                team_digest.as_ptr(),
                requirement_digest.as_ptr(),
            )
        } {
            1 => Ok(true),
            0 => Ok(false),
            _ => Err(IdentityError::IdentityUnavailable),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (expected_identifier, team_digest, requirement_digest);
        Err(IdentityError::IdentityUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_and_dead_process_ids() {
        assert!(matches!(
            process_matches_current_team(-1),
            Err(IdentityError::InvalidProcess)
        ));
        assert!(matches!(
            process_matches_current_team(i32::MAX),
            Err(IdentityError::IdentityUnavailable)
        ));
    }

    #[test]
    fn ad_hoc_test_process_has_no_authorized_team_identity() {
        assert!(matches!(
            process_matches_current_team(std::process::id() as i32),
            Err(IdentityError::TeamUnavailable)
        ));
    }
}
