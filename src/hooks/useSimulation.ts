import { useCallback, useEffect, useRef, useState } from "react";
import { telemetryAt } from "../domain/simulation";
export type ScanStatus = "idle" | "running" | "complete" | "cancelled";
export interface Scan {
	status: ScanStatus;
	progress: number;
}
export interface ActivityEntry {
	id: string;
	time: string;
	title: string;
	detail: string;
	kind: "local" | "demo";
}
export function useActivity() {
	const [entries, setEntries] = useState<ActivityEntry[]>(() => [
		{
			id: "init",
			time: new Date().toLocaleTimeString("en-GB"),
			title: "Interface initialized",
			detail: "Local workspace ready. No services connected.",
			kind: "local",
		},
	]);
	const log = useCallback(
		(title: string, detail: string, kind: "local" | "demo" = "local") => {
			setEntries((previous) =>
				[
					{
						id: crypto.randomUUID(),
						time: new Date().toLocaleTimeString("en-GB"),
						title,
						detail,
						kind,
					},
					...previous,
				].slice(0, 80),
			);
		},
		[],
	);
	return { entries, log };
}
export function useDiagnostic(
	log: (title: string, detail: string, kind?: "local" | "demo") => void,
) {
	const [scan, setScan] = useState<Scan>({ status: "idle", progress: 0 });
	const active = useRef(false);
	const started = useRef(0);
	const start = useCallback(() => {
		if (active.current) return;
		active.current = true;
		started.current = performance.now();
		setScan({ status: "running", progress: 0 });
		log(
			"Diagnostic simulation started",
			"Illustrative sequence. No hardware access.",
			"demo",
		);
	}, [log]);
	const cancel = useCallback(() => {
		if (!active.current) return;
		active.current = false;
		setScan((previous) => ({ ...previous, status: "cancelled" }));
		log("Diagnostic cancelled", "The simulation was stopped by you.", "demo");
	}, [log]);
	useEffect(() => {
		if (scan.status !== "running") return;
		const timer = setInterval(() => {
			if (!active.current) return;
			const progress = Math.min(
				100,
				Math.floor((performance.now() - started.current) / 60),
			);
			if (progress === 100) {
				active.current = false;
				setScan({ status: "complete", progress });
				log(
					"Diagnostic simulation complete",
					"All four demo modules returned their scripted result.",
					"demo",
				);
			} else setScan({ status: "running", progress });
		}, 100);
		return () => clearInterval(timer);
	}, [scan.status, log]);
	return { scan, start, cancel };
}
export function useTelemetry(enabled: boolean) {
	const [tick, setTick] = useState(0);
	useEffect(() => {
		if (!enabled) return;
		const timer = setInterval(() => setTick((t) => t + 1), 1200);
		return () => clearInterval(timer);
	}, [enabled]);
	return { ...telemetryAt(tick), tick };
}
