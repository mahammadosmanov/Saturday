import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { type ReactNode, type RefObject, useRef } from "react";
import type { VoiceLanguage } from "../domain/voice";
import { voiceCopy } from "../domain/voice-copy";

export function Modal({
	open,
	onOpenChange,
	title,
	description,
	children,
	className = "",
	initialFocus,
	language,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	title: string;
	description: string;
	children: ReactNode;
	className?: string;
	initialFocus?: RefObject<HTMLElement | null>;
	language: VoiceLanguage;
}) {
	const returnFocus = useRef<HTMLElement | null>(null);
	return (
		<Dialog.Root open={open} onOpenChange={onOpenChange}>
			<Dialog.Portal container={document.getElementById("overlay-root") ?? undefined}>
				<Dialog.Overlay className="modal-overlay" />
				<Dialog.Content
					className={`modal-content ${className}`}
					onOpenAutoFocus={(event) => {
						returnFocus.current = document.activeElement instanceof HTMLElement
							? document.activeElement
							: null;
						if (initialFocus?.current) {
							event.preventDefault();
							initialFocus.current.focus();
						}
					}}
					onCloseAutoFocus={(event) => {
						event.preventDefault();
						if (document.querySelector('[role="dialog"][data-state="open"]')) return;
						const target = returnFocus.current;
						if (target?.isConnected) target.focus();
						else document.getElementById("command-trigger")?.focus();
					}}
				>
					<div className="modal-topline">
						<span>{voiceCopy(language, "ui.interface")}</span>
						<Dialog.Close
							className="button icon-button"
							aria-label={voiceCopy(language, "ui.close_dialog")}
						>
							<X size={17} />
						</Dialog.Close>
					</div>
					<Dialog.Title className="modal-title">{title}</Dialog.Title>
					<Dialog.Description className="modal-description">{description}</Dialog.Description>
					{children}
				</Dialog.Content>
			</Dialog.Portal>
		</Dialog.Root>
	);
}
