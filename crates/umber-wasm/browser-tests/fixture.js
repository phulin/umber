import { loadComputerModernTextFont } from "/package/cm-fonts.js";
import { compile } from "/package/compile.js";
import { HttpManifestResolver } from "/package/manifest-resolver.js";
import { renderedSourceLocationFromPoint } from "/package/source-map.js";
import initWasm, { CompilerSession, contentHash } from "/package/umber_wasm.js";
import { compileInWorker } from "/package/worker-controller.js";

const encode = (value) => new TextEncoder().encode(value);

function assert(condition, message) {
	if (!condition) throw new Error(message);
}

async function sha256Hex(bytes) {
	const digest = await crypto.subtle.digest("SHA-256", bytes);
	return [...new Uint8Array(digest)]
		.map((byte) => byte.toString(16).padStart(2, "0"))
		.join("");
}

async function rejected(operation, code) {
	try {
		await operation();
	} catch (error) {
		assert(error?.code === code, `expected ${code}, received ${error?.code}`);
		return error;
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
	const manifestSha256 = await fetch("/manifest.sha256").then((response) =>
		response.text(),
	);
	const resolver = {
		manifestUrl,
		manifestSha256,
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
	const sessionOptions = {
		mainPath: "main.tex",
		clock: { year: 1970, month: 1, day: 1, minutes: 0 },
	};
	const first = await compileInWorker(sessionOptions, files, resolver);
	assert(first.dvi.byteLength > 0, "cold compilation returned no DVI");
	assert(first.log instanceof Uint8Array, "log is not binary");
	assert(first.terminal.includes("remote-loaded"), "remote input did not run");
	const auxiliary = first.files.find((file) =>
		file.path.endsWith("result.aux"),
	);
	assert(auxiliary?.bytes instanceof Uint8Array, "auxiliary output is absent");
	const expectedNativeDvi = await fetch("/native-dvi.sha256").then((response) =>
		response.text(),
	);
	assert(
		(await sha256Hex(first.dvi)) === expectedNativeDvi,
		"native and browser cold DVI differ",
	);

	const cold = await fetch("/stats").then((response) => response.json());
	assert(
		cold.objectRequests === 4,
		`expected one shard and 3 cold DVI objects, got ${cold.objectRequests}`,
	);
	assert(
		cold.networkRequests === 5,
		`expected one root, one shard, and 3 cold objects, got ${cold.networkRequests} requests`,
	);
	assert(cold.maximumActive >= 2, "TFM/object downloads were not concurrent");
	const second = await compileInWorker(sessionOptions, files, resolver);
	assert(
		second.dvi.byteLength === first.dvi.byteLength &&
			second.dvi.every((byte, index) => byte === first.dvi[index]),
		"warm DVI changed",
	);
	const warm = await fetch("/stats").then((response) => response.json());
	assert(
		warm.networkRequests === cold.networkRequests,
		"warm IndexedDB run performed a network request",
	);
	const offlineOutput = await compileInWorker(sessionOptions, files, {
		...resolver,
		offline: true,
	});
	assert(
		offlineOutput.dvi.byteLength === first.dvi.byteLength &&
			offlineOutput.dvi.every((byte, index) => byte === first.dvi[index]),
		"offline DVI changed",
	);
	const offline = await fetch("/stats").then((response) => response.json());
	assert(
		offline.networkRequests === cold.networkRequests,
		"offline IndexedDB run performed a network request",
	);

	await initWasm();
	const cmr10 = new Uint8Array(
		await fetch("/fixture-cmr10.tfm").then((response) =>
			response.arrayBuffer(),
		),
	);
	const htmlFont = await loadComputerModernTextFont(
		"cmr10",
		contentHash(cmr10),
	);
	const htmlOptions = {
		mainPath: "html.tex",
		html: { fonts: [htmlFont] },
	};
	const htmlFiles = new Map([
		[
			"html.tex",
			encode(
				"\\font\\tenrm=cmr10\\relax\\shipout\\hbox{\\kern-2pt\\vrule width3pt height4pt depth1pt\\tenrm AV office}\\end",
			),
		],
	]);
	const retained = new CompilerSession(htmlOptions);
	retained.addUserFile("cmr10.tfm", cmr10);
	retained.addUserFile("html.tex", htmlFiles.get("html.tex"));
	const retainedMissing = retained.advance();
	assert(
		retainedMissing.kind === "need-resources",
		"retained session did not request its HTML font",
	);
	retained.provideResources(
		retainedMissing.required.map((request) => ({
			...request,
			container: "woff2",
			bytes: htmlFont.woff2,
			provenance: htmlFont.provenance,
		})),
	);
	const retainedAttempt = retained.advance();
	assert(
		retainedAttempt.kind === "complete",
		`retained HTML compile failed: ${JSON.stringify(retainedAttempt)}`,
	);
	const htmlFirst = retainedAttempt.output;
	const retainedRepeat = retained.advance();
	assert(retainedRepeat.kind === "complete", "retained output reread failed");
	const htmlSecond = retainedRepeat.output;
	assert(
		htmlFirst.html instanceof Uint8Array,
		"session returned no HTML bytes",
	);
	assert(htmlFirst.dvi.byteLength > 0, "joint HTML compile returned no DVI");
	assert(htmlFirst.htmlAssets.length === 0, "embedded HTML returned assets");
	assert(
		htmlFirst.html.length === htmlSecond.html.length &&
			htmlFirst.html.every((byte, index) => byte === htmlSecond.html[index]),
		"accepted session HTML reread was not deterministic",
	);
	const generatedGeometry = await installAndMeasureGeneratedHtml(
		htmlFirst.html,
	);
	const clickSource = assertClickToSource(
		retained,
		htmlFiles.get("html.tex"),
		htmlFont.encoding,
	);
	retained.dispose();
	globalThis.__umberGeneratedGeometry = (zoom = 1) =>
		measureGeneratedHtml(zoom);

	const plain = await compileInWorker(
		{ mainPath: "main.tex", html: { fonts: [htmlFont] } },
		new Map([["main.tex", encode("Plain browser format.\\par\\bye")]]),
		{ ...resolver, format: "plain" },
	);
	assert(plain.dvi.byteLength > 0, "Plain format returned no DVI");
	assert(plain.html instanceof Uint8Array, "Plain format returned no HTML");
	const plainPageMargins = serializedPageMargins(plain.html);
	assert(
		plainPageMargins.left === plainPageMargins.right,
		`Plain horizontal margins differ: ${JSON.stringify(plainPageMargins)}`,
	);
	assert(
		plainPageMargins.top === plainPageMargins.bottom,
		`Plain vertical margins differ: ${JSON.stringify(plainPageMargins)}`,
	);

	const direct = await HttpManifestResolver.create({
		manifestUrl,
		manifestSha256,
	});
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
	const missingInputError = await rejected(
		() =>
			compileInWorker(
				{ mainPath: "main.tex" },
				new Map([["main.tex", encode("\\input absent \\end")]]),
				resolver,
			),
		"compile",
	);
	assert(
		missingInputError.diagnostic?.message.includes("absent"),
		"missing input did not preserve its compile diagnostic",
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
	manifest.objectsBaseUrl = "../escape/";
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
		plainPageMargins,
		htmlBytes: htmlFirst.html.byteLength,
		clickSource,
		geometry: generatedGeometry,
	};
}

function serializedPageMargins(bytes) {
	const html = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
	const doc = new DOMParser().parseFromString(html, "text/html");
	const page = doc.querySelector(".umber-page");
	const root = page?.querySelector('.umber-box[data-umber-event="0"]');
	assert(page && root, "serialized HTML has no page/root box");
	const number = (element, name) => Number(element.getAttribute(name));
	const originX = number(page, "data-umber-origin-x-sp");
	const originY = number(page, "data-umber-origin-y-sp");
	const left = originX + number(root, "data-umber-x-sp");
	const top = originY + number(root, "data-umber-y-sp");
	return {
		left,
		right:
			number(page, "data-umber-width-sp") -
			left -
			number(root, "data-umber-width-sp"),
		top,
		bottom:
			number(page, "data-umber-height-sp") -
			top -
			number(root, "data-umber-height-sp"),
	};
}

function assertClickToSource(session, source, encoding) {
	const iframe = document.querySelector("#generated-html-fixture");
	const doc = iframe.contentDocument;
	const text = doc.querySelector(".umber-run-text");
	const node = text.firstChild;
	const range = doc.createRange();
	range.setStart(node, 0);
	range.setEnd(node, 1);
	const rect = range.getBoundingClientRect();
	const location = renderedSourceLocationFromPoint(
		session,
		doc,
		rect.left + Math.min(0.1, rect.width / 10),
		rect.top + rect.height / 2,
		{ encoding },
	);
	const expected = new TextDecoder().decode(source).indexOf("AV");
	assert(location?.kind === "current", "click did not resolve current source");
	assert(location.path === "/job/html.tex", "click resolved the wrong file");
	assert(location.start === expected, "click resolved the wrong source offset");
	return { path: location.path, start: location.start };
}

async function installAndMeasureGeneratedHtml(bytes) {
	const iframe = document.createElement("iframe");
	iframe.id = "generated-html-fixture";
	iframe.style.cssText = "border:0;width:900px;height:500px";
	document.body.append(iframe);
	const loaded = new Promise((resolve) =>
		iframe.addEventListener("load", resolve, { once: true }),
	);
	iframe.srcdoc = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
	await loaded;
	await iframe.contentDocument.fonts.ready;
	assert(
		iframe.contentDocument.fonts.status === "loaded",
		"generated fonts did not settle",
	);
	for (const run of iframe.contentDocument.querySelectorAll(
		".umber-run-text",
	)) {
		const style = iframe.contentWindow.getComputedStyle(run.parentElement);
		assert(
			iframe.contentDocument.fonts.check(
				`${style.fontSize} ${style.fontFamily}`,
				run.textContent,
			),
			"generated run font did not load or cover its text",
		);
		assert(
			fontCovers(iframe.contentDocument, style, run.textContent),
			"generated face lacks a mapped glyph and would fall back",
		);
	}
	return measureGeneratedHtml(1);
}

function fontCovers(doc, style, text) {
	const context = doc.createElement("canvas").getContext("2d");
	for (const character of new Set(text)) {
		context.font = `${style.fontSize} ${style.fontFamily}, monospace`;
		const mono = context.measureText(character).width;
		context.font = `${style.fontSize} ${style.fontFamily}, serif`;
		const serif = context.measureText(character).width;
		if (Math.abs(mono - serif) > 0.01) return false;
	}
	return true;
}

function measureGeneratedHtml(zoom) {
	const iframe = document.querySelector("#generated-html-fixture");
	const doc = iframe.contentDocument;
	const page = doc.querySelector(".umber-page");
	page.style.zoom = String(zoom);
	const pageRect = page.getBoundingClientRect();
	const mag = Number(page.dataset.umberMag);
	const px = (raw) => (Number(raw) * mag * 48) / (65536 * 5 * 7227);
	const originX = Number(page.dataset.umberOriginXSp);
	const originY = Number(page.dataset.umberOriginYSp);
	const tolerance = 1 / 30 + 1e-6;
	const close = (actual, expected, label) =>
		assert(
			Math.abs(actual - expected) <= tolerance,
			`${label} at zoom ${zoom}: ${actual} != ${expected}`,
		);
	close(pageRect.width, px(page.dataset.umberWidthSp) * zoom, "page width");
	close(pageRect.height, px(page.dataset.umberHeightSp) * zoom, "page height");
	const rootBox = page.querySelector('.umber-box[data-umber-event="0"]');
	assert(rootBox, "generated HTML has no root page box event");
	const rootRect = rootBox.getBoundingClientRect();
	const pageMargins = {
		left: rootRect.left - pageRect.left,
		right: pageRect.right - rootRect.right,
		top: rootRect.top - pageRect.top,
		bottom: pageRect.bottom - rootRect.bottom,
	};
	close(pageMargins.left, pageMargins.right, "horizontal page margins");
	close(pageMargins.top, pageMargins.bottom, "vertical page margins");
	const rule = page.querySelector(".umber-rule");
	assert(rule, "generated HTML has no rule event");
	const ruleRect = rule.getBoundingClientRect();
	close(
		ruleRect.left - pageRect.left,
		px(originX + Number(rule.dataset.umberXSp)) * zoom,
		"rule x",
	);
	close(
		ruleRect.top - pageRect.top,
		px(originY + Number(rule.dataset.umberYSp)) * zoom,
		"rule y",
	);
	close(ruleRect.width, px(rule.dataset.umberWidthSp) * zoom, "rule width");
	close(ruleRect.height, px(rule.dataset.umberHeightSp) * zoom, "rule height");
	assert(
		Number(rule.dataset.umberXSp) < 0,
		"negative generated coordinate was not exercised",
	);
	const run = page.querySelector(".umber-run");
	const baseline = run.querySelector(".umber-baseline").getBoundingClientRect();
	close(
		baseline.left - pageRect.left,
		px(originX + Number(run.dataset.umberXSp)) * zoom,
		"run x",
	);
	close(
		baseline.top - pageRect.top,
		px(originY + Number(run.dataset.umberBaselineSp)) * zoom,
		"baseline",
	);
	return {
		fontLoaded: true,
		zoom,
		dpr: devicePixelRatio,
		ruleX: ruleRect.left - pageRect.left,
		pageMargins,
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
