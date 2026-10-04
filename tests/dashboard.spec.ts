import { test, expect } from "@playwright/test";
test("compact home exposes the demo boundary and working workspace navigation", async ({
	page,
}) => {
	await page.goto("/");
	await expect(
		page.getByRole("heading", { name: "J.A.R.V.I.S", exact: true }),
	).toBeVisible();
	await expect(
		page.getByText("Local preview · no AI connected", { exact: true }),
	).toBeVisible();
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
	await expect(
		page.getByRole("heading", { name: "Diagnostics", exact: true }),
	).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(
		page.getByRole("heading", { name: "J.A.R.V.I.S", exact: true }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Workspace", exact: true }),
	).toBeFocused();
});
