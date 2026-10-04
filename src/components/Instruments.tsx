import { Activity, ArrowUpRight, Cpu, Network, Shield } from "lucide-react";
import { useEffect, useState } from "react";
import { telemetryAt } from "../domain/simulation";
import { type ActivityEntry, useTelemetry } from "../hooks/useSimulation";
import { Panel } from "./Panel";

function Sparkline({
	tick,
	field,
}: {
	tick: number;
	field: "cpu" | "memory" | "network";
}) {
	const points = Array.from({ length: 38 }, (_, i) => {
		const value = telemetryAt(tick - 37 + i)[field];
		return `${i * 5.5},${43 - value * (field === "memory" ? 0.5 : 0.8)}`;
	}).join(" ");
	return (
		<svg
			className={`sparkline ${field}`}
			viewBox="0 0 204 46"
			preserveAspectRatio="none"
			aria-hidden="true"
		>
			<path d="M0 12H204M0 28H204M0 44H204" className="chart-grid" />
			<polyline points={`0,46 ${points} 204,46`} className="chart-fill" />
			<polyline points={points} className="chart-line" />
		</svg>
	);
}
export function Telemetry({
	enabled,
	expanded = false,
}: {
	enabled: boolean;
	expanded?: boolean;
}) {
	const t = useTelemetry(enabled);
	return (
		<Panel
			title="System telemetry"
			code="01"
			tag="SIMULATED"
			className={`telemetry-panel ${expanded ? "expanded" : ""}`}
		>
			<div className="telemetry-metrics">
				{[
					{
						key: "cpu",
						label: "PROCESSOR",
						value: t.cpu,
						unit: "%",
						hint: "Compute utilization",
						Icon: Cpu,
					},
					{
						key: "memory",
						label: "MEMORY",
						value: t.memory,
						unit: "%",
						hint: "Working allocation",
						Icon: Activity,
					},
					{
						key: "network",
						label: "NETWORK",
						value: t.network,
						unit: "Mb/s",
						hint: "Signal throughput",
						Icon: Network,
					},
				].map(({ key, label, value, unit, hint, Icon }) => (
					<div className="telemetry-metric" key={key}>
						<div className="metric-label">
							<span>
								<Icon size={11} />
								{label}
							</span>
							<span>{key === "network" ? "VIRTUAL" : "NOMINAL"}</span>
						</div>
						<div className="metric-value">
							<strong>
								{value}
								<small>{unit}</small>
							</strong>
							<Sparkline
								tick={t.tick}
								field={key as "cpu" | "memory" | "network"}
							/>
						</div>
						<div className="metric-caption">
							<span>{hint}</span>
							<span>{key === "network" ? `${t.latency} ms` : "DEMO FEED"}</span>
						</div>
					</div>
				))}
			</div>
			<div className="instrument-footnote">
				<i className="status-dot" />
				ILLUSTRATIVE DATA · NOT YOUR DEVICE
			</div>
		</Panel>
	);
}
export function RuntimePanel() {
	const [started] = useState(() => performance.now());
	const [elapsed, setElapsed] = useState(0);
	useEffect(() => {
		const timer = setInterval(
			() => setElapsed(Math.floor((performance.now() - started) / 1000)),
			1000,
		);
		return () => clearInterval(timer);
	}, [started]);
	const time = `${String(Math.floor(elapsed / 3600)).padStart(2, "0")}:${String(Math.floor(elapsed / 60) % 60).padStart(2, "0")}:${String(elapsed % 60).padStart(2, "0")}`;
	return (
		<Panel title="Environment" code="05" tag="LOCAL" className="runtime-panel">
			<div className="runtime-identity">
				<span className="runtime-icon">
					<Shield size={23} strokeWidth={1.2} />
				</span>
				<div>
					<h3>Private by design</h3>
					<p>Your workspace stays on this device.</p>
				</div>
			</div>
			<dl className="runtime-facts">
				<div>
					<dt>Session uptime</dt>
					<dd className="actual-time">{time}</dd>
				</div>
				<div>
					<dt>AI provider</dt>
					<dd>Not connected</dd>
				</div>
				<div>
					<dt>Microphone</dt>
					<dd>Not accessed</dd>
				</div>
			</dl>
		</Panel>
	);
}
const continents = [
	"M48 58 59 43 78 39 83 32 105 31 112 43 127 49 125 61 111 67 105 80 90 91 81 86 79 75 65 73 57 65Z",
	"M100 92 113 95 127 109 126 125 116 140 112 155 102 164 95 147 96 130 88 111Z",
	"M170 57 178 45 193 40 205 48 203 59 188 65 178 65Z",
	"M181 71 197 69 211 78 224 89 215 103 204 113 199 139 189 143 180 125 174 107 162 91 165 78Z",
	"M204 45 224 39 240 46 259 39 280 47 300 52 312 65 299 79 282 79 269 96 253 95 237 85 226 72 210 67Z",
	"M267 123 284 117 301 127 312 144 298 153 278 150 261 138Z",
	"M160 153 156 161 158 167 164 164Z",
];
export function SignalNetwork({ expanded = false }: { expanded?: boolean }) {
	return (
		<Panel
			title="Signal topology"
			code="03"
			tag="VIRTUAL NETWORK"
			className={`network-panel ${expanded ? "expanded" : ""}`}
		>
			<div className="network-content">
				<div className="world-map">
					<svg
						viewBox="0 0 360 188"
						aria-label="Illustrative virtual connection map"
						role="img"
					>
						<title>Illustrative virtual connection map</title>
						<defs>
							<pattern
								id="map-dots"
								width="4"
								height="4"
								patternUnits="userSpaceOnUse"
							>
								<circle cx="1" cy="1" r=".7" fill="currentColor" />
							</pattern>
							<linearGradient id="signal-gradient">
								<stop stopColor="currentColor" stopOpacity="0" />
								<stop offset=".6" stopColor="currentColor" stopOpacity=".5" />
								<stop offset="1" stopColor="currentColor" stopOpacity="0" />
							</linearGradient>
						</defs>
						<g className="map-grid">
							<path d="M20 35h320M20 75h320M20 115h320M20 155h320M60 15v160M120 15v160M180 15v160M240 15v160M300 15v160" />
						</g>
						<g className="continents">
							{continents.map((path) => (
								<path key={path} d={path} fill="url(#map-dots)" />
							))}
						</g>
						<g fill="none" className="map-routes">
							<path d="M82 67Q126 5 188 63T277 75M188 63Q223 83 285 137M82 67Q57 103 107 122M107 122Q145 151 188 63" />
						</g>
						<g fill="none" className="route-pulses">
							<path d="M82 67Q126 5 188 63T277 75M188 63Q223 83 285 137M82 67Q57 103 107 122" />
						</g>
						{[
							{ x: 82, y: 67, name: "01" },
							{ x: 188, y: 63, name: "CORE" },
							{ x: 277, y: 75, name: "02" },
							{ x: 107, y: 122, name: "03" },
							{ x: 285, y: 137, name: "04" },
						].map(({ x, y, name }) => (
							<g key={name} transform={`translate(${x} ${y})`}>
								<circle r={name === "CORE" ? 12 : 8} className="node-halo" />
								<circle r={name === "CORE" ? 3 : 2} className="node-dot" />
								<text x="8" y="-9">
									{name}
								</text>
							</g>
						))}
						<rect
							className="map-scan"
							x="25"
							y="18"
							width="35"
							height="151"
							fill="url(#signal-gradient)"
						/>
					</svg>
					<span className="map-label">SIMULATED LINKS · NO REAL LOCATIONS</span>
				</div>
				<div className="network-readout">
					<div className="network-status">
						<i className="status-dot" />
						<span>VIRTUAL MESH</span>
					</div>
					<h3>Connected by design.</h3>
					<p>
						A preview of your future
						<br />
						connected ecosystem.
					</p>
					<dl>
						<div>
							<dt>Demo nodes</dt>
							<dd>04</dd>
						</div>
						<div>
							<dt>External connections</dt>
							<dd>00</dd>
						</div>
					</dl>
				</div>
			</div>
			{expanded && (
				<div className="network-detail">
					<h3>Integration boundary</h3>
					<p>
						These links are illustrative. No device, location, account or
						service is being discovered or contacted. Future integrations will
						require explicit connection and permission.
					</p>
				</div>
			)}
		</Panel>
	);
}
export function ActivityPanel({
	entries,
	onExpand,
	expanded = false,
}: {
	entries: ActivityEntry[];
	onExpand?: () => void;
	expanded?: boolean;
}) {
	return (
		<Panel
			title="Activity stream"
			code="06"
			tag={expanded ? `${entries.length} EVENTS` : undefined}
			className={`activity-panel ${expanded ? "expanded" : ""}`}
			action={
				onExpand && (
					<button
						type="button"
						className="text-button"
						aria-label="View all activity"
						onClick={onExpand}
					>
						<ArrowUpRight size={13} />
					</button>
				)
			}
		>
			<ol
				className="activity-list"
				tabIndex={expanded ? undefined : 0}
				aria-label="Recent activity events"
			>
				{entries.slice(0, expanded ? 80 : 3).map((entry) => (
					<li key={entry.id}>
						<span className={`event-dot ${entry.kind}`} />
						<div>
							<div className="event-title">
								<strong>{entry.title}</strong>
								<time>{entry.time}</time>
							</div>
							<p>{entry.detail}</p>
							<span className="event-source">
								{entry.kind === "demo" ? "SIMULATION" : "LOCAL EVENT"}
							</span>
						</div>
					</li>
				))}
			</ol>
			{entries.length === 1 && !expanded && (
				<div className="activity-waiting">
					<span className="waiting-pulse" />
					<p>Listening for your next action.</p>
				</div>
			)}
		</Panel>
	);
}
