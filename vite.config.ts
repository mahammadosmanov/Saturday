import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
export default defineConfig({
	plugins: [react()],
	clearScreen: false,
	server: {
		host: "127.0.0.1",
		port: 1420,
		strictPort: true,
		watch: { ignored: ["**/src-tauri/**"] },
	},
	build: { target: "safari15", sourcemap: false },
	test: { include: ["src/**/*.test.ts"], environment: "node" },
});
