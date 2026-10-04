export function telemetryAt(tick: number) {
	const t = Number.isFinite(tick) ? tick : 0;
	return {
		source: "simulation" as const,
		cpu: Math.round(32 + Math.sin(t * 0.32) * 9),
		memory: Math.round(58 + Math.sin(t * 0.13) * 3),
		network: Number((24 + Math.sin(t * 0.28) * 7).toFixed(1)),
		latency: Math.round(12 + Math.sin(t * 0.41) * 3),
	};
}
