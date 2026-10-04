import { defineConfig, devices } from "@playwright/test";
export default defineConfig({
	testDir: "./tests",
	fullyParallel: true,
	forbidOnly: true,
	retries: 0,
	workers: 3,
	reporter: [["list"], ["html", { open: "never" }]],
	use: {
		baseURL: "http://127.0.0.1:1420",
		trace: "retain-on-failure",
		screenshot: "only-on-failure",
	},
	projects: [
		{
			name: "chromium",
			use: {
				...devices["Desktop Chrome"],
				viewport: { width: 1440, height: 900 },
			},
		},
		{
			name: "webkit",
			use: {
				...devices["Desktop Safari"],
				viewport: { width: 1440, height: 900 },
			},
		},
	],
	webServer: {
		command: "npm run dev",
		url: "http://127.0.0.1:1420",
		reuseExistingServer: !process.env.CI,
	},
});
