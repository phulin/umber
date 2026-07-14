import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readdir, readFile, rm, stat } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const directory = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(directory, "../../..");
const packageDirectory = path.resolve(
	process.argv[2] ?? path.join(repository, "target/umber-wasm-package"),
);
const chrome =
	process.env.CHROME ??
	(process.platform === "darwin"
		? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
		: "/usr/bin/google-chrome");
const encoder = new TextEncoder();

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const objectEntry = (virtualPath, bytes, dependencies = []) => {
	const sha256 = digest(bytes);
	return {
		virtualPath,
		object: `sha256-${sha256}`,
		sha256,
		bytes: bytes.byteLength,
		dependencies,
	};
};

await stat(path.join(packageDirectory, "umber_wasm_bg.wasm"));
await stat(chrome);
const packageFiles = await readdir(packageDirectory);
assert(
	!packageFiles.some((name) => name.endsWith(".test.js")),
	"tests leaked into package",
);
for (const required of [
	"compile.js",
	"compile.d.ts",
	"worker-entry.js",
	"worker-controller.js",
	"worker-controller.d.ts",
	"manifest-resolver.js",
	"manifest-resolver.d.ts",
	"html-preview.js",
	"html-preview.d.ts",
	"cm-fonts.js",
	"cm-fonts.d.ts",
	"umber_wasm.js",
	"umber_wasm.d.ts",
	"THIRD_PARTY_NOTICES.md",
]) {
	assert(packageFiles.includes(required), `package is missing ${required}`);
}

const remote = encoder.encode("\\message{remote-loaded}");
const cmr10 = await readFile(
	path.join(repository, "crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"),
);
const cmtt10 = await readFile(
	path.join(repository, "crates/tex-fonts/tests/fixtures/cm/cmtt10.tfm"),
);
const corruptExpected = encoder.encode("expected object");
const corruptActual = encoder.encode("tampered object");
const plain = await readFile(path.join(packageDirectory, "assets/plain.fmt"));
const plainMetadata = JSON.parse(
	await readFile(
		path.join(packageDirectory, "assets/plain-format.json"),
		"utf8",
	),
);
const files = {
	"tex:remote.tex": objectEntry("/texlive/tex/remote.tex", remote),
	"tfm:cmr10.tfm": objectEntry("/texlive/fonts/cmr10.tfm", cmr10),
	"tfm:cmtt10.tfm": objectEntry("/texlive/fonts/cmtt10.tfm", cmtt10),
	"tex:corrupt.tex": objectEntry("/texlive/tex/corrupt.tex", corruptExpected),
};
const objectBytes = new Map([
	[files["tex:remote.tex"].object, remote],
	[files["tfm:cmr10.tfm"].object, cmr10],
	[files["tfm:cmtt10.tfm"].object, cmtt10],
	[files["tex:corrupt.tex"].object, corruptActual],
	[plainMetadata.object, plain],
]);
const { name: formatName, schema: _schema, ...formatMetadata } = plainMetadata;
const manifest = {
	schema: 1,
	distribution: "umber-browser-fixture",
	objectsBaseUrl: "",
	files,
	formats: { [formatName]: formatMetadata },
};
const statistics = { objectRequests: 0, active: 0, maximumActive: 0 };

const server = http.createServer(async (request, response) => {
	try {
		const url = new URL(request.url, "http://fixture.invalid");
		if (url.pathname === "/manifest.json") {
			const body = JSON.stringify({
				...manifest,
				objectsBaseUrl: `http://127.0.0.1:${server.address().port}/objects/`,
			});
			return send(response, 200, body, "application/json");
		}
		if (url.pathname === "/stats") {
			return send(
				response,
				200,
				JSON.stringify(statistics),
				"application/json",
			);
		}
		if (url.pathname === "/fixture-cmr10.tfm") {
			return send(response, 200, cmr10, "application/octet-stream");
		}
		if (url.pathname.startsWith("/objects/")) {
			const bytes = objectBytes.get(url.pathname.slice("/objects/".length));
			if (bytes === undefined)
				return send(response, 404, "missing", "text/plain");
			statistics.objectRequests += 1;
			statistics.active += 1;
			statistics.maximumActive = Math.max(
				statistics.maximumActive,
				statistics.active,
			);
			await new Promise((resolve) => setTimeout(resolve, 25));
			statistics.active -= 1;
			response.setHeader(
				"cache-control",
				"public, max-age=31536000, immutable",
			);
			return send(response, 200, bytes, "application/octet-stream");
		}
		const staticRoot = url.pathname.startsWith("/package/")
			? packageDirectory
			: directory;
		const relative = url.pathname.startsWith("/package/")
			? url.pathname.slice("/package/".length)
			: url.pathname === "/"
				? "fixture.html"
				: url.pathname.slice(1);
		const candidate = path.resolve(staticRoot, relative);
		if (!candidate.startsWith(`${staticRoot}${path.sep}`)) {
			return send(response, 403, "forbidden", "text/plain");
		}
		const bytes = await readFile(candidate);
		return send(response, 200, bytes, contentType(candidate));
	} catch (error) {
		if (error?.code === "ENOENT")
			return send(response, 404, "missing", "text/plain");
		response.destroy(error);
	}
});

await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const profile = await mkdtemp(path.join(os.tmpdir(), "umber-chrome-"));
const browser = spawn(
	chrome,
	[
		"--headless=new",
		"--disable-gpu",
		"--no-first-run",
		"--no-default-browser-check",
		"--remote-debugging-port=0",
		`--user-data-dir=${profile}`,
		origin,
	],
	{ stdio: ["ignore", "ignore", "pipe"] },
);
let browserError = "";
browser.stderr.on("data", (chunk) => {
	browserError += chunk;
});

try {
	const port = await debuggingPort(profile, browser);
	const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then(
		(reply) => reply.json(),
	);
	const target = targets.find((candidate) => candidate.type === "page");
	assert(target, "Chrome did not expose a page target");
	const cdp = await protocol(target.webSocketDebuggerUrl);
	try {
		await cdp.call("Runtime.enable");
		const result = await poll(async () => {
			const evaluated = await cdp.call("Runtime.evaluate", {
				expression: "globalThis.__umberResult",
				returnByValue: true,
			});
			return evaluated.result.value;
		});
		assert(result.ok, result.error);
		const geometryMatrix = [];
		for (const deviceScaleFactor of [1, 2]) {
			await cdp.call("Emulation.setDeviceMetricsOverride", {
				width: 1280,
				height: 900,
				deviceScaleFactor,
				mobile: false,
			});
			for (const zoom of [1, 1.25, 2]) {
				const measured = await cdp.call("Runtime.evaluate", {
					expression: `globalThis.__umberGeneratedGeometry(${zoom})`,
					returnByValue: true,
				});
				geometryMatrix.push(measured.result.value);
			}
		}
		process.stdout.write(
			`browser integration passed ${JSON.stringify({ ...result.value, geometryMatrix })}\n`,
		);
	} finally {
		cdp.close();
	}
} catch (error) {
	throw new Error(`${error.stack}\nChrome stderr:\n${browserError}`);
} finally {
	browser.kill("SIGTERM");
	server.close();
	await rm(profile, { recursive: true, force: true });
}

function send(response, status, body, type) {
	response.writeHead(status, {
		"content-type": type,
		"content-length": Buffer.byteLength(body),
	});
	response.end(body);
}

function contentType(file) {
	if (file.endsWith(".js")) return "text/javascript";
	if (file.endsWith(".wasm")) return "application/wasm";
	if (file.endsWith(".json")) return "application/json";
	if (file.endsWith(".html")) return "text/html";
	return "application/octet-stream";
}

async function debuggingPort(profile, child) {
	const activePort = path.join(profile, "DevToolsActivePort");
	for (let attempt = 0; attempt < 200; attempt += 1) {
		if (child.exitCode !== null)
			throw new Error(`Chrome exited ${child.exitCode}`);
		try {
			return Number((await readFile(activePort, "utf8")).split("\n")[0]);
		} catch (error) {
			if (error.code !== "ENOENT") throw error;
		}
		await new Promise((resolve) => setTimeout(resolve, 25));
	}
	throw new Error("Chrome debugging endpoint did not start");
}

async function protocol(url) {
	const socket = new WebSocket(url);
	await new Promise((resolve, reject) => {
		socket.addEventListener("open", resolve, { once: true });
		socket.addEventListener("error", reject, { once: true });
	});
	let next = 1;
	const pending = new Map();
	socket.addEventListener("message", (event) => {
		const message = JSON.parse(event.data);
		const request = pending.get(message.id);
		if (request === undefined) return;
		pending.delete(message.id);
		if (message.error) request.reject(new Error(message.error.message));
		else request.resolve(message.result);
	});
	return {
		call(method, params = {}) {
			const id = next++;
			return new Promise((resolve, reject) => {
				pending.set(id, { resolve, reject });
				socket.send(JSON.stringify({ id, method, params }));
			});
		},
		close() {
			socket.close();
		},
	};
}

async function poll(operation) {
	for (let attempt = 0; attempt < 600; attempt += 1) {
		const value = await operation();
		if (value !== undefined) return value;
		await new Promise((resolve) => setTimeout(resolve, 50));
	}
	throw new Error("browser integration timed out");
}
