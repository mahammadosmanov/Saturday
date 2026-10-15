use crate::voice::contract::{
    AcceptedUtterance, AppIntentDescriptor, DecimalU64, Language, OperationId, ReadinessSnapshot,
    ReadinessState, RouteKind, RouteOutcome, RouterDecision, TranscriptEligibility,
    UtteranceSource, VoiceOwner,
};
use crate::voice::text::normalize_for_matching;

pub struct DeterministicIntentRouter;

impl DeterministicIntentRouter {
    pub fn route(
        &self,
        utterance: &AcceptedUtterance,
        readiness: &ReadinessSnapshot,
        owner: &VoiceOwner,
        intent_id: OperationId,
    ) -> RouterDecision {
        let prefix = response_prefix(&utterance.text);
        let response_language = prefix.map_or(utterance.language, |(language, _)| language);
        let body = prefix.map_or(utterance.text.as_str(), |(_, body)| body);
        let normalized = normalize_for_matching(body);
        if utterance.source == UtteranceSource::Speech
            && utterance.eligibility != TranscriptEligibility::Eligible
        {
            return resolved(
                RouteKind::Clarification,
                response_language,
                clarification_text(response_language),
                None,
                false,
            );
        }
        if matches_ambiguous_command(&normalized, body) {
            return resolved(
                RouteKind::Clarification,
                response_language,
                clarification_text(response_language),
                None,
                false,
            );
        }
        match classify_normalized(&normalized) {
            Some(PhraseIntent::OutOfScope) => resolved(
                RouteKind::Unavailable,
                response_language,
                out_of_scope_text(response_language),
                None,
                false,
            ),
            Some(PhraseIntent::Status) => resolved(
                RouteKind::Deterministic,
                response_language,
                status_text(readiness, response_language),
                None,
                false,
            ),
            Some(PhraseIntent::Help) => resolved(
                RouteKind::Deterministic,
                response_language,
                help_text(readiness, response_language),
                None,
                false,
            ),
            Some(PhraseIntent::Overview) => resolved(
                RouteKind::Deterministic,
                response_language,
                String::new(),
                Some(AppIntentDescriptor::Overview {
                    owner: owner.clone(),
                    intent_id: DecimalU64(intent_id.0),
                }),
                false,
            ),
            Some(PhraseIntent::Conversation) => resolved(
                RouteKind::Deterministic,
                response_language,
                String::new(),
                Some(AppIntentDescriptor::Conversation {
                    owner: owner.clone(),
                    intent_id: DecimalU64(intent_id.0),
                }),
                false,
            ),
            Some(PhraseIntent::PauseMotion) => resolved(
                RouteKind::Deterministic,
                response_language,
                String::new(),
                Some(AppIntentDescriptor::Motion {
                    owner: owner.clone(),
                    intent_id: DecimalU64(intent_id.0),
                    enabled: false,
                }),
                false,
            ),
            Some(PhraseIntent::ResumeMotion) => resolved(
                RouteKind::Deterministic,
                response_language,
                String::new(),
                Some(AppIntentDescriptor::Motion {
                    owner: owner.clone(),
                    intent_id: DecimalU64(intent_id.0),
                    enabled: true,
                }),
                false,
            ),
            Some(PhraseIntent::SetTurkish) => language_preference(Language::Tr, response_language),
            Some(PhraseIntent::SetEnglish) => language_preference(Language::En, response_language),
            None if readiness.llm.state != ReadinessState::Ready => resolved(
                RouteKind::Unavailable,
                response_language,
                model_unavailable_text(response_language),
                None,
                false,
            ),
            None => {
                let mut request = utterance.clone();
                request.text = body.to_owned();
                RouterDecision::Unhandled {
                    utterance: request,
                    response_language: (response_language != utterance.language)
                        .then_some(response_language),
                }
            }
        }
    }
}

fn resolved(
    kind: RouteKind,
    language: Language,
    text: String,
    intent: Option<AppIntentDescriptor>,
    spoken_reply: bool,
) -> RouterDecision {
    RouterDecision::Resolved(RouteOutcome {
        kind,
        language,
        text,
        intent,
        spoken_reply,
    })
}

fn language_preference(language: Language, response_language: Language) -> RouterDecision {
    let text = match (language, response_language) {
        (Language::Tr, Language::Tr) => "Dil tercihiniz Türkçe olarak güncellendi.",
        (Language::Tr, Language::En) => "Your language preference is now Turkish.",
        (Language::En, Language::Tr) => "Dil tercihiniz İngilizce olarak güncellendi.",
        (Language::En, Language::En) => "Your language preference is now English.",
    };
    RouterDecision::LanguagePreference {
        language,
        response: RouteOutcome {
            kind: RouteKind::Deterministic,
            language: response_language,
            text: text.to_owned(),
            intent: None,
            spoken_reply: false,
        },
    }
}

fn response_prefix(text: &str) -> Option<(Language, &str)> {
    let (prefix, body) = text.split_once(':')?;
    let language = match normalize_for_matching(prefix).as_str() {
        "answer in turkish" | "türkçe yanıtla" => Language::Tr,
        "answer in english" | "ingilizce yanıtla" => Language::En,
        _ => return None,
    };
    (!body.trim().is_empty()).then_some((language, body.trim()))
}

fn status_text(readiness: &ReadinessSnapshot, language: Language) -> String {
    let capture = readiness_label(readiness.capture.state, language);
    let asr = readiness_label(readiness.asr.state, language);
    let router = readiness_label(readiness.router.state, language);
    let model = readiness_label(readiness.llm.state, language);
    let speech = readiness_label(readiness.speech.state, language);
    let routine = readiness.routine_tasks;
    match language {
        Language::En => format!(
            "Capture: {capture}; ASR: {asr}; router: {router}; local model: {model}; speech: {speech}. Completed routine tasks: {}. Completed without language-model inference: {}.",
            routine.completed, routine.without_llm
        ),
        Language::Tr => format!(
            "Mikrofon: {capture}; yazıya döküm: {asr}; yönlendirici: {router}; yerel model: {model}; konuşma: {speech}. Tamamlanan rutin görevler: {}. Dil modeli çıkarımı olmadan tamamlananlar: {}.",
            routine.completed, routine.without_llm
        ),
    }
}

fn readiness_label(state: ReadinessState, language: Language) -> &'static str {
    match (language, state) {
        (Language::En, ReadinessState::Ready) => "ready",
        (Language::En, ReadinessState::SetupRequired) => "setup required",
        (Language::En, ReadinessState::Unavailable) => "unavailable",
        (Language::Tr, ReadinessState::Ready) => "hazır",
        (Language::Tr, ReadinessState::SetupRequired) => "kurulum gerekli",
        (Language::Tr, ReadinessState::Unavailable) => "kullanılamıyor",
    }
}

fn help_text(readiness: &ReadinessSnapshot, language: Language) -> String {
    let counts = readiness.routine_tasks;
    match language {
        Language::En => format!(
            "Supported: status, help, return to core, open conversation, pause or resume animation, and set language to Turkish or English. Routine tasks completed: {}; completed without language-model inference: {}. I do not access files, browse the internet, run commands, start listening, or provide live weather.",
            counts.completed, counts.without_llm
        ),
        Language::Tr => format!(
            "Desteklenenler: durum, yardım, ana ekrana dön, sohbeti aç, animasyonu duraklat veya sürdür ve dili Türkçe ya da İngilizce yap. Tamamlanan rutin görevler: {}; dil modeli çıkarımı olmadan tamamlananlar: {}. Dosyalara erişemem, internette gezinemem, komut çalıştıramam, dinlemeyi başlatamam veya güncel hava durumu veremem.",
            counts.completed, counts.without_llm
        ),
    }
}

fn clarification_text(language: Language) -> String {
    match language {
        Language::En => "I couldn't safely identify one supported request. Please say a single supported phrase.".to_owned(),
        Language::Tr => "Tek bir desteklenen isteği güvenle belirleyemedim. Lütfen desteklenen ifadelerden birini söyleyin.".to_owned(),
    }
}

fn out_of_scope_text(language: Language) -> String {
    match language {
        Language::En => "That request is outside my supported local voice tasks.".to_owned(),
        Language::Tr => "Bu istek desteklenen yerel ses görevlerimin dışında.".to_owned(),
    }
}

fn model_unavailable_text(language: Language) -> String {
    match language {
        Language::En => {
            "Open-ended local conversation is unavailable until the approved local model is ready."
                .to_owned()
        }
        Language::Tr => {
            "Onaylı yerel model hazır olana kadar serbest yerel sohbet kullanılamıyor.".to_owned()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhraseIntent {
    Status,
    Help,
    Overview,
    Conversation,
    PauseMotion,
    ResumeMotion,
    SetTurkish,
    SetEnglish,
    OutOfScope,
}

#[cfg(test)]
fn classify_phrase(utterance: &str) -> Option<PhraseIntent> {
    classify_normalized(&normalize_for_matching(utterance))
}

fn classify_normalized(normalized: &str) -> Option<PhraseIntent> {
    let phrase = normalized
        .strip_prefix("please ")
        .or_else(|| normalized.strip_prefix("lütfen "))
        .unwrap_or(normalized);
    match phrase {
        "status"
        | "voice status"
        | "provider status"
        | "what is ready"
        | "durum"
        | "ses durumu"
        | "sağlayıcı durumu"
        | "neler hazır" => Some(PhraseIntent::Status),
        "help"
        | "show supported commands"
        | "what can you do"
        | "yardım"
        | "desteklenen komutları göster"
        | "ne yapabilirsin" => Some(PhraseIntent::Help),
        "return to the core" | "return to core" | "focus" | "ana ekrana dön" | "çekirdeğe dön" => {
            Some(PhraseIntent::Overview)
        }
        "open conversation" | "show conversation" | "sohbeti aç" | "konuşmayı aç" => {
            Some(PhraseIntent::Conversation)
        }
        "pause animations"
        | "pause animation"
        | "animasyonları duraklat"
        | "animasyonu duraklat" => Some(PhraseIntent::PauseMotion),
        "resume animations" | "resume animation" | "animasyonları sürdür" | "animasyonu sürdür" => {
            Some(PhraseIntent::ResumeMotion)
        }
        "set language to turkish" | "dili türkçe yap" | "türkçeye geç" => {
            Some(PhraseIntent::SetTurkish)
        }
        "switch language to english" | "dili ingilizce yap" | "ingilizceye geç" => {
            Some(PhraseIntent::SetEnglish)
        }
        "open my files"
        | "dosyalarımı aç"
        | "browse the internet"
        | "internette gezin"
        | "run a shell command"
        | "kabuk komutu çalıştır"
        | "start listening"
        | "dinlemeyi başlat"
        | "tell me today's weather"
        | "bugünün hava durumunu söyle" => Some(PhraseIntent::OutOfScope),
        _ => None,
    }
}

fn matches_ambiguous_command(normalized: &str, original: &str) -> bool {
    const PHRASES: &[&str] = &[
        "status",
        "voice status",
        "provider status",
        "what is ready",
        "durum",
        "ses durumu",
        "sağlayıcı durumu",
        "neler hazır",
        "help",
        "show supported commands",
        "what can you do",
        "yardım",
        "desteklenen komutları göster",
        "ne yapabilirsin",
        "return to the core",
        "return to core",
        "focus",
        "ana ekrana dön",
        "çekirdeğe dön",
        "open conversation",
        "show conversation",
        "sohbeti aç",
        "konuşmayı aç",
        "pause animations",
        "pause animation",
        "resume animations",
        "resume animation",
        "animasyonları duraklat",
        "animasyonu duraklat",
        "animasyonları sürdür",
        "animasyonu sürdür",
        "set language to turkish",
        "switch language to english",
        "dili türkçe yap",
        "türkçeye geç",
        "dili ingilizce yap",
        "ingilizceye geç",
    ];
    let mut mentions = 0;
    let mut cursor = 0;
    while cursor < normalized.len() {
        let before = normalized[..cursor].chars().next_back();
        if before.is_none_or(|character| !character.is_alphanumeric()) {
            let matched_len = PHRASES
                .iter()
                .filter(|phrase| normalized[cursor..].starts_with(**phrase))
                .filter_map(|phrase| {
                    let end = cursor + phrase.len();
                    normalized[end..]
                        .chars()
                        .next()
                        .is_none_or(|character| !character.is_alphanumeric())
                        .then_some(phrase.len())
                })
                .max()
                .unwrap_or(0);
            if matched_len > 0 {
                mentions += 1;
                if mentions > 1 {
                    return true;
                }
                cursor += matched_len;
                continue;
            }
        }
        let Some(character) = normalized[cursor..].chars().next() else {
            break;
        };
        cursor += character.len_utf8();
    }
    if mentions == 0 {
        return false;
    }
    quoted_phrase(original)
        || normalized
            .split(|character: char| !character.is_alphanumeric() && character != '\'')
            .any(|word| {
                matches!(
                    word,
                    "not"
                        | "never"
                        | "don't"
                        | "cannot"
                        | "can't"
                        | "if"
                        | "when"
                        | "whether"
                        | "unless"
                        | "but"
                        | "and"
                        | "or"
                        | "then"
                        | "değil"
                        | "hayır"
                        | "yapma"
                        | "açma"
                        | "duraklatma"
                        | "sürdürme"
                        | "eğer"
                        | "ise"
                        | "ve"
                        | "veya"
                        | "ama"
                        | "sonra"
                )
            })
}

fn quoted_phrase(text: &str) -> bool {
    text.contains('"')
        || text.contains('‘')
        || text.contains('’')
        || text.contains('“')
        || text.contains('”')
        || (text.trim().starts_with('\'') && text.trim().ends_with('\''))
}

#[cfg(test)]
mod tests {
    use super::{
        classify_phrase, matches_ambiguous_command, response_prefix, DeterministicIntentRouter,
        PhraseIntent,
    };
    use crate::voice::contract::{
        AcceptedUtterance, AppIntentDescriptor, Language, OperationId, ReadinessSnapshot,
        ReadinessState, RouteKind, RouterDecision, TranscriptEligibility, UtteranceSource,
        VoiceOwner,
    };

    fn utterance(text: &str, language: Language) -> AcceptedUtterance {
        AcceptedUtterance {
            text: text.to_owned(),
            language,
            source: UtteranceSource::Typed,
            eligibility: TranscriptEligibility::Eligible,
        }
    }

    fn owner() -> VoiceOwner {
        VoiceOwner {
            session_id: "session".to_owned(),
            turn_id: "turn".to_owned(),
            generation: 1,
        }
    }

    fn route(text: &str, language: Language, readiness: &ReadinessSnapshot) -> RouterDecision {
        DeterministicIntentRouter.route(
            &utterance(text, language),
            readiness,
            &owner(),
            OperationId(1),
        )
    }

    #[test]
    fn all_declared_phrases_route_without_llm() {
        let cases = [
            ("status", Language::En),
            ("voice status", Language::En),
            ("provider status", Language::En),
            ("what is ready", Language::En),
            ("durum", Language::Tr),
            ("ses durumu", Language::Tr),
            ("sağlayıcı durumu", Language::Tr),
            ("neler hazır", Language::Tr),
            ("help", Language::En),
            ("show supported commands", Language::En),
            ("what can you do", Language::En),
            ("yardım", Language::Tr),
            ("desteklenen komutları göster", Language::Tr),
            ("ne yapabilirsin", Language::Tr),
            ("return to the core", Language::En),
            ("return to core", Language::En),
            ("focus", Language::En),
            ("ana ekrana dön", Language::Tr),
            ("çekirdeğe dön", Language::Tr),
            ("open conversation", Language::En),
            ("show conversation", Language::En),
            ("sohbeti aç", Language::Tr),
            ("konuşmayı aç", Language::Tr),
            ("pause animations", Language::En),
            ("pause animation", Language::En),
            ("resume animations", Language::En),
            ("resume animation", Language::En),
            ("animasyonları duraklat", Language::Tr),
            ("animasyonu duraklat", Language::Tr),
            ("animasyonları sürdür", Language::Tr),
            ("animasyonu sürdür", Language::Tr),
            ("set language to Turkish", Language::En),
            ("switch language to English", Language::En),
            ("dili Türkçe yap", Language::Tr),
            ("Türkçeye geç", Language::Tr),
            ("dili İngilizce yap", Language::Tr),
            ("İngilizceye geç", Language::Tr),
        ];
        let readiness = ReadinessSnapshot::setup_blocked();
        for (text, language) in cases {
            let result = route(text, language, &readiness);
            assert!(
                !matches!(result, RouterDecision::Unhandled { .. }),
                "phrase must not reach model generation: {text}"
            );
            assert_eq!(
                classify_phrase(&format!("please {text}.")),
                classify_phrase(text)
            );
            assert_eq!(
                classify_phrase(&format!("lütfen {text}!")),
                classify_phrase(text)
            );
        }
    }

    #[test]
    fn turkish_case_and_diacritics_match_without_synonym_expansion() {
        assert_eq!(classify_phrase("DURUM"), Some(PhraseIntent::Status));
        assert_eq!(
            classify_phrase("İNGİLİZCEYE GEÇ"),
            Some(PhraseIntent::SetEnglish)
        );
        assert_eq!(classify_phrase("IĞDIR"), None);
        assert_eq!(classify_phrase("dil tercihini değiştir"), None);
    }

    #[test]
    fn negated_multiple_quoted_and_conditional_commands_clarify() {
        for phrase in [
            "do not open conversation",
            "don't open conversation",
            "open conversation and pause animation",
            "open conversation or show conversation",
            "if I say focus, return to the core",
            "please tell me whether status is ready",
            "say \"focus\"",
            "status? help",
        ] {
            let normalized = crate::voice::text::normalize_for_matching(phrase);
            assert!(matches_ambiguous_command(&normalized, phrase), "{phrase}");
            assert!(matches!(
                route(phrase, Language::En, &ReadinessSnapshot::setup_blocked()),
                RouterDecision::Resolved(outcome)
                    if outcome.kind == RouteKind::Clarification && outcome.intent.is_none()
            ));
        }
    }

    #[test]
    fn explicit_out_of_scope_requests_have_no_intent_or_model_route() {
        for (phrase, language) in [
            ("open my files", Language::En),
            ("browse the internet", Language::En),
            ("run a shell command", Language::En),
            ("start listening", Language::En),
            ("tell me today's weather", Language::En),
            ("dosyalarımı aç", Language::Tr),
            ("internette gezin", Language::Tr),
            ("kabuk komutu çalıştır", Language::Tr),
            ("dinlemeyi başlat", Language::Tr),
            ("bugünün hava durumunu söyle", Language::Tr),
        ] {
            assert_eq!(classify_phrase(phrase), Some(PhraseIntent::OutOfScope));
            assert!(matches!(
                route(phrase, language, &ReadinessSnapshot::setup_blocked()),
                RouterDecision::Resolved(outcome)
                    if outcome.kind == RouteKind::Unavailable && outcome.intent.is_none()
            ));
        }
    }

    #[test]
    fn per_turn_language_prefix_does_not_change_mode_or_input_language() {
        assert_eq!(
            response_prefix("answer in turkish: What is a tree?"),
            Some((Language::Tr, "What is a tree?"))
        );
        assert_eq!(
            response_prefix("TÜRKÇE YANITLA: Explain this"),
            Some((Language::Tr, "Explain this"))
        );
        let mut readiness = ReadinessSnapshot::setup_blocked();
        readiness.llm.state = ReadinessState::Ready;
        assert!(matches!(
            route("answer in turkish: Tell me a short story", Language::En, &readiness),
            RouterDecision::Unhandled {
                utterance: request,
                response_language: Some(Language::Tr),
            } if request.language == Language::En && request.text == "Tell me a short story"
        ));
    }

    #[test]
    fn persistent_language_change_is_native_not_ui_acknowledged() {
        assert!(matches!(
            route("Türkçeye geç", Language::Tr, &ReadinessSnapshot::setup_blocked()),
            RouterDecision::LanguagePreference {
                language: Language::Tr,
                response: outcome,
            } if outcome.intent.is_none() && outcome.kind == RouteKind::Deterministic
        ));
    }

    #[test]
    fn app_actions_have_finite_arguments_and_no_premature_success_text() {
        assert!(matches!(
            route("pause animation", Language::En, &ReadinessSnapshot::setup_blocked()),
            RouterDecision::Resolved(outcome)
                if outcome.text.is_empty()
                    && matches!(&outcome.intent, Some(AppIntentDescriptor::Motion { enabled: false, .. }))
        ));
    }

    #[test]
    fn status_is_deterministic_and_reports_native_task_counts() {
        let mut readiness = ReadinessSnapshot::setup_blocked();
        readiness.routine_tasks.completed = 17;
        readiness.routine_tasks.without_llm = 15;
        assert!(matches!(
            route("status", Language::En, &readiness),
            RouterDecision::Resolved(outcome)
                if outcome.kind == RouteKind::Deterministic
                    && outcome.intent.is_none()
                    && outcome.text.contains("Completed routine tasks: 17")
                    && outcome.text.contains("without language-model inference: 15")
        ));
    }

    #[test]
    fn unsupported_text_is_honest_until_real_model_is_ready() {
        assert!(matches!(
            route("Explain why leaves change color", Language::En, &ReadinessSnapshot::setup_blocked()),
            RouterDecision::Resolved(outcome)
                if outcome.kind == RouteKind::Unavailable && outcome.intent.is_none()
        ));
        let mut readiness = ReadinessSnapshot::setup_blocked();
        readiness.llm.state = ReadinessState::Ready;
        assert!(matches!(
            route("Explain why leaves change color", Language::En, &readiness),
            RouterDecision::Unhandled { .. }
        ));
    }
}
