import { test, expect } from "@playwright/test";
test("closing a pending command prevents late actions and repeat send does not overlap", async ({
	page,
}) => {
	await page.goto("/");
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	const input = page.getByRole("textbox", { name: "Message Saturday" });
	await input.fill("Run a diagnostic");
	await input.press("Enter");
	await page.getByRole("button", { name: "Close dialog" }).click();
	await page.waitForTimeout(1100);
	await expect(page.locator(".core-panel")).toHaveAttribute(
		"data-state",
		"standby",
	);
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	await expect(
		page.getByText(
			"The pending preview was cancelled when you closed the conversation.",
			{ exact: true },
		),
	).toBeVisible();
	await input.fill("Run a diagnostic");
	await input.press("Enter");
	await input.press("Enter");
	await expect(
		page.getByText(/Starting the diagnostic simulation/),
	).toBeVisible();
	await page.keyboard.press("Escape");
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	await page.getByRole("button", { name: "Activity", exact: true }).click();
	await expect(
		page.getByText("Diagnostic simulation started", { exact: true }),
	).toHaveCount(1);
	await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
	await page
		.getByRole("button", { name: "Cancel diagnostic", exact: true })
		.click();
});
