import type { BuiltInModuleStatus } from "../platform/module-registry";

export function BuiltInModules({
	modules,
}: {
	modules: readonly BuiltInModuleStatus[];
}) {
	return (
		<section
			className="settings-section module-status"
			aria-labelledby="built-in-modules-title"
		>
			<h3 id="built-in-modules-title">Built-in modules</h3>
			<p className="settings-hint">
				Bundled with J.A.R.V.I.S and reviewed as part of this app. External plugin
				installation is disabled.
			</p>
			<ul>
				{modules.map((module) => (
					<li key={module.id}>
						<div className="module-status__identity">
							<strong>{module.name}</strong>
							<code>{module.id}</code>
						</div>
						<span className={`module-state module-state--${module.state}`}>
							Trusted · {module.state}
						</span>
					</li>
				))}
			</ul>
		</section>
	);
}
