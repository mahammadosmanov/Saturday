import { ArrowRight, Command, Search } from "lucide-react";
import { motion } from "motion/react";
import { type ComponentType, useEffect, useRef, useState } from "react";
import { Modal } from "./Modal";
export interface CommandItem {
	id: string;
	label: string;
	description: string;
	keywords: string;
	icon: ComponentType<{ size?: number; strokeWidth?: number }>;
	run: () => void;
}
export function CommandPalette({
	open,
	onOpenChange,
	commands,
	animate,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	commands: CommandItem[];
	animate: boolean;
}) {
	const [query, setQuery] = useState("");
	const [selected, setSelected] = useState(0);
	const input = useRef<HTMLInputElement>(null);
	useEffect(() => {
		if (open) {
			setQuery("");
			setSelected(0);
		}
	}, [open]);
	const results = commands.filter((item) =>
		`${item.label} ${item.description} ${item.keywords}`
			.toLowerCase()
			.includes(query.trim().toLowerCase()),
	);
	const active = results[selected];
	function execute(item: CommandItem) {
		onOpenChange(false);
		item.run();
	}
	return (
		<Modal
			open={open}
			onOpenChange={onOpenChange}
			title="Command palette"
			description="Search and run local commands."
			initialFocus={input}
		>
			<div className="palette-search">
				<Search size={18} />
				<input
					ref={input}
					role="combobox"
					aria-label="Search commands"
					aria-expanded={open}
					aria-controls="command-results"
					aria-autocomplete="list"
					aria-activedescendant={active ? `command-${active.id}` : undefined}
					placeholder="What would you like to do?"
					value={query}
					maxLength={200}
					onChange={(e) => {
						setQuery(e.target.value);
						setSelected(0);
					}}
					onKeyDown={(e) => {
						if (e.key === "ArrowDown" || e.key === "ArrowUp") {
							e.preventDefault();
							if (!results.length) return;
							const next =
								(selected + (e.key === "ArrowDown" ? 1 : -1) + results.length) %
								results.length;
							setSelected(next);
							document
								.getElementById(`command-${results[next]?.id}`)
								?.scrollIntoView({ block: "nearest" });
						} else if (e.key === "Enter" && active) {
							e.preventDefault();
							execute(active);
						}
					}}
				/>
				<kbd>ESC</kbd>
			</div>
			<p className="palette-caption">
				{query ? "MATCHING COMMANDS" : "QUICK ACCESS"}
				<span>{results.length} AVAILABLE</span>
			</p>
			<div
				id="command-results"
				className="command-results"
				role="listbox"
				aria-label="Available commands"
			>
				{results.map((item, index) => (
					<motion.button
						initial={animate ? { opacity: 0, y: 4 } : false}
						animate={{ opacity: 1, y: 0 }}
						transition={{
							duration: animate ? 0.16 : 0,
							delay: animate ? Math.min(index * 0.015, 0.1) : 0,
						}}
						type="button"
						role="option"
						tabIndex={-1}
						aria-selected={selected === index}
						className="command-result"
						id={`command-${item.id}`}
						key={item.id}
						onPointerMove={() => setSelected(index)}
						onClick={() => execute(item)}
					>
						<span className="command-icon">
							<item.icon size={18} strokeWidth={1.4} />
						</span>
						<span>
							<strong>{item.label}</strong>
							<small>{item.description}</small>
						</span>
						<ArrowRight size={15} />
					</motion.button>
				))}
			</div>
			{results.length === 0 && (
				<p className="empty-copy">
					No matching commands. Try “voice” or “conversation”.
				</p>
			)}
			<div className="palette-footer">
				<span>
					<kbd>↑</kbd>
					<kbd>↓</kbd> to navigate <kbd>↵</kbd> to select
				</span>
				<span>
					<Command size={11} /> LOCAL COMMANDS ONLY
				</span>
			</div>
		</Modal>
	);
}
