import { compile } from "/package/compile.js";
import { HttpManifestResolver } from "/package/manifest-resolver.js";
import { compileInWorker } from "/package/worker-controller.js";

const encode = (value) => new TextEncoder().encode(value);

function assert(condition, message) {
	if (!condition) throw new Error(message);
}

async function rejected(operation, code) {
	try {
		await operation();
	} catch (error) {
		assert(error?.code === code, `expected ${code}, received ${error?.code}`);
		return;
	}
	throw new Error(`expected rejection ${code}`);
}

async function resetCache() {
	await new Promise((resolve, reject) => {
		const request = indexedDB.deleteDatabase("umber-texlive-cache");
		request.onsuccess = resolve;
		request.onerror = () => reject(request.error);
		request.onblocked = () => reject(new Error("cache deletion was blocked"));
	});
}

async function integration() {
	await resetCache();
	const manifestUrl = new URL("/manifest.json", location.href).href;
	const resolver = {
		manifestUrl,
		persistentCache: "indexeddb",
		concurrency: 3,
	};
	const source = encode(
		"\\input remote \\font\\a=cmr10\\relax \\font\\b=cmtt10\\relax " +
			"\\immediate\\openout0=result.aux " +
			"\\immediate\\write0{browser fixture aux}\\immediate\\closeout0 " +
			"\\shipout\\hbox{\\a A\\b B}\\end",
	);
	const files = new Map([["main.tex", source]]);
	const first = await compileInWorker(
		{ mainPath: "main.tex" },
		files,
		resolver,
	);
	assert(first.dvi.byteLength > 0, "cold compilation returned no DVI");
	assert(first.log instanceof Uint8Array, "log is not binary");
	assert(first.terminal.includes("remote-loaded"), "remote input did not run");
	const auxiliary = first.files.find((file) =>
		file.path.endsWith("result.aux"),
	);
	assert(auxiliary?.bytes instanceof Uint8Array, "auxiliary output is absent");

	const cold = await fetch("/stats").then((response) => response.json());
	assert(
		cold.objectRequests === 3,
		`expected 3 cold objects, got ${cold.objectRequests}`,
	);
	assert(cold.maximumActive >= 2, "TFM/object downloads were not concurrent");
	const second = await compileInWorker(
		{ mainPath: "main.tex" },
		files,
		resolver,
	);
	assert(
		second.dvi.byteLength === first.dvi.byteLength,
		"warm DVI length changed",
	);
	const warm = await fetch("/stats").then((response) => response.json());
	assert(warm.objectRequests === 3, "warm IndexedDB run fetched an object");

	const plain = await compileInWorker(
		{ mainPath: "main.tex" },
		new Map([["main.tex", encode("Plain browser format.\\par\\bye")]]),
		{ ...resolver, format: "plain" },
	);
	assert(plain.dvi.byteLength > 0, "Plain format returned no DVI");

	const direct = await HttpManifestResolver.create({ manifestUrl });
	const directOutput = await compile(
		{ mainPath: "main.tex" },
		new Map([["main.tex", encode("\\shipout\\hbox{}\\end")]]),
		direct,
	);
	assert(
		directOutput.dvi.byteLength > 0,
		"default facade did not initialize WASM",
	);
	await rejected(
		() => direct.resolve([{ kind: "tex", name: "corrupt.tex" }]),
		"object-digest",
	);
	await rejected(
		() =>
			compileInWorker(
				{ mainPath: "main.tex" },
				new Map([["main.tex", encode("\\input absent \\end")]]),
				resolver,
			),
		"resolve",
	);
	await rejected(
		() =>
			compileInWorker(
				{ mainPath: "main.tex", limits: { userSourceBytes: 1 } },
				new Map([["main.tex", encode("\\end")]]),
				resolver,
			),
		"limit",
	);
	await rejected(
		() =>
			compileInWorker(
				{ mainPath: "main.tex" },
				new Map([["main.tex", encode("\\def\\loop{\\loop}\\loop")]]),
				resolver,
				{ timeoutMs: 50 },
			),
		"timeout",
	);

	const manifest = await fetch(manifestUrl).then((response) => response.json());
	manifest.files["tex:remote.tex"].virtualPath = "/texlive/../escape.tex";
	assert(
		(() => {
			try {
				new HttpManifestResolver(manifest);
				return false;
			} catch (error) {
				return error.code === "invalid-manifest";
			}
		})(),
		"traversal manifest was accepted",
	);

	return {
		dviBytes: first.dvi.byteLength,
		auxBytes: auxiliary.bytes.byteLength,
		coldObjects: cold.objectRequests,
		maximumActive: cold.maximumActive,
		plainDviBytes: plain.dvi.byteLength,
	};
}

integration().then(
	(value) => {
		globalThis.__umberResult = { ok: true, value };
	},
	(error) => {
		globalThis.__umberResult = {
			ok: false,
			error: error instanceof Error ? `${error.stack}` : String(error),
		};
	},
);
