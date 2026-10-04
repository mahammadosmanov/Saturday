import { AudioLines, Mic, Send } from "lucide-react";
import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import type { Message } from "../hooks/useConversation";
import { Emblem } from "./Chrome";
import { Modal } from "./Modal";

const bars = Array.from({ length: 31 }, (_, i) => ({
	id: `wave-${i}`,
	height: 5 + Math.abs(Math.sin(i * 1.3)) * 20,
	delay: (i % 7) * -0.17,
}));
export function Conversation({
	open,
	onOpenChange,
	messages,
	pending,
	onSend,
	onVoice,
	animate,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	messages: Message[];
	pending: boolean;
	onSend: (message: string) => boolean;
	onVoice: () => void;
	animate: boolean;
}) {
	const [draft, setDraft] = useState("");
	const input = useRef<HTMLInputElement>(null);
	const log = useRef<HTMLDivElement>(null);
	const latestId = messages.at(-1)?.id;
	useEffect(() => {
		if (open && log.current && latestId)
			log.current.scrollTop = log.current.scrollHeight;
	}, [open, latestId]);
	return (
		<Modal
			open={open}
			onOpenChange={onOpenChange}
			title="Conversation"
			description="Scripted text replies. Microphone visualization is local, without recording or transcription."
			className="conversation-dialog"
			initialFocus={input}
		>
			<div className="conversation-identity">
				<Emblem small />
				<span>
					J.A.R.V.I.S<strong>PERSONAL ASSISTANT / DEMO</strong>
				</span>
				<span className="conversation-badge">
					<i className="status-dot" />
					LOCAL
				</span>
			</div>
			<div
				ref={log}
				className="conversation-messages"
				tabIndex={open ? 0 : undefined}
				role="log"
				aria-label="Conversation messages"
				aria-live="polite"
				aria-relevant="additions"
			>
				{messages.map((message) => (
					<motion.div
						className={`message ${message.role}`}
						key={message.id}
						initial={
							animate ? { opacity: 0, transform: "translateY(6px)" } : false
						}
						animate={{ opacity: 1, transform: "translateY(0px)" }}
						transition={{ duration: animate ? 0.22 : 0 }}
					>
						<span className="message-label">
							{message.role === "assistant"
								? "J.A.R.V.I.S · SCRIPTED"
								: message.sample
									? "SAMPLE PROMPT · NOT TRANSCRIBED"
									: "YOU"}
						</span>
						<p>{message.text}</p>
					</motion.div>
				))}
			</div>
			<div className="conversation-phase" role="status">
				{pending ? (
					<>
						<div className="voice-wave" aria-hidden="true">
							{bars.map((bar) => (
								<i
									key={bar.id}
									style={{
										height: `${bar.height}px`,
										animationDelay: `${bar.delay}s`,
									}}
								/>
							))}
						</div>
						<p>Preparing a scripted response…</p>
					</>
				) : (
					<>
						<AudioLines size={16} />
						<p>Ready for your next directive.</p>
					</>
				)}
			</div>
			<div className="suggestions">
				{["Status briefing", "Run a diagnostic", "Focus mode"].map((text) => (
					<button
						type="button"
						key={text}
						disabled={pending}
						onClick={() => onSend(text)}
					>
						{text}
					</button>
				))}
			</div>
			<form
				className="conversation-composer"
				onSubmit={(e) => {
					e.preventDefault();
					if (onSend(draft)) setDraft("");
				}}
			>
				<input
					ref={input}
					aria-label="Message J.A.R.V.I.S"
					placeholder="Enter a directive…"
					maxLength={500}
					value={draft}
					onChange={(e) => setDraft(e.target.value)}
					readOnly={pending}
				/>
				<button
					type="button"
					aria-label="Start listening"
					title="Open local microphone visualization"
					onClick={onVoice}
					disabled={pending}
				>
					<Mic size={16} />
				</button>
				<button
					type="submit"
					className="send-message"
					aria-label="Send message"
					disabled={!draft.trim() || pending}
				>
					<Send size={16} />
				</button>
			</form>
			<div className="conversation-foot">
				<span>{draft.length}/500 · SESSION-ONLY HISTORY</span>
				<span>ENTER TO SEND</span>
			</div>
		</Modal>
	);
}
