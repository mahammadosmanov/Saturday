use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("process identifier is invalid")]
    InvalidProcess,
    #[error("current process has no accepted team identity")]
    TeamUnavailable,
    #[error("peer process identity is unavailable")]
    IdentityUnavailable,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_process_matches_current_team(peer_pid: libc::pid_t) -> libc::c_int;
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
