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

	it("rolls back activations from this attempt in reverse order and permits retry", () => {
		const cleanupOrder: string[] = [];
		let shouldFail = true;
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(
			module("core.first", "first", {
				activate: (context) => ({
					contributions: [{ id: `${context.prefix}first` }],
					dispose: () => cleanupOrder.push("first"),
				}),
			}),
		);
		registry.register(
			module("core.second", "second", {
				activate: (context) => ({
					contributions: [{ id: `${context.prefix}second` }],
					dispose: () => cleanupOrder.push("second"),
				}),
			}),
		);
		registry.register(
			module("core.broken", "broken", {
				activate: () => {
					if (shouldFail) throw new Error("activation failed");
					return { contributions: [{ id: "command:broken" }] };
				},
			}),
		);

		expect(() => registry.activateAll({ prefix: "command:" })).toThrow(
			"activation failed",
		);
		expect(cleanupOrder).toEqual(["second", "first"]);
		expect(registry.getContributions()).toEqual([]);
		expect(registry.list()).toEqual([
			expect.objectContaining({ id: "core.first", state: "inactive" }),
			expect.objectContaining({ id: "core.second", state: "inactive" }),
			expect.objectContaining({
				id: "core.broken",
				state: "failed",
				error: "activation failed",
			}),
		]);

		shouldFail = false;
		registry.activateAll({ prefix: "retry:" });
		expect(registry.getContributions()).toEqual([
			{ id: "retry:first" },
			{ id: "retry:second" },
			{ id: "command:broken" },
		]);
	});

	it("leaves modules active before an attempt untouched", () => {
		const dispose = vi.fn();
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(
			module("core.preactive", "preactive", {
				activate: () => ({
					contributions: [{ id: "command:preactive" }],
					dispose,
				}),
			}),
		);
		registry.activateAll({ prefix: "command:" });
		registry.register(
			module("core.broken", "broken", {
				activate: () => {
					throw new Error("later activation failed");
				},
			}),
		);

		expect(() => registry.activateAll({ prefix: "command:" })).toThrow(
			"later activation failed",
		);
		expect(dispose).not.toHaveBeenCalled();
		expect(registry.getContributions()).toEqual([{ id: "command:preactive" }]);
		expect(registry.list()).toEqual([
			expect.objectContaining({ id: "core.preactive", state: "active" }),
			expect.objectContaining({ id: "core.broken", state: "failed" }),
		]);
	});

	it("continues rollback cleanup when a disposer throws", () => {
		const cleanupOrder: string[] = [];
		const activationError = new Error("activation failed");
		const registry = new BuiltInModuleRegistry<Context, Contribution>();
		registry.register(
			module("core.first", "first", {
				activate: () => ({
					contributions: [{ id: "command:first" }],
					dispose: () => cleanupOrder.push("first"),
				}),
			}),
		);
		registry.register(
			module("core.second", "second", {
				activate: () => ({
					contributions: [{ id: "command:second" }],
					dispose: () => {
						cleanupOrder.push("second");
						throw new Error("disposal failed");
					},
				}),
			}),
		);
		registry.register(
			module("core.broken", "broken", {
				activate: () => {
					throw activationError;
				},
			}),
		);

		let thrown: unknown;
		try {
			registry.activateAll({ prefix: "command:" });
		} catch (error) {
			thrown = error;
		}
		expect(thrown).toBe(activationError);
		expect(cleanupOrder).toEqual(["second", "first"]);
		expect(registry.getContributions()).toEqual([]);
		expect(registry.list().map(({ state }) => state)).toEqual([
			"inactive",
			"inactive",
			"failed",
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
