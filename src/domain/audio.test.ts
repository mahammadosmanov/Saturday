import { describe, expect, it } from "vitest";
import { analyzeAudio } from "./audio";

describe("local audio analysis", () => {
	it("keeps silence quiet and handles empty buffers", () => {
		expect(
			analyzeAudio(new Float32Array(1024), new Uint8Array(512), 48000),
		).toEqual({ level: 0, low: 0, mid: 0, high: 0 });
		expect(analyzeAudio(new Float32Array(), new Uint8Array(), 0)).toEqual({
			level: 0,
			low: 0,
			mid: 0,
			high: 0,
		});
	});
	it("increases normalized energy with signal amplitude", () => {
		const quiet = analyzeAudio(
			new Float32Array(1024).fill(0.01),
			new Uint8Array(512),
			48000,
		);
		const loud = analyzeAudio(
			new Float32Array(1024).fill(0.2),
			new Uint8Array(512),
			48000,
		);
		expect(quiet.level).toBeGreaterThan(0);
		expect(loud.level).toBeGreaterThan(quiet.level);
		expect(loud.level).toBeGreaterThan(0.5);
		expect(loud.level).toBeLessThanOrEqual(1);
	});
	it("separates low-frequency energy from higher bands", () => {
		const bins = new Uint8Array(512);
		bins.fill(255, 2, 6);
		const result = analyzeAudio(new Float32Array(1024).fill(0.1), bins, 48000);
		expect(result.low).toBeGreaterThan(0.5);
		expect(result.mid).toBe(0);
		expect(result.high).toBe(0);
	});
	it("bounds exceptional input without producing NaN", () => {
		const result = analyzeAudio(
			new Float32Array([NaN, Infinity, 100, -100]),
			new Uint8Array(512).fill(255),
			48000,
		);
		for (const value of Object.values(result)) {
			expect(Number.isFinite(value)).toBe(true);
			expect(value).toBeGreaterThanOrEqual(0);
			expect(value).toBeLessThanOrEqual(1);
		}
	});
});
