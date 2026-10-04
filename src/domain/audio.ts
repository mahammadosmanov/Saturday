export interface AudioFrame {
	level: number;
	low: number;
	mid: number;
	high: number;
}
export interface AudioSignal extends AudioFrame {
	active: boolean;
}
export const SILENT_AUDIO: AudioSignal = {
	active: false,
	level: 0,
	low: 0,
	mid: 0,
	high: 0,
};

export function analyzeAudio(
	time: Float32Array,
	frequency: Uint8Array,
	sampleRate: number,
): AudioFrame {
	let sum = 0;
	for (const sample of time) {
		const bounded = Number.isFinite(sample)
			? Math.max(-1, Math.min(1, sample))
			: 0;
		sum += bounded * bounded;
	}
	const rms = time.length ? Math.sqrt(sum / time.length) : 0;
	// A small noise gate and logarithmic range keep quiet speech visible.
	const level = rms <= 0.006 ? 0 : Math.min(1, Math.log10(rms / 0.006) / 1.55);
	const band = (from: number, to: number) => {
		if (!frequency.length || !Number.isFinite(sampleRate) || sampleRate <= 0)
			return 0;
		const binHz = sampleRate / (frequency.length * 2);
		const start = Math.min(frequency.length, Math.floor(from / binHz));
		const end = Math.min(frequency.length, Math.ceil(to / binHz));
		let energy = 0;
		for (let i = start; i < end; i++) energy += frequency[i] / 255;
		return end > start ? energy / (end - start) : 0;
	};
	return {
		level,
		low: band(80, 300),
		mid: band(300, 2000),
		high: band(2000, 8000),
	};
}
