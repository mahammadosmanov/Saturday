import {
	Activity,
	Command,
	Cpu,
	LayoutDashboard,
	Network,
	Settings2,
} from "lucide-react";
import { useEffect, useState } from "react";
export type View = "overview" | "systems" | "network" | "activity";
export function Emblem({ small = false }: { small?: boolean }) {
	return (
		<svg
			className={small ? "emblem small" : "emblem"}
			viewBox="0 0 48 48"
			aria-hidden="true"
		>
			<path d="M24 2 43 13v22L24 46 5 35V13Z" />
			<path d="M24 10 36 17v14l-12 7-12-7V17Z" />
			<path d="M18 17h12v11a6 6 0 0 1-12 0" className="emblem-j" />
		</svg>
	);
}
export function Clock() {
	const [now, setNow] = useState(() => new Date());
	useEffect(() => {
		const timer = setInterval(() => setNow(new Date()), 1000);
		return () => clearInterval(timer);
	}, []);
	return (
		<div className="clock">
			<time dateTime={now.toISOString()}>
				{now.toLocaleTimeString("en-GB", { hour12: false })}
			</time>
			<span>
				{now
					.toLocaleDateString("en-GB", {
						day: "2-digit",
						month: "short",
						year: "numeric",
					})
					.toUpperCase()}{" "}
				· LOCAL
			</span>
		</div>
	);
}
export function Header() {
	return (
		<header className="topbar">
			<a href="#main" className="skip-link">
				Skip to main content
			</a>
			<div className="brand">
				<Emblem />
				<div>
					<strong>J.A.R.V.I.S</strong>
					<span>PERSONAL COMMAND INTERFACE</span>
				</div>
				<span className="version">MK. I</span>
			</div>
			<div className="topbar-status">
				<span className="status-dot" />
				<span>INTERFACE ONLINE</span>
				<span className="topbar-separator">/</span>
				<span className="demo-label">Demo environment</span>
			</div>
			<Clock />
		</header>
	);
}
const navigation = [
	{ id: "overview", label: "Overview", Icon: LayoutDashboard },
	{ id: "systems", label: "Systems", Icon: Cpu },
	{ id: "network", label: "Network", Icon: Network },
	{ id: "activity", label: "Activity", Icon: Activity },
] as const;
export function Sidebar({
	view,
	onNavigate,
	onSettings,
	onCommand,
}: {
	view: View;
	onNavigate: (view: View) => void;
	onSettings: () => void;
	onCommand: () => void;
}) {
	return (
		<aside className="sidebar">
			<span className="rail-caption">WORKSPACE</span>
			<nav aria-label="Primary navigation">
				{navigation.map(({ id, label, Icon }) => (
					<button
						key={id}
						type="button"
						className={`nav-button ${view === id ? "selected" : ""}`}
						onClick={() => onNavigate(id)}
						aria-label={label}
						aria-current={view === id ? "page" : undefined}
					>
						<Icon size={20} strokeWidth={1.5} />
						<span>{label}</span>
						<i />
					</button>
				))}
			</nav>
			<div className="rail-bottom">
				<button
					type="button"
					className="nav-button"
					onClick={onCommand}
					aria-label="Open command palette"
				>
					<Command size={20} strokeWidth={1.5} />
					<span>Command</span>
				</button>
				<button
					type="button"
					className="nav-button"
					onClick={onSettings}
					aria-label="Settings"
				>
					<Settings2 size={20} strokeWidth={1.5} />
					<span>Settings</span>
				</button>
				<div className="user-avatar" title="Local workspace · Rohtolos">
					R<span />
				</div>
			</div>
		</aside>
	);
}
export const viewTitles: Record<
	View,
	{ title: string; subtitle: string; code: string }
> = {
	overview: {
		title: "Command center",
		subtitle: "At your command, Rohtolos.",
		code: "01",
	},
	systems: {
		title: "Systems diagnostics",
		subtitle: "A closer look beneath the interface.",
		code: "02",
	},
	network: {
		title: "Signal network",
		subtitle: "Every connection, in perspective.",
		code: "03",
	},
	activity: {
		title: "Activity stream",
		subtitle: "A record of this workspace session.",
		code: "04",
	},
};
