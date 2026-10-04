import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { type ReactNode, type RefObject, useRef } from "react";
export function Modal({
	open,
	onOpenChange,
	title,
	description,
	children,
	className = "",
	initialFocus,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	title: string;
	description: string;
	children: ReactNode;
	className?: string;
	initialFocus?: RefObject<HTMLElement | null>;
}) {
	const returnFocus = useRef<HTMLElement | null>(null);
	return (
		<Dialog.Root open={open} onOpenChange={onOpenChange}>
			<Dialog.Portal
				container={document.getElementById("overlay-root") ?? undefined}
			>
				<Dialog.Overlay className="modal-overlay" />
				<Dialog.Content
					className={`modal-content ${className}`}
					onOpenAutoFocus={(event) => {
						returnFocus.current =
							document.activeElement instanceof HTMLElement
								? document.activeElement
								: null;
						if (initialFocus?.current) {
							event.preventDefault();
							initialFocus.current.focus();
						}
					}}
					onCloseAutoFocus={(event) => {
						event.preventDefault();
						if (document.querySelector('[role="dialog"][data-state="open"]'))
							return;
						const target = returnFocus.current;
						if (target?.isConnected) target.focus();
						else document.getElementById("command-trigger")?.focus();
					}}
				>
					<div className="modal-topline">
						<span>J.A.R.V.I.S / INTERFACE</span>
						<Dialog.Close
							className="button icon-button"
							aria-label="Close dialog"
						>
							<X size={17} />
						</Dialog.Close>
					</div>
					<Dialog.Title className="modal-title">{title}</Dialog.Title>
					<Dialog.Description className="modal-description">
						{description}
					</Dialog.Description>
					{children}
				</Dialog.Content>
			</Dialog.Portal>
		</Dialog.Root>
	);
}
