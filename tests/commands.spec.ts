import { test, expect } from "@playwright/test";
test("command palette supports keyboard search, execution, empty results and focus restoration", async ({
	page,
}) => {
	await page.goto("/");
	await page.locator("#command-trigger").focus();
	await page.keyboard.press("Meta+k");
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toBeVisible();
	const input = page.getByRole("combobox", { name: "Search commands" });
	await expect(input).toBeFocused();
	await input.fill("chat");
	await input.press("Enter");
	await expect(
		page.getByRole("heading", { name: "Conversation", exact: true }),
	).toBeVisible();
	await expect(page.getByRole("dialog", { name: "Conversation" })).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(page.locator("#command-trigger")).toBeFocused();
	await page.keyboard.press("Control+k");
	await input.fill("no-such-command");
	await expect(
		page.getByText("No matching commands. Try “voice” or “conversation”."),
	).toBeVisible();
	await input.press("Enter");
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(page.getByRole("dialog")).toHaveCount(0);
});
for (const [query, title] of [["chat", "Conversation"]]) {
	test(`exiting command palette cannot swallow Escape for ${title}`, async ({
		page,
	}) => {
		await page.goto("/");
		await page.addStyleTag({
			content:
				'.minimal-app .modal-content[data-state="closed"] { animation-duration: 600ms !important; }',
		});
		await page.locator("#command-trigger").focus();
		await page.keyboard.press("Meta+k");
		const search = page.getByRole("combobox", { name: "Search commands" });
		await search.fill(query);
		// Deliver Escape at the replacement's first committed DOM state,
		// before the exiting layer has had time to leave Radix's stack.
		await page.evaluate((dialogTitle) => {
			const observer = new MutationObserver(() => {
				const active = [
					...document.querySelectorAll('[role="dialog"][data-state="open"]'),
				].find(
					(element) => element.querySelector("h2")?.textContent === dialogTitle,
				);
				if (!active) return;
				observer.disconnect();
				document.documentElement.dataset.escapedDialog = dialogTitle;
				active.dispatchEvent(
					new KeyboardEvent("keydown", {
						key: "Escape",
						bubbles: true,
						cancelable: true,
					}),
				);
			});
			observer.observe(document.body, {
				childList: true,
				subtree: true,
				attributes: true,
			});
		}, title);
		await search.press("Enter");
		await expect(page.locator("html")).toHaveAttribute(
			"data-escaped-dialog",
			title,
		);
		await expect(
			page.locator('[role="dialog"][data-state="open"]'),
		).toHaveCount(0);
		await expect(page.getByRole("dialog")).toHaveCount(0);
		await expect(page.locator("#command-trigger")).toBeFocused();
	});
}

test("conversation is explicitly scripted, supports safe text and cancels pending work", async ({
	page,
}) => {
	await page.goto("/");
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	await expect(
		page.getByText(
			"Scripted text replies. Microphone visualization is local, without recording or transcription.",
			{ exact: true },
		),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Run a diagnostic", exact: true }),
	).toHaveCount(0);
	const input = page.getByRole("textbox", { name: "Message S.A.T.U.R.D.A.Y" });
	await input.fill("status");
	await input.press("Enter");
	await expect(
		page.getByText(
			"The local interface is online. This reply is scripted. No AI provider is connected.",
			{ exact: true },
		),
	).toBeVisible();
	await input.fill("<img src=x onerror=alert(1)>");
	await input.press("Enter");
	await expect(
		page.getByText("<img src=x onerror=alert(1)>", { exact: true }),
	).toBeVisible();
	await page.getByRole("button", { name: "Close dialog" }).click();
	await page.waitForTimeout(1200);
	await expect(page.getByRole("dialog")).toHaveCount(0);
	await expect(
		page.getByRole("region", { name: "Assistant core" }),
	).toHaveAttribute("data-state", "standby");
	await expect(
		page.getByRole("button", { name: "Start listening", exact: true }),
	).toBeVisible();
});
