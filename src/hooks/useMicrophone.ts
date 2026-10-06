import { useCallback, useEffect, useRef, useState } from "react";
import { type AudioSignal, analyzeAudio, SILENT_AUDIO } from "../domain/audio";

type Status = "idle" | "requesting" | "listening" | "error";
interface Capture {
	context: AudioContext;
	stream?: MediaStream;
	source?: MediaStreamAudioSourceNode;
	analyser?: AnalyserNode;
	time?: Float32Array<ArrayBuffer>;
	frequency?: Uint8Array<ArrayBuffer>;
	frame: number;
	cleanup?: () => void;
}
function release(capture: Capture) {
	cancelAnimationFrame(capture.frame);
	capture.cleanup?.();
	for (const track of capture.stream?.getTracks() ?? []) track.stop();
	capture.source?.disconnect();
	capture.analyser?.disconnect();
	capture.time?.fill(0);
	capture.frequency?.fill(0);
	if (capture.context.state !== "closed")
		void capture.context.close().catch(() => {});
}
function explanation(error: unknown) {
	const name = error instanceof Error ? error.name : "";
	if (name === "NotAllowedError" || name === "SecurityError")
		return "Microphone permission denied. Allow access in system or browser settings, then retry.";
	if (name === "OverconstrainedError")
		return "Selected microphone unavailable. Clear the saved app choice to use the system default.";
	if (name === "NotFoundError")
		return "No microphone found. Connect one and try again.";
	if (name === "NotReadableError")
		return "Microphone unavailable. Check other apps and try again.";
	return "Microphone could not start. Check audio permissions and try again.";
}

export function useMicrophone() {
	const [status, setStatus] = useState<Status>("idle");
	const [error, setError] = useState("");
	const [canUseSystemDefault, setCanUseSystemDefault] = useState(false);
	const signal = useRef<AudioSignal>({ ...SILENT_AUDIO });
	const capture = useRef<Capture | null>(null);
	const mounted = useRef(true);
	const dispose = useCallback(() => {
		const previous = capture.current;
		capture.current = null;
		signal.current = { ...SILENT_AUDIO };
		if (previous) release(previous);
	}, []);
	const stop = useCallback(() => {
		dispose();
		if (mounted.current) {
			setStatus("idle");
			setError("");
			setCanUseSystemDefault(false);
		}
	}, [dispose]);
	const start = useCallback(
		async (deviceId = "") => {
			if (capture.current || !mounted.current || document.hidden) return;
			setError("");
			setCanUseSystemDefault(false);
			if (!navigator.mediaDevices?.getUserMedia || !window.AudioContext) {
				setStatus("error");
				setError(
					"Microphone capture is unavailable in this browser or app runtime.",
				);
				return;
			}
			setStatus("requesting");
			let owned: Capture | undefined;
			try {
				// Construct/resume in the initiating click, before the permission await.
				const context = new AudioContext({ latencyHint: "interactive" });
				owned = { context, frame: 0 };
				capture.current = owned;
				const resumed = context.resume().then(
					() => true,
					() => false,
				);
				const stream = await navigator.mediaDevices.getUserMedia({
					audio: {
						...(deviceId ? { deviceId: { exact: deviceId } } : {}),
						echoCancellation: true,
						noiseSuppression: true,
						autoGainControl: false,
					},
					video: false,
				});
				if (capture.current !== owned || !mounted.current || document.hidden) {
					for (const track of stream.getTracks()) track.stop();
					return;
				}
				owned.stream = stream;
				const current = owned;
				const tracks = stream.getAudioTracks();
				const disconnected = () => {
					if (capture.current !== current) return;
					stop();
					setStatus("error");
					setError(
						"Microphone disconnected. Start listening to try again.",
					);
				};
				for (const track of tracks)
					track.addEventListener("ended", disconnected, { once: true });
				owned.cleanup = () => {
					for (const track of tracks)
						track.removeEventListener("ended", disconnected);
				};
				if (
					!tracks.length ||
					tracks.some((track) => track.readyState !== "live")
				) {
					disconnected();
					return;
				}
				if (!(await resumed) || context.state !== "running")
					throw new Error("Audio context not running");
				if (capture.current !== owned) return;
				if (tracks.some((track) => track.readyState !== "live")) {
					disconnected();
					return;
				}
				const analyser = context.createAnalyser();
				analyser.fftSize = 1024;
				analyser.smoothingTimeConstant = 0.55;
				owned.analyser = analyser;
				owned.source = context.createMediaStreamSource(stream);
				owned.source.connect(analyser); // No speaker destination, recorder, or upload.
				const time = new Float32Array(analyser.fftSize);
				const frequency = new Uint8Array(analyser.frequencyBinCount);
				owned.time = time;
				owned.frequency = frequency;
				const sample = () => {
					if (capture.current !== current) return;
					analyser.getFloatTimeDomainData(time);
					analyser.getByteFrequencyData(frequency);
					signal.current = {
						active: true,
						...analyzeAudio(time, frequency, context.sampleRate),
					};
					current.frame = requestAnimationFrame(sample);
				};
				setStatus("listening");
				sample();
			} catch (cause) {
				if (!mounted.current || (owned && capture.current !== owned)) return;
				dispose();
				setStatus("error");
				setError(explanation(cause));
				setCanUseSystemDefault(
					Boolean(deviceId && cause instanceof Error && cause.name === "OverconstrainedError"),
				);
			}
		},
		[dispose, stop],
	);
	useEffect(() => {
		mounted.current = true;
		const visibility = () => {
			if (document.hidden) stop();
		};
		document.addEventListener("visibilitychange", visibility);
		window.addEventListener("pagehide", stop);
		return () => {
			mounted.current = false;
			document.removeEventListener("visibilitychange", visibility);
			window.removeEventListener("pagehide", stop);
			dispose();
		};
	}, [dispose, stop]);
	return { status, error, canUseSystemDefault, signal, start, stop };
}
