# Speak with Saturday in Turkish and English

Date: 2026-10-06
Status: **User-approved written specification for implementation planning (2026-10-06).** The implementation plan requires separate owner review. This approval authorizes no implementation, acquisition, installation, microphone capture, model inference, key provisioning, signing, or release.
Audience: the owner reviewing the voice experience, then engineers and reviewers defining its security and lifecycle contracts.

This design specifies a complete local voice turn: deliberately activate the microphone, speak Turkish or English, see the recognized text, receive a deterministic supported response or a real local-model answer, and hear that answer through local speech synthesis. It preserves Saturday's broader modular work-partner goal without claiming that the current scripted preview or microphone animation already delivers conversation. Written-spec approval was received on 2026-10-06 for planning; [`../plans/2026-10-06-bilingual-voice.md`](../plans/2026-10-06-bilingual-voice.md) needs separate owner review before product work. The owner has already selected 6.1 Sol orchestration with six `openai-codex/gpt-6-luna` agents and no model fallback.

## 1. Intent, approval, and scope

The owner prioritizes listening, understanding, and answering aloud in Turkish and English. The conversational reply approved the design brief; on 2026-10-06 the owner separately approved this written specification for implementation planning. Neither approval grants product implementation or acquisition authority.

Saturday's larger goal remains a modular, secure, fast, customizable, cross-platform, aesthetically excellent, low-cost local-first work partner. At least 99% of routine task execution should avoid large language model (LLM) inference where deterministic execution is possible. This voice slice establishes one trustworthy input/output path; it does not replace that goal with a scripted chatbot or expand into machine control.

Included proposed behavior:

- Explicit click-to-start/click-to-finish and app-local push-to-talk, with a separate cancel control.
- Bounded, ephemeral audio capture and local automatic speech recognition (ASR).
- Turkish, English, and conservative automatic language selection; visible editable transcript.
- Deterministic routing before any LLM invocation, with bilingual responses for a declared supported intent set.
- Optional, deliberately provisioned real local LLM for open-ended conversation.
- Local text-to-speech (TTS), visible response text, actual playback state, and immediate interruption.
- Honest setup, permission, unsupported-platform, unavailable-provider, and failure states.
- Modular native provider interfaces, deny-by-default authority, bounded resources, and verified cleanup.

Not included: always-on microphone, wake word, automatic relistening, acoustic barge-in, speaker identification, persistent conversation memory, file indexing, filesystem actions, external accounts, network inference, paid services, arbitrary plugins, tool execution, or computer control. Follow-on features still require their own reviewed contracts and authority.

## 2. Current source baseline, not future claims

The inspected checkout is `~/Documents/GitHub/Saturday`. The following paths are repository-relative; older dated documents retain historical names and results that must not be treated as current verification.

| Current evidence | What it establishes |
| --- | --- |
| `AGENTS.md`, `ARCHITECTURE.md` | Trusted compile-time modules are organizational boundaries, not sandboxes. New native authority and voice behavior require contracts and executable evidence. |
| `src/App.tsx`: `voice`, `useSystemDefault`, panel effects | Current microphone activation cancels scripted conversation; opening any panel stops capture. This wiring cannot serve the proposed integrated voice drawer unchanged. |
| `src/hooks/useMicrophone.ts`: `useMicrophone`, `release` | User-triggered Web Audio loudness/spectrum measurement with late-permission fencing, track cleanup, exact selected-device handling, and hidden/pagehide cleanup. It does not transcribe or play speech. |
| `src/domain/audio.ts`: `analyzeAudio`; `src-tauri/Info.plist`: `NSMicrophoneUsageDescription` | The current RMS display gate is not voice activity detection. The current native permission explanation promises visualization only; a future voice cutover must replace that explanation honestly before requesting transcription capture. |
| `src/domain/conversation.ts`: `ConversationProvider`, `ConversationRuntime`, `createScriptedConversationProvider` | Provider-neutral text contract with `AbortSignal`; current provider is scripted and accepts a trimmed 500-character request. `ConversationReply.action` currently permits only `focus`. |
| `src/hooks/useConversation.ts`: `send`, `endOwnership`, `cancel` | Renderer-owned scripted request generations, late-result suppression, up to 40 session messages, and timer-driven visual phases. The timer is not evidence of actual synthesis or playback. |
| `src/components/Core.tsx`: `CoreState`; `src/platform/builtin-command-modules.ts`: `buildBuiltInCommands` | Current phases are `standby/listening/processing/responding`; four palette commands are `overview`, `conversation`, `voice`, and `motion`. |
| `src/platform/module-registry.ts`: `BuiltInModuleRegistry` | Static trusted registration, transactional activation rollback, and contribution disposal patterns to retain. |
| `src-tauri/src/main.rs`: `main`; `src-tauri/tauri.conf.json` | No application invoke handler and empty capability list. Native owner CLI/self-test modes are not renderer capabilities. Content Security Policy currently denies webview media resources. |
| `src-tauri/src/signed_helper/model_provider.rs`: `ModelBackend`, `ModelProvider`, `ProviderState` | Dormant bounded model state-machine machinery, not an active inference backend. Cancellation tracking here is a reusable contract, not proof of a working voice provider. |
| `src-tauri/src/signed_helper/supervisor.rs`: `run_bundled_self_test`; `src-tauri/src/helper/sandbox.rs`: `DENY_NETWORK_PROFILE` | Directly owned, authenticated dormant helper self-test, with network/fork denial and bounded cleanup. Its current allow-default profile is not a complete filesystem sandbox. |
| `src-tauri/src/owner_authorization/issuer.rs`, `binding.rs`; `src-tauri/resources/stage1-owner-public-key.json`, `stage1-owner-binding.json` | Native owner authorization source exists, but tracked resources are `unprovisioned` and `unfrozen`; `ModelTransferState` allows only `Blocked`. No speech acquisition authority follows from this source. |
| `docs/local-model-feasibility.md`, `docs/local-model-candidates.md`, `docs/spikes/005-prebuilt-runtime-candidate/README.md`, `.rps/STATE.md` | Historical local-model experiments and current metadata-only candidate remain no-go for model integration/transfer. A failed source-build ownership path cannot be reused. |

The coordinator observed Apple M5 and 17,179,869,184 bytes of RAM, or 16 GiB. This is host inventory, not a voice/model performance benchmark. A harmless `say -v '?'` inventory listed `Yelda tr_TR`, `Samantha en_US`, and other voices. Listing does not prove installed usable assets, equivalence with native API enumeration, offline synthesis, pronunciation quality, or successful playback. No test speech or microphone audio was produced for this design.

## 3. Chosen approach and alternatives

The proposed approach is a native-owned, half-duplex voice session with replaceable ASR, conversation, and TTS adapters. The renderer displays state and sends narrow commands; it does not own audio, launch inference, choose arbitrary files, or mint capabilities.

| Approach | Decision and trade-off |
| --- | --- |
| Native capture, local Whisper-family ASR, deterministic router, optional private local LLM worker, platform TTS | Recommended. Preserves a cross-platform interface and local data boundaries while allowing the first physical target to be macOS arm64. Requires reviewed native capture and worker contracts. |
| Renderer capture plus browser speech-recognition/synthesis APIs | Not selected for product voice. Browser fixtures remain useful for UI tests, but browser recognition can depend on an external service, voice locality varies, and JavaScript capture would undermine the native-only audio authority chosen here. |
| Cloud conversation or speech provider, always-on wake word, or a separately installed background inference daemon | Rejected for this slice. These introduce privacy, operating cost, lifecycle, or false-activation risks contrary to the approved brief. |

Half-duplex means capture ends before response playback begins. Each new microphone turn needs a new deliberate action; finishing speech does not arm the next capture. A person can interrupt playback with **Cancel** or an explicit new push-to-talk action, but Saturday never listens for an acoustic interruption while speaking.

## 4. Component responsibilities and boundaries

All names below describe proposed interfaces, not existing source symbols.

| Component | Owns | Depends on and exposes |
| --- | --- | --- |
| `VoiceSessionSupervisor` in Rust | Sole session state, generations, turn ordering, resource budgets, cancellation, provider selection, and cleanup receipts | Receives validated renderer commands; controls capture/router/providers; emits revisioned snapshots and bounded events. |
| `NativeCaptureProvider` | OS permission request, selected-device handle, capture clock, resampling, bounded PCM memory, input-end/disconnection detection | OS capture API and ephemeral native buffers. Returns audio handles to the supervisor and amplitude-only meter events. |
| `LocalAsrProvider` | Approved ASR identity, local transcribe operation, language evidence, bounded transcript result | Read-only approved weights, ephemeral PCM handle, deadline and cancellation context. No file output or renderer-supplied audio. |
| `DeterministicIntentRouter` | Bilingual normalization, exact supported grammar, ambiguity decisions, response templates, and approved app-local intent descriptors | Accepted transcript and explicit language. Returns handled/clarify/unhandled without invoking the LLM. |
| `LocalConversationProvider` | Real local text generation with pinned model/template/runtime identity | Bounded completed conversation context and cancellation context. Emits text only, never action authority. |
| `LocalSpeechProvider` | Selected locale/voice, bounded synthesis/playback queue, actual start/end/cancel callbacks | Accepted answer segments and output language. No renderer media URL, cloud fallback, or shell command. |
| `AssetCatalog` and native asset manager | Readiness, digest/license/provenance metadata, approved selection, disable/removal and recovery | Reviewed purpose-bound acquisition authority, app-owned storage, and verified file identities. Acquisition is a separate gate from session IPC. |
| React voice facade | Controls, transcript editing, notices, preferences, accessible rendering, and amplitude visualization | Revisioned native snapshots and narrow command results. It owns no independent voice phase or native permission. |
| Trusted built-in voice module | Discoverable palette/panel contributions and disposal of subscriptions | Existing trusted registry context with narrow voice facade; deactivation cancels the native session before disposal completes. |

Audio capture and playback remain native; raw PCM never traverses Tauri IPC into JavaScript. An ASR worker receives only an ephemeral owned audio mapping/descriptor. The model worker receives only text and read-only approved model data. No worker receives the renderer, keychain credentials, transfer token, arbitrary path, network, process-fork, or machine-control capability.

Inference must run off both the webview thread and Tauri's main thread. A directly owned, nonprivileged worker process contains native model parser crashes and permits hard termination. It uses a fixed signed executable, fixed structured arguments, private length-framed transport, exact nested schemas, bounded queues, operation challenges, and live process identity verification. It exposes no port, HTTP server, daemon, or third-party plugin loader. The existing signed health helper stays dormant until a separately reviewed changed helper identity and inference contract pass; adding a provider trait does not activate it.

The current helper's network/fork denial is retained as a minimum, not called a full sandbox. Production inference requires explicit read-only model access and deny-by-default filesystem access except reviewed OS/GPU requirements. Each allowance needs native evidence. No privileged or long-lived service is needed for this slice; XPC/audit-token authentication remains a gate if a later design introduces one. Windows and Linux need their own process identity, containment, and cleanup proof rather than a claimed portable macOS sandbox.

The supervisor enforces `microphone.capture` and `audio.speak` outside the renderer, with private provider/model-read grants restricted to approved asset handles. The authorized webview window/origin and validated app state are necessary but do not prove a human click against compromised JavaScript. Native first-use consent, OS microphone permission, a visible capture indicator, duration limits, native window-loss cancellation, and a native stop path remain independent protections. Do not describe a renderer-supplied `user_gesture` flag as trusted authorization.

## 5. Session, turn, and IPC contract

The supervisor has one active session per application, bound to the authorized main window. Native-generated `session_id`, `turn_id`, and monotonic `generation` identify ownership; `revision` orders state snapshots. IDs are lifecycle identifiers, not bearer authority. Every operation and event includes its owning tuple and protocol version.

### Commands from the authorized renderer

| Proposed operation | Required behavior |
| --- | --- |
| `voice.get_status()` | Read an honest readiness/session snapshot; never starts capture or loads a model. |
| `voice.start({language_mode, input_device_handle})` | Native checks visible authorized window, readiness, explicit activation eligibility, grants, and idle ownership before requesting capture. Missing capabilities fail closed. Device handles come from native inventory, not paths. |
| `voice.finish({session_id, turn_id, generation})` | Valid only while listening; releases the microphone, finalizes the bounded utterance, and starts local transcription. Empty/no-speech input never reaches the router or LLM. |
| `voice.submit_text({text, language_mode})` | Uses the same routing/context/cancellation pipeline without microphone access. Enforces bounded input and rejects a busy owner. A text turn does not speak automatically unless spoken replies are explicitly enabled. |
| `voice.confirm_transcript({owner, text, language})` | Valid only in `needs_review`; validates edited text and an explicit `tr` or `en` choice before routing. |
| `voice.cancel({owner, reason})` | Idempotently revokes the active generation, releases capture, cancels inference, stops actual playback, discards pending segments, and reports cleanup status. Native hide/close/shutdown hooks perform the same operation without trusting a renderer event. |
| `voice.clear_session()` | Cancels, verifies cleanup, removes in-memory transcript/history/context, and opens no new microphone session. |
| `voice.select_preferences({language_mode, input_device_handle, voice_ids, spoken_reply})` | Validates catalog identifiers. Any active turn is canceled before changing providers/devices/language. Does not install assets or grant authority. |

Asset inventory and removal use separate narrow native operations once their contract is approved. Session IPC never accepts a runtime path, model URL, digest override, shell string, native executable, arbitrary voice data, or acquisition authorization. Repeated/extra fields, wrong protocol versions, stale ownership, oversized payloads, invalid enum values, foreign windows, and duplicate non-idempotent commands fail before effects. A busy session requires cancel-and-cleanup before a new turn; there is no hidden request queue.

Snapshots contain phase, owner, revision, language mode/resolved language, catalog readiness, cancel availability, route kind, output progress, and fixed error/recovery codes. Bounded transcript/answer deltas have increasing sequence numbers; unknown owners, duplicate/out-of-order sequence numbers, and late generations cannot mutate UI or audio. The renderer resynchronizes through a full snapshot after subscription loss. Speech text is rendered as text, not executable HTML/Markdown actions.

Bound structured control frames to 65,536 bytes and individual text deltas to 4,096 bytes, consistent with the existing helper framing limits, but do not treat its current health protocol as an ASR protocol. Cap pending control/text events at 64 records; a full queue applies backpressure or fails safely, never drops a terminal/cancel event. Coalesce meter updates separately. Fresh worker challenges and monotonically increasing request IDs prevent replay; a snapshot gap triggers resynchronization, not a renderer-created phase.

### State transitions

| Phase | Meaning and permitted transition |
| --- | --- |
| `unavailable` / `setup_required` | Platform or catalog cannot provide the requested capability. Setup/status and typed deterministic help remain available. No capture, false listening, or fallback inference. |
| `idle` | No microphone, active generation, or playback. Explicit start goes to `requesting_permission`; typed submit goes to `routing`. |
| `requesting_permission` | OS permission may be pending. Cancel invalidates ownership; a late permission result is immediately released. Success with a live device goes to `listening`. |
| `listening` | Actual capture is live, with visible timer and meter. Finish/PTT release goes to `transcribing`; cancel discards audio. A hard duration limit stops capture and marks the result for review. |
| `transcribing` | Microphone is already released; local ASR owns only bounded ephemeral audio. Successful eligible transcript goes to `routing`; ambiguity/truncation goes to `needs_review`; no-speech goes to a recoverable notice. |
| `needs_review` | Capture and inference are stopped. Display the transcript/reason; confirm/edit routes it, retry requires a new explicit capture, cancel discards the pending turn. |
| `routing` | Deterministic parser checks accepted text first. A handled result produces response text; ambiguity requests clarification; unhandled text may go to `thinking` only with enabled ready local LLM. |
| `thinking` | Actual local answer generation or first speech segment preparation is pending. Streaming text appears without pretending playback has started. |
| `speaking` | Actual local playback has started; bounded generation may still produce later answer segments. Completion waits for both answer finalization and playback drain. Missing/broken TTS leaves readable text and an explicit speech failure, not a false speaking phase. |
| `cancelling` | Ownership is revoked and real resources are stopping. No new start is accepted until cleanup proves safe. |
| `error` | A fixed recoverable failure is visible. Resource-clean error can retry explicitly; unproven cleanup locks voice until safe process termination or application restart. |

A completed voice turn returns to `idle`, not `listening`. The conversation drawer is part of the voice surface: opening it must not cancel its own voice turn. Opening the unrelated command palette, leaving the voice surface, hiding/minimizing the native window, locking/suspending the session, pagehide/reload, module disable, app close, or shutdown cancels. A voice-routed harmless navigation command is serialized with its own response; it must not accidentally trigger the unrelated-navigation cancellation rule.

## 6. Language and transcript behavior

Language mode is `tr`, `en`, or `auto`. Default follows an explicitly stored choice; with no choice, show the app's Turkish/English language selector before the first microphone turn. Do not silently infer a permanent preference from OS locale, accent, microphone name, or the last model reply.

- In `tr` or `en`, ASR transcribes that language without translating the utterance. Response templates, model instructions, and TTS follow the selected request language unless the person explicitly asks for the other supported output language.
- In `auto`, obtain ASR language evidence across supported and unsupported languages, then accept only a confident Turkish or English result. A low-confidence, short, mixed, or unsupported-language result asks for `Türkçe` or `English` before routing. Do not force every non-English utterance into Turkish.
- The provider must expose the underlying language ranking, score gap, no-speech evidence, and transcription quality flags, not a fabricated universal confidence percentage. Freeze eligibility thresholds against the bilingual/no-speech corpus before enabling auto mode. Until that evidence exists, auto requests require an explicit language choice/review; explicit `tr`/`en` can still pass their own verified transcription gates.
- Mixed-language speech stays visible as recognized text. The person chooses the response language in review; no mid-utterance model/voice switching or silent translation occurs.
- An explicit language request changes output language for that turn. Changing the persistent session mode requires the language control or the unambiguous supported language-setting intent.
- Ambiguous recognition cannot execute even a harmless app intent. A visible transcript enables correction; the original audio is not retained for review or retry.

After an eligible transcript completes, display it and proceed to the answer automatically. There is no mandatory second **Send** click for ordinary speech. Review is reserved for ambiguity, truncation, or an explicitly enabled review-before-send preference. Manual edits require **Send** and are treated as typed text with an explicit language choice.

The domain router uses Unicode normalization, whitespace normalization, and Turkish-aware casing (`I/ı`, `İ/i`). It preserves the original display text. It matches complete allowed phrases, not substring guesses or generated tool-call JSON. Every proposed voice control, setup/error notice, transcript label, command reply, and privacy disclosure must have independently reviewed Turkish and English text before release. The examples below are proposed copy, not a claim that localization exists today. Proper names, numbers, punctuation, apostrophes, Turkish diacritics, and English loanwords belong in the verification corpus.

## 7. Deterministic understanding and real conversation

The router distinguishes `handled`, `needs_clarification`, and `unhandled`. Only `unhandled` conversational text can invoke the optional LLM. Missing/disabled LLM produces an honest unavailable answer and shows the supported deterministic commands; it never returns a scripted imitation of general intelligence.

Initial deterministic intents are app-local only: report current voice/provider readiness, show supported commands, open conversation, return to the compact core, pause/resume animation, and explicitly select Turkish/English language mode. Grammar and IDs must map to the existing trusted `overview`, `conversation`, and `motion` contributions where applicable. Microphone activation always needs a deliberate UI/PTT action; a transcript or model reply cannot arm capture. **Cancel** is a UI/PTT control, not a requirement to recognize speech while listening or speaking.

Harmless UI changes require a native-validated intent descriptor and a renderer acknowledgement before saying they succeeded. They cannot produce filesystem, network, process, credential, or automation authority. Unknown, negated, multiple, or ambiguous commands request clarification or fall into conversation; substring matches such as the current scripted `/focus/` behavior are not the proposed parser.

| Accepted text | Required interpretation |
| --- | --- |
| `Return to the core` / `Ana ekrana dön` | The finite app-local core-navigation intent, with acknowledged UI state. |
| `Don't return to the core` / `Ana ekrana dönme` | No navigation; negation cannot trigger a substring command. |
| `Focus and tell me today's weather` | No partial navigation or fabricated weather. Explain unavailable live weather; clarify the requested app-local command if appropriate. |
| `What is photosynthesis?` / `Fotosentez nedir?` | Open-ended informational answer from a real enabled local LLM; absent model means honest setup/unavailable. |
| `Open my files` / `Dosyalarımı aç` | Explicit out-of-scope result; no filesystem action, permission prompt, or model tool execution. |

`LocalConversationProvider` accepts accepted text, explicit response language, bounded completed turn context, approved provider identity, token limits, deadline, and cancellation ownership. It returns ordered answer text, completion, usage counters, or typed failure. It has no `action` field. The future cutover must remove generated-output action handling from real conversation consumers rather than allowing the current `ConversationReply.action` to carry native authority.

The shared normalized request is `utterance{text, source:typed|speech, language:tr|en|und}`, explicit `response_language:tr|en`, bounded completed turns, provider identity, limits, and owner tuple. The routing layer returns a discriminated `answer{text, language, source:deterministic|local_model}`, `app_intent{allowed_id, validated_arguments}`, `clarification{question, language}`, or `unsupported{reason:model_unavailable|out_of_scope, language}`. Only the deterministic router may produce `app_intent`; local-model output cannot be parsed into that variant. A renderer facade may retain the existing `respond(request, {signal})` shape, but abort must invoke native cancellation and await cleanup, not merely suppress JavaScript promises. All typed and speech consumers migrate to the same ownership contract; no second active scripted/runtime state machine remains.

Context includes only completed accepted turns and final user-visible answers; canceled/incomplete output and model reasoning are excluded. Use at most 4,096 total context tokens, including instructions, current request, history, and the reserved 256-token answer allowance. Drop oldest complete turns without generating a summary; show when context is shortened. If the newest request plus instructions/reserved output still exceeds the budget, reject it visibly rather than slicing the current utterance. This design chooses concise 256-token spoken answers instead of the research report's 512-token alternative to bound conversational playback and generation time; a length-limited answer is visibly marked. The model receives no filesystem view, memory index, credentials, tools, or internet. It must disclose uncertainty and cannot truthfully claim completed machine tasks. Its output is untrusted text even when it resembles a command.

The routine-task target is measured over a frozen representative supported task corpus: at least 99% complete through deterministic routing with zero LLM calls, while all declared deterministic intent cases require zero calls. Rejections or fake unavailable answers do not count as task completion. Open-ended conversation is reported separately. ASR and TTS may use neural models, but they are not LLM task planning; this distinction cannot be used to claim inference-free audio.

## 8. Proposed runtime, model, and voice choices

The following choices are proposals for reviewed feasibility, not pinned install manifests or passed quality gates. Runtime code, model weights, conversion provenance, dependency licenses, and each voice license must be assessed separately. Research links are primary sources; moving URLs never substitute for immutable artifact identities.

| Layer | Proposed choice | Evidence, license, and boundary |
| --- | --- | --- |
| ASR runtime | Exact pinned `whisper.cpp` source built locally as a static native library and linked into the directly owned worker | The owner-selected source-build route pins commit `d1be6fde11ac6e0407606b4e42fe72d34add8037`, tree `6fe2755682242a5c01aba65a22b3c0305d747841`; it permits only a reviewed offline arm64 build with native Metal/Accelerate and the matching native API. Exact archive digest, license/notice closure, toolchain, built output and ABI remain separately reviewable inputs; no CLI, server, downloader, curl, dynamic backend, or runtime-source build at ordinary app startup. |
| ASR weights | Multilingual Whisper `base` first, with `small` as the accuracy comparison | [OpenAI Whisper](https://github.com/openai/whisper) and [model license](https://github.com/openai/whisper/blob/main/LICENSE) establish multilingual transcription and MIT terms; [whisper.cpp model catalog](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md) documents converted artifacts. Upstream reference sizes are 142 MiB for base and 466 MiB for small, not this host's measured footprint. Reject `.en` variants. Accept base only if Turkish and English quality gates pass; changing to small requires its own exact artifact freeze/consent and the same full-stack budgets. |
| Conversation runtime | Exact pinned `llama.cpp` source built locally as a static native library and linked into the app-owned worker; no server/daemon | The owner-selected source-build route pins commit `8345f333951c661d166b00e6f9362e553768f292`, tree `88538af4019aa9d18026ff43e446c34fb7456adf`; build is permitted only after immutable archive/tree/license/dependency review and in the reviewed offline descendant-owned boundary. Use the native llama API and its approved model loader, never CLI inference, tools, server, plugins, network fetch, or runtime-source build at app startup. |
| Conversation weights | Compare official `Qwen/Qwen3-4B-GGUF` Q4_K_M first | [Official model card](https://huggingface.co/Qwen/Qwen3-4B-GGUF) documents Apache-2.0, multilingual capability, and non-thinking mode. Prefer upstream conversion provenance for a new comparative evaluation, not a production winner. `docs/local-model-candidates.md` records historical metadata: revision `bc640142c66e1fdd12af0bd68f40445458f3869b`, 2,497,280,256 bytes, SHA-256 `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`; reverify it before a new freeze. Existing candidate 005's Qwen3.5-4B third-party GGUF remains a separate metadata-only alternative and is not silently replaced or authorized. Neither model's Turkish/English quality or integration is accepted. |
| First macOS TTS | Direct native `AVSpeechSynthesizer` adapter with an explicitly selected verified Turkish voice and English voice | [Apple synthesizer](https://developer.apple.com/documentation/avfaudio/avspeechsynthesizer), [voice inventory](https://developer.apple.com/documentation/avfaudio/avspeechsynthesisvoice), and [stop API](https://developer.apple.com/documentation/avfaudio/avspeechsynthesizer/stopspeaking(at:)). OS-supplied voice assets remain governed by Apple/platform terms; this design does not redistribute them. Confirm direct API identity and offline synthesis before readiness. |
| Portable TTS comparison if native voices fail | `sherpa-onnx` with Supertonic 3 multilingual INT8 ONNX | Official Sherpa examples document [Turkish `tr`](https://k2-fsa.github.io/sherpa/onnx/tts/all/Turkish/supertonic-3-tr.html) and [English `en`](https://k2-fsa.github.io/sherpa/onnx/tts/all/English/supertonic-3-en.html), 24 kHz, and ten preset speaker IDs. The published archive name is `sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2`, not an immutable approval. [Sherpa runtime](https://github.com/k2-fsa/sherpa-onnx/blob/master/LICENSE) is Apache-2.0; [Supertonic 3 weights](https://huggingface.co/Supertone/supertonic-3) are marked OpenRAIL with separate use/redistribution terms. Review exact terms, conversion, notices, digest, size, quality, and cancellation before any provisioning. This is not a selected default or automatic fallback. |
| Later portable TTS alternative | Maintained OHF Piper only after distribution/license and voice review | [OHF-Voice/piper1-gpl](https://github.com/OHF-Voice/piper1-gpl) is GPL-3.0, not MIT; [license](https://github.com/OHF-Voice/piper1-gpl/blob/main/COPYING). The archived [rhasspy/piper](https://github.com/rhasspy/piper) has a different MIT history and is not interchangeable with the maintained engine. Each selected ONNX voice needs its own model-card/license/dependency review. |

No voice can silently substitute English pronunciation for Turkish or a different device/voice after disappearance. Missing local synthesis assets produce `voice_unavailable` with text output and explicit setup instructions. Native OS voice acquisition, if needed, uses the OS's deliberate user-facing management flow; Saturday must neither trigger hidden downloads nor claim OS assets can be removed by its app-data deletion action.

For the official Qwen comparison, use its validated non-thinking template mode; do not expose or speak model reasoning. Pin template/tokenizer identity and validate runtime compatibility before acquisition approval. The owner-selected reviewed source-build route replaces only the earlier prebuilt-only direction: runtime archives are exact GitHub/codeload commit-addressed sources (Whisper `d1be6fde11ac6e0407606b4e42fe72d34add8037`, tree `6fe2755682242a5c01aba65a22b3c0305d747841`; llama `8345f333951c661d166b00e6f9362e553768f292`, tree `88538af4019aa9d18026ff43e446c34fb7456adf`). Each source archive is capped at 64 MiB and independently reviewed for archive digest, exact signed tree, active build/dependency/license closure, fixed toolchain, output ABI, and containment before any runtime build. The source route does not authorize model transfer by itself; official Qwen3 weights remain a separate purpose-bound transfer and review. Clearer provenance is the reason to compare official Qwen3 first; it is not evidence that it outperforms Qwen3.5 or meets bilingual quality gates.

## 9. Capture, speech output, and bounded ephemeral data

Proposed data/resource limits are rejection gates, not measured results.

| Resource | Initial bound and overflow behavior |
| --- | --- |
| Utterance capture | 30 seconds maximum, mono 16 kHz Float32 ASR input: 480,000 samples, 1,920,000 bytes. Native device-format buffers and resampling workspace also require fixed caps, not unlimited duplication. Limit stops capture and marks the transcript for review; it never silently executes a truncated command. |
| Native audio buffers | One utterance plus bounded conversion/capture ring; fixed 8 MiB total application-owned raw-audio workspace. No audio file or full-recording IPC representation. Backpressure/overflow fails the turn rather than dropping audio invisibly. |
| Accepted spoken transcript | At most 2,000 Unicode scalar values and 8,000 UTF-8 bytes, to accommodate a 30-second question without inheriting the current typed 500-character slice. Oversized ASR output yields an explicitly clipped bounded review preview and cannot auto-route; rerecord or submit an edited bounded transcript. |
| Typed composer input | At most 500 Unicode scalar values and 2,000 UTF-8 bytes. Reject overflow before effects; never silently slice a submitted request. Edited voice-review text uses the spoken transcript bound. |
| Session transcript | At most 40 messages and 64 KiB text overall; oldest completed messages evict as whole records, with a visible notice. Active bounded turn owns separate pending buffers. |
| LLM context/output | At most 4,096 context tokens, 256 output tokens, and 8 KiB answer text. Exceeding protocol/text bounds fails generation; a token-length finish is marked visibly as length-limited. |
| Speech queue | One active segment and at most two waiting segments, each at most 280 Unicode scalar values and 1,120 UTF-8 bytes; at most 2,000 spoken scalar values per answer. Split stable sentences at whitespace when needed. Apply generation backpressure; reject an indivisible oversized segment or disclose nonspoken text, never silently truncate. |
| Meter/UI events | Amplitude/band summaries at most 20 Hz; text/snapshot updates coalesced at most 20 Hz, with bounded buffers and revision/sequence checks. No per-audio-frame React render. |
| Active processes | One capture owner, one ASR operation, one LLM operation, one speech playback stream; no parallel user turns or background daemon. |
| Installed app-managed weight data | Combined speech/conversation weights at most 4 GiB for this target, with actual runtime/dependency/rollback space disclosed separately. |
| Generated speech memory | Application-owned output PCM at most 4 MiB; provider overflow cancels speech visibly. Opaque OS/library buffers also count toward measured whole-stack RSS, even when the app cannot directly zero them. |
| Stage watchdogs | Cold initialization 10 seconds; ASR operation 15 seconds; local generation 60 seconds; total speech playback 90 seconds. An overrun revokes ownership and terminates work; usability gates below can reject a candidate sooner. Independent monitoring also enforces aggregate RSS and bounded stderr. |

Capture uses native device format and one bounded resampling path into 16 kHz mono. No compression, `MediaRecorder`, WAV temporary file, loopback capture, output monitoring, network stream, or camera capture is permitted. Do not request capture merely to enumerate inputs. A saved device must either be selected exactly or fail explicitly; **Use system default** requires another deliberate action. The native catalog must resolve any historical webview device ID through a reviewed migration or ask for reselection; it cannot assume cross-API identifier equivalence.

Process canonical PCM in reusable 20 ms blocks: 320 samples, or 1,280 bytes, at 16 kHz Float32. Reject nonfinite samples and unsupported formats; bound values to `[-1, 1]`. Downsample before retaining the utterance rather than duplicating a full 48 kHz recording. No preactivation preroll is allowed. A postactivation onset ring of at most 300 ms fits within the 30-second sample/workspace cap; it is not extra duration. A speech/no-speech detector must preserve quiet speech and cannot reuse the visualization RMS cutoff as recognition authority. This slice uses explicit Finish/PTT release, not automatic silence endpointing; no-speech analysis is a recognition safety gate, not permission to end or restart capture.

Speech generation follows accepted answer language. The supervisor emits `speaking` only after actual playback begins and returns to idle only after final playback ends. A worker failure after some text/audio must mark the answer interrupted, cancel queued segments, and retain only clearly identified partial visible text. It must not append a fabricated successful completion. Answers containing code, URLs, or complex formatting use an explicit bounded plain-speech projection; the full text stays visible, and the UI discloses omitted nonspoken portions.

`LocalSpeechProvider.speak` accepts the owner tuple, operation ID, plain text segment, `tr`/`en`, and a catalog voice ID. It returns acceptance or typed `unavailable/failed/text_too_long`, then exactly one `completed/cancelled/failed` terminal event. Progress may distinguish `synthesizing` and `playing`, but percentage/word timing is optional and must not be fabricated. Normalize the speech projection to Unicode NFC and whitespace; remove control/bidirectional formatting and markup-like tags rather than interpreting SSML, HTML, expression tags, or links. Preserve Turkish letters, numbers, and punctuation. Display the original escaped answer separately.

Use the system's current default output route in this slice; Saturday never changes an OS-wide default or raises system volume. Respect mute and system volume; any app-local gain stays in `[0, 1]`. Route loss cancels with a visible error. A deliberate OS route change can follow the system route only when its native semantics are verified and disclosed; it cannot substitute a missing explicitly selected voice. Before a new explicit capture, require native playback stop/drain plus a 250 ms acoustic-tail guard with the microphone still closed. Verify that guard physically; it is not proof that every room/device has no residual echo.

Audio is discarded after ASR finishes, fails, or is canceled. Transcript/context survives only in current process memory and disappears on **Clear conversation**, shutdown, or crash. No transcript, prompt, completion, PCM, local model path, or credential enters app-managed logs, analytics, retained diagnostic evidence, crash attachments, local storage, IndexedDB, or exports. Store only validated nonsensitive preferences, catalog identities, and fixed content-free lifecycle counters/receipt codes.

Release/overwrite owned PCM and mutable provider buffers where possible, and drop shared mappings/KV caches on disposal. This is best-effort ownership cleanup, not guaranteed erasure of copies controlled by an OS/library. Ordinary application code cannot promise exclusion from OS swap, system crash dumps, privileged inspection, or process compromise. Any later persistent history requires a separate retention/encryption/deletion design and explicit consent.

## 10. Cancellation, interruption, and cleanup

Cancellation revokes output ownership before awaiting any provider. `AbortSignal` suppression in React is insufficient: cancellation must stop actual capture, synthesis, and inference workload.

The supervisor performs this order:

1. Increment generation and reject old transcript, token, meter, action, and playback events.
2. Stop native capture and output playback; prevent new speech segments or intent dispatch.
3. Request ASR/LLM provider cancellation and release pending audio/text queues.
4. Await bounded cancellation acknowledgement; terminate the directly owned worker if it does not comply.
5. Reap workers, close descriptors/mappings/subscriptions, clear turn-owned buffers, and verify no active audio/capture workload remains.
6. Emit clean `idle` or a cleanup failure that blocks voice restart. Never hide failed termination behind an idle animation.

Proposed thresholds: visible cancellation acknowledgement and audio stop within 250 ms; worker cancellation acknowledgement within 250 ms; forced worker termination/reaping within 2 seconds. Late ASR results, late permission grants, model tokens, synthesis callbacks, and renderer intent acknowledgements remain fenced after cancellation. If cancellation interrupts an app-local intent before its acknowledgement, do not announce success; report its final acknowledged state if needed, without replay.

A new push-to-talk press while speaking means cancel the previous turn, wait for verified cleanup, then honor only that still-active deliberate press. Releasing the key during cleanup cancels the new request; completion never starts a microphone by itself. Focus loss, window hide, or missing keyup must fail closed. Module deactivation cancels before dropping subscriptions; provider change/removal waits for cleanup and rejects stale handles.

The app may keep approved weights loaded only while the visible voice/conversation feature is enabled and within aggregate memory limits. It unloads on disable, asset deletion, window/session loss, or shutdown. No idle keepalive inference or prompt summarization is permitted.

## 11. Acquisition, installation, and authorization gates

Metadata review alone grants no authority. The owner has separately selected bounded acquisition of the two exact pinned runtime source archives and locally reviewed static-library builds; only those approved source bytes and the source-build route are covered. Exact archive digests are calculated from the acquired bytes, then independently reviewed with the extracted signed commit trees, license/dependency closure, and build policy before compilation. This approval does not authorize build-tool installation, dependency/runtime/model downloads beyond those exact source archives, key provisioning, signing, microphone/speech use, model inference, or any model-weight transfer.

The current tracked key and binding are unprovisioned/unfrozen. Before any future transfer, the owner-controlled trust source, signed app continuity, purpose-bound issuer/verifier, immutable reviewed binding, one-time durable consumption, crash recovery, and exact native launch boundary must pass their existing approval gates. User-presence signing has no software-key fallback. A caller-minted record, digest, nonce, or alternate ledger is not authority.

Separate runtime, ASR-weight, LLM-weight, and portable-voice approvals must bind all of:

- Purpose, platform/architecture, exact artifact roster, source version/commit, runtime build identity, publisher, immutable digest, model conversion/template identity, and dependency closure.
- Transfer and installed byte counts, temporary/rollback requirements, current free-space preflight on the exact storage volume, and maximum transfer/extraction/time limits.
- Exact origin/redirect policy, credentials confined to approved native acquisition memory, no ambient proxy or unreviewed network destination, and no inference-time network authority.
- Reviewed license texts/notices for runtime, dependencies, weights, conversion, and each voice; redistribution mode must be explicit.
- Owner-visible data disclosure, optional feature consequences, disable/removal/rollback behavior, expiration, one-time generation, and independently reviewed candidate/evidence identities.

Consent shows exact asset names and versions, sources, licenses, byte sizes, disk requirement, local data behavior, and removal controls **before** transfer. Runtime inspection/relocation/dependency proof precedes separate model approval. Failed/denied/canceled consent causes no transfer. Revoked review or changed bytes require a new freeze and fresh approval. Approval is not inherited across assets or versions.

Acquisition stages into a private native-owned root, verifies immutable digests before parsing/activation, rejects path traversal/symlink escapes, and promotes atomically only after complete verification. Interrupted downloads remain nonready and are cleaned or explicitly recovered; no startup auto-download/resume or model auto-selection. Disable immediately revokes session capability and unloads workers; delete removes only app-managed assets through verified identities and confirms removal. Failed deletion is visible; rollback retains only disclosed verified versions and never activates a partial artifact.

The rejected process-group-only build path remains prohibited. The owner-selected source-build route is limited to the exact Whisper and llama commits above and requires a direct fixed-tool invocation with a scrubbed environment, immutable reviewed inputs, fresh private output root, OS-enforced no-network and write confinement inherited by every descendant, bounded resources, and cancellation that destroys/reaps the complete execution boundary before cleanup. Verify exact source-tree replay, toolchain/SDK identity, CMake options, archive/header/API hashes, static link closure, arm64/minimum-OS/Metal/Accelerate requirements, output inventory/digests, and zero surviving descendants. No build occurs until the exact source archive digests, tree/mode rosters, active build graph, and confinement boundary have been independently reviewed and the build explicitly authorized. The current deliverable contains a metadata-only graph, not compiler or product evidence: `scripts/build_voice_runtimes.py` records each fixed translation-unit command, generated-header and Metal resource transforms, and separate static archive member/link contracts. Vendor compilation remains disabled until independent source-graph and containment review and Main greenlight; no runtime libraries have been built.

## 12. UI, setup, accessibility, and errors

Keep the compact core/composer and session conversation drawer. Add only voice-specific setup and controls; do not restore removed Workspace/Social Studio/settings areas as unrelated scope. The core reflects actual native phase rather than a fixed response timer, while retaining the current restrained visual direction and bundled/offline assets.

Always expose: selected language, microphone state, elapsed capture/limit, local-provider identity/readiness, visible transcript, readable answer, **Finish utterance**, and **Cancel** when relevant. Explain at first use: explicit activation, ephemeral bounded audio, local transcription, optional installed local LLM, local speaker playback, no saved audio, and no cloud fallback. A spoken reply may be audible to nearby people; allow speech output off and per-turn cancellation.

Default voice mode produces an answer aloud when verified ASR and both selected language voices are ready; absence of the optional LLM restricts it honestly to supported deterministic intents. Display this capability limit before capture. Missing TTS permits clearly labeled text-only operation, not a claim that the listen/understand/speak acceptance has passed. Missing ASR keeps microphone voice unavailable, while typed deterministic commands may remain useful.

The proposed string catalog uses stable keys and complete `en`/`tr` coverage. Dynamic locale/asset names are validated catalog values, never raw error/path text:

| Key | English | Turkish |
| --- | --- | --- |
| `voice.start` | Start voice conversation | Sesli konuşmayı başlat |
| `voice.end_capture` | Finish speaking | Konuşmayı bitir |
| `voice.cancel` | Cancel voice turn | Sesli turu iptal et |
| `voice.listening` | Listening. Audio stays on this device and is not saved. | Dinleniyor. Ses bu cihazda kalır ve kaydedilmez. |
| `voice.transcribing` | Transcribing on this device… | Bu cihazda yazıya dönüştürülüyor… |
| `voice.review` | Check the transcript before sending | Göndermeden önce metni kontrol edin |
| `voice.thinking` | Preparing a local response… | Yerel yanıt hazırlanıyor… |
| `voice.speaking` | Speaking | Yanıt seslendiriliyor |
| `voice.canceling` | Stopping audio and local processing… | Ses ve yerel işlem durduruluyor… |
| `voice.language_choice` | Choose Turkish or English | Türkçe veya İngilizce seçin |
| `voice.model_unavailable` | No local language model is ready. Open voice setup for open-ended replies. | Hazır bir yerel dil modeli yok. Serbest yanıtlar için sesli konuşma kurulumunu açın. |
| `voice.voice_unavailable` | No verified offline voice is ready for the selected language. Use text or open voice setup. | Seçilen dil için doğrulanmış çevrimdışı ses hazır değil. Metni kullanın veya sesli konuşma kurulumunu açın. |
| `voice.permission_denied` | Microphone access is off. Allow it in system settings, then try again. | Mikrofon erişimi kapalı. Sistem ayarlarından izin verip yeniden deneyin. |
| `voice.device_lost` | Microphone disconnected. The captured audio was discarded. Reconnect and start again. | Mikrofon bağlantısı kesildi. Yakalanan ses bırakıldı. Yeniden bağlayıp tekrar başlayın. |
| `voice.duration_limit` | The 30-second limit was reached. Check the transcript or record again. | 30 saniyelik sınıra ulaşıldı. Metni kontrol edin veya yeniden konuşun. |
| `voice.cleanup_failed` | Voice could not stop safely. Restart the app before starting another turn. | Sesli konuşma güvenli biçimde durdurulamadı. Yeni bir tur başlatmadan önce uygulamayı yeniden açın. |

Require native localized microphone usage text and truthful ASR/TTS/provider disclosure before product activation. Do not retain the existing “not transcribed” permission promise once transcription is enabled. Show setup as blocked when acquisition prerequisites are unmet; the word “setup” must not imply permission to download.

| Consumer-visible failure | Required recovery/behavior |
| --- | --- |
| Permission denied/restricted, pending permission canceled | No listening state; explain OS permission control and explicit retry. A late granted stream cannot restart capture. |
| No input, selected input missing, capture busy/disconnected | Release resources; display exact problem. No silent device replacement. Offer explicit reselection/system default. |
| Silence/no speech, short ambiguous phrase, uncertain/unsupported/mixed language | No intent or LLM invocation; display review/retry/language choice. Never hallucinate a successful transcript. |
| Duration/audio buffer/transcript limit | Stop safely, disclose limit, require review or rerecord; no silent truncation action. |
| ASR unavailable, missing/corrupt weights, changed digest | Mark nonready, do not capture or load unsafe bytes; offer reviewed setup/removal, never auto-download. |
| Unsupported intent and LLM absent/disabled | Explain local conversation unavailable and list deterministic scope; no scripted pretend answer or cloud fallback. |
| LLM load/resource/deadline/crash/protocol/language failure | Fence output, stop worker/queued speech, mark partial answer if any, preserve only honest text, offer explicit retry after cleanup. |
| Wrong/missing voice, synthesis/playback failure, output device loss | Stop speech, keep readable answer and locale-specific speech error. Never silently change voice/language or label silence as speaking. |
| Cancellation timeout/unproven cleanup | Block new voice until cleanup or restart; retain fixed redacted diagnostic code. |
| Preference persistence failure/corrupt storage | Apply safe validated in-session values and disclose unsaved preference; preserve existing unrelated records. Never save audio/text as a workaround. |
| Asset transfer/verification/promotion/deletion failure | Display nonready/failed removal truthfully, preserve active verified data, and follow reviewed recovery. |

Every essential state has text and a semantic control, not color or animation alone. Keyboard controls are app-local and avoid editable fields, key-repeat, and global capture. Always provide click/tap activation and finish; push-to-talk is an additional affordance, never the only way to speak. Screen readers receive coalesced polite phase/complete-transcript announcements, not per-token audio/text floods; waveform/canvas decoration is `aria-hidden`. Respect OS reduced motion and the existing animation preference independently of microphone/playback; pausing visuals never stops or hides the ability to cancel audio. Maintain focus return, meaningful accessible labels, contrast, at least 44×44 CSS-pixel hit targets for new voice controls, and fit at existing 600×600 native minimum and 960×720 default dimensions. Voice setup/error content may scroll inside the drawer without clipping **Cancel**.

## 13. Platform and release limits

The first required physical proof target is macOS arm64 on the observed Apple M5/16 GiB host. The current bundle minimum macOS 12.0 is a configuration fact, not proof that every proposed capture/TTS/worker/security API or artifact supports that minimum. Verify actual API availability, oldest supported OS, signed bundle permission behavior, selected native voices, offline synthesis, and distribution lifecycle before advertising voice support.

Windows, Linux, and Intel macOS remain future targets of the same provider interfaces. For each, support requires native permission/device/output adapters, local Turkish/English ASR/TTS assets, immutable runtime provenance, OS-specific capability enforcement, cancellation/termination, packaging, accessibility, and physical offline verification. Until then, show `platform_voice_unavailable`; do not claim cross-platform product voice because an upstream runtime compiles there.

No Developer ID signing, notarization, hardened-runtime distribution certification, Windows release signature, Linux package trust policy, or hosted CI result is established here. Historical Apple Development/helper self-tests do not certify the changed inference helper. Runtime third-party plugins remain disabled under the architecture's existing external-plugin gate.

Conditional platform-native TTS adapters are macOS AVFoundation, a reviewed Windows SAPI/WinRT adapter, and a reviewed Linux desktop speech/output adapter where present. Their availability does not promise Turkish assets or offline behavior. Sherpa/Supertonic is the explicit-model comparison for platforms without verified bilingual native voices, not a claim of support. No new cloud, SDK auto-download, personal/custom voice cloning, or external voice-builder service is allowed.

## 14. Executable acceptance matrix

These are required future verification scenarios, not checks run for this document. Product claims require both deterministic automated evidence and owner-assisted physical-device evidence, with exact sources/artifacts and content-free retained results. Freeze licensed, nonpersonal fixtures and a bilingual corpus before execution.

Quality acceptance needs a frozen, licensed fixture set with at least 40 utterances per language, covering quiet speech, ordinary room noise, names/numbers, and all supported intents. Record word error rate (WER), character error rate (CER), and intent success separately for each language, speaker group, and noise condition; do not average Turkish failures away with English success. Proposed initial gates are WER at most 20% per language and supported spoken-intent recognition at least 95%, with zero false action dispatches across silence/noise/negation/ambiguity adversarial cases. Base versus small selection follows these gates and owner-assisted physical speech, not reference model size alone.

For real conversation, freeze twenty held-out informational prompts per language. A reviewer marks relevance, intelligibility, requested-language adherence, and honest capability statements using a published rubric. Require at least eighteen acceptable answers per language and zero claims of performed machine actions; report every rejection. For each proposed voice, test twenty pronunciation phrases per language and require nineteen intelligible results with no systematic Turkish-diacritic/number error, plus the owner's audible acceptance. These are proposed product gates, not a model certification or observed score.

| Requirement | Executable scenario and acceptance |
| --- | --- |
| Real Turkish voice turn | With verified local assets and network denied, explicitly capture `Merhaba, bugün birlikte ne üzerinde çalışabiliriz?`; show intelligible Turkish text, produce a relevant real local answer, and hear verified Turkish speech. No mandatory second Send for eligible recognition. |
| Real English voice turn | Repeat with `Hello, what can we work on together today?`; show English transcript and hear an English answer. No unsupported scripted placeholder can satisfy this test. |
| Deterministic-first | Inject every declared Turkish/English intent and spy on LLM calls: zero calls, correct intent/acknowledged outcome/response language. Frozen representative routine task corpus completes at least 99% without LLM; report unsupported/open-ended cases separately. |
| Open-ended conversation | Submit held-out Turkish/English requests beyond the deterministic grammar; verify actual local model identity and relevant answers with fixed human scoring. Missing LLM yields explicit unavailable state, not fabricated content. |
| Language policy | Turkish casing/diacritics, English names/numbers, mixed speech, short phrases, silence, noise, and unsupported languages produce correct language or review, never silent language replacement/action. Freeze eligibility thresholds before enabling auto mode. |
| Capture bounds | Feed exact-limit and oversized PCM/transcripts, input-end/disconnection, unsupported sample formats, and queue overflow; prove fixed allocation/buffer bounds, device release, visible limit/review, no silent execution of truncation. |
| No capture on discovery/startup | Launch, open setup, enumerate devices/voices, query status, open drawer, and load saved preferences; observe zero microphone sessions and no automatic model/helper/acquisition launch. |
| Permission race | Delay/deny permission; cancel/hide/close/reload before completion; deliver late success. No owned capture survives or listening state revives. Test native TCC behavior separately from fixtures. |
| Cancel each phase | Cancel requesting/listening/transcribing/review/routing/thinking/speaking, then inject late events and rapid reactivation. Stop actual workload/audio within defined limits, reject stale events, permit restart only after cleanup. |
| Half-duplex/feedback | During physical speaker playback, verify zero mic capture and no echo-fed transcript. End of playback never restarts listening. PTT interruption stops old audio before new capture. |
| True playback state | Delay synthesis and playback callbacks, simulate wrong voice/missing assets/output loss; speaking starts/ends with real audio events, readable text survives an honest speech error. |
| Exact native voice proof | Enumerate `AVSpeechSynthesisVoice.speechVoices()` in the packaged app, resolve exact Turkish/English IDs, then synthesize/play `Merhaba, nasılsın?` and `Hello, how are you?` under outbound network denial and after restart. Confirm audible nonempty output, locale, route, completion, and missing-voice behavior. `say` inventory cannot satisfy this test. |
| Authority and malformed IPC | Foreign window/origin, forged IDs, arbitrary paths/URLs, oversized frames, extra fields, unknown enums, duplicate commands, replayed generations, malicious model tool text: reject before effects; zero machine/network/process authority. |
| Worker containment | Signed wrong-peer/tampered executable, malformed framing, attempts to fork/network/read/write outside reviewed grants, crash, delayed cancellation, shutdown: fail closed, terminate/reap, retain content-free receipts, no daemon/residue. |
| Privacy | Inspect app-managed storage/logs/diagnostics/IPC after success/failure/cancel/shutdown: no audio/transcript/prompt/completion/model-path data persisted. Verify clear-session memory ownership and disclose OS-managed limits instead of claiming universal erasure. |
| Asset lifecycle | Missing/corrupt/partial/wrong digest, denied/expired/replayed acquisition, insufficient exact-volume space, interrupted promotion, disable/delete/rollback and failed removal: no partial activation/authority, truthful state, verified deletion/recovery. Do not perform transfer without its gate. |
| Bounded session context | Exceed message/byte/token caps and cancel partial model output; evict oldest complete turns with notice, omit incomplete/reasoning output from next prompt, never invoke summarizing LLM. |
| Performance on target | Measure entire stack, not individual model allowance: steady RSS ≤6 GiB, peak ≤8 GiB; cold-ready ≤10 seconds; 30-second ASR completion ≤5 seconds after finish; first visible model token ≤5 seconds once ready; ≥8 tokens/second on 256-token sample; first audible deterministic response ≤2 seconds after ASR and first audible model sentence ≤8 seconds once model-ready. UI thread never blocks an interaction >100 ms. |
| Stability and unload | Ten alternating Turkish/English turns including real playback, canceled turns, and asset/provider changes under network denial; no crash/unbounded growth, worker/audio cleanup, memory unload on disable/window loss/shutdown. |
| Offline | Deny outbound traffic after deliberate provisioning across full ASR/LLM/TTS turn, including OS voice resolution; no network request, account, key, hidden download, localhost inference server, or cloud fallback. |
| Accessibility/layout | Keyboard-only/PTT focus loss/repeated keys, screen-reader announcements, reduced motion, local-storage failure, long Turkish errors, 600×600 and 960×720 layouts: usable controls and visible cancel throughout. |
| Release truth | Verify native origin, permission prompt, physical devices, voice quality, identity/signature/resource seals and supported OS matrix. Browser fixtures/source builds cannot certify packaged physical release. |

Latency/RSS values above are proposed rejection thresholds inherited or strengthened from the existing feasibility gates, not observed measurements. If the complete stack fails them, change the reviewed candidate/design or reject support; never reduce measurement scope to LLM-only throughput or call an animation proof a conversation proof.

The eventual implementation plan must use the canonical scripts in `package.json` and the directly inspected `.github/workflows/ci.yml`: frontend `npm run check`, browser `npm run test:e2e`, approved native app build/preparation, Python discovery, Rust tests/formatting/Clippy, then the new voice-specific native and physical cases. The current native CI order fetches locked Cargo dependencies, builds the Tauri app so its hooks prepare public bootstrap/helper inputs, then runs Python/Rust checks. Preserve current user artifacts and use an isolated source/target copy rather than deleting active generated data. Exact execution commands belong in the separately reviewed plan, not a fabricated verification record here.

## 15. Decisions, required evidence, and review gate

The chosen contract has one native lifecycle owner, deliberate half-duplex capture, automatic response after eligible transcription, review only when needed, deterministic-first execution, optional real local conversation, direct native local synthesis, bounded ephemeral data, and real workload cancellation. The renderer receives no acquisition, model-file, shell, keychain, or machine-control authority.

Five domain reports informed the integrated contract. Contradictions resolve as follows: native rather than renderer capture; 30-second rather than 15-second utterance cap; Whisper base first with small quality comparison; official Qwen3 comparative evaluation without invalidating candidate 005; conditional native TTS with explicit Sherpa/Supertonic comparison and no Piper license conflation; automatic normal response rather than mandatory transcript Send; 2,000-scalar spoken versus 500-scalar typed input; concise 256-token answers rather than 512; 20 Hz meter events; aggregate rather than per-provider 6/8 GiB memory; and proposed measurable latency gates rather than unearned tighter performance claims. No source-report recommendation supersedes existing acquisition, authority, or release no-go gates.

The following prerequisites remain unresolved and block the corresponding future activity:

- Separate owner review of the implementation plan and explicit implementation authority. Written-spec approval for planning and the six-Luna execution method were received on 2026-10-06; runtime/model/voice acquisition still needs its own exact approval.
- Owner-controlled authorization bootstrap/continuity and a frozen independently reviewed purpose-bound acquisition binding; tracked resources currently prove neither.
- Immutable runtime/ASR/model/voice candidate selection, full dependency/conversion/license provenance, permitted distribution mode, and authorized transfer.
- Production inference/capture/TTS worker and narrow IPC threat-model approval, OS containment and actual cancellation/cleanup proof; current dormant health machinery is not that proof.
- Bilingual recognition/answer/voice quality corpus and auto-language eligibility thresholds, complete-stack performance evidence, native physical microphone/playback/offline tests, platform packaging and release gates.

These are required verification gates with defined failure behavior, not missing product placeholders or permission to bypass no-go decisions. The existing model-transfer block, signed-helper dormancy, third-party-plugin block, and no-cloud defaults remain active. This document makes no README/product feature claim and changes no product behavior.

Spec self-review checks: the current/proposed distinction is explicit; opening the voice drawer does not cancel its own turn; Finish differs from Cancel; ordinary speech needs no extra Send; optional LLM cannot fake a conversation; every live resource has an owner and bounded cleanup; every absent capability has an honest state; asset authority is separate from session authority; privacy promises cover app-managed storage rather than impossible OS guarantees; platform and performance claims require executable evidence. Next safe action is owner review of the written implementation plan, not implementation or acquisition.

Documentation-only work record: read-only source/primary-source research and inline requirements/consistency review informed this proposal. No build, test, linter, formatter, native microphone/speaker test, runtime/model/voice installation, key provisioning, paid/external service invocation, commit, push, or production code change was performed. Existing historical verification remains historical.

Planning checkpoint (2026-10-06): the owner approved this written specification for planning. The separate implementation plan records executable future gates; no production behavior, acquisition authority, installed asset, or verification result changed. Plan review remains required before implementation.
