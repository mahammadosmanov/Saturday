export type Widget = "telemetry" | "network" | "activity" | "objectives";
export interface Objective {
	id: string;
	text: string;
	done: boolean;
}
export interface Workspace {
	version: 1;
	accent: "cyan" | "amber";
	motion: boolean;
	microphoneDeviceId: string;
	density: "comfortable" | "compact";
	widgets: Record<Widget, boolean>;
	objectives: Objective[];
}
export const STORAGE_KEY = "jarvis.workspace.v1";
export function defaultWorkspace(): Workspace {
	return {
		version: 1,
		accent: "cyan",
		motion: true,
		microphoneDeviceId: "",
		density: "comfortable",
		widgets: {
			telemetry: true,
			network: true,
			activity: true,
			objectives: true,
		},
		objectives: [
			{ id: "calibrate", text: "Explore the command interface", done: true },
			{ id: "diagnostic", text: "Run a simulated diagnostic", done: false },
			{ id: "personalize", text: "Make this workspace your own", done: false },
		],
	};
}
function record(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function parseWorkspace(raw: string | null): Workspace {
	const defaults = defaultWorkspace();
	if (!raw || raw.length > 100_000) return defaults;
	try {
		const value: unknown = JSON.parse(raw);
		if (!record(value) || value.version !== 1) return defaults;
		const widgets = { ...defaults.widgets };
		if (record(value.widgets))
			for (const key of Object.keys(widgets) as Widget[]) {
				if (typeof value.widgets[key] === "boolean")
					widgets[key] = value.widgets[key];
			}
		const seen = new Set<string>();
		const objectives: Objective[] = [];
		if (Array.isArray(value.objectives))
			for (const entry of value.objectives.slice(0, 100)) {
				if (
					!record(entry) ||
					typeof entry.id !== "string" ||
					!entry.id ||
					entry.id.length > 100 ||
					seen.has(entry.id) ||
					typeof entry.text !== "string" ||
					!entry.text.trim()
				)
					continue;
				seen.add(entry.id);
				objectives.push({
					id: entry.id,
					text: entry.text.trim().slice(0, 100),
					done: entry.done === true,
				});
				if (objectives.length === 20) break;
			}
		return {
			version: 1,
			accent: value.accent === "amber" ? "amber" : "cyan",
			motion: typeof value.motion === "boolean" ? value.motion : true,
			microphoneDeviceId:
				typeof value.microphoneDeviceId === "string" &&
				value.microphoneDeviceId.length <= 512
					? value.microphoneDeviceId
					: "",
			density: value.density === "compact" ? "compact" : "comfortable",
			widgets,
			objectives: Array.isArray(value.objectives)
				? objectives
				: defaults.objectives,
		};
	} catch {
		return defaults;
	}
}
