import { RotateCcw } from "lucide-react";
import { useState } from "react";
import type { Workspace } from "../domain/preferences";
import type { BuiltInModuleStatus } from "../platform/module-registry";
import { BuiltInModules } from "./BuiltInModules";
import { MicrophoneSelect } from "./MicrophoneSelect";
import { Modal } from "./Modal";
export function SettingsDialog({
	open,
	onOpenChange,
	workspace,
	modules,
	onUpdate,
	onReset,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	workspace: Workspace;
	modules: readonly BuiltInModuleStatus[];
	onUpdate: (update: (state: Workspace) => Workspace) => void;
	onReset: () => void;
}) {
	const [confirmReset, setConfirmReset] = useState(false);
	return (
		<Modal
			open={open}
			onOpenChange={(next) => {
				onOpenChange(next);
				setConfirmReset(false);
			}}
			title="Interface settings"
			description="Appearance, motion and microphone. Saved on this device."
			className="settings-dialog"
		>
			<div className="settings-section">
				<div className="setting-row">
					<div>
						<strong>Accent color</strong>
						<p>Choose your interface color.</p>
					</div>
					<div className="segmented">
						<button
							type="button"
							aria-pressed={workspace.accent === "cyan"}
							onClick={() => onUpdate((w) => ({ ...w, accent: "cyan" }))}
						>
							<i className="swatch cyan" />
							Cyan
						</button>
						<button
							type="button"
							aria-pressed={workspace.accent === "amber"}
							onClick={() => onUpdate((w) => ({ ...w, accent: "amber" }))}
						>
							<i className="swatch amber" />
							Amber
						</button>
					</div>
				</div>
				<label className="setting-row">
					<div>
						<strong>Ambient animations</strong>
						<p>
							Core rotation and particles.
							<br />
							System reduced motion takes priority.
						</p>
					</div>
					<input
						className="switch"
						type="checkbox"
						aria-label="Ambient animations"
						checked={workspace.motion}
						onChange={(event) =>
							onUpdate((w) => ({ ...w, motion: event.target.checked }))
						}
					/>
				</label>
			</div>
			<MicrophoneSelect
				open={open}
				value={workspace.microphoneDeviceId}
				onChange={(microphoneDeviceId) =>
					onUpdate((w) => ({ ...w, microphoneDeviceId }))
				}
			/>
			<BuiltInModules modules={modules} />
			<div className="settings-foot">
				<p>No accounts or API keys. Nothing leaves this app.</p>
				{confirmReset ? (
					<fieldset className="reset-confirm" aria-label="Reset confirmation">
						<span>Reset preferences and all objectives?</span>
						<div>
							<button
								className="button small"
								type="button"
								onClick={() => setConfirmReset(false)}
							>
								Keep workspace
							</button>
							<button
								className="button small primary"
								type="button"
								onClick={() => {
									onReset();
									setConfirmReset(false);
								}}
							>
								Confirm reset
							</button>
						</div>
					</fieldset>
				) : (
					<button
						className="text-button"
						type="button"
						onClick={() => setConfirmReset(true)}
					>
						<RotateCcw size={12} />
						Reset workspace
					</button>
				)}
			</div>
		</Modal>
	);
}
