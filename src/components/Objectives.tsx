import { Plus, X } from "lucide-react";
import { useState } from "react";
import type { Workspace } from "../domain/preferences";
import { Panel } from "./Panel";
export function Objectives({
	workspace,
	onUpdate,
}: {
	workspace: Workspace;
	onUpdate: (update: (state: Workspace) => Workspace) => void;
}) {
	const [draft, setDraft] = useState("");
	const completed = workspace.objectives.filter((o) => o.done).length;
	return (
		<Panel
			title="Objectives"
			code="04"
			tag={`${completed}/${workspace.objectives.length}`}
			className="objectives-panel"
		>
			<div className="objectives-intro">
				<span>YOUR NEXT MOVES</span>
				<span>LOCAL CHECKLIST</span>
			</div>
			<ul className="objective-list">
				{workspace.objectives.map((objective) => (
					<li
						key={objective.id}
						className={objective.done ? "objective done" : "objective"}
					>
						<label>
							<input
								type="checkbox"
								checked={objective.done}
								onChange={(e) =>
									onUpdate((w) => ({
										...w,
										objectives: w.objectives.map((o) =>
											o.id === objective.id
												? { ...o, done: e.target.checked }
												: o,
										),
									}))
								}
							/>
							<span>{objective.text}</span>
						</label>
						<button
							type="button"
							className="delete-objective"
							aria-label={`Delete objective: ${objective.text}`}
							onClick={() =>
								onUpdate((w) => ({
									...w,
									objectives: w.objectives.filter((o) => o.id !== objective.id),
								}))
							}
						>
							<X size={12} />
						</button>
					</li>
				))}
			</ul>
			{workspace.objectives.length === 0 && (
				<p className="empty-copy">A clear runway. Add your first objective.</p>
			)}
			<form
				className="add-objective"
				onSubmit={(e) => {
					e.preventDefault();
					const text = draft.trim().slice(0, 100);
					if (!text || workspace.objectives.length >= 20) return;
					onUpdate((w) =>
						w.objectives.length >= 20
							? w
							: {
									...w,
									objectives: [
										...w.objectives,
										{ id: crypto.randomUUID(), text, done: false },
									],
								},
					);
					setDraft("");
				}}
			>
				<input
					aria-label="New objective"
					placeholder="Add an objective…"
					maxLength={100}
					value={draft}
					onChange={(e) => setDraft(e.target.value)}
					disabled={workspace.objectives.length >= 20}
				/>
				<button
					type="submit"
					aria-label="Add objective"
					disabled={!draft.trim() || workspace.objectives.length >= 20}
				>
					<Plus size={15} />
				</button>
			</form>
			<p className="objective-note">
				{workspace.objectives.length >= 20
					? "20 objective limit reached."
					: "Sample starting list · edits saved on this device"}
			</p>
		</Panel>
	);
}
