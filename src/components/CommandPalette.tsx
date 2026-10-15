import { ArrowRight, Command, Search } from "lucide-react";
import { motion } from "motion/react";
import { type ComponentType, useEffect, useRef, useState } from "react";
import type { VoiceLanguage } from "../domain/voice";
import { voiceCopy } from "../domain/voice-copy";
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
	language,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	commands: CommandItem[];
	animate: boolean;
	language: VoiceLanguage;
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
	const normalizedQuery = query.trim().toLocaleLowerCase(language);
	const results = commands.filter((item) =>
		`${item.label} ${item.description} ${item.keywords}`
			.toLocaleLowerCase(language)
			.includes(normalizedQuery),
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
			title={voiceCopy(language, "ui.command_palette")}
			description={voiceCopy(language, "ui.search_commands_help")}
			initialFocus={input}
			language={language}
		>
			<div className="palette-search">
				<Search size={18} aria-hidden="true" />
				<input
					ref={input}
					role="combobox"
					aria-label={voiceCopy(language, "ui.search_commands")}
					aria-expanded={open}
					aria-controls="command-results"
					aria-autocomplete="list"
					aria-activedescendant={active ? `command-${active.id}` : undefined}
					placeholder={voiceCopy(language, "ui.search_placeholder")}
					value={query}
					maxLength={200}
					onChange={(event) => {
						setQuery(event.target.value);
						setSelected(0);
					}}
					onKeyDown={(event) => {
						if (event.key === "ArrowDown" || event.key === "ArrowUp") {
							event.preventDefault();
							if (!results.length) return;
							const next = (selected + (event.key === "ArrowDown" ? 1 : -1) + results.length) % results.length;
							setSelected(next);
							document.getElementById(`command-${results[next]?.id}`)?.scrollIntoView({ block: "nearest" });
						} else if (event.key === "Enter" && active) {
							event.preventDefault();
							execute(active);
						}
					}}
				/>
				<kbd>ESC</kbd>
			</div>
			<p className="palette-caption">
				{voiceCopy(language, query ? "ui.matching" : "ui.quick_access")}
				<span>{results.length} {voiceCopy(language, "ui.available")}</span>
			</p>
			<div id="command-results" className="command-results" role="listbox" aria-label={voiceCopy(language, "ui.local_commands")}>
				{results.map((item, index) => (
					<motion.button
						initial={animate ? { opacity: 0, y: 4 } : false}
						animate={{ opacity: 1, y: 0 }}
						transition={{ duration: animate ? 0.16 : 0, delay: animate ? Math.min(index * 0.015, 0.1) : 0 }}
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
						<span className="command-icon" aria-hidden="true"><item.icon size={18} strokeWidth={1.4} /></span>
						<span><strong>{item.label}</strong><small>{item.description}</small></span>
						<ArrowRight size={15} aria-hidden="true" />
					</motion.button>
				))}
			</div>
			{results.length === 0 && (
				<p className="empty-copy">
					{voiceCopy(language, "ui.no_matches")}
				</p>
			)}
			<div className="palette-footer">
				<span>{voiceCopy(language, "ui.navigation_help")}</span>
				<span><Command size={11} aria-hidden="true" /> {voiceCopy(language, "ui.local_commands")}</span>
			</div>
		</Modal>
	);
}
