import type {
	AssetDigestStatus,
	AssetPurpose,
	AssetRetention,
	AssetState,
	AssetStatusCode,
	ReadinessCode,
	VoiceCaptureLifecycleState,
	VoiceErrorCode,
	VoiceLanguage,
	VoiceNoticeCode,
	VoicePhase,
} from "./voice";

const copy = {
	"voice.title": {
		en: "Voice conversation",
		tr: "Sesli konuşma",
	},
	"voice.description": {
		en: "Speak deliberately, review what Saturday heard, and read or hear the native response.",
		tr: "Bilinçli olarak konuşun, Saturday’nin duyduğunu gözden geçirin ve yerel yanıtı okuyun ya da dinleyin.",
	},
	"voice.start": { en: "Start voice conversation", tr: "Sesli konuşmayı başlat" },
	"voice.end_capture": { en: "Finish speaking", tr: "Konuşmayı bitir" },
	"voice.cancel": { en: "Cancel voice turn", tr: "Sesli turu iptal et" },
	"voice.clear_session": { en: "Clear conversation", tr: "Konuşmayı temizle" },
	"voice.listening": {
		en: "Listening. Audio stays on this device and is not saved.",
		tr: "Dinleniyor. Ses bu cihazda kalır ve kaydedilmez.",
	},
	"voice.transcribing": { en: "Transcribing on this device…", tr: "Bu cihazda yazıya dönüştürülüyor…" },
	"voice.review": { en: "Check the transcript before sending", tr: "Göndermeden önce metni kontrol edin" },
	"voice.thinking": { en: "Preparing a local response…", tr: "Yerel yanıt hazırlanıyor…" },
	"voice.speaking": { en: "Speaking", tr: "Yanıt seslendiriliyor" },
	"voice.canceling": { en: "Stopping audio and local processing…", tr: "Ses ve yerel işlem durduruluyor…" },
	"voice.language_choice": { en: "Choose Turkish or English", tr: "Türkçe veya İngilizce seçin" },
	"voice.model_unavailable": {
		en: "No local language model is ready. Open voice setup for open-ended replies.",
		tr: "Hazır bir yerel dil modeli yok. Serbest yanıtlar için sesli konuşma kurulumunu açın.",
	},
	"voice.voice_unavailable": {
		en: "No verified offline voice is ready for the selected language. Use text or open voice setup.",
		tr: "Seçilen dil için doğrulanmış çevrimdışı ses hazır değil. Metni kullanın veya sesli konuşma kurulumunu açın.",
	},
	"voice.permission_denied": {
		en: "Microphone access is off. Allow it in system settings, then try again.",
		tr: "Mikrofon erişimi kapalı. Sistem ayarlarından izin verip yeniden deneyin.",
	},
	"voice.device_lost": {
		en: "Microphone disconnected. The captured audio was discarded. Reconnect and start again.",
		tr: "Mikrofon bağlantısı kesildi. Yakalanan ses bırakıldı. Yeniden bağlayıp tekrar başlayın.",
	},
	"voice.duration_limit": {
		en: "The 30-second limit was reached. Check the transcript or record again.",
		tr: "30 saniyelik sınıra ulaşıldı. Metni kontrol edin veya yeniden konuşun.",
	},
	"voice.cleanup_failed": {
		en: "Voice could not stop safely. Restart the app before starting another turn.",
		tr: "Sesli konuşma güvenli biçimde durdurulamadı. Yeni bir tur başlatmadan önce uygulamayı yeniden açın.",
	},
	"voice.platform_unavailable": {
		en: "Native voice is not supported on this platform. No microphone or speech fallback is available.",
		tr: "Bu platform yerel sesli konuşmayı desteklemiyor. Mikrofon veya konuşma yedeği yok.",
	},
	"voice.browser_unavailable": {
		en: "Voice controls are unavailable in a browser. Open the Saturday desktop app to use native voice features.",
		tr: "Sesli konuşma kontrolleri tarayıcıda kullanılamaz. Yerel ses özellikleri için Saturday masaüstü uygulamasını açın.",
	},
	"voice.first_use": {
		en: "Start and finish each turn yourself. Each capture lasts at most 30 seconds. Audio is converted on this device and sent only to the private native ASR worker; it is not written to a file or WAV and is never sent to cloud services. Answers may be spoken aloud to nearby people.",
		tr: "Her turu kendiniz başlatıp bitirin. Her kayıt en fazla 30 saniye sürer. Ses bu cihazda dönüştürülür ve yalnızca özel yerel ASR çalışanına gönderilir; dosyaya veya WAV olarak yazılmaz ve bulut hizmetlerine gönderilmez. Yanıtlar yakındaki kişiler tarafından duyulabilir.",
	},
	"voice.consent_required": {
		en: "Starting a transfer requires a separate, named approval. Opening setup never downloads anything.",
		tr: "Bir aktarımı başlatmak için ayrıca, adı belirtilmiş bir onay gerekir. Kurulumu açmak hiçbir şeyi indirmez.",
	},
	"voice.setup_blocked": {
		en: "Setup is blocked. Required runtime, model, voice, trust, or review evidence is not ready.",
		tr: "Kurulum engellendi. Gerekli çalışma zamanı, model, ses, güven veya inceleme kanıtı hazır değil.",
	},
	"voice.approval_required": {
		en: "Approval is required before this item can be transferred or activated.",
		tr: "Bu öğe aktarılmadan veya etkinleştirilmeden önce onay gerekir.",
	},
	"voice.asset_details": { en: "Verified item details", tr: "Doğrulanmış öğe ayrıntıları" },
	"voice.disable_asset": { en: "Disable item", tr: "Öğeyi devre dışı bırak" },
	"voice.remove_asset": { en: "Remove app-managed item", tr: "Uygulama tarafından yönetilen öğeyi kaldır" },
	"voice.os_voice_management": {
		en: "Manage macOS speech voices in System Settings. Saturday does not download or remove OS voices.",
		tr: "macOS konuşma seslerini Sistem Ayarları’ndan yönetin. Saturday işletim sistemi seslerini indirmez veya kaldırmaz.",
	},
	"voice.asset_transfer_failed": { en: "The item was not transferred. It remains unavailable.", tr: "Öğe aktarılmadı. Kullanılamaz durumda kaldı." },
	"voice.asset_verification_failed": { en: "Verification failed. The item remains unavailable.", tr: "Doğrulama başarısız oldu. Öğe kullanılamaz durumda kaldı." },
	"voice.asset_removal_failed": { en: "The item could not be removed safely.", tr: "Öğe güvenli biçimde kaldırılamadı." },
	"voice.typed_limit": { en: "Typed requests are limited to 500 Unicode characters and 2,000 UTF-8 bytes. Nothing was sent.", tr: "Yazılı istekler 500 Unicode karakteri ve 2.000 UTF-8 baytı aşamaz. Hiçbir şey gönderilmedi." },
	"voice.transcript_limit": { en: "This transcript exceeds the 2,000-character or 8,000-byte limit. Edit it before sending.", tr: "Bu döküm 2.000 karakter veya 8.000 bayt sınırını aşıyor. Göndermeden önce düzenleyin." },
	"voice.invalid_unicode": { en: "This text contains invalid Unicode and was not sent.", tr: "Bu metin geçersiz Unicode içeriyor ve gönderilmedi." },
	"voice.no_speech": { en: "No speech was recognized. Nothing was sent. Start a new turn to try again.", tr: "Konuşma algılanmadı. Hiçbir şey gönderilmedi. Yeniden denemek için yeni bir tur başlatın." },
	"voice.unsupported_language": { en: "The language could not be confirmed. Choose Turkish or English and review the transcript.", tr: "Dil doğrulanamadı. Türkçe veya İngilizce seçip metni gözden geçirin." },
	"voice.mixed_language": { en: "The transcript contains mixed-language evidence. Choose a language and review it before sending.", tr: "Dökümde birden fazla dil kanıtı var. Göndermeden önce bir dil seçip gözden geçirin." },
	"voice.partial_answer": { en: "The response was interrupted. The visible text is incomplete.", tr: "Yanıt kesildi. Görünen metin tamamlanmadı." },
	"voice.answer_length_limited": { en: "The local response reached its length limit.", tr: "Yerel yanıt uzunluk sınırına ulaştı." },
	"voice.history_evicted": { en: "Older completed turns were removed to stay within the session limit.", tr: "Oturum sınırını korumak için eski tamamlanmış turlar kaldırıldı." },
	"voice.preference_unsaved": { en: "The preference was applied for this session but could not be saved.", tr: "Tercih bu oturum için uygulandı ancak kaydedilemedi." },
	"voice.speech_disclosure": {
		en: "Speech uses the selected macOS voice only after you activate playback. If it isn’t available, the answer remains readable and speech stops; Saturday never downloads or substitutes a voice.",
		tr: "Konuşma, yalnızca oynatmayı etkinleştirdiğinizde seçili macOS sesini kullanır. Ses kullanılamazsa yanıt okunabilir kalır ve konuşma durur; Saturday ses indirmez veya başka bir sesle değiştirmez.",
	},
	"voice.microphone_disclosure": {
		en: "Saturday requests macOS microphone access only after you choose Start voice conversation. Each capture is limited to 30 seconds. Audio is converted on this device and sent only through the private native ASR worker; it is not saved as a file or WAV, or sent to a cloud service. You must explicitly grant microphone access.",
		tr: "Saturday, mikrofon erişimini yalnızca Sesli konuşmayı başlat seçimini yaptıktan sonra macOS'tan ister. Her kayıt 30 saniyeyle sınırlıdır. Ses bu cihazda dönüştürülür ve yalnızca özel yerel ASR çalışanı üzerinden gönderilir; dosya veya WAV olarak kaydedilmez ve bulut hizmetine gönderilmez. Mikrofon erişimini açıkça vermeniz gerekir.",
	},
	"voice.language_mode": { en: "Voice language", tr: "Konuşma dili" },
	"voice.automatic": { en: "Choose automatically", tr: "Otomatik seç" },
	"voice.english": { en: "English", tr: "İngilizce" },
	"voice.turkish": { en: "Turkish", tr: "Türkçe" },
	"voice.input_device": { en: "Microphone", tr: "Mikrofon" },
	"voice.system_default": { en: "System default input", tr: "Sistem varsayılanı giriş" },
	"voice.select_device": { en: "Choose a microphone", tr: "Mikrofon seçin" },
	"voice.select_voice": { en: "Select exact voice", tr: "Tam sesi seçin" },
	"voice.spoken_reply": { en: "Speak answers aloud", tr: "Yanıtları seslendir" },
	"voice.review_before_send": { en: "Review every transcript before routing", tr: "Her dökümü yönlendirmeden önce gözden geçir" },
	"voice.submit_text": { en: "Send typed request", tr: "Yazılı isteği gönder" },
	"voice.type_placeholder": { en: "Type a request for the local assistant…", tr: "Yerel asistana bir istek yazın…" },
	"voice.transcript": { en: "Recognized text", tr: "Tanınan metin" },
	"voice.answer": { en: "Response", tr: "Yanıt" },
	"voice.choose_transcript_language": { en: "Confirm transcript language", tr: "Döküm dilini onaylayın" },
	"voice.confirm_transcript": { en: "Confirm and continue", tr: "Onayla ve devam et" },
	"voice.setup": { en: "Voice setup", tr: "Sesli konuşma kurulumu" },
	"voice.open_setup": { en: "Open voice setup", tr: "Sesli konuşma kurulumunu aç" },
	"voice.close": { en: "Close", tr: "Kapat" },
	"voice.error": { en: "Voice request failed. No fallback was used.", tr: "Sesli istek başarısız oldu. Yedek bir yöntem kullanılmadı." },
	"voice.starting": { en: "Starting microphone…", tr: "Mikrofon başlatılıyor…" },
	"voice.capture_elapsed": { en: "Capture elapsed", tr: "Kayıt süresi" },
	"voice.capture_maximum": { en: "30-second maximum", tr: "En fazla 30 saniye" },
	"voice.idle": { en: "No active voice turn.", tr: "Etkin bir sesli tur yok." },
	"voice.setup_required": { en: "Voice setup is required before capture is available.", tr: "Sesli kayıt kullanılmadan önce kurulum gerekir." },
	"voice.text_route_unavailable": { en: "Typed local requests are unavailable until the native router is ready.", tr: "Yerel yönlendirici hazır olana kadar yazılı yerel istekler kullanılamaz." },
	"voice.model_ready": { en: "Local model ready", tr: "Yerel model hazır" },
	"voice.asr_ready": { en: "On-device transcription ready", tr: "Cihaz üzerinde yazıya dönüştürme hazır" },
	"voice.speech_ready": { en: "Verified offline speech ready", tr: "Doğrulanmış çevrimdışı konuşma hazır" },
	"voice.provider_not_ready": { en: "Not ready", tr: "Hazır değil" },
	"voice.asset_not_ready": { en: "Not installed or not verified", tr: "Yüklenmedi veya doğrulanmadı" },
	"voice.source_transfer_notice": {
		en: "A transfer needs separate review and consent. Source code, dependencies, model weights, and macOS voices are separate items; setup does not start a transfer.",
		tr: "Aktarım için ayrı inceleme ve onay gerekir. Kaynak kod, bağımlılıklar, model ağırlıkları ve macOS sesleri ayrı öğelerdir; kurulum aktarım başlatmaz.",
	},
	"voice.local_privacy": {
		en: "Voice content stays in session memory only. No transcript, prompt, answer, or audio is stored in browser storage.",
		tr: "Sesli içerik yalnızca oturum belleğinde kalır. Döküm, istem, yanıt veya ses tarayıcı depolamasına kaydedilmez.",
	},
	"voice.language_ui": { en: "Interface language", tr: "Arayüz dili" },
	"voice.countdown": { en: "Elapsed seconds", tr: "Geçen saniye" },
	"voice.unknown_asset": { en: "Unrecognized item", tr: "Tanınmayan öğe" },
"ui.app_title": { en: "Saturday local assistant", tr: "Saturday yerel asistan" },
"ui.skip_main": { en: "Skip to main content", tr: "Ana içeriğe geç" },
"ui.open_palette": { en: "Open command palette", tr: "Komut paletini aç" },
"ui.open_voice": { en: "Open voice conversation", tr: "Sesli konuşmayı aç" },
"ui.open_conversation": { en: "Open conversation", tr: "Konuşmayı aç" },
"ui.send": { en: "Send request", tr: "İsteği gönder" },
"ui.typed_input": { en: "Request to the local assistant", tr: "Yerel asistana istek" },
"ui.typed_placeholder": { en: "Type a request for the local assistant…", tr: "Yerel asistana bir istek yazın…" },
"ui.command_palette": { en: "Command palette", tr: "Komut paleti" },
"ui.search_commands": { en: "Search commands", tr: "Komutları ara" },
"ui.search_placeholder": { en: "What would you like to do?", tr: "Ne yapmak istersiniz?" },
"ui.search_commands_help": { en: "Search and run local commands.", tr: "Yerel komutları arayın ve çalıştırın." },
"ui.matching": { en: "Matching commands", tr: "Eşleşen komutlar" },
"ui.quick_access": { en: "Quick access", tr: "Hızlı erişim" },
"ui.available": { en: "available", tr: "kullanılabilir" },
"ui.no_matches": { en: "No matching commands.", tr: "Eşleşen komut yok." },
"ui.navigation_help": { en: "Use arrows to navigate; Enter to select.", tr: "Oklarla gezinin; seçmek için Enter’a basın." },
"ui.local_commands": { en: "Local commands only", tr: "Yalnızca yerel komutlar" },
"ui.close_dialog": { en: "Close dialog", tr: "İletişim kutusunu kapat" },
"ui.interface": { en: "Saturday / interface", tr: "Saturday / arayüz" },
"ui.no_storage": { en: "Changes are session-only. Browser storage is unavailable.", tr: "Değişiklikler yalnızca oturum boyunca geçerlidir. Tarayıcı depolaması kullanılamıyor." },
"ui.saved_preferences_unreadable": { en: "Saved preferences were unreadable. Safe defaults are active.", tr: "Kaydedilen tercihler okunamadı. Güvenli varsayılanlar etkin." },
"voice.conversation_tab": { en: "Conversation", tr: "Konuşma" },
"voice.setup_tab": { en: "Setup", tr: "Kurulum" },
"voice.phase_unavailable": { en: "Unavailable", tr: "Kullanılamıyor" },
"voice.phase_setup_required": { en: "Setup required", tr: "Kurulum gerekli" },
"voice.phase_idle": { en: "Ready for an explicit request", tr: "Açık bir isteği bekliyor" },
"voice.phase_requesting_permission": { en: "Waiting for macOS microphone permission", tr: "macOS mikrofon izni bekleniyor" },
"voice.phase_listening": { en: "Listening on this device", tr: "Bu cihazda dinleniyor" },
"voice.phase_transcribing": { en: "Transcribing on this device", tr: "Bu cihazda yazıya dönüştürülüyor" },
"voice.phase_needs_review": { en: "Transcript review required", tr: "Döküm incelemesi gerekli" },
"voice.phase_routing": { en: "Routing the request locally", tr: "İstek yerel olarak yönlendiriliyor" },
"voice.phase_thinking": { en: "Preparing a local response", tr: "Yerel yanıt hazırlanıyor" },
"voice.phase_speaking": { en: "Speaking with the selected voice", tr: "Seçili sesle yanıt veriliyor" },
"voice.phase_cancelling": { en: "Stopping and confirming cleanup", tr: "Durduruluyor ve temizlik doğrulanıyor" },
"voice.phase_error": { en: "Voice operation needs attention", tr: "Sesli işlem ilgilenilmesini gerektiriyor" },
"voice.connecting": { en: "Connecting to native voice status…", tr: "Yerel ses durumuna bağlanılıyor…" },
"voice.not_connected": { en: "Native voice status is not connected.", tr: "Yerel ses durumu bağlı değil." },
"voice.browser_typed_unavailable": { en: "Typed assistant requests are also unavailable in this browser preview.", tr: "Bu tarayıcı önizlemesinde yazılı asistan istekleri de kullanılamıyor." },
"voice.no_active_turn": { en: "No active voice turn.", tr: "Etkin bir sesli tur yok." },
"voice.native_status": { en: "Native session status", tr: "Yerel oturum durumu" },
"voice.reselect_device": { en: "The saved microphone is unavailable. Choose a current native device again.", tr: "Kaydedilen mikrofon kullanılamıyor. Güncel bir yerel cihazı yeniden seçin." },
"voice.no_devices": { en: "No native microphone devices are listed.", tr: "Yerel mikrofon cihazı listelenmiyor." },
"voice.no_voices": { en: "No exact macOS voices are available for this language.", tr: "Bu dil için tam eşleşen macOS sesleri yok." },
"voice.select_exact_voice": { en: "Choose an exact voice; Saturday never substitutes another voice.", tr: "Tam eşleşen bir ses seçin; Saturday başka bir sesle değiştirmez." },
"voice.test_output": { en: "Test selected voice", tr: "Seçili sesi dene" },
"voice.voice_en": { en: "English speech voice", tr: "İngilizce konuşma sesi" },
"voice.voice_tr": { en: "Turkish speech voice", tr: "Türkçe konuşma sesi" },
"voice.transcript_language_required": { en: "Choose Turkish or English explicitly before continuing.", tr: "Devam etmeden önce Türkçe veya İngilizceyi açıkça seçin." },
"voice.review_text": { en: "Edit transcript", tr: "Dökümü düzenle" },
"voice.review_text_label": { en: "Transcript to send", tr: "Gönderilecek döküm" },
"voice.review_not_edited": { en: "Review is required by native recognition confidence; check the wording before continuing.", tr: "Yerel tanıma güveni nedeniyle inceleme gerekli; devam etmeden önce metni kontrol edin." },
"voice.text_invalid": { en: "The text is empty, invalid, or over its size limit. Nothing was sent.", tr: "Metin boş, geçersiz veya boyut sınırını aşıyor. Hiçbir şey gönderilmedi." },
"voice.request_rejected": { en: "The native assistant did not accept this request. No scripted response was used.", tr: "Yerel asistan bu isteği kabul etmedi. Senaryolu yanıt kullanılmadı." },
"voice.requesting_permission": { en: "macOS may ask for microphone permission after you start.", tr: "Başlattıktan sonra macOS mikrofon izni isteyebilir." },
"voice.capture_time": { en: "Capture time", tr: "Kayıt süresi" },
"voice.clear_confirm": { en: "Clear the native session conversation?", tr: "Yerel oturum konuşması temizlensin mi?" },
"voice.confirm_clear": { en: "Clear session", tr: "Oturumu temizle" },
"voice.intent_prompt": { en: "Saturday requests permission for this local action.", tr: "Saturday bu yerel eylem için izin istiyor." },
"voice.accept_intent": { en: "Allow this action", tr: "Bu eyleme izin ver" },
"voice.reject_intent": { en: "Reject this action", tr: "Bu eylemi reddet" },
"voice.assets_title": { en: "Native assets and readiness", tr: "Yerel öğeler ve hazırlık" },
"voice.refresh_assets": { en: "Refresh native item status", tr: "Yerel öğe durumunu yenile" },
"voice.loading": { en: "Loading native status…", tr: "Yerel durum yükleniyor…" },
"voice.no_assets": { en: "No native asset records were returned.", tr: "Yerel öğe kaydı döndürülmedi." },
"voice.asset_install": { en: "Review and request transfer", tr: "İnceleyip aktarım iste" },
"voice.asset_rollback": { en: "Restore this native version", tr: "Bu yerel sürümü geri yükle" },
"voice.asset_blocked": { en: "Blocked", tr: "Engellendi" },
"voice.asset_awaiting_approval": { en: "Eligible for separate approval", tr: "Ayrı onay için uygun" },
"voice.asset_ready": { en: "Ready according to native status", tr: "Yerel duruma göre hazır" },
"voice.asset_disabled": { en: "Disabled", tr: "Devre dışı" },
"voice.asset_recovery": { en: "Recovery required", tr: "Kurtarma gerekli" },
"voice.asset_not_reviewed": { en: "Not independently reviewed", tr: "Bağımsız olarak incelenmedi" },
"voice.asset_source_review": { en: "Source review is still required", tr: "Kaynak incelemesi hâlâ gerekli" },
"voice.asset_build_required": { en: "A reviewed native runtime build is still required", tr: "İncelenmiş yerel çalışma zamanı derlemesi hâlâ gerekli" },
"voice.asset_model_approval": { en: "Model approval is still required", tr: "Model onayı hâlâ gerekli" },
"voice.asset_transfer_not_granted": { en: "Transfer is not authorized", tr: "Aktarıma izin verilmedi" },
"voice.asset_missing": { en: "The item is not installed", tr: "Öğe yüklenmedi" },
"voice.asset_verified": { en: "Native verification passed", tr: "Yerel doğrulama başarılı" },
"voice.asset_status_disabled": { en: "The item is disabled", tr: "Öğe devre dışı" },
"voice.asset_recovery_required": { en: "Native recovery is required", tr: "Yerel kurtarma gerekli" },
"voice.asset_candidate": { en: "Candidate", tr: "Aday" },
"voice.asset_version": { en: "Version", tr: "Sürüm" },
"voice.asset_source": { en: "Source", tr: "Kaynak" },
"voice.asset_revision": { en: "Revision", tr: "Revizyon" },
"voice.asset_expected_size": { en: "Expected bytes", tr: "Beklenen bayt" },
"voice.asset_digest": { en: "SHA-256 digest", tr: "SHA-256 özeti" },
"voice.asset_license": { en: "License identifiers", tr: "Lisans tanımlayıcıları" },
"voice.asset_notices": { en: "Notices", tr: "Bildirimler" },
"voice.asset_retention": { en: "Retention policy", tr: "Saklama ilkesi" },
"voice.asset_transfer_limits": { en: "Staging / retained / disk limits", tr: "Hazırlık / saklanan / disk sınırları" },
"voice.asset_build_disclosure": { en: "Offline build disclosure", tr: "Çevrimdışı derleme açıklaması" },
"voice.asset_source_inputs": { en: "Pinned source inputs", tr: "Sabitlenmiş kaynak girdileri" },
"voice.asset_transfer_cancel": { en: "The native consent dialog controls transfer approval and cancellation.", tr: "Aktarım onayı ve iptali yerel izin iletişim kutusuyla yönetilir." },
"voice.status_not_inspected": { en: "Not inspected", tr: "İncelenmedi" },
"voice.status_source_review_required": { en: "Source review required", tr: "Kaynak incelemesi gerekli" },
"voice.status_runtime_build_required": { en: "Runtime build required", tr: "Çalışma zamanı derlemesi gerekli" },
"voice.status_asset_approval_required": { en: "Asset approval required", tr: "Öğe onayı gerekli" },
"voice.status_asset_unavailable": { en: "Asset unavailable", tr: "Öğe kullanılamıyor" },
"voice.status_provider_unavailable": { en: "Provider unavailable", tr: "Sağlayıcı kullanılamıyor" },
"voice.status_permission_required": { en: "Permission required", tr: "İzin gerekli" },
"voice.status_offline_evidence_required": { en: "Offline evidence required", tr: "Çevrimdışı kanıt gerekli" },
"voice.status_quality_evidence_required": { en: "Quality evidence required", tr: "Kalite kanıtı gerekli" },
"voice.status_ready": { en: "Ready", tr: "Hazır" },
"voice.source_origin_label": { en: "Origin", tr: "Kaynak adresi" },
"voice.source_tree_label": { en: "Source tree identity", tr: "Kaynak ağacı kimliği" },
"voice.transfer_ceiling": { en: "Maximum transfer bytes", tr: "En fazla aktarım baytı" },
"voice.observed_size": { en: "Observed bytes", tr: "Gözlenen bayt" },
"voice.recipe_identity": { en: "Build recipe", tr: "Derleme tarifi" },
"voice.toolchain_identity": { en: "Toolchain", tr: "Araç zinciri" },
"voice.offline_network_policy": { en: "Network policy: none; writes limited to the app-owned build workspace.", tr: "Ağ ilkesi: yok; yazma işlemleri uygulamaya ait derleme alanıyla sınırlı." },
"voice.delete_asset": { en: "Remove app-managed item", tr: "Uygulama yönetimindeki öğeyi kaldır" },
"voice.disable_asset_action": { en: "Disable native item", tr: "Yerel öğeyi devre dışı bırak" },
"voice.asset_action_failed": { en: "The native item action did not complete.", tr: "Yerel öğe işlemi tamamlanmadı." },
"voice.preference_saved": { en: "Preference sent to native session.", tr: "Tercih yerel oturuma gönderildi." },
"voice.auto_language_note": { en: "Auto language is uncalibrated. Native uncertainty may require transcript review.", tr: "Otomatik dil kalibre edilmemiştir. Yerel belirsizlik döküm incelemesi gerektirebilir." },
"voice.language_en": { en: "English", tr: "İngilizce" },
"voice.language_tr": { en: "Turkish", tr: "Türkçe" },
"voice.language_auto": { en: "Auto (uncalibrated)", tr: "Otomatik (kalibre edilmemiş)" },
"voice.text_bytes": { en: "UTF-8 bytes", tr: "UTF-8 baytı" },
"voice.cancelled": { en: "The voice turn was cancelled.", tr: "Sesli tur iptal edildi." },
"voice.busy": { en: "Another native voice operation is active.", tr: "Başka bir yerel ses işlemi etkin." },
"voice.timeout": { en: "The native operation timed out. No fallback was used.", tr: "Yerel işlemin süresi doldu. Yedek yöntem kullanılmadı." },
"voice.cleanup_pending": { en: "Native audio cleanup is still pending.", tr: "Yerel ses temizliği hâlâ bekliyor." },
"voice.tail_guard": { en: "Playback is stopping at the native safety boundary.", tr: "Oynatma yerel güvenlik sınırında durduruluyor." },
"voice.confirmation_required": { en: "This action needs your explicit approval.", tr: "Bu eylem için açık onayınız gerekiyor." },
"voice.asset_purpose_runtime": { en: "Runtime source", tr: "Çalışma zamanı kaynağı" },
"voice.asset_purpose_asr_weights": { en: "Speech recognition weights", tr: "Konuşma tanıma ağırlıkları" },
"voice.asset_purpose_llm_weights": { en: "Language model weights", tr: "Dil modeli ağırlıkları" },
"voice.asset_purpose_portable_voice": { en: "Portable voice", tr: "Taşınabilir ses" },
"voice.retention_app_managed": { en: "App-managed until you explicitly remove it", tr: "Açıkça kaldırana kadar uygulama tarafından yönetilir" },
"voice.retention_provenance": { en: "Build inputs retained for provenance", tr: "Kaynak doğrulanabilirliği için derleme girdileri saklanır" },
"voice.retention_removed_after_promotion": { en: "Removed after promotion", tr: "Yükseltme sonrasında kaldırılır" },
"voice.digest_not_published": { en: "Digest not published", tr: "Özet yayımlanmadı" },
"voice.digest_observed_unreviewed": { en: "Observed but not independently reviewed", tr: "Gözlendi ancak bağımsız incelenmedi" },
"voice.digest_independently_verified": { en: "Independently verified", tr: "Bağımsız olarak doğrulandı" },
"voice.ready_true": { en: "All required readiness checks report ready.", tr: "Gerekli tüm hazırlık denetimleri hazır bildiriyor." },
"voice.ready_false": { en: "The native service does not report the complete voice product ready.", tr: "Yerel hizmet tam ses ürününün hazır olduğunu bildirmiyor." },
"voice.routine_tasks": { en: "Native routine tasks completed / without a language model", tr: "Dil modeli olmadan tamamlanan / toplam yerel rutin görevler" },
"voice.default_device": { en: "Default", tr: "Varsayılan" },
"voice.no_voice_selected": { en: "No voice selected", tr: "Ses seçilmedi" },
"voice.asset_status_failed": { en: "Native asset status could not be read.", tr: "Yerel öğe durumu okunamadı." },
"voice.asset_redirects": { en: "Permitted redirect origins", tr: "İzin verilen yönlendirme kaynakları" },
"voice.no_redirects": { en: "None", tr: "Yok" },
"voice.retained_limit": { en: "Maximum retained bytes", tr: "En fazla saklanan bayt" },
"voice.build_workspace_limit": { en: "Maximum build workspace bytes", tr: "En büyük derleme çalışma alanı baytı" },
"voice.build_output_limit": { en: "Maximum build output bytes", tr: "En büyük derleme çıktısı baytı" },
"voice.build_cpu_limit": { en: "CPU limit", tr: "CPU sınırı" },
"voice.build_memory_limit": { en: "Memory limit bytes", tr: "Bellek sınırı baytı" },
"voice.build_deadline": { en: "Build deadline seconds", tr: "Derleme süre sınırı saniye" },
"voice.readiness_capture": { en: "Capture", tr: "Ses yakalama" },
"voice.readiness_asr": { en: "Speech recognition", tr: "Konuşma tanıma" },
"voice.readiness_router": { en: "Intent router", tr: "Niyet yönlendiricisi" },
"voice.readiness_llm": { en: "Language model", tr: "Dil modeli" },
"voice.readiness_speech": { en: "Speech output", tr: "Sesli yanıt" },
"voice.role_you": { en: "You", tr: "Siz" },
"voice.role_assistant": { en: "S.A.T.U.R.D.A.Y assistant", tr: "S.A.T.U.R.D.A.Y yanıtı" },
"voice.source_typed": { en: "Typed request", tr: "Yazılı istek" },
"voice.source_speech": { en: "Local speech transcript", tr: "Yerel konuşma dökümü" },
"voice.empty_history": { en: "No native conversation messages yet.", tr: "Henüz yerel konuşma iletisi yok." },
"voice.capture_level": { en: "Native capture level", tr: "Yerel ses yakalama düzeyi" },
"ui.switch_to_english": { en: "Switch interface to English", tr: "Arayüzü İngilizceye geçir" },
"ui.switch_to_turkish": { en: "Switch interface to Turkish", tr: "Arayüzü Türkçeye geçir" },
"voice.intent_overview": { en: "Return to the main view", tr: "Ana görünüme dön" },
"voice.intent_conversation": { en: "Open the conversation panel", tr: "Konuşma panelini aç" },
"voice.intent_motion_on": { en: "Enable motion effects", tr: "Hareket efektlerini aç" },
"voice.intent_motion_off": { en: "Reduce motion effects", tr: "Hareket efektlerini azalt" },
"voice.local_assistant": { en: "Native session", tr: "Yerel oturum" },
"voice.seconds": { en: "seconds", tr: "saniye" },
"ui.return_to_core": { en: "Return to core", tr: "Çekirdeğe dön" },
"ui.close_secondary_panels": { en: "Close secondary panels", tr: "İkincil panelleri kapat" },
"ui.open_voice_description": { en: "Open the native voice and text session", tr: "Yerel ses ve yazılı istek oturumunu aç" },
"ui.pause_motion": { en: "Pause animations", tr: "Animasyonları duraklat" },
"ui.resume_motion": { en: "Resume animations", tr: "Animasyonları sürdür" },
"ui.motion_description": { en: "Pause or resume interface motion", tr: "Arayüz hareketini duraklatın veya sürdürün" },
"voice.status_local_trust_setup_required": { en: "Local trust setup required", tr: "Yerel güven kurulumu gerekli" },
"voice.bootstrap_separate": { en: "Stage1/bootstrap remains a separate blocked trust ceremony, not a voice model, and cannot make voice readiness true.", tr: "Stage1/önyükleme ayrı ve engelli bir güven töreni olarak kalır; ses modeli değildir ve ses hazırlığını hazır duruma getiremez." },
} as const;

export type VoiceCopyKey = keyof typeof copy;
export const VOICE_COPY_KEYS = Object.keys(copy) as VoiceCopyKey[];

export function voiceCopy(language: VoiceLanguage, key: VoiceCopyKey): string {
	return copy[key][language];
}

const ERROR_KEYS: Record<VoiceErrorCode, VoiceCopyKey> = {
	invalid_request: "voice.error",
	unauthorized: "voice.error",
	unavailable: "voice.setup_blocked",
	setup_required: "voice.setup_required",
	busy: "voice.busy",
	stale_owner: "voice.cancelled",
	stale_catalog: "voice.reselect_device",
	unsupported: "voice.platform_unavailable",
	permission_denied: "voice.permission_denied",
	too_large: "voice.typed_limit",
	timeout: "voice.timeout",
	cancelled: "voice.cancelled",
	provider_failed: "voice.setup_blocked",
	protocol_error: "voice.error",
	sequence_exhausted: "voice.error",
	transfer_expired: "voice.approval_required",
	cleanup_failed: "voice.cleanup_failed",
	internal: "voice.error",
};

const NOTICE_KEYS: Record<VoiceNoticeCode, VoiceCopyKey> = {
	no_speech: "voice.no_speech",
	transcript_needs_review: "voice.review",
	transcript_overflow_review: "voice.transcript_limit",
	unsupported_language: "voice.unsupported_language",
	model_unavailable: "voice.model_unavailable",
	speech_unavailable: "voice.voice_unavailable",
	partial_answer: "voice.partial_answer",
	not_spoken: "voice.voice_unavailable",
	cleanup_pending: "voice.cleanup_pending",
	tail_guard: "voice.tail_guard",
	confirmation_required: "voice.confirmation_required",
};

const PHASE_KEYS: Record<VoicePhase, VoiceCopyKey> = {
	unavailable: "voice.phase_unavailable",
	setup_required: "voice.phase_setup_required",
	idle: "voice.phase_idle",
	requesting_permission: "voice.phase_requesting_permission",
	listening: "voice.phase_listening",
	transcribing: "voice.phase_transcribing",
	needs_review: "voice.phase_needs_review",
	routing: "voice.phase_routing",
	thinking: "voice.phase_thinking",
	speaking: "voice.phase_speaking",
	cancelling: "voice.phase_cancelling",
	error: "voice.phase_error",
};

const READINESS_KEYS: Record<ReadinessCode, VoiceCopyKey> = {
	not_inspected: "voice.status_not_inspected",
	source_review_required: "voice.status_source_review_required",
	runtime_build_required: "voice.status_runtime_build_required",
	asset_approval_required: "voice.status_asset_approval_required",
	owner_trust_setup_required: "voice.status_local_trust_setup_required",
	provider_unavailable: "voice.status_provider_unavailable",
	permission_required: "voice.status_permission_required",
	offline_evidence_required: "voice.status_offline_evidence_required",
	quality_evidence_required: "voice.status_quality_evidence_required",
	ready: "voice.status_ready",
};

const ASSET_STATE_KEYS: Record<AssetState, VoiceCopyKey> = {
	blocked: "voice.asset_blocked",
	awaiting_approval: "voice.asset_awaiting_approval",
	ready: "voice.asset_ready",
	disabled: "voice.asset_disabled",
	recovery_required: "voice.asset_recovery",
};

const ASSET_CODE_KEYS: Record<AssetStatusCode, VoiceCopyKey> = {
	not_reviewed: "voice.asset_not_reviewed",
	source_review_required: "voice.asset_source_review",
	runtime_build_required: "voice.asset_build_required",
	model_approval_required: "voice.asset_model_approval",
	owner_trust_setup_required: "voice.status_local_trust_setup_required",
	transfer_not_granted: "voice.asset_transfer_not_granted",
	asset_missing: "voice.asset_missing",
	verified: "voice.asset_verified",
	disabled: "voice.asset_status_disabled",
	recovery_required: "voice.asset_recovery_required",
};

const CAPTURE_LIFECYCLE_KEYS: Record<VoiceCaptureLifecycleState, VoiceCopyKey> = {
	permission_pending: "voice.requesting_permission",
	started: "voice.phase_listening",
	permission_denied: "voice.permission_denied",
	device_lost: "voice.device_lost",
	cancelled: "voice.cancelled",
	failed: "voice.error",
};

const ASSET_PURPOSE_KEYS: Record<AssetPurpose, VoiceCopyKey> = {
	runtime: "voice.asset_purpose_runtime",
	asr_weights: "voice.asset_purpose_asr_weights",
	llm_weights: "voice.asset_purpose_llm_weights",
	portable_voice: "voice.asset_purpose_portable_voice",
};

const ASSET_RETENTION_KEYS: Record<AssetRetention, VoiceCopyKey> = {
	app_managed_until_explicit_removal: "voice.retention_app_managed",
	build_inputs_retained_for_provenance: "voice.retention_provenance",
	removed_after_promotion: "voice.retention_removed_after_promotion",
};

const ASSET_DIGEST_KEYS: Record<AssetDigestStatus, VoiceCopyKey> = {
	not_published: "voice.digest_not_published",
	observed_unreviewed: "voice.digest_observed_unreviewed",
	independently_verified: "voice.digest_independently_verified",
};

export function voiceAssetPurposeCopyKey(purpose: AssetPurpose): VoiceCopyKey {
	return ASSET_PURPOSE_KEYS[purpose];
}

export function voiceAssetRetentionCopyKey(retention: AssetRetention): VoiceCopyKey {
	return ASSET_RETENTION_KEYS[retention];
}

export function voiceAssetDigestCopyKey(state: AssetDigestStatus): VoiceCopyKey {
	return ASSET_DIGEST_KEYS[state];
}

export function voiceErrorCopyKey(code: VoiceErrorCode): VoiceCopyKey {
	return ERROR_KEYS[code];
}

export function voiceNoticeCopyKey(code: VoiceNoticeCode): VoiceCopyKey {
	return NOTICE_KEYS[code];
}

export function voicePhaseCopyKey(phase: VoicePhase): VoiceCopyKey {
	return PHASE_KEYS[phase];
}

export function voiceReadinessCopyKey(code: ReadinessCode): VoiceCopyKey {
	return READINESS_KEYS[code];
}

export function voiceAssetStateCopyKey(state: AssetState): VoiceCopyKey {
	return ASSET_STATE_KEYS[state];
}

export function voiceAssetCodeCopyKey(code: AssetStatusCode): VoiceCopyKey {
	return ASSET_CODE_KEYS[code];
}

export function voiceCaptureLifecycleCopyKey(state: VoiceCaptureLifecycleState): VoiceCopyKey {
	return CAPTURE_LIFECYCLE_KEYS[state];
}
