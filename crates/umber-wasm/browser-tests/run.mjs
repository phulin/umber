import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
	mkdtemp,
	readdir,
	readFile,
	rm,
	stat,
	writeFile,
} from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const directory = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(directory, "../../..");
const arguments_ = process.argv.slice(2);
const mathOnly = arguments_.includes("--math-only");
const [packageArgument, nativeBinaryArgument] = arguments_.filter(
	(argument) => !argument.startsWith("--"),
);
const packageDirectory = path.resolve(
	packageArgument ?? path.join(repository, "target/umber-wasm-package"),
);
const nativeBinary = path.resolve(
	nativeBinaryArgument ?? path.join(repository, "target/debug/umber"),
);
const chrome =
	process.env.CHROME ??
	(process.platform === "darwin"
		? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
		: "/usr/bin/google-chrome");
const chromeSandboxArguments =
	process.env.CHROME_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
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
await stat(nativeBinary);
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
	"source-map.js",
	"source-map.d.ts",
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
const mathWoff2 = await readFile(
	path.join(repository, "crates/tex-fonts/tests/fixtures/stix-two-math.woff2"),
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
const corruptPlain = Uint8Array.from(plain);
corruptPlain[corruptPlain.byteLength - 1] ^= 1;
const corruptPlainSha256 = digest(corruptPlain);
const cmr10Entry = objectEntry("/texlive/fonts/cmr10.tfm", cmr10);
const cmtt10Entry = objectEntry("/texlive/fonts/cmtt10.tfm", cmtt10);
const files = {
	"tex:remote.tex": objectEntry("/texlive/tex/remote.tex", remote, [
		dependencyEntry("tfm:cmr10.tfm", cmr10Entry),
		dependencyEntry("tfm:cmtt10.tfm", cmtt10Entry),
	]),
	"tfm:cmr10.tfm": cmr10Entry,
	"tfm:cmtt10.tfm": cmtt10Entry,
	"tex:corrupt.tex": objectEntry("/texlive/tex/corrupt.tex", corruptExpected),
};

function dependencyEntry(key, entry) {
	return {
		key,
		virtualPath: entry.virtualPath,
		object: entry.object,
		sha256: entry.sha256,
		bytes: entry.bytes,
	};
}
const objectBytes = new Map([
	[files["tex:remote.tex"].object, remote],
	[files["tfm:cmr10.tfm"].object, cmr10],
	[files["tfm:cmtt10.tfm"].object, cmtt10],
	[files["tex:corrupt.tex"].object, corruptActual],
	[plainMetadata.object, plain],
	[`sha256-${corruptPlainSha256}`, corruptPlain],
]);
const { name: formatName, schema: _schema, ...formatMetadata } = plainMetadata;
const shardBytes = encoder.encode(
	`${JSON.stringify({
		schema: 1,
		distribution: "umber-browser-fixture",
		index: 0,
		files,
	})}\n`,
);
const shardDigest = digest(shardBytes);
objectBytes.set(`sha256-${shardDigest}`, shardBytes);
const manifest = () => ({
	schema: 2,
	distribution: "umber-browser-fixture",
	objectsBaseUrl: `http://127.0.0.1:${server.address().port}/objects/`,
	shardBits: 0,
	shardCount: 1,
	shards: [shardDigest],
	formats: {
		[formatName]: formatMetadata,
		"plain-corrupt": {
			...formatMetadata,
			object: `sha256-${corruptPlainSha256}`,
			sha256: corruptPlainSha256,
			bytes: corruptPlain.byteLength,
		},
	},
});
const manifestBytes = () => encoder.encode(`${JSON.stringify(manifest())}\n`);
const statistics = {
	networkRequests: 0,
	objectRequests: 0,
	active: 0,
	maximumActive: 0,
};
let nativeDviSha256;
let browserObjectOverlap;
let releaseBrowserObjectOverlap;

const server = http.createServer(async (request, response) => {
	try {
		const url = new URL(request.url, "http://fixture.invalid");
		if (url.pathname === "/manifest.json") {
			statistics.networkRequests += 1;
			return send(response, 200, manifestBytes(), "application/json");
		}
		if (url.pathname === "/manifest.sha256") {
			return send(response, 200, digest(manifestBytes()), "text/plain");
		}
		if (url.pathname === "/stats") {
			return send(
				response,
				200,
				JSON.stringify(statistics),
				"application/json",
			);
		}
		if (url.pathname === "/native-dvi.sha256") {
			return send(response, 200, nativeDviSha256, "text/plain");
		}
		if (url.pathname === "/fixture-cmr10.tfm") {
			return send(response, 200, cmr10, "application/octet-stream");
		}
		if (url.pathname === "/fixture-stix-two-math.woff2") {
			return send(response, 200, mathWoff2, "font/woff2");
		}
		if (url.pathname.startsWith("/objects/")) {
			const object = url.pathname.slice("/objects/".length);
			const bytes = objectBytes.get(object);
			if (bytes === undefined)
				return send(response, 404, "missing", "text/plain");
			statistics.networkRequests += 1;
			statistics.objectRequests += 1;
			statistics.active += 1;
			statistics.maximumActive = Math.max(
				statistics.maximumActive,
				statistics.active,
			);
			if (
				browserObjectOverlap !== undefined &&
				object !== `sha256-${shardDigest}`
			) {
				if (statistics.active >= 2) releaseBrowserObjectOverlap();
				await withTimeout(
					browserObjectOverlap,
					2_000,
					"browser object downloads did not overlap",
				);
			} else {
				await new Promise((resolve) => setTimeout(resolve, 25));
			}
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
const nativeRoot = await mkdtemp(path.join(os.tmpdir(), "umber-native-fetch-"));
const nativeSource = path.join(nativeRoot, "main.tex");
const nativeDvi = path.join(nativeRoot, "main.dvi");
await writeFile(nativeSource, remoteSource());
const nativeEnvironment = {
	...process.env,
	SOURCE_DATE_EPOCH: "0",
	XDG_CACHE_HOME: path.join(nativeRoot, "cache"),
};
const nativeArguments = [
	"run",
	"--distribution",
	`${origin}/manifest.json`,
	"--distribution-sha256",
	digest(manifestBytes()),
	"--dvi",
	nativeDvi,
	nativeSource,
];
await runNative(nativeArguments, nativeEnvironment, nativeRoot);
const nativeCold = { ...statistics };
assert.equal(nativeCold.networkRequests, 5, "native cold fetch request count");
await runNative(nativeArguments, nativeEnvironment, nativeRoot);
assert.equal(
	statistics.networkRequests,
	nativeCold.networkRequests,
	"native warm run performed a network request",
);
await runNative(
	[...nativeArguments.slice(0, 1), "--offline", ...nativeArguments.slice(1)],
	nativeEnvironment,
	nativeRoot,
);
assert.equal(
	statistics.networkRequests,
	nativeCold.networkRequests,
	"native offline run performed a network request",
);
nativeDviSha256 = digest(await readFile(nativeDvi));
Object.assign(statistics, {
	networkRequests: 0,
	objectRequests: 0,
	active: 0,
	maximumActive: 0,
});
browserObjectOverlap = new Promise((resolve) => {
	releaseBrowserObjectOverlap = resolve;
});
const profile = await mkdtemp(path.join(os.tmpdir(), "umber-chrome-"));
const browser = spawn(
	chrome,
	[
		"--headless=new",
		...chromeSandboxArguments,
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
		if (mathOnly) {
			await cdp.call("Page.enable");
			await cdp.call("Page.navigate", {
				url: `${origin}/html-prototype.html`,
			});
			const mathResult = await poll(async () => {
				const evaluated = await cdp.call("Runtime.evaluate", {
					expression: "globalThis.__mathContract",
					returnByValue: true,
				});
				return evaluated.result.value;
			});
			assert(mathResult.ok, mathResult.error);
			assert.deepEqual(mathResult.cases, [
				"script",
				"fraction",
				"radical",
				"accent",
				"operator",
				"limit",
				"delimiter",
				"assembly",
			]);
			assert.deepEqual(mathResult.coordinates, [
				[30, 45],
				[90, 80],
				[155, 90],
				[225, 55],
				[285, 95],
				[285, 125],
				[370, 95],
				[450, 95],
			]);
			process.stdout.write(
				`browser math integration passed ${JSON.stringify(mathResult)}\n`,
			);
		} else {
			const result = await poll(async () => {
				const evaluated = await cdp.call("Runtime.evaluate", {
					expression: "globalThis.__umberResult",
					returnByValue: true,
				});
				return evaluated.result.value;
			});
			assert(result.ok, result.error);
			assert.equal(result.value.explicitOpenType.requestCount, 1);
			assert.match(result.value.explicitOpenType.text, /^µ £ ¥ é AV office$/u);
			assert.match(result.value.explicitOpenType.fontFamily, /umber-font-/);
			assert(Number.isFinite(result.value.explicitOpenType.xSp));
			assert(Number.isFinite(result.value.explicitOpenType.baselineSp));
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
		}
	} finally {
		cdp.close();
	}
} catch (error) {
	throw new Error(`${error.stack}\nChrome stderr:\n${browserError}`);
} finally {
	await stopBrowser(browser);
	server.close();
	await rm(profile, {
		recursive: true,
		force: true,
		maxRetries: 5,
		retryDelay: 100,
	});
	await rm(nativeRoot, { recursive: true, force: true });
}

function remoteSource() {
	return encoder.encode(
		"\\input remote \\font\\a=cmr10\\relax \\font\\b=cmtt10\\relax " +
			"\\immediate\\openout0=result.aux " +
			"\\immediate\\write0{browser fixture aux}\\immediate\\closeout0 " +
			"\\shipout\\hbox{\\a A\\b B}\\end",
	);
}

async function runNative(args, env, cwd) {
	const child = spawn(nativeBinary, args, {
		cwd,
		env,
		stdio: ["ignore", "pipe", "pipe"],
	});
	let stdout = "";
	let stderr = "";
	child.stdout.on("data", (chunk) => (stdout += chunk));
	child.stderr.on("data", (chunk) => (stderr += chunk));
	const code = await new Promise((resolve, reject) => {
		child.once("error", reject);
		child.once("exit", resolve);
	});
	assert.equal(code, 0, `native fetch run failed:\n${stdout}\n${stderr}`);
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

async function stopBrowser(child) {
	let closed = waitForClose(child, 5_000);
	child.kill("SIGTERM");
	if (await closed) return;

	closed = waitForClose(child, 5_000);
	child.kill("SIGKILL");
	if (!(await closed)) throw new Error("Chrome did not exit after SIGKILL");
}

function waitForClose(child, timeout) {
	if (child.exitCode !== null || child.signalCode !== null) {
		return Promise.resolve(true);
	}
	return new Promise((resolve) => {
		const onClose = () => {
			clearTimeout(timer);
			resolve(true);
		};
		const timer = setTimeout(() => {
			child.off("close", onClose);
			resolve(false);
		}, timeout);
		child.once("close", onClose);
	});
}

function withTimeout(operation, timeout, message) {
	let timer;
	return Promise.race([
		operation,
		new Promise((_, reject) => {
			timer = setTimeout(() => reject(new Error(message)), timeout);
		}),
	]).finally(() => clearTimeout(timer));
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
