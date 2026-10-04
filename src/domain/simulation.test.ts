import { expect, it } from "vitest";
import { telemetryAt } from "./simulation";

it("produces repeatable, bounded and explicitly simulated telemetry", () => {
	for (const time of [
		0,
		1,
		20,
		100,
		10000,
		-1,
		Number.NaN,
		Number.POSITIVE_INFINITY,
	]) {
		const sample = telemetryAt(time);
		expect(sample.source).toBe("simulation");
		expect(sample.cpu).toBeGreaterThanOrEqual(20);
		expect(sample.cpu).toBeLessThanOrEqual(45);
		expect(sample.memory).toBeGreaterThanOrEqual(50);
		expect(sample.memory).toBeLessThanOrEqual(65);
		expect(sample.network).toBeGreaterThan(0);
		expect(sample.latency).toBeGreaterThan(0);
	}
	expect(telemetryAt(12)).toEqual(telemetryAt(12));
});
