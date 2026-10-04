import type { ReactNode } from "react";
export function Panel({
	title,
	code,
	tag,
	className = "",
	children,
	action,
}: {
	title: string;
	code: string;
	tag?: string;
	className?: string;
	children: ReactNode;
	action?: ReactNode;
}) {
	return (
		<section className={`panel ${className}`} aria-label={title}>
			<header className="panel-header">
				<h2>
					<span className="panel-code">{code}</span>
					{title}
				</h2>
				{tag && <span className="panel-tag">{tag}</span>}
				{action}
			</header>
			{children}
			<span className="panel-corner" aria-hidden="true" />
		</section>
	);
}
