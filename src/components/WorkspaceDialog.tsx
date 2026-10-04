import type { Workspace } from "../domain/preferences";
import type { ActivityEntry, Scan } from "../hooks/useSimulation";
import { Diagnostic } from "./Diagnostics";
import { ActivityPanel } from "./Instruments";
import { Modal } from "./Modal";
import { Objectives } from "./Objectives";
export type WorkspaceTab = "tasks" | "diagnostics" | "activity";
export function WorkspaceDialog({
	open,
	onOpenChange,
	tab,
	onTab,
	workspace,
	onUpdate,
	scan,
	onStart,
	onCancel,
	entries,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	tab: WorkspaceTab;
	onTab: (tab: WorkspaceTab) => void;
	workspace: Workspace;
	onUpdate: (update: (w: Workspace) => Workspace) => void;
	scan: Scan;
	onStart: () => void;
	onCancel: () => void;
	entries: ActivityEntry[];
}) {
	return (
		<Modal
			open={open}
			onOpenChange={onOpenChange}
			title="Workspace"
			description="Tasks, diagnostics, and activity."
			className="workspace-dialog"
		>
			<nav className="workspace-tabs" aria-label="Workspace sections">
				{(["tasks", "diagnostics", "activity"] as const).map((value) => (
					<button
						key={value}
						type="button"
						aria-pressed={tab === value}
						onClick={() => onTab(value)}
					>
						{value === "tasks"
							? "Tasks"
							: value === "diagnostics"
								? "Diagnostics"
								: "Activity"}
					</button>
				))}
			</nav>
			<div className="workspace-pane">
				{tab === "tasks" && (
					<Objectives workspace={workspace} onUpdate={onUpdate} />
				)}
				{tab === "diagnostics" && (
					<Diagnostic
						scan={scan}
						onStart={onStart}
						onCancel={onCancel}
						expanded
					/>
				)}
				{tab === "activity" && <ActivityPanel entries={entries} expanded />}
			</div>
		</Modal>
	);
}
