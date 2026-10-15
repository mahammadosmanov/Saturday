use saturday::platform::voice_speech::{
    NativeSpeechEvent, NativeSpeechEventKind, NativeVoiceRecord,
};
use saturday::voice::contract::{Language, OperationId, SpeechSegment, VoiceId, VoiceOwner};
use saturday::voice::speech::{
    resolve_voice, voice_catalog_from_native, BoundedSpeechQueue, PlainSpeechProjection,
    SpeechProjectionDisclosure, SpeechQueueAdmission,
};
use serde_json::Value;

const CORPUS: &str = include_str!("../../tests/fixtures/voice/tts-pronunciation.json");

fn owner(generation: u64) -> VoiceOwner {
    VoiceOwner {
        session_id: "session-1".to_owned(),
        turn_id: "turn-1".to_owned(),
        generation,
    }
}

fn segment(text: &str, language: Language, voice_id: &str) -> SpeechSegment {
    SpeechSegment {
        text: text.to_owned(),
        language,
        voice_id: VoiceId(voice_id.to_owned()),
    }
}

fn native_event(
    owner: &VoiceOwner,
    operation_id: u64,
    utterance_id: u64,
    kind: NativeSpeechEventKind,
) -> NativeSpeechEvent {
    NativeSpeechEvent {
        owner: owner.clone(),
        operation_id,
        utterance_id,
        error_code: 0,
        kind,
    }
}

#[test]
fn licensed_bilingual_corpus_has_twenty_original_phrases_per_language() {
    let corpus: Value = serde_json::from_str(CORPUS).expect("valid corpus");
    assert_eq!(corpus["provenance"]["license"], "CC0-1.0");
    assert_eq!(corpus["phrases"]["tr"].as_array().unwrap().len(), 20);
    assert_eq!(corpus["phrases"]["en"].as_array().unwrap().len(), 20);
    assert_eq!(
        corpus["provenance"]["quality_review"],
        "pending explicit owner playback and independent fluent review; no result is claimed"
    );
}

#[test]
fn accepted_segment_is_not_speaking_until_matching_native_start() {
    let turn = owner(1);
    let mut queue = BoundedSpeechQueue::new();
    assert_eq!(
        queue.admit(turn.clone(), OperationId(7), 71),
        Ok(SpeechQueueAdmission::Start)
    );
    assert!(!queue.is_speaking());

    assert!(queue
        .on_native_event(&native_event(&turn, 7, 71, NativeSpeechEventKind::DidStart))
        .is_some());
    assert!(queue.is_speaking());
}

#[test]
fn late_duplicate_and_wrong_operation_callbacks_cannot_finish_current_speech() {
    let turn = owner(1);
    let mut queue = BoundedSpeechQueue::new();
    queue.admit(turn.clone(), OperationId(7), 71).unwrap();
    assert!(queue
        .on_native_event(&native_event(&turn, 7, 71, NativeSpeechEventKind::DidStart))
        .is_some());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            7,
            71,
            NativeSpeechEventKind::DidFinish
        ))
        .is_some());

    assert_eq!(
        queue.admit(turn.clone(), OperationId(8), 72),
        Ok(SpeechQueueAdmission::Start)
    );
    assert!(!queue.is_speaking());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            7,
            71,
            NativeSpeechEventKind::DidFinish
        ))
        .is_none());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            9,
            72,
            NativeSpeechEventKind::DidFinish
        ))
        .is_none());
    assert!(!queue.is_speaking());

    assert!(queue
        .on_native_event(&native_event(&turn, 8, 72, NativeSpeechEventKind::DidStart))
        .is_some());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            8,
            72,
            NativeSpeechEventKind::DidFinish
        ))
        .is_some());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            8,
            72,
            NativeSpeechEventKind::DidFinish
        ))
        .is_none());
}

#[test]
fn cancel_before_start_revokes_queued_work_without_waiting_for_did_cancel() {
    let turn = owner(3);
    let mut queue = BoundedSpeechQueue::new();
    queue.admit(turn.clone(), OperationId(11), 111).unwrap();
    assert_eq!(queue.cancel_owner(&turn).len(), 1);
    assert!(!queue.is_speaking());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            11,
            111,
            NativeSpeechEventKind::DidStart
        ))
        .is_none());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            11,
            111,
            NativeSpeechEventKind::DidFinish
        ))
        .is_none());
}

#[test]
fn bounded_queue_accepts_one_active_and_two_waiting_segments_only() {
    let turn = owner(5);
    let mut queue = BoundedSpeechQueue::new();
    for index in 0..3 {
        assert!(queue
            .admit(turn.clone(), OperationId(index + 1), index + 101)
            .is_ok());
    }
    assert!(queue.admit(turn.clone(), OperationId(4), 104).is_err());
    assert_eq!(queue.queued_len(), 3);
}

#[test]
fn plain_projection_preserves_turkish_nfc_numbers_and_punctuation() {
    let projection =
        PlainSpeechProjection::from_answer("I\u{307}zmir'de 3,75 °C ölçüldü; çağrı saat 14.30'da.")
            .unwrap();
    let spoken = projection.segments.join(" ");
    assert!(spoken.contains("İzmir'de"));
    assert!(spoken.contains("3,75 °C"));
    assert!(spoken.contains("14.30'da"));
    assert!(spoken.chars().all(|character| character != '\u{307}'));
}

#[test]
fn plain_projection_omits_markup_urls_and_code_with_disclosure() {
    let projection = PlainSpeechProjection::from_answer(
        "Read <b>this</b>. See https://example.invalid/path. `let secret = 4;`\n```rust\nfn hidden() {}\n```",
    )
    .unwrap();
    let spoken = projection.segments.join(" ");
    assert!(spoken.contains("Read this."));
    assert!(!spoken.contains("https://"));
    assert!(!spoken.contains("secret"));
    assert!(!spoken.contains("hidden"));
    assert!(projection
        .disclosures
        .contains(&SpeechProjectionDisclosure::NonSpokenContentOmitted));
}

#[test]
fn catalog_filters_other_locales_before_cap_and_keeps_exact_candidates() {
    let mut records: Vec<_> = (0..130)
        .map(|index| NativeVoiceRecord {
            id: format!("unrelated-{index}"),
            display_name: format!("Other {index}"),
            locale: "fr-FR".to_owned(),
            language: Language::En,
        })
        .collect();
    records.push(NativeVoiceRecord {
        id: "com.apple.voice.compact.en-US.Samantha".to_owned(),
        display_name: "Samantha".to_owned(),
        locale: "en-US".to_owned(),
        language: Language::En,
    });
    let catalog = voice_catalog_from_native(records, 9).unwrap();
    assert_eq!(catalog.revision, 9);
    assert_eq!(catalog.english.len(), 1);
    assert!(catalog.turkish.is_empty());
}

#[test]
fn selected_voice_never_falls_back_and_missing_voice_keeps_display_text() {
    let catalog = voice_catalog_from_native(
        vec![NativeVoiceRecord {
            id: "com.apple.voice.compact.en-US.Samantha".to_owned(),
            display_name: "Samantha".to_owned(),
            locale: "en-US".to_owned(),
            language: Language::En,
        }],
        1,
    )
    .unwrap();
    assert_eq!(
        resolve_voice(
            &catalog,
            &segment(
                "Merhaba.",
                Language::Tr,
                "com.apple.voice.compact.en-US.Samantha",
            ),
        ),
        Err(saturday::voice::contract::SpeechError::WrongLanguage)
    );
    let absent = voice_catalog_from_native(Vec::new(), 2).unwrap();
    let answer = "Yanıt metni korunur.";
    let projection = PlainSpeechProjection::from_answer(answer).unwrap();
    assert_eq!(
        resolve_voice(
            &absent,
            &segment(
                "Yanıt metni korunur.",
                Language::Tr,
                "com.apple.voice.super-compact.tr-TR.Yelda",
            ),
        ),
        Err(saturday::voice::contract::SpeechError::VoiceUnavailable)
    );
    assert_eq!(projection.display_text, answer);
}

#[test]
fn unsupported_voices_do_not_change_supported_voice_cap() {
    let records: Vec<_> = (0..=128)
        .map(|index| NativeVoiceRecord {
            id: format!("supported-{index}"),
            display_name: format!("English {index}"),
            locale: "en-US".to_owned(),
            language: Language::En,
        })
        .collect();
    assert_eq!(
        voice_catalog_from_native(records, 1),
        Err(saturday::voice::contract::SpeechError::Unavailable)
    );
}

#[test]
fn route_loss_revokes_speaking_and_drains_only_matching_owner_queue() {
    let turn = owner(8);
    let mut queue = BoundedSpeechQueue::new();
    queue.admit(turn.clone(), OperationId(21), 201).unwrap();
    queue.admit(turn.clone(), OperationId(22), 202).unwrap();
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            21,
            201,
            NativeSpeechEventKind::DidStart
        ))
        .is_some());
    assert!(queue.is_speaking());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            21,
            201,
            NativeSpeechEventKind::RouteLost
        ))
        .is_some());
    assert!(!queue.is_speaking());
    assert!(queue
        .on_native_event(&native_event(
            &turn,
            22,
            202,
            NativeSpeechEventKind::RouteLost
        ))
        .is_some());
    assert_eq!(queue.queued_len(), 0);
}
