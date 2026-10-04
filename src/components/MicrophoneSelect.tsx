import { useEffect, useState } from "react";

export function MicrophoneSelect({
	open,
	value,
	onChange,
}: {
	open: boolean;
	value: string;
	onChange: (value: string) => void;
}) {
	const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
	const [error, setError] = useState("");
	const [loading, setLoading] = useState(true);
	useEffect(() => {
		if (!open) return;
		let active = true;
		let request = 0;
		const media = navigator.mediaDevices;
		const refresh = async () => {
			const token = ++request;
			setLoading(true);
			try {
				if (!media?.enumerateDevices)
					throw new Error("Enumeration unavailable");
				const list = await media.enumerateDevices();
				if (!active || token !== request) return;
				const inputs = new Map<string, MediaDeviceInfo>();
				for (const device of list)
					if (
						device.kind === "audioinput" &&
						device.deviceId &&
						device.deviceId !== "default"
					)
						inputs.set(device.deviceId, device);
				setDevices([...inputs.values()]);
				setError("");
			} catch {
				if (active && token === request)
					setError("Could not list microphones. Reopen Settings to retry.");
			} finally {
				if (active && token === request) setLoading(false);
			}
		};
		void refresh();
		media?.addEventListener?.("devicechange", refresh);
		return () => {
			active = false;
			media?.removeEventListener?.("devicechange", refresh);
		};
	}, [open]);
	const missing = value && !devices.some((device) => device.deviceId === value);
	return (
		<div className="microphone-select">
			<label htmlFor="microphone-input">
				<strong>Microphone input</strong>
			</label>
			<select
				id="microphone-input"
				aria-describedby="microphone-input-help"
				value={value}
				disabled={loading}
				onChange={(event) => onChange(event.target.value)}
			>
				<option value="">System default</option>
				{missing && (
					<option value={value}>
						Saved microphone (not currently available)
					</option>
				)}
				{devices.map((device, index) => (
					<option key={device.deviceId} value={device.deviceId}>
						{device.label || `Microphone ${index + 1}`}
					</option>
				))}
			</select>
			<p id="microphone-input-help">
				For J.A.R.V.I.S only. Your Mac’s default input stays unchanged.
			</p>
			{!loading && !error && !devices.some((device) => device.label) && (
				<p>
					Allow microphone access from the core once to reveal device names.
				</p>
			)}
			{error && <p role="status">{error}</p>}
		</div>
	);
}
