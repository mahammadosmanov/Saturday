import { test, expect } from "@playwright/test";

test("saved tasks, accent, motion and legacy settings survive the compact UI", async ({
	page,
}) => {
	await page.goto("/");
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	await page
		.getByRole("checkbox", { name: "Run a simulated diagnostic" })
		.check();
	await page
		.getByRole("textbox", { name: "New objective" })
		.fill("Build the next chapter");
	await page
		.getByRole("button", { name: "Add objective", exact: true })
		.click();
	const legacy = await page.evaluate(
		() =>
			JSON.parse(localStorage.getItem("jarvis.workspace.v1") ?? "{}").widgets,
	);
	await page.keyboard.press("Escape");
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	await page.getByRole("button", { name: "Amber", exact: true }).click();
	await page.getByRole("checkbox", { name: "Ambient animations" }).uncheck();
	await page.keyboard.press("Escape");
	await page.reload();
	await expect(page.locator(".app")).toHaveAttribute("data-accent", "amber");
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	expect(
		await page.evaluate(
			() =>
				JSON.parse(localStorage.getItem("jarvis.workspace.v1") ?? "{}").widgets,
		),
	).toEqual(legacy);
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	await expect(
		page.getByRole("checkbox", { name: "Run a simulated diagnostic" }),
	).toBeChecked();
	await expect(
		page.getByRole("checkbox", { name: "Build the next chapter" }),
	).toBeVisible();
	await page
		.getByRole("button", { name: "Delete objective: Build the next chapter" })
		.click();
	await expect(
		page.getByRole("checkbox", { name: "Build the next chapter" }),
	).toHaveCount(0);
});

test("settings discloses trusted built-in module status without an installer", async ({
	page,
}) => {
	await page.goto("/");
	await page.getByRole("button", { name: "Settings", exact: true }).click();

	const modules = page.getByRole("region", { name: "Built-in modules" });
	await expect(modules).toBeVisible();
	await expect(modules.getByRole("listitem")).toHaveCount(4);
	await expect(modules.getByText("Core navigation")).toBeVisible();
	await expect(modules.getByText("Workspace", { exact: true })).toBeVisible();
	await expect(modules.getByText("Local voice visualization")).toBeVisible();
	await expect(modules.getByText("Appearance and settings")).toBeVisible();
	await expect(modules.getByText("Trusted · active")).toHaveCount(4);
	await expect(
		page.getByRole("button", { name: /install plugin/i }),
	).toHaveCount(0);
});
