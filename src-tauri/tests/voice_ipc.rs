use saturday::voice::contract::{
    decode_public_request, DecimalU64, PublicCommand, VoiceError, VoicePhase,
};

#[test]
fn duplicate_keys_are_rejected_before_value_conversion() {
    let request =
        br#"{"request":{"protocol_version":1,"protocol_version":1,"mode":"snapshot_begin"}}"#;
    assert_eq!(
        decode_public_request(request, PublicCommand::GetStatus),
        Err(VoiceError::InvalidRequest)
    );
}

#[test]
fn unknown_outer_and_nested_fields_are_rejected() {
    let outer = br#"{"request":{"protocol_version":1,"mode":"snapshot_begin"},"extra":true}"#;
    let nested = br#"{"request":{"protocol_version":1,"mode":"snapshot_begin","owner":null}}"#;
    assert_eq!(
        decode_public_request(outer, PublicCommand::GetStatus),
        Err(VoiceError::InvalidRequest)
    );
    assert_eq!(
        decode_public_request(nested, PublicCommand::GetStatus),
        Err(VoiceError::InvalidRequest)
    );
}

#[test]
fn status_modes_decode_only_their_declared_fields() {
    let begin = br#"{"request":{"protocol_version":1,"mode":"snapshot_begin"}}"#;
    let decoded = decode_public_request(begin, PublicCommand::GetStatus).unwrap();
    assert!(matches!(
        decoded,
        saturday::voice::contract::VoiceCommand::GetStatus(_)
    ));

    let wrong_version = br#"{"request":{"protocol_version":2,"mode":"snapshot_begin"}}"#;
    assert_eq!(
        decode_public_request(wrong_version, PublicCommand::GetStatus),
        Err(VoiceError::InvalidRequest)
    );
}

#[test]
fn canonical_decimal_u64_round_trips_without_json_numbers() {
    let maximum = DecimalU64(u64::MAX);
    assert_eq!(
        serde_json::to_string(&maximum).unwrap(),
        format!("\"{}\"", u64::MAX)
    );
    assert_eq!(
        serde_json::from_str::<DecimalU64>(&format!("\"{}\"", u64::MAX)).unwrap(),
        maximum
    );
    for invalid in ["+1", "01", "-1", "1.0", "18446744073709551616"] {
        assert!(serde_json::from_str::<DecimalU64>(&format!("\"{invalid}\"")).is_err());
    }
    assert!(serde_json::from_str::<DecimalU64>("1").is_err());
}

#[test]
fn all_voice_phases_are_fixed_wire_values() {
    let expected = [
        "unavailable",
        "setup_required",
        "idle",
        "requesting_permission",
        "listening",
        "transcribing",
        "needs_review",
        "routing",
        "thinking",
        "speaking",
        "cancelling",
        "error",
    ];
    let actual: Vec<_> = [
        VoicePhase::Unavailable,
        VoicePhase::SetupRequired,
        VoicePhase::Idle,
        VoicePhase::RequestingPermission,
        VoicePhase::Listening,
        VoicePhase::Transcribing,
        VoicePhase::NeedsReview,
        VoicePhase::Routing,
        VoicePhase::Thinking,
        VoicePhase::Speaking,
        VoicePhase::Cancelling,
        VoicePhase::Error,
    ]
    .iter()
    .map(|phase| {
        serde_json::to_string(phase)
            .unwrap()
            .trim_matches('"')
            .to_owned()
    })
    .collect();
    assert_eq!(actual, expected);
}
