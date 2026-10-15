use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("voice-parent-auth-inspect: {error:?}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), saturday_voice_parent_auth::ParentAuthError> {
    let mut args = std::env::args_os();
    let _program = args.next();
    let Some(path) = args.next() else {
        return Err(saturday_voice_parent_auth::ParentAuthError::InvalidGrant);
    };
    if args.next().is_some() {
        return Err(saturday_voice_parent_auth::ParentAuthError::InvalidGrant);
    }
    let identity = saturday_voice_parent_auth::inspect_signed_code_at(Path::new(&path))?;
    println!("{}", serde_json::to_string(&identity).map_err(|_| saturday_voice_parent_auth::ParentAuthError::InvalidGrant)?);
    Ok(())
}
