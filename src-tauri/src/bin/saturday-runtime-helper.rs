use saturday::signed_helper::helper::{
    run_helper, HelperCoreError, TeamParentVerifier, HELPER_BUILD_ID,
};
use std::io::{self, BufReader, BufWriter, Write};

fn main() {
    sanitize_environment();
    // SAFETY: alarm configures a process-local absolute lifetime cap. The default
    // SIGALRM action terminates a wedged helper without running unsafe handlers.
    unsafe {
        libc::alarm(10);
    }
    let parent_pid = unsafe { libc::getppid() };
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let result = run_helper(
        &mut reader,
        &mut writer,
        parent_pid,
        &TeamParentVerifier,
        HELPER_BUILD_ID,
    );
    // SAFETY: cancelling the process-local alarm has no memory-safety preconditions.
    unsafe {
        libc::alarm(0);
    }
    let _ = writer.flush();
    let code = match result {
        Ok(()) => 0,
        Err(HelperCoreError::UnauthorizedParent) => 77,
        Err(_) => 70,
    };
    std::process::exit(code);
}

fn sanitize_environment() {
    let home = std::env::var_os("HOME");
    let temporary = std::env::var_os("TMPDIR");
    let keys: Vec<_> = std::env::vars_os().map(|(key, _)| key).collect();
    for key in keys {
        std::env::remove_var(key);
    }
    std::env::set_var("LANG", "C");
    std::env::set_var("LC_ALL", "C");
    if let Some(home) = home {
        std::env::set_var("HOME", home);
    }
    if let Some(temporary) = temporary {
        std::env::set_var("TMPDIR", temporary);
    }
}
