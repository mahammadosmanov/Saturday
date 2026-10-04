import { Check, ScanLine, X } from "lucide-react";
import type { Scan } from "../hooks/useSimulation";
import { Panel } from "./Panel";

const scanSteps = [
	"Interface handshake",
	"Neural matrix",
	"Signal pathways",
	"Response protocol",
];
export function Diagnostic({
	scan,
	onStart,
	onCancel,
	expanded = false,
}: {
	scan: Scan;
	onStart: () => void;
	onCancel: () => void;
	expanded?: boolean;
}) {
	const active = scan.status === "running";
	const labels = {
		idle: "Ready for a system check",
		running: "Simulation running",
		complete: "Simulation complete",
		cancelled: "Diagnostic cancelled",
	};
	return (
		<Panel
			title="Diagnostics"
			code="02"
			tag="SIMULATED"
			className={`diagnostic-panel ${expanded ? "expanded" : ""}`}
		>
			<div className="diagnostic-content">
				<div className="diagnostic-symbol">
					<ScanLine size={22} strokeWidth={1.2} />
				</div>
				<div className="diagnostic-summary">
					<h3 role="status">{labels[scan.status]}</h3>
					<p>
						{scan.status === "complete"
							? "No hardware was accessed."
							: active
								? scanSteps[Math.min(3, Math.floor(scan.progress / 25))]
								: "A safe preview of the diagnostic workflow."}
					</p>
				</div>
			</div>
			{(active || expanded) && (
				<div
					className="scan-progress"
					role="progressbar"
					aria-label="Diagnostic simulation progress"
					aria-valuenow={scan.progress}
					aria-valuemin={0}
					aria-valuemax={100}
				>
					<span style={{ width: `${scan.progress}%` }} />
				</div>
			)}
			{expanded && (
				<ol className="scan-steps">
					{scanSteps.map((step, index) => (
						<li
							key={step}
							className={scan.progress >= (index + 1) * 25 ? "complete" : ""}
						>
							<span className="step-index">
								{scan.progress >= (index + 1) * 25 ? (
									<Check size={14} />
								) : (
									`0${index + 1}`
								)}
							</span>
							<div>
								<strong>{step}</strong>
								<p>Demonstration sequence · no external IO</p>
							</div>
							<span>
								{scan.progress >= (index + 1) * 25
									? "COMPLETE"
									: active && Math.floor(scan.progress / 25) === index
										? "RUNNING"
										: "WAITING"}
							</span>
						</li>
					))}
				</ol>
			)}
			<div className="diagnostic-actions">
				<span>
					{active
						? `${String(scan.progress).padStart(2, "0")}%`
						: "4 MODULES / DEMO"}
				</span>
				<button
					type="button"
					className="button small primary"
					onClick={active ? onCancel : onStart}
				>
					{active ? <X size={12} /> : <ScanLine size={12} />}
					{active ? "Cancel diagnostic" : "Run diagnostic"}
				</button>
			</div>
		</Panel>
	);
}
