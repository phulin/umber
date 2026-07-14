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
	const geometry = await htmlGeometryContract();

	return {
		dviBytes: first.dvi.byteLength,
		auxBytes: auxiliary.bytes.byteLength,
		coldObjects: cold.objectRequests,
		maximumActive: cold.maximumActive,
		plainDviBytes: plain.dvi.byteLength,
		geometry,
	};
}

async function htmlGeometryContract() {
	const style = document.createElement("style");
	style.textContent = `
		@font-face{font-family:'umber-contract-cm';src:url('/package/assets/cmu-serif-500-roman.woff2') format('woff2');font-display:block}
		#geometry-page{position:relative;width:400px;height:240px;contain:strict}
		.contract-run{position:absolute;left:0;top:0;width:0;height:0;overflow:visible;font:32px 'umber-contract-cm';font-kerning:normal;font-variant-ligatures:common-ligatures}
		#geometry-rule{position:absolute;left:31.125px;top:88.375px;width:47.625px;height:3.25px}
	`;
	document.head.append(style);
	await document.fonts.load("32px umber-contract-cm", "AV office");
	assert(document.fonts.check("32px umber-contract-cm", "AV office"), "pinned CM web font did not load");
	const page = document.createElement("div");
	page.id = "geometry-page";
	page.innerHTML = `
		<svg class="contract-run"><rect id="geometry-baseline" x="17.375px" y="73.625px" width="0" height="0"></rect><text id="geometry-run" x="17.375px" y="73.625px">AV office</text></svg>
		<svg class="contract-run" style="font-kerning:none;font-variant-ligatures:none"><rect id="geometry-baseline-unshaped" x="17.375px" y="123.625px" width="0" height="0"></rect><text id="geometry-run-unshaped" x="17.375px" y="123.625px">AV office</text></svg>
		<div id="geometry-negative" style="position:absolute;left:-2.375px;top:-1.625px;width:1px;height:1px"></div>
		<div id="geometry-rule"></div>`;
	document.body.append(page);
	const pageRect = page.getBoundingClientRect();
	const run = page.querySelector("#geometry-run");
	const plain = page.querySelector("#geometry-run-unshaped");
	const baseline = page.querySelector("#geometry-baseline").getBoundingClientRect();
	const plainBaseline = page.querySelector("#geometry-baseline-unshaped").getBoundingClientRect();
	const rule = page.querySelector("#geometry-rule").getBoundingClientRect();
	const negative = page.querySelector("#geometry-negative").getBoundingClientRect();
	const tolerance = 1 / 60 + 1e-6;
	const close = (actual, expected, label) => assert(Math.abs(actual - expected) <= tolerance, `${label}: ${actual} != ${expected}`);
	close(baseline.left, pageRect.left + 17.375, "run anchor");
	close(baseline.top, pageRect.top + 73.625, "run baseline");
	close(plainBaseline.top, pageRect.top + 123.625, "unshaped baseline");
	close(rule.left, pageRect.left + 31.125, "rule left");
	close(rule.top, pageRect.top + 88.375, "rule top");
	close(rule.width, 47.625, "rule width");
	close(rule.height, 3.25, "rule height");
	close(negative.left, pageRect.left - 2.375, "negative x");
	close(negative.top, pageRect.top - 1.625, "negative y");
	return {
		fontLoaded: true,
		shapedWidth: run.getBoundingClientRect().width,
		unshapedWidth: plain.getBoundingClientRect().width,
		baseline: baseline.top - pageRect.top,
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
