import { test, expect } from "@playwright/test";

test("closing a pending conversation prevents late replies and duplicate sends", async ({
	page,
}) => {
	await page.goto("/");
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	const input = page.getByRole("textbox", { name: "Message S.A.T.U.R.D.A.Y" });
	await input.fill("status");
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
	await input.fill("status");
	await input.press("Enter");
	await input.press("Enter");
	await expect(
		page.getByText(
			"The local interface is online. This reply is scripted. No AI provider is connected.",
			{ exact: true },
		),
	).toHaveCount(1);
});
