import { describe, expect, it, vi } from "vitest";
import {
	BUILTIN_MODULE_API_VERSION,
	BuiltInModuleRegistry,
	type BuiltInModule,
} from "./module-registry";

type Contribution = { id: string };
type Context = { prefix: string };

function module(
	id: string,
	contributionId: string,
	overrides: Partial<BuiltInModule<Context, Contribution>> = {},
): BuiltInModule<Context, Contribution> {
	return {
		manifest: {
			id,
			name: id,
			version: "1.0.0",
			apiVersion: BUILTIN_MODULE_API_VERSION,
		},
		activate: (context) => ({
			contributions: [{ id: `${context.prefix}${contributionId}` }],
		}),
		...overrides,
	};
}

describe("BuiltInModuleRegistry", () => {
	it("activates trusted modules in registration order", () => {
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(module("core.navigation", "home"));
		registry.register(module("core.workspace", "tasks"));

		registry.activateAll({ prefix: "command:" });

		expect(registry.getContributions()).toEqual([
			{ id: "command:home" },
			{ id: "command:tasks" },
		]);
		expect(registry.list()).toEqual([
			expect.objectContaining({ id: "core.navigation", state: "active" }),
			expect.objectContaining({ id: "core.workspace", state: "active" }),
		]);
	});

	it("rejects duplicate module IDs before activation", () => {
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(module("core.workspace", "tasks"));

		expect(() => registry.register(module("core.workspace", "activity"))).toThrow(
			'Duplicate built-in module ID "core.workspace"',
		);
	});

	it("rejects modules targeting an incompatible core API", () => {
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		const incompatible = module("core.future", "future", {
			manifest: {
				id: "core.future",
				name: "Future",
				version: "1.0.0",
				apiVersion: BUILTIN_MODULE_API_VERSION + 1,
			},
		});

		expect(() => registry.register(incompatible)).toThrow(
			"targets module API 2; core supports 1",
		);
	});

	it("rolls back a failed activation without publishing contributions", () => {
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(
			module("core.broken", "broken", {
				activate: () => {
					throw new Error("activation failed");
				},
			}),
		);

		expect(() => registry.activateAll({ prefix: "command:" })).toThrow(
			"activation failed",
		);
		expect(registry.getContributions()).toEqual([]);
		expect(registry.list()).toEqual([
			expect.objectContaining({ id: "core.broken", state: "failed" }),
		]);
	});

	it("disposes owned resources and removes contributions on deactivation", () => {
		const dispose = vi.fn();
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(
			module("core.voice", "voice", {
				activate: () => ({
					contributions: [{ id: "command:voice" }],
					dispose,
				}),
			}),
		);
		registry.activateAll({ prefix: "command:" });

		registry.deactivate("core.voice");

		expect(dispose).toHaveBeenCalledOnce();
		expect(registry.getContributions()).toEqual([]);
		expect(registry.list()).toEqual([
			expect.objectContaining({ id: "core.voice", state: "inactive" }),
		]);
	});
});
