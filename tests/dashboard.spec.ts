import { test, expect } from "@playwright/test";

test("home uses dotted branding, omits removed surfaces, and retains saved data", async ({
	page,
}) => {
	await page.addInitScript(() => {
		localStorage.setItem(
			"saturday.workspace.v1",
			JSON.stringify({
				version: 1,
				accent: "amber",
				motion: false,
				microphoneDeviceId: "saved-microphone",
				objectives: [{ id: "saved", text: "Keep this task", done: false }],
			}),
		);
		localStorage.setItem("saturday.social-studio.v1", "preserve-social-record");
	});
	await page.goto("/");
	await expect(page).toHaveTitle("S.A.T.U.R.D.A.Y — Command Interface");
	await expect(
		page.getByRole("heading", { name: "S.A.T.U.R.D.A.Y", exact: true }),
	).toBeVisible();
	await expect(page.locator("#main-request")).toBeDisabled();

	for (const label of ["Social Studio", "Workspace", "Settings"]) {
		await expect(page.getByRole("button", { name: label, exact: true })).toHaveCount(
			0,
		);
	}

	await page.locator("#command-trigger").focus();
	await page.keyboard.press("Meta+k");
	const search = page.getByRole("combobox", { name: "Search commands" });
	for (const query of ["tasks", "diagnostic", "activity", "settings"]) {
		await search.fill(query);
		await expect(
			page.getByRole("listbox").getByRole("option"),
		).toHaveCount(0);
	}
	await page.keyboard.press("Escape");

	const saved = await page.evaluate(() => ({
		workspace: JSON.parse(
			localStorage.getItem("saturday.workspace.v1") ?? "null",
		),
		social: localStorage.getItem("saturday.social-studio.v1"),
	}));
	expect(saved.workspace.objectives).toContainEqual(
		expect.objectContaining({ id: "saved", text: "Keep this task" }),
	);
	expect(saved.workspace.microphoneDeviceId).toBe("saved-microphone");
	expect(saved.social).toBe("preserve-social-record");
});
