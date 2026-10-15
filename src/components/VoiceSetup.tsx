import { useCallback, useEffect, useState } from "react";
import type {
	AssetCatalog,
	AssetStatus,
	ReadinessCode,
	VoiceInputDevice,
	VoiceLanguage,
	VoiceLanguageMode,
	VoicePreferences,
} from "../domain/voice";
import {
	voiceAssetCodeCopyKey,
	voiceAssetDigestCopyKey,
	voiceAssetPurposeCopyKey,
	voiceAssetRetentionCopyKey,
	voiceAssetStateCopyKey,
	voiceCopy,
	voiceReadinessCopyKey,
} from "../domain/voice-copy";
import type { VoiceSessionController } from "../hooks/useVoiceSession";

const DEFAULT_DEVICE = "__system_default__";
const RESELECT_DEVICE = "__reselect_required__";

function initialDeviceChoice(preferences: VoicePreferences, devices: VoiceInputDevice[]): string {
	const handle = preferences.input_device_handle;
	if (handle === null) return DEFAULT_DEVICE;
	return devices.some((device) => device.handle === handle) ? handle : RESELECT_DEVICE;
}

function isVoiceLanguageMode(value: string): value is VoiceLanguageMode {
	return value === "auto" || value === "en" || value === "tr";
}

function ReadinessCard({
	label,
	readiness,
	language,
}: {
	label: string;
	readiness: { code: ReadinessCode };
	language: VoiceLanguage;
}) {
	return (
		<div className="voice-readiness-item">
			<strong>{label}</strong>
			<span>{voiceCopy(language, voiceReadinessCopyKey(readiness.code))}</span>
		</div>
	);
}

function AssetCard({
	asset,
	language,
	busy,
	onInstall,
	onDisable,
	onDelete,
	onRollback,
}: {
	asset: AssetStatus;
	language: VoiceLanguage;
	busy: boolean;
	onInstall: (asset: AssetStatus) => void;
	onDisable: (asset: AssetStatus) => void;
	onDelete: (asset: AssetStatus) => void;
	onRollback: (asset: AssetStatus) => void;
}) {
	const disclosure = asset.asset_disclosure;
	const canRemove = disclosure.retention === "app_managed_until_explicit_removal";
	return (
		<article className="voice-asset-card">
			<div className="voice-asset-heading">
				<div>
					<h3>{disclosure.candidate_name}</h3>
					<p>{voiceCopy(language, voiceAssetPurposeCopyKey(asset.purpose))} · {disclosure.version}</p>
				</div>
			<div className="voice-asset-state">
				<strong>{voiceCopy(language, voiceAssetStateCopyKey(asset.status))}</strong>
				<span>{voiceCopy(language, voiceAssetCodeCopyKey(asset.code))}</span>
			</div>
			</div>
			<dl className="voice-disclosure-grid">
				<div><dt>{voiceCopy(language, "voice.source_origin_label")}</dt><dd>{disclosure.source_origin}</dd></div>
				<div><dt>{voiceCopy(language, "voice.source_tree_label")}</dt><dd>{disclosure.source_revision}</dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_expected_size")}</dt><dd>{disclosure.expected_bytes} {voiceCopy(language, "voice.text_bytes")}</dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_digest")}</dt><dd><code>{disclosure.digest_sha256}</code></dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_transfer_limits")}</dt><dd>{disclosure.max_staging_bytes} / {disclosure.max_retained_bytes} / {disclosure.max_disk_bytes} {voiceCopy(language, "voice.text_bytes")}</dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_retention")}</dt><dd>{voiceCopy(language, voiceAssetRetentionCopyKey(disclosure.retention))}</dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_license")}</dt><dd>{disclosure.license_ids.map((id) => <code key={id}>{id}</code>)}</dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_notices")}</dt><dd>{disclosure.notice_ids.map((id) => <code key={id}>{id}</code>)}</dd></div>
				<div><dt>{voiceCopy(language, "voice.asset_redirects")}</dt><dd>{disclosure.redirect_origins.length === 0 ? voiceCopy(language, "voice.no_redirects") : disclosure.redirect_origins.join(", ")}</dd></div>
				{disclosure.build && (
					<>
						<div><dt>{voiceCopy(language, "voice.recipe_identity")}</dt><dd>{disclosure.build.recipe_identity}</dd></div>
						<div><dt>{voiceCopy(language, "voice.toolchain_identity")}</dt><dd>{disclosure.build.toolchain_identity}</dd></div>
						<div><dt>{voiceCopy(language, "voice.asset_build_disclosure")}</dt><dd>{voiceCopy(language, "voice.offline_network_policy")}</dd></div>
						<div><dt>{voiceCopy(language, "voice.build_workspace_limit")}</dt><dd>{disclosure.build.maximum_workspace_bytes}</dd></div>
						<div><dt>{voiceCopy(language, "voice.build_output_limit")}</dt><dd>{disclosure.build.maximum_output_bytes}</dd></div>
						<div><dt>{voiceCopy(language, "voice.build_cpu_limit")}</dt><dd>{disclosure.build.cpu_limit}</dd></div>
						<div><dt>{voiceCopy(language, "voice.build_memory_limit")}</dt><dd>{disclosure.build.memory_limit_bytes}</dd></div>
						<div><dt>{voiceCopy(language, "voice.build_deadline")}</dt><dd>{disclosure.build.deadline_seconds}</dd></div>
						{disclosure.build.source_inputs.map((input) => (
							<div className="voice-source-input" key={`${input.name}:${input.revision}`}>
								<dt>{input.name} · {input.revision}</dt>
								<dd>
									{voiceCopy(language, "voice.source_origin_label")}: {input.origin}; {voiceCopy(language, "voice.source_tree_label")}: {input.source_tree_id}; {voiceCopy(language, "voice.transfer_ceiling")}: {input.maximum_transfer_bytes}; {voiceCopy(language, "voice.observed_size")}: {input.observed_bytes ?? "—"}; {voiceCopy(language, "voice.asset_digest")}: {input.archive_sha256 ?? voiceCopy(language, "voice.digest_not_published")}; {voiceCopy(language, voiceAssetDigestCopyKey(input.digest_status))}; {input.license_ids.join(", ")}; {input.notice_ids.join(", ")}
								</dd>
							</div>
						))}
					</>
				)}
			</dl>
			<p className="voice-asset-consent">{voiceCopy(language, "voice.asset_transfer_cancel")}</p>
			<div className="voice-asset-actions">
				{asset.status === "awaiting_approval" && (
					<button type="button" disabled={busy} onClick={() => onInstall(asset)}>
						{voiceCopy(language, "voice.asset_install")}
					</button>
				)}
				{asset.status === "ready" && (
					<button type="button" disabled={busy} onClick={() => onDisable(asset)}>
						{voiceCopy(language, "voice.disable_asset_action")}
					</button>
				)}
				{asset.status === "recovery_required" && (
					<button type="button" disabled={busy} onClick={() => onRollback(asset)}>
						{voiceCopy(language, "voice.asset_rollback")}
					</button>
				)}
				{canRemove && asset.status !== "blocked" && (
					<button type="button" disabled={busy} onClick={() => onDelete(asset)}>
						{voiceCopy(language, "voice.delete_asset")}
					</button>
				)}
			</div>
		</article>
	);
}

export function VoiceSetup({
	language,
	session,
	active,
}: {
	language: VoiceLanguage;
	session: VoiceSessionController;
	active: boolean;
}) {
	const { supported, status: sessionStatus, state, busy, assetsGetStatus, selectPreferences } = session;
	const snapshot = state?.snapshot;
	const preferences = snapshot?.metadata.preferences;
	const devices = snapshot?.input_devices ?? [];
	const voices = snapshot?.voices ?? [];
	const [assets, setAssets] = useState<AssetCatalog | null>(null);
	const [assetStatus, setAssetStatus] = useState<"idle" | "loading" | "failed">("idle");
	const [deviceChoice, setDeviceChoice] = useState(DEFAULT_DEVICE);
	const [deviceTouched, setDeviceTouched] = useState(false);
	const [languageMode, setLanguageMode] = useState<VoiceLanguageMode>("auto");
	const [preferenceError, setPreferenceError] = useState(false);

	useEffect(() => {
		if (preferences) {
			setLanguageMode(preferences.language_mode ?? "auto");
			if (!deviceTouched) setDeviceChoice(initialDeviceChoice(preferences, devices));
		}
	}, [preferences, devices, deviceTouched]);

	const refreshAssets = useCallback(async () => {
		if (!supported) {
			setAssets(null);
			setAssetStatus("failed");
			return;
		}
		setAssetStatus("loading");
		const result = await assetsGetStatus();
		if (result) {
			setAssets(result);
			setAssetStatus("idle");
		} else {
			setAssets(null);
			setAssetStatus("failed");
		}
	}, [assetsGetStatus, supported]);

	useEffect(() => {
		if (active && sessionStatus === "ready") void refreshAssets();
	}, [active, sessionStatus, refreshAssets]);

	const updatePreferences = useCallback(async (next: VoicePreferences) => {
		setPreferenceError(false);
		const result = await selectPreferences(next);
		if (!result?.accepted) setPreferenceError(true);
	}, [selectPreferences]);

	const updateDevice = (choice: string) => {
		setDeviceTouched(true);
		setDeviceChoice(choice);
		if (!preferences) return;
		const handle = choice === DEFAULT_DEVICE ? null : devices.find((device) => device.handle === choice)?.handle;
		if (handle === undefined) return;
		void updatePreferences({ ...preferences, input_device_handle: handle });
	};

	const updateMode = (next: VoiceLanguageMode) => {
		setLanguageMode(next);
		if (preferences) void updatePreferences({ ...preferences, language_mode: next });
	};

	const updatePreferenceFlag = (key: "spoken_reply" | "review_before_send", value: boolean) => {
		if (preferences) void updatePreferences({ ...preferences, [key]: value });
	};

	const updateVoice = (voiceLanguage: VoiceLanguage, voiceId: string | null) => {
		if (!preferences) return;
		void updatePreferences({ ...preferences, voice_ids: { ...preferences.voice_ids, [voiceLanguage]: voiceId } });
	};

	const installAsset = async (asset: AssetStatus) => {
		const outcome = await session.requestAssetInstall(asset);
		if (outcome?.accepted) await refreshAssets();
	};
	const disableAsset = async (asset: AssetStatus) => {
		if (await session.disableAsset(asset)) await refreshAssets();
	};
	const deleteAsset = async (asset: AssetStatus) => {
		const outcome = await session.deleteAsset(asset);
		if (outcome?.removed) await refreshAssets();
	};
	const rollbackAsset = async (asset: AssetStatus) => {
		if (await session.rollbackAsset(asset)) await refreshAssets();
	};

	if (!supported) {
		return (
			<section className="voice-setup" aria-labelledby="voice-setup-title">
				<h2 id="voice-setup-title">{voiceCopy(language, "voice.setup")}</h2>
				<p role="status">{voiceCopy(language, "voice.browser_unavailable")}</p>
			</section>
		);
	}

	return (
		<section className="voice-setup" aria-labelledby="voice-setup-title">
			<h2 id="voice-setup-title">{voiceCopy(language, "voice.setup")}</h2>
			<p>{voiceCopy(language, "voice.first_use")}</p>
			<p>{voiceCopy(language, "voice.microphone_disclosure")}</p>
			<p>{voiceCopy(language, "voice.speech_disclosure")}</p>
			<p>{voiceCopy(language, "voice.source_transfer_notice")}</p>
			<p>{voiceCopy(language, "voice.bootstrap_separate")}</p>
			<p>{voiceCopy(language, "voice.auto_language_note")}</p>
			{snapshot && (
				<>
					<p className="voice-product-readiness" role="status">
						{voiceCopy(language, snapshot.metadata.readiness.product_ready ? "voice.ready_true" : "voice.ready_false")}
					</p>
					<div className="voice-readiness-grid">
						<ReadinessCard label={voiceCopy(language, "voice.readiness_capture")} readiness={snapshot.metadata.readiness.capture} language={language} />
						<ReadinessCard label={voiceCopy(language, "voice.readiness_asr")} readiness={snapshot.metadata.readiness.asr} language={language} />
						<ReadinessCard label={voiceCopy(language, "voice.readiness_router")} readiness={snapshot.metadata.readiness.router} language={language} />
						<ReadinessCard label={voiceCopy(language, "voice.readiness_llm")} readiness={snapshot.metadata.readiness.llm} language={language} />
						<ReadinessCard label={voiceCopy(language, "voice.readiness_speech")} readiness={snapshot.metadata.readiness.speech} language={language} />
					</div>
					<p>{voiceCopy(language, "voice.routine_tasks")}: {snapshot.metadata.readiness.routine_tasks.without_llm} / {snapshot.metadata.readiness.routine_tasks.completed}</p>
				</>
			)}
			{preferences ? (
				<div className="voice-preferences">
					<label>
						{voiceCopy(language, "voice.language_mode")}
						<select
							value={languageMode}
							disabled={busy}
							onChange={(event) => {
								const value = event.currentTarget.value;
								if (isVoiceLanguageMode(value)) updateMode(value);
							}}
						>
							<option value="auto">{voiceCopy(language, "voice.language_auto")}</option>
							<option value="en">{voiceCopy(language, "voice.language_en")}</option>
							<option value="tr">{voiceCopy(language, "voice.language_tr")}</option>
						</select>
					</label>
					<label>
						{voiceCopy(language, "voice.input_device")}
						<select value={deviceChoice} disabled={busy} onChange={(event) => updateDevice(event.currentTarget.value)}>
							{deviceChoice === RESELECT_DEVICE && <option value={RESELECT_DEVICE} disabled>{voiceCopy(language, "voice.reselect_device")}</option>}
							<option value={DEFAULT_DEVICE}>{voiceCopy(language, "voice.system_default")}</option>
							{devices.map((device) => <option key={device.handle} value={device.handle}>{device.label}{device.is_default ? ` · ${voiceCopy(language, "voice.default_device")}` : ""}</option>)}
						</select>
					</label>
					<fieldset>
						<legend>{voiceCopy(language, "voice.select_exact_voice")}</legend>
						{(["en", "tr"] as const).map((voiceLanguage) => {
							const savedVoice = preferences.voice_ids[voiceLanguage];
							const selectedVoiceExists = savedVoice === null || voices.some((voice) => voice.id === savedVoice && voice.language === voiceLanguage);
							const selectedVoice = voices.find((voice) => voice.id === savedVoice && voice.language === voiceLanguage);
							return (
								<div className="voice-voice-choice" key={voiceLanguage}>
									<label>
										{voiceCopy(language, voiceLanguage === "en" ? "voice.language_en" : "voice.language_tr")}
										<select
											value={selectedVoiceExists ? savedVoice ?? "" : RESELECT_DEVICE}
											disabled={busy}
											onChange={(event) => updateVoice(voiceLanguage, event.currentTarget.value || null)}
										>
											{!selectedVoiceExists && <option value={RESELECT_DEVICE} disabled>{voiceCopy(language, "voice.select_exact_voice")}</option>}
											<option value="">{voiceCopy(language, "voice.no_voice_selected")}</option>
											{voices.filter((voice) => voice.language === voiceLanguage).map((voice) => <option key={voice.id} value={voice.id}>{voice.display_name}</option>)}
										</select>
									</label>
									{selectedVoice && (
										<button type="button" disabled={busy} onClick={() => void session.testOutput(selectedVoice)}>
											{voiceCopy(language, "voice.test_output")}
										</button>
									)}
								</div>
							);
						})}
					</fieldset>
					<label className="voice-checkbox"><input type="checkbox" checked={preferences.spoken_reply} disabled={busy} onChange={(event) => updatePreferenceFlag("spoken_reply", event.currentTarget.checked)} />{voiceCopy(language, "voice.spoken_reply")}</label>
					<label className="voice-checkbox"><input type="checkbox" checked={preferences.review_before_send} disabled={busy} onChange={(event) => updatePreferenceFlag("review_before_send", event.currentTarget.checked)} />{voiceCopy(language, "voice.review_before_send")}</label>
					{preferenceError && <p role="alert">{voiceCopy(language, "voice.preference_unsaved")}</p>}
				</div>
			) : <p role="status">{voiceCopy(language, "voice.loading")}</p>}
			<div className="voice-device-summary">
				<h3>{voiceCopy(language, "voice.input_device")}</h3>
				{devices.length === 0 ? <p>{voiceCopy(language, "voice.no_devices")}</p> : <ul>{devices.map((device) => <li key={device.handle}>{device.label}{device.is_default ? ` · ${voiceCopy(language, "voice.default_device")}` : ""}</li>)}</ul>}
			</div>
			<div className="voice-assets-section">
				<div className="voice-section-heading">
					<h3>{voiceCopy(language, "voice.assets_title")}</h3>
					<button type="button" disabled={assetStatus === "loading" || busy} onClick={() => void refreshAssets()}>{voiceCopy(language, "voice.refresh_assets")}</button>
				</div>
				{assetStatus === "loading" && <p role="status">{voiceCopy(language, "voice.loading")}</p>}
				{assetStatus === "failed" && <p role="alert">{voiceCopy(language, "voice.asset_status_failed")}</p>}
				{assets && assets.assets.length === 0 && <p>{voiceCopy(language, "voice.no_assets")}</p>}
				{assets?.assets.map((asset) => (
					<AssetCard key={asset.asset_id} asset={asset} language={language} busy={busy}
						onInstall={(item) => void installAsset(item)}
						onDisable={(item) => void disableAsset(item)}
						onDelete={(item) => void deleteAsset(item)}
						onRollback={(item) => void rollbackAsset(item)}
					/>
				))}
			</div>
		</section>
	);
}
