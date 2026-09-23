import { spawn } from "node:child_process";
import { access, readdir, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export async function runBrowserFixture(fixtureUrl, temporary) {
	const executable = await browserExecutable();
	if (executable === undefined) return undefined;
	const profile = path.join(temporary, "chrome");
	const browser = spawn(
		executable,
		[
			"--headless",
			"--no-sandbox",
			"--disable-gpu",
			"--remote-debugging-port=0",
			"--remote-allow-origins=*",
			`--user-data-dir=${profile}`,
			"about:blank",
		],
		{ stdio: "ignore" },
	);
	try {
		const port = await waitForDebugPort(browser, profile);
		const response = await fetch(
			`http://127.0.0.1:${port}/json/new?${encodeURIComponent(fixtureUrl)}`,
			{ method: "PUT" },
		);
		if (!response.ok)
			throw new Error(
				`Chrome debugging endpoint returned HTTP ${response.status}`,
			);
		const tab = await response.json();
		return await evaluateIntegration(tab.webSocketDebuggerUrl);
	} finally {
		browser.kill();
	}
}

async function browserExecutable() {
	if (process.env.CHROME_BIN) {
		try {
			await access(process.env.CHROME_BIN);
			return process.env.CHROME_BIN;
		} catch {
			return undefined;
		}
	}
	for (const name of [
		"chromium",
		"chromium-browser",
		"google-chrome",
		"google-chrome-stable",
	]) {
		for (const directory of (process.env.PATH ?? "").split(path.delimiter)) {
			const candidate = path.join(directory, name);
			try {
				await access(candidate);
				return candidate;
			} catch {}
		}
	}
	const cache = path.join(os.homedir(), ".cache/ms-playwright");
	try {
		for (const entry of (await readdir(cache)).sort().reverse()) {
			if (!entry.startsWith("chromium_headless_shell-")) continue;
			const candidate = path.join(cache, entry, "chrome-linux/headless_shell");
			try {
				await access(candidate);
				return candidate;
			} catch {}
		}
	} catch {}
	return undefined;
}

async function waitForDebugPort(browser, profile) {
	const deadline = Date.now() + 10_000;
	while (Date.now() < deadline) {
		try {
			const content = await readFile(
				path.join(profile, "DevToolsActivePort"),
				"utf8",
			);
			return Number(content.split("\n", 1)[0]);
		} catch {
			if (browser.exitCode !== null)
				throw new Error(`browser exited with ${browser.exitCode}`);
			await new Promise((resolve) => setTimeout(resolve, 100));
		}
	}
	throw new Error("browser did not expose a debugging port");
}

async function evaluateIntegration(url) {
	const socket = new WebSocket(url);
	await new Promise((resolve, reject) => {
		socket.addEventListener("open", resolve, { once: true });
		socket.addEventListener("error", reject, { once: true });
	});
	let nextId = 0;
	const pending = new Map();
	const exceptions = [];
	socket.addEventListener("close", () => {
		for (const { reject } of pending.values())
			reject(new Error("browser debugging connection closed"));
		pending.clear();
	});
	socket.addEventListener("message", ({ data }) => {
		const message = JSON.parse(data);
		if (message.method === "Runtime.exceptionThrown") {
			exceptions.push(
				message.params.exceptionDetails.exception?.description ??
					message.params.exceptionDetails.text,
			);
		}
		if (!pending.has(message.id)) return;
		const { resolve, reject } = pending.get(message.id);
		pending.delete(message.id);
		if (message.error) reject(new Error(message.error.message));
		else resolve(message.result);
	});
	const send = (method, params) =>
		new Promise((resolve, reject) => {
			const id = ++nextId;
			pending.set(id, { resolve, reject });
			socket.send(JSON.stringify({ id, method, params }));
		});
	try {
		await send("Runtime.enable");
		const deadline = Date.now() + 60_000;
		while (Date.now() < deadline) {
			const result = await send("Runtime.evaluate", {
				expression: "globalThis.umberBrowserIntegration ?? null",
				awaitPromise: true,
				returnByValue: true,
			});
			if (result.exceptionDetails)
				throw new Error(
					result.exceptionDetails.exception?.description ??
						result.exceptionDetails.text,
				);
			if (result.result.value !== null && result.result.value !== undefined)
				return result.result.value;
			if (exceptions.length > 0) throw new Error(exceptions.join("\n"));
			await new Promise((resolve) => setTimeout(resolve, 100));
		}
		throw new Error(
			`browser fixture did not finish within 60 seconds: ${exceptions.join("; ")}`,
		);
	} finally {
		socket.close();
	}
}
