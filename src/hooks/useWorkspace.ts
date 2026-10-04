import { useEffect, useState, useSyncExternalStore } from "react";
import {
	defaultWorkspace,
	parseWorkspace,
	STORAGE_KEY,
	type Workspace,
} from "../domain/preferences";

function load() {
	let raw: string | null;
	try {
		raw = localStorage.getItem(STORAGE_KEY);
	} catch {
		return {
			workspace: defaultWorkspace(),
			warning: "Changes are session-only. Local storage is unavailable.",
		};
	}
	let readable = true;
	if (raw !== null) {
		try {
			const stored: unknown = raw.length > 100000 ? null : JSON.parse(raw);
			readable =
				typeof stored === "object" &&
				stored !== null &&
				"version" in stored &&
				stored.version === 1;
		} catch {
			readable = false;
		}
	}
	return {
		workspace: parseWorkspace(raw),
		warning: readable
			? ""
			: "Saved workspace was unreadable. Safe defaults were restored.",
	};
}
export function useWorkspace() {
	const [initial] = useState(load);
	const [workspace, setWorkspace] = useState(initial.workspace);
	const [storageWarning, setStorageWarning] = useState(initial.warning);
	useEffect(() => {
		try {
			localStorage.setItem(STORAGE_KEY, JSON.stringify(workspace));
		} catch {
			setStorageWarning(
				"Changes are session-only. Local storage is unavailable.",
			);
		}
	}, [workspace]);
	function updateWorkspace(update: (workspace: Workspace) => Workspace) {
		setWorkspace(update);
	}
	return {
		workspace,
		updateWorkspace,
		storageWarning,
		reset: () => setWorkspace(defaultWorkspace()),
	};
}
function subscribeMotion(callback: () => void) {
	const media = window.matchMedia("(prefers-reduced-motion: reduce)");
	media.addEventListener("change", callback);
	document.addEventListener("visibilitychange", callback);
	return () => {
		media.removeEventListener("change", callback);
		document.removeEventListener("visibilitychange", callback);
	};
}
function motionAllowed() {
	return (
		!window.matchMedia("(prefers-reduced-motion: reduce)").matches &&
		document.visibilityState !== "hidden"
	);
}
export function useMotionEnabled(requested: boolean) {
	return (
		useSyncExternalStore(subscribeMotion, motionAllowed, () => false) &&
		requested
	);
}
