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
