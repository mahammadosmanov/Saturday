export const BUILTIN_MODULE_API_VERSION = 1 as const;

export type BuiltInModuleState =
	| "registered"
	| "active"
	| "inactive"
	| "failed";

export interface BuiltInModuleManifest {
	readonly id: string;
	readonly name: string;
	readonly version: string;
	readonly apiVersion: number;
}

export interface BuiltInModuleActivation<Contribution> {
	readonly contributions: readonly Contribution[];
	readonly dispose?: () => void;
}

export interface BuiltInModule<Context, Contribution> {
	readonly manifest: BuiltInModuleManifest;
	activate(context: Context): BuiltInModuleActivation<Contribution>;
}

export interface BuiltInModuleStatus {
	readonly id: string;
	readonly name: string;
	readonly version: string;
	readonly apiVersion: number;
	readonly state: BuiltInModuleState;
	readonly error?: string;
}

type ModuleRecord<Context, Contribution> = {
	module: BuiltInModule<Context, Contribution>;
	state: BuiltInModuleState;
	activation?: BuiltInModuleActivation<Contribution>;
	error?: string;
};

const MODULE_ID = /^[a-z][a-z0-9]*(?:\.[a-z][a-z0-9-]*)+$/;
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

export class BuiltInModuleRegistry<Context, Contribution> {
	private readonly modules = new Map<
		string,
		ModuleRecord<Context, Contribution>
	>();

	register(module: BuiltInModule<Context, Contribution>): void {
		const { manifest } = module;
		if (!MODULE_ID.test(manifest.id)) {
			throw new Error(
				`Invalid built-in module ID "${manifest.id}"; use a reverse-domain ID`,
			);
		}
		if (!SEMVER.test(manifest.version)) {
			throw new Error(
				`Invalid version "${manifest.version}" for built-in module "${manifest.id}"`,
			);
		}
		if (manifest.apiVersion !== BUILTIN_MODULE_API_VERSION) {
			throw new Error(
				`Built-in module "${manifest.id}" targets module API ${manifest.apiVersion}; core supports ${BUILTIN_MODULE_API_VERSION}`,
			);
		}
		if (this.modules.has(manifest.id)) {
			throw new Error(`Duplicate built-in module ID "${manifest.id}"`);
		}
		this.modules.set(manifest.id, { module, state: "registered" });
	}

	activateAll(context: Context): void {
		for (const record of this.modules.values()) {
			if (record.state === "active") continue;
			try {
				const activation = record.module.activate(context);
				record.activation = activation;
				record.error = undefined;
				record.state = "active";
			} catch (error) {
				record.activation = undefined;
				record.error = error instanceof Error ? error.message : String(error);
				record.state = "failed";
				throw error;
			}
		}
	}

	deactivate(moduleId: string): void {
		const record = this.modules.get(moduleId);
		if (record?.state !== "active") return;
		try {
			record.activation?.dispose?.();
		} finally {
			record.activation = undefined;
			record.error = undefined;
			record.state = "inactive";
		}
	}

	getContributions(): Contribution[] {
		return Array.from(this.modules.values()).flatMap((record) =>
			record.state === "active"
				? Array.from(record.activation?.contributions ?? [])
				: [],
		);
	}

	list(): BuiltInModuleStatus[] {
		return Array.from(this.modules.values(), (record) => ({
			id: record.module.manifest.id,
			name: record.module.manifest.name,
			version: record.module.manifest.version,
			apiVersion: record.module.manifest.apiVersion,
			state: record.state,
			...(record.error ? { error: record.error } : {}),
		}));
	}
}
