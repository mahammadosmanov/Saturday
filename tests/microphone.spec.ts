import { test, expect, type Page } from "@playwright/test";

// Substitute only the physical device boundary. Real Web Audio processes an
// oscillator stream without speaker output; tests never capture the owner's mic.
async function installMicrophone(page: Page, mode = "live") {
	await page.addInitScript((mode) => {
		let gain: GainNode | undefined;
		let stream: MediaStream | undefined;
		let resolve: ((stream: MediaStream) => void) | undefined;
		const fixture = {
			calls: 0,
			requests: [] as MediaStreamConstraints[],
			stops: 0,
			tracks: [] as MediaStreamTrack[],
			states() {
				return fixture.tracks.map((t) => t.readyState);
			},
			volume(value: number) {
				if (gain) gain.gain.value = value;
			},
			grant() {
				resolve?.(createStream());
			},
			ended() {
				stream?.getAudioTracks()[0].dispatchEvent(new Event("ended"));
			},
		};
		function createStream() {
			const context = new AudioContext();
			const oscillator = context.createOscillator();
			gain = context.createGain();
			gain.gain.value = 0;
			oscillator.frequency.value = 220;
			const output = context.createMediaStreamDestination();
			oscillator.connect(gain);
			gain.connect(output);
			oscillator.start();
			void context.resume().catch(() => {});
			stream = output.stream;
			const track = stream.getAudioTracks()[0];
			// Retain the wrapper: WebKit may collect expando-only track wrappers.
			fixture.tracks.push(track);
			const stop = track.stop.bind(track);
			let stopped = false;
			track.stop = () => {
				if (stopped) return;
				stopped = true;
				fixture.stops++;
				stop();
				void context.close().catch(() => {});
			};
			return stream;
		}
		Object.defineProperty(window, "__microphone", { value: fixture });
		Object.defineProperty(navigator, "mediaDevices", {
			configurable: true,
			value: {
				enumerateDevices: async () => [
					{
						kind: "audioinput",
						deviceId: "BuiltIn-MiC",
						label: "MacBook Pro Microphone",
					},
					{
						kind: "audioinput",
						deviceId: "Aggregate-MiC",
						label: "Aggregate Device",
					},
					{ kind: "audiooutput", deviceId: "speakers", label: "Speakers" },
				],
				getUserMedia: (constraints: MediaStreamConstraints) => {
					fixture.requests.push(constraints);
					fixture.calls++;
					if (mode === "missing")
						return Promise.reject(
							new DOMException("fixture missing input", "OverconstrainedError"),
						);
					if (mode === "denied")
						return Promise.reject(
							new DOMException("test denial", "NotAllowedError"),
						);
					if (mode === "pending")
						return new Promise<MediaStream>((r) => {
							resolve = r;
						});
					const result = createStream();
					if (mode === "ended") result.getAudioTracks()[0].stop();
					return Promise.resolve(result);
				},
			},
		});
	}, mode);
}
interface MicrophoneFixture {
	requests: MediaStreamConstraints[];
	states: () => string[];
	calls: number;
	stops: number;
	volume: (value: number) => void;
	grant: () => void;
	ended: () => void;
}
declare global {
	interface Window {
		__microphone: MicrophoneFixture;
	}
}
async function ringPixels(page: Page) {
	return page.locator(".core-particles").evaluate((element) => {
		const canvas = element as HTMLCanvasElement;
		const pixels = canvas
			.getContext("2d")
			?.getImageData(0, 0, canvas.width, canvas.height).data;
		if (!pixels) return 0;
		let count = 0;
		for (let y = 0; y < canvas.height; y++)
			for (let x = 0; x < canvas.width; x++) {
				const radius =
					(Math.hypot(x + 0.5 - canvas.width / 2, y + 0.5 - canvas.height / 2) *
						256) /
					canvas.width;
				if (radius > 104 && pixels[(y * canvas.width + x) * 4 + 3] > 32)
					count++;
			}
		return count;
	});
}

test("microphone reacts to real audio processing and releases capture on Stop", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.goto("/");
	expect(await page.evaluate(() => window.__microphone.calls)).toBe(0);
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText("Listening locally · no recording"),
	).toBeVisible();
	const quiet = await ringPixels(page);
	await page.evaluate(() => window.__microphone.volume(0.3));
	await expect.poll(() => ringPixels(page)).toBeGreaterThan(quiet + 80);
	await page.evaluate(() => window.__microphone.volume(0));
	await expect.poll(() => ringPixels(page)).toBeLessThan(quiet + 80);
	await page
		.getByRole("button", { name: "Stop listening", exact: true })
		.click();
	await expect(
		page.getByRole("button", { name: "Start listening", exact: true }),
	).toBeVisible();
	expect(await page.evaluate(() => window.__microphone.stops)).toBe(1);
	await page.getByRole("button", { name: "Open conversation" }).click();
	await expect(
		page.getByText("Give me a status briefing.", { exact: true }),
	).toHaveCount(0);
});

test("ended input cannot be promoted into a listening session", async ({
	page,
}) => {
	const pageErrors: string[] = [];
	page.on("pageerror", (error) => pageErrors.push(error.message));
	await installMicrophone(page, "ended");
	await page.goto("/");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText(/Microphone (disconnected|unavailable)/),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Start listening", exact: true }),
	).toBeVisible();
	await expect(page.locator(".core-panel")).toHaveAttribute(
		"data-state",
		"standby",
	);
	expect(await page.evaluate(() => window.__microphone.states())).toEqual([
		"ended",
	]);
	await page.waitForTimeout(0);
	expect(pageErrors).toEqual([]);
});

for (const [width, height] of [
	[960, 720],
	[600, 600],
]) {
	test(`permission errors fit ${width}x${height} without scrolling`, async ({
		page,
	}) => {
		await installMicrophone(page, "denied");
		await page.setViewportSize({ width, height });
		await page.goto("/");
		await page
			.getByRole("button", { name: "Start listening", exact: true })
			.click();
		await expect(page.getByText(/Microphone permission denied/)).toBeVisible();
		expect(
			await page.evaluate(() => document.documentElement.scrollHeight),
		).toBeLessThanOrEqual(height);
	});
}

test("microphone selection stays in Settings, persists, and targets only the chosen device", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.goto("/");
	await expect(
		page.getByRole("combobox", { name: "Microphone input" }),
	).toHaveCount(0);
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	const input = page.getByRole("combobox", { name: "Microphone input" });
	await expect(input.locator("option")).toHaveCount(3);
	await input.selectOption("BuiltIn-MiC");
	expect(await page.evaluate(() => window.__microphone.calls)).toBe(0);
	await page.reload();
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	await expect(input).toHaveValue("BuiltIn-MiC");
	await page.keyboard.press("Escape");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText("Listening locally · no recording"),
	).toBeVisible();
	expect(await page.evaluate(() => window.__microphone.requests[0])).toEqual({
		audio: {
			deviceId: { exact: "BuiltIn-MiC" },
			echoCancellation: true,
			noiseSuppression: true,
			autoGainControl: false,
		},
		video: false,
	});
	await page.keyboard.press("Escape");
	expect(await page.evaluate(() => window.__microphone.states())).toEqual([
		"ended",
	]);
});

test("missing saved microphone stays explicit instead of silently using another input", async ({
	page,
}) => {
	await installMicrophone(page, "missing");
	await page.addInitScript(() =>
		localStorage.setItem(
			"saturday.workspace.v1",
			JSON.stringify({ version: 1, microphoneDeviceId: "missing-id" }),
		),
	);
	await page.goto("/");
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	await expect(
		page.getByRole("combobox", { name: "Microphone input" }),
	).toHaveValue("missing-id");
	await expect(
		page.getByRole("option", {
			name: "Saved microphone (not currently available)",
		}),
	).toHaveCount(1);
	await page.keyboard.press("Escape");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText(
			"Selected microphone unavailable. Choose another input in Settings.",
		),
	).toBeVisible();
	expect(await page.evaluate(() => window.__microphone.calls)).toBe(1);
});

test("device-list failure stays recoverable and never requests capture", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.addInitScript(() => {
		navigator.mediaDevices.enumerateDevices = async () => {
			throw new Error("fixture");
		};
	});
	await page.goto("/");
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	await expect(
		page.getByText("Could not list microphones. Reopen Settings to retry."),
	).toBeVisible();
	await expect(
		page.getByRole("combobox", { name: "Microphone input" }),
	).toBeEnabled();
	expect(await page.evaluate(() => window.__microphone.calls)).toBe(0);
});

test("ordinary speech visibly drives the outer core rather than only the center", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.goto("/");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText("Listening locally · no recording"),
	).toBeVisible();
	const outerPixels = () =>
		page.evaluate(() => {
			const canvas =
				document.querySelector<HTMLCanvasElement>(".core-particles");
			const stage = document.querySelector(".core-stage");
			const ctx = canvas?.getContext("2d");
			if (!canvas || !stage || !ctx) throw new Error("Core canvas unavailable");
			const ratio =
				canvas.getBoundingClientRect().width /
				stage.getBoundingClientRect().width;
			const image = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
			let count = 0;
			for (let y = 0; y < canvas.height; y++)
				for (let x = 0; x < canvas.width; x++) {
					const radius =
						(Math.hypot(
							x + 0.5 - canvas.width / 2,
							y + 0.5 - canvas.height / 2,
						) /
							canvas.width) *
						ratio;
					if (radius > 0.36 && image[(y * canvas.width + x) * 4 + 3] > 45)
						count++;
				}
			return count;
		});
	const quiet = await outerPixels();
	await page.evaluate(() => window.__microphone.volume(0.045));
	await expect.poll(outerPixels).toBeGreaterThan(quiet + 150);
	await page.evaluate(() => window.__microphone.volume(0));
	await expect.poll(outerPixels).toBeLessThan(quiet + 50);
	await page.keyboard.press("Escape");
});

test("permission denial stays honest and allows a retry", async ({ page }) => {
	await installMicrophone(page, "denied");
	await page.goto("/");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(page.getByText(/Microphone permission denied/)).toBeVisible();
	await expect(page.locator(".core-panel")).toHaveAttribute(
		"data-state",
		"standby",
	);
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	expect(await page.evaluate(() => window.__microphone.calls)).toBe(2);
});

test("cancelled permission cannot reopen capture when a grant arrives late", async ({
	page,
}) => {
	await installMicrophone(page, "pending");
	await page.goto("/");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText("Waiting for microphone permission…"),
	).toBeVisible();
	await page.getByRole("button", { name: "Cancel microphone request" }).click();
	await page.evaluate(() => window.__microphone.grant());
	await expect
		.poll(() => page.evaluate(() => window.__microphone.stops))
		.toBe(1);
	await expect(
		page.getByRole("button", { name: "Start listening", exact: true }),
	).toBeVisible();
	await expect(page.locator(".core-panel")).toHaveAttribute(
		"data-state",
		"standby",
	);
});

for (const exit of ["Escape", "Workspace", "hidden", "pagehide"] as const) {
	test(`microphone closes on ${exit} and never auto-restarts`, async ({
		page,
	}) => {
		await installMicrophone(page);
		await page.goto("/");
		await page
			.getByRole("button", { name: "Start listening", exact: true })
			.click();
		await expect(
			page.getByRole("button", { name: "Stop listening", exact: true }),
		).toBeVisible();
		if (exit === "Escape") await page.keyboard.press("Escape");
		else if (exit === "Workspace")
			await page
				.getByRole("button", { name: "Workspace", exact: true })
				.click();
		else
			await page.evaluate((exit) => {
				if (exit === "hidden") {
					Object.defineProperty(document, "hidden", {
						configurable: true,
						value: true,
					});
					document.dispatchEvent(new Event("visibilitychange"));
					Object.defineProperty(document, "hidden", {
						configurable: true,
						value: false,
					});
					document.dispatchEvent(new Event("visibilitychange"));
				} else window.dispatchEvent(new Event("pagehide"));
			}, exit);
		expect(await page.evaluate(() => window.__microphone.stops)).toBe(1);
		expect(await page.evaluate(() => window.__microphone.calls)).toBe(1);
	});
}

test("disconnected input releases capture and explains recovery", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.goto("/");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByRole("button", { name: "Stop listening", exact: true }),
	).toBeVisible();
	await page.evaluate(() => window.__microphone.ended());
	await expect(page.getByText(/Microphone disconnected/)).toBeVisible();
	expect(await page.evaluate(() => window.__microphone.stops)).toBe(1);
});

test("microphone opens from conversation without generating a transcript", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.goto("/");
	await page.getByRole("button", { name: "Open conversation" }).click();
	await page
		.getByRole("dialog")
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(page.getByRole("dialog")).toHaveCount(0);
	await expect(
		page.getByText("Listening locally · no recording"),
	).toBeVisible();
	await page.keyboard.press("Escape");
	expect(await page.evaluate(() => window.__microphone.stops)).toBe(1);
});

test("reduced motion freezes the visual while Stop remains usable", async ({
	page,
}) => {
	await installMicrophone(page);
	await page.emulateMedia({ reducedMotion: "reduce" });
	await page.goto("/");
	await page
		.getByRole("button", { name: "Start listening", exact: true })
		.click();
	await expect(
		page.getByText("Listening locally · no recording"),
	).toBeVisible();
	const before = await page
		.locator(".core-particles")
		.evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL());
	await page.evaluate(() => window.__microphone.volume(0.3));
	await page.waitForTimeout(350);
	expect(
		await page
			.locator(".core-particles")
			.evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL()),
	).toBe(before);
	expect(await page.evaluate(() => window.__microphone.states())).toEqual([
		"live",
	]);
	await page
		.getByRole("button", { name: "Stop listening", exact: true })
		.click();
	expect(await page.evaluate(() => window.__microphone.states())).toEqual([
		"ended",
	]);
	expect(await page.evaluate(() => window.__microphone.stops)).toBe(1);
});

test("larger core remains within the default desktop window", async ({
	page,
}) => {
	await page.setViewportSize({ width: 960, height: 720 });
	await page.goto("/");
	const core = await page.locator(".core-stage").boundingBox();
	expect(core?.width).toBeGreaterThanOrEqual(440);
	expect(
		await page.evaluate(() => document.documentElement.scrollHeight),
	).toBeLessThanOrEqual(720);
});
