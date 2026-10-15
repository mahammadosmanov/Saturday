use tauri::ipc::{InvokeBody, Request};
use tauri::WebviewWindow;

use crate::voice::contract::{decode_public_request, PublicCommand, VoiceCommand, VoiceError};

pub fn decode_invocation(
    request: &Request,
    command: PublicCommand,
) -> Result<VoiceCommand, VoiceError> {
    decode_invoke_body(request.body(), command)
}

pub fn decode_invoke_body(
    body: &InvokeBody,
    command: PublicCommand,
) -> Result<VoiceCommand, VoiceError> {
    match body {
        InvokeBody::Raw(bytes) => decode_public_request(bytes, command),
        InvokeBody::Json(_) => Err(VoiceError::ProtocolError),
    }
}
const RELEASE_ORIGIN: &str = "tauri://localhost";
#[cfg(debug_assertions)]
const DEVELOPMENT_ORIGIN: &str = "http://127.0.0.1:1420";

/// Snapshot of the actual invoking webview, never request data. Revalidate it immediately
/// before every effect because a window can lose focus or navigate after command dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCallerContext {
    window_label: String,
    origin: String,
    visible: bool,
    focused: bool,
    minimized: bool,
}

impl NativeCallerContext {
    pub fn from_request(request: &Request<'_>, window: &WebviewWindow) -> Result<Self, VoiceError> {
        let context = Self::from_window(window)?;
        let mut origins = request.headers().get_all("origin").iter();
        let origin = origins
            .next()
            .and_then(|value| value.to_str().ok())
            .ok_or(VoiceError::Unauthorized)?;
        if origins.next().is_some() || origin != context.origin() {
            return Err(VoiceError::Unauthorized);
        }
        if !context.is_trusted() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(context)
    }
    pub fn from_window(window: &WebviewWindow) -> Result<Self, VoiceError> {
        let url = window.url().map_err(|_| VoiceError::Unauthorized)?;
        let origin = trusted_origin(
            url.scheme(),
            url.host_str(),
            url.port(),
            url.username(),
            url.password(),
        )
        .ok_or(VoiceError::Unauthorized)?;
        Ok(Self {
            window_label: window.label().to_owned(),
            origin: origin.to_owned(),
            visible: window.is_visible().map_err(|_| VoiceError::Unauthorized)?,
            focused: window.is_focused().map_err(|_| VoiceError::Unauthorized)?,
            minimized: window
                .is_minimized()
                .map_err(|_| VoiceError::Unauthorized)?,
        })
    }

    pub fn window_label(&self) -> &str {
        &self.window_label
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn is_trusted(&self) -> bool {
        self.window_label == "main"
            && (self.origin == RELEASE_ORIGIN || {
                #[cfg(debug_assertions)]
                {
                    self.origin == DEVELOPMENT_ORIGIN
                }
                #[cfg(not(debug_assertions))]
                {
                    false
                }
            })
            && self.visible
            && self.focused
            && !self.minimized
    }

    pub fn ensure_live(&self, window: &WebviewWindow) -> Result<(), VoiceError> {
        let current = Self::from_window(window)?;
        if self.is_trusted() && current == *self && current.is_trusted() {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
}

fn trusted_origin(
    scheme: &str,
    host: Option<&str>,
    port: Option<u16>,
    username: &str,
    password: Option<&str>,
) -> Option<&'static str> {
    match (scheme, host, port, username, password) {
        ("tauri", Some("localhost"), None, "", None) => Some(RELEASE_ORIGIN),
        #[cfg(debug_assertions)]
        ("http", Some("127.0.0.1"), Some(1420), "", None) => Some(DEVELOPMENT_ORIGIN),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_local_application_origins_are_accepted() {
        assert_eq!(
            trusted_origin("tauri", Some("localhost"), None, "", None),
            Some(RELEASE_ORIGIN)
        );
        assert_eq!(
            trusted_origin("tauri", Some("localhost.evil"), None, "", None),
            None
        );
        assert_eq!(
            trusted_origin("tauri", Some("localhost"), Some(443), "", None),
            None
        );
        assert_eq!(
            trusted_origin("tauri", Some("localhost"), None, "user", None),
            None
        );
        #[cfg(debug_assertions)]
        assert_eq!(
            trusted_origin("http", Some("127.0.0.1"), Some(1420), "", None),
            Some(DEVELOPMENT_ORIGIN)
        );
        assert_eq!(
            trusted_origin("http", Some("127.0.0.1"), Some(1421), "", None),
            None
        );
    }

    #[test]
    fn caller_snapshot_requires_visible_focused_main_window() {
        let caller = NativeCallerContext {
            window_label: "main".into(),
            origin: RELEASE_ORIGIN.into(),
            visible: true,
            focused: true,
            minimized: false,
        };
        assert!(caller.is_trusted());
        let mut hidden = caller.clone();
        hidden.visible = false;
        assert!(!hidden.is_trusted());
        let mut foreign = caller;
        foreign.window_label = "auxiliary".into();
        assert!(!foreign.is_trusted());
    }

    #[test]
    fn command_boundary_accepts_only_duplicate_checked_raw_envelopes() {
        let valid = InvokeBody::Raw(
            br#"{"request":{"protocol_version":1,"mode":"snapshot_begin"}}"#.to_vec(),
        );
        assert!(decode_invoke_body(&valid, PublicCommand::GetStatus).is_ok());

        let duplicate = InvokeBody::Raw(
            br#"{"request":{"protocol_version":1,"protocol_version":1,"mode":"snapshot_begin"}}"#
                .to_vec(),
        );
        assert_eq!(
            decode_invoke_body(&duplicate, PublicCommand::GetStatus),
            Err(VoiceError::InvalidRequest)
        );

        let converted_json = InvokeBody::Json(serde_json::json!({
            "request": {"protocol_version": 1, "mode": "snapshot_begin"}
        }));
        assert_eq!(
            decode_invoke_body(&converted_json, PublicCommand::GetStatus),
            Err(VoiceError::ProtocolError)
        );
    }
}

fn decode_for_window(
    request: &Request<'_>,
    window: &WebviewWindow,
    command: PublicCommand,
) -> Result<(NativeCallerContext, VoiceCommand), VoiceError> {
    let caller = NativeCallerContext::from_request(request, window)?;
    Ok((caller, decode_invocation(request, command)?))
}

#[tauri::command]
pub fn voice_get_status(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::supervisor::VoiceStatusRead, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::GetStatus)?;
    let VoiceCommand::GetStatus(status) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.authorize(&caller, &VoiceCommand::GetStatus(status.clone()))?;
    caller.ensure_live(&window)?;
    supervisor.get_status(caller, status)
}

#[tauri::command]
pub fn voice_start(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::Start)?;
    let VoiceCommand::Start(start) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.start(caller, &window, start)
}

#[tauri::command]
pub fn voice_finish(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::Finish)?;
    let VoiceCommand::Finish(owner) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.finish(caller, &window, owner.owner)
}

#[tauri::command]
pub fn voice_submit_text(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::SubmitText)?;
    let VoiceCommand::SubmitText(text) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.submit_text(caller, &window, text)
}

#[tauri::command]
pub fn voice_confirm_transcript(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::ConfirmTranscript)?;
    let VoiceCommand::ConfirmTranscript(transcript) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.confirm_transcript(caller, &window, transcript)
}

#[tauri::command]
pub fn voice_cancel(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::Cancel)?;
    let VoiceCommand::Cancel(cancel) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.cancel(caller, &window, cancel)
}

#[tauri::command]
pub fn voice_clear_session(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::ClearSession)?;
    if decoded != VoiceCommand::ClearSession {
        return Err(VoiceError::InvalidRequest);
    }
    supervisor.clear_session(caller, &window)
}

#[tauri::command]
pub fn voice_select_preferences(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::SelectPreferences)?;
    let VoiceCommand::SelectPreferences(preferences) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.select_preferences(caller, &window, preferences)
}

#[tauri::command]
pub fn voice_ack_intent(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::AckIntent)?;
    let VoiceCommand::AckIntent(ack) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.ack_intent(caller, &window, ack)
}

#[tauri::command]
pub fn voice_assets_get_status(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::AssetCatalog, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::AssetsGetStatus)?;
    if decoded != VoiceCommand::AssetsGetStatus {
        return Err(VoiceError::InvalidRequest);
    }
    caller.ensure_live(&window)?;
    supervisor.assets_get_status(&caller)
}

#[tauri::command]
pub async fn voice_assets_request_install(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::AssetInstallOutcome, VoiceError> {
    let (caller, decoded) =
        decode_for_window(&request, &window, PublicCommand::AssetsRequestInstall)?;
    let VoiceCommand::AssetsRequestInstall(install) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.authorize(
        &caller,
        &VoiceCommand::AssetsRequestInstall(install.clone()),
    )?;
    caller.ensure_live(&window)?;
    let manager = supervisor
        .asset_manager()
        .ok_or(VoiceError::SetupRequired)?;
    let cancellation = crate::voice::contract::CancelToken::default();
    let task = tauri::async_runtime::spawn_blocking(move || {
        manager
            .request_install(&caller, &install.asset_id, &cancellation)
            .map_err(|error| match error {
                crate::voice::contract::AssetError::ApprovalRequired => VoiceError::SetupRequired,
                crate::voice::contract::AssetError::Cancelled => VoiceError::Cancelled,
                crate::voice::contract::AssetError::Timeout => VoiceError::Timeout,
                crate::voice::contract::AssetError::CleanupFailed => VoiceError::CleanupFailed,
                crate::voice::contract::AssetError::NotFound
                | crate::voice::contract::AssetError::Unavailable => VoiceError::Unavailable,
                crate::voice::contract::AssetError::WrongPurpose => VoiceError::Unauthorized,
                crate::voice::contract::AssetError::InvalidBinding
                | crate::voice::contract::AssetError::DigestMismatch => VoiceError::ProtocolError,
                crate::voice::contract::AssetError::StorageFailure => VoiceError::Internal,
                crate::voice::contract::AssetError::Failed => VoiceError::ProviderFailed,
            })
    })
    .await
    .map_err(|_| VoiceError::Internal)??;
    Ok(task)
}

#[tauri::command]
pub fn voice_assets_disable(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::AssetCatalog, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::AssetsDisable)?;
    let VoiceCommand::AssetsDisable(asset) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.authorize(&caller, &VoiceCommand::AssetsDisable(asset.clone()))?;
    caller.ensure_live(&window)?;
    supervisor.assets_disable(&asset.asset_id)
}

#[tauri::command]
pub fn voice_assets_delete(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::AssetRemovalOutcome, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::AssetsDelete)?;
    let VoiceCommand::AssetsDelete(asset) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.authorize(&caller, &VoiceCommand::AssetsDelete(asset.clone()))?;
    caller.ensure_live(&window)?;
    supervisor.assets_delete(&asset.asset_id)
}

#[tauri::command]
pub fn voice_assets_rollback(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::AssetCatalog, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::AssetsRollback)?;
    let VoiceCommand::AssetsRollback(asset) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.authorize(&caller, &VoiceCommand::AssetsRollback(asset.clone()))?;
    caller.ensure_live(&window)?;
    supervisor.assets_rollback(&asset.asset_id, &asset.version_id)
}

#[tauri::command]
pub fn voice_test_output(
    request: Request<'_>,
    window: WebviewWindow,
    supervisor: tauri::State<'_, crate::voice::supervisor::VoiceSessionSupervisor>,
) -> Result<crate::voice::contract::VoiceCommandWireResult, VoiceError> {
    let (caller, decoded) = decode_for_window(&request, &window, PublicCommand::TestOutput)?;
    let VoiceCommand::TestOutput(test) = decoded else {
        return Err(VoiceError::InvalidRequest);
    };
    supervisor.test_output(caller, &window, test)
}
