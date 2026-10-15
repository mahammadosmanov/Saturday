import { describe, expect, it } from "vitest";
import { voiceAssetCodeCopyKey, voiceReadinessCopyKey, VOICE_COPY_KEYS, voiceCopy } from "./voice-copy";

const requiredKeys = [
	"voice.start",
	"voice.end_capture",
	"voice.cancel",
	"voice.listening",
	"voice.transcribing",
	"voice.review",
	"voice.thinking",
	"voice.speaking",
	"voice.canceling",
	"voice.language_choice",
	"voice.model_unavailable",
	"voice.voice_unavailable",
	"voice.permission_denied",
	"voice.device_lost",
	"voice.duration_limit",
	"voice.cleanup_failed",
	"voice.title",
	"voice.description",
	"voice.idle",
	"voice.setup_required",
	"voice.platform_unavailable",
	"voice.browser_unavailable",
	"voice.first_use",
	"voice.consent_required",
	"voice.setup_blocked",
	"voice.approval_required",
	"voice.asset_details",
	"voice.disable_asset",
	"voice.remove_asset",
	"voice.os_voice_management",
	"voice.asset_transfer_failed",
	"voice.asset_verification_failed",
	"voice.asset_removal_failed",
	"voice.typed_limit",
	"voice.transcript_limit",
	"voice.invalid_unicode",
	"voice.no_speech",
	"voice.unsupported_language",
	"voice.mixed_language",
	"voice.partial_answer",
	"voice.answer_length_limited",
	"voice.history_evicted",
	"voice.preference_unsaved",
	"voice.speech_disclosure",
	"voice.microphone_disclosure",
] as const;

describe("voice interface copy", () => {
	it("translates every required status, setup, privacy and error family independently", () => {
		for (const key of requiredKeys) {
			expect(VOICE_COPY_KEYS).toContain(key);
			const english = voiceCopy("en", key);
			const turkish = voiceCopy("tr", key);
			expect(english.trim().length).toBeGreaterThan(0);
			expect(turkish.trim().length).toBeGreaterThan(0);
			expect(turkish).not.toBe(english);
		}
	});

	it("shows local trust setup only for the exact native trust-setup statuses", () => {
		for (const language of ["en", "tr"] as const) {
			const readiness = voiceReadinessCopyKey("owner_trust_setup_required");
			const asset = voiceAssetCodeCopyKey("owner_trust_setup_required");
			expect(voiceCopy(language, readiness)).toContain(language === "en" ? "trust" : "güven");
			expect(voiceCopy(language, asset)).toBe(voiceCopy(language, readiness));
			expect(voiceReadinessCopyKey("asset_approval_required")).not.toBe(readiness);
		}
	});

	it("has no untranslated-key fallback in either language", () => {
		for (const key of VOICE_COPY_KEYS) {
			expect(voiceCopy("en", key)).not.toBe(key);
			expect(voiceCopy("tr", key)).not.toBe(key);
		}
	});
});
