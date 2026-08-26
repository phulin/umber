import assert from "node:assert/strict";
import test from "node:test";

import {
	HtmlPatchError,
	HtmlPatchMount,
	HtmlResourceRegistry,
} from "./html-patch.js";

class FakeNode {
	constructor(document, name, namespace = "html") {
		this.ownerDocument = document;
		this.name = name;
		this.namespace = namespace;
		this.children = [];
		this.parentNode = null;
		this.attributes = new Map();
		this.style = {};
		this.textContent = "";
		this.scrollTop = 0;
		this.scrollLeft = 0;
		this.replaceCount = 0;
	}

	set className(value) {
		if (this.namespace.endsWith("/svg")) {
			throw new TypeError("SVGElement.className is read-only");
		}
		this.htmlClassName = value;
	}

	get className() {
		return this.htmlClassName ?? "";
	}

	append(...nodes) {
		for (const node of nodes) {
			if (node.name === "#fragment") {
				this.append(...[...node.children]);
				continue;
			}
			node.remove();
			node.parentNode = this;
			this.children.push(node);
		}
	}

	insertBefore(node, before) {
		node.remove();
		const index =
			before === null ? this.children.length : this.children.indexOf(before);
		if (index < 0) throw new Error("missing before child");
		node.parentNode = this;
		this.children.splice(index, 0, node);
	}

	replaceChildren(...nodes) {
		this.replaceCount += 1;
		for (const child of this.children) child.parentNode = null;
		this.children = [];
		this.append(...nodes);
	}

	remove() {
		if (!this.parentNode) return;
		const index = this.parentNode.children.indexOf(this);
		if (index >= 0) this.parentNode.children.splice(index, 1);
		this.parentNode = null;
	}

	replaceWith(node) {
		if (!this.parentNode) throw new Error("detached replacement");
		const parent = this.parentNode;
		const index = parent.children.indexOf(this);
		this.parentNode = null;
		node.remove();
		node.parentNode = parent;
		parent.children[index] = node;
	}

	setAttribute(name, value) {
		this.attributes.set(name, String(value));
	}

	focus() {
		this.ownerDocument.activeElement = this;
	}
}

class FakeDocument {
	constructor() {
		this.activeElement = null;
		this.fonts = {
			values: new Set(),
			add: (value) => this.fonts.values.add(value),
			delete: (value) => this.fonts.values.delete(value),
		};
	}

	createElement(name) {
		return new FakeNode(this, name);
	}

	createElementNS(namespace, name) {
		return new FakeNode(this, name, namespace);
	}

	createDocumentFragment() {
		return new FakeNode(this, "#fragment");
	}
}

const key = (digit) => digit.repeat(32);
const digest = (digit) => digit.repeat(16);

function page(pageKey, ordinal, nodeKey, text) {
	return {
		key: pageKey,
		ordinal,
		widthSp: 1_000,
		heightSp: 2_000,
		originXSp: 0,
		originYSp: 0,
		mag: 1_000,
		nodes: [
			{
				key: nodeKey,
				kind: "text",
				xSp: 10,
				baselineSp: 20,
				text,
				family: `umber-font-${"a".repeat(24)}`,
				fontSizeSp: 655_360,
				positionsSp: [10],
				features: [],
				variations: [],
				direction: "ltr",
			},
		],
	};
}

function snapshot() {
	return {
		kind: "snapshot",
		schemaVersion: 1,
		sessionId: key("a"),
		revision: 1,
		digest: digest("b"),
		title: "Test",
		language: "en",
		resources: [],
		pages: [
			page(key("1"), 1, key("2"), "one"),
			page(key("3"), 2, key("4"), "two"),
		],
	};
}

function renderPatch(overrides = {}) {
	return {
		kind: "patch",
		schemaVersion: 1,
		sessionId: key("a"),
		baseRevision: 1,
		targetRevision: 2,
		beforeDigest: digest("b"),
		afterDigest: digest("c"),
		resourceAdditions: [],
		resourceReleases: [],
		operations: [],
		...overrides,
	};
}

test("valid patch preserves untouched DOM identity, focus, scroll, and mounted root", async () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	root.scrollTop = 120;
	const mount = new HtmlPatchMount(root, { document });
	await mount.mountSnapshot(snapshot());
	const unchangedPage = mount.nodeForKey(key("3"));
	const unchangedText = mount.nodeForKey(key("4"));
	unchangedText.focus();
	const changed = { ...snapshot().pages[0].nodes[0], text: "changed" };
	const patch = {
		kind: "patch",
		schemaVersion: 1,
		sessionId: key("a"),
		baseRevision: 1,
		targetRevision: 2,
		beforeDigest: digest("b"),
		afterDigest: digest("c"),
		resourceAdditions: [],
		resourceReleases: [],
		operations: [{ kind: "update-node", page: key("1"), node: changed }],
	};
	const acknowledgement = await mount.applyPatch(patch);

	assert.equal(acknowledgement.revision, 2);
	assert.equal(mount.nodeForKey(key("3")), unchangedPage);
	assert.equal(mount.nodeForKey(key("4")), unchangedText);
	assert.equal(document.activeElement, unchangedText);
	assert.equal(root.scrollTop, 120);
	assert.equal(root.replaceCount, 1, "ordinary patches never replace the root");
	assert.notEqual(mount.nodeForKey(key("2")), snapshot().pages[0].nodes[0]);
	assert.equal(mount.metrics.updated, 1);
});

test("typed nodes project complete page, text, rule, special, and math geometry", async () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	const mount = new HtmlPatchMount(root, { document });
	const value = snapshot();
	value.pages = [
		{
			...value.pages[0],
			originXSp: 30,
			originYSp: 40,
			nodes: [
				{
					key: key("2"),
					kind: "box",
					boxId: 7,
					boxKind: "hbox",
					xSp: 1,
					ySp: 2,
					widthSp: 3,
					heightSp: 4,
					baselineSp: 5,
				},
				{
					key: key("3"),
					kind: "rule",
					xSp: 6,
					ySp: 7,
					widthSp: 8,
					heightSp: 9,
					color: "#aabbcc",
				},
				{
					key: key("4"),
					kind: "special",
					xSp: 10,
					ySp: 11,
					class: "dvi",
					payloadHex: "00ff",
					action: "inert",
				},
				{ ...page(key("1"), 1, key("5"), "text").nodes[0] },
				{
					key: key("6"),
					kind: "math-glyph",
					xSp: 12,
					baselineSp: 13,
					widthSp: 14,
					heightSp: 15,
					depthSp: 2,
					glyphId: 16,
					ssty: 1,
					fontInstance: digest("d"),
					drawing: "text",
					text: "x",
					family: `umber-font-${"a".repeat(24)}`,
					fontSizeSp: 655_360,
					variations: [],
				},
			],
		},
	];
	await mount.mountSnapshot(value);
	const pageElement = mount.nodeForKey(key("1"));
	assert.equal(pageElement.style.position, "relative");
	assert.notEqual(pageElement.children[0].style.left, "0px");
	assert.equal(mount.nodeForKey(key("2")).className, "umber-box");
	assert.equal(mount.nodeForKey(key("3")).style.background, "currentColor");
	assert.equal(
		mount.nodeForKey(key("4")).attributes.get("data-umber-special-hex"),
		"00ff",
	);
	assert.match(
		mount.nodeForKey(key("5")).children[1].attributes.get("x"),
		/px$/u,
	);
	assert.equal(
		mount.nodeForKey(key("6")).children[0].children[0].textContent,
		"x",
	);
});

test("page removal derives descendants from the validated base model", async () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	const mount = new HtmlPatchMount(root, { document });
	await mount.mountSnapshot(snapshot());
	const retainedPage = mount.nodeForKey(key("3"));
	const patch = {
		kind: "patch",
		schemaVersion: 1,
		sessionId: key("a"),
		baseRevision: 1,
		targetRevision: 2,
		beforeDigest: digest("b"),
		afterDigest: digest("c"),
		resourceAdditions: [],
		resourceReleases: [],
		operations: [{ kind: "remove-page", key: key("1") }],
	};

	await mount.applyPatch(patch);
	assert.equal(mount.nodeForKey(key("1")), null);
	assert.equal(mount.nodeForKey(key("2")), null);
	assert.equal(mount.nodeForKey(key("3")), retainedPage);
	assert.equal(root.children.length, 1);
});

test("forward page moves use post-removal target indexes", async () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	const mount = new HtmlPatchMount(root, { document });
	await mount.mountSnapshot(snapshot());
	const first = mount.nodeForKey(key("1"));
	const second = mount.nodeForKey(key("3"));
	await mount.applyPatch({
		kind: "patch",
		schemaVersion: 1,
		sessionId: key("a"),
		baseRevision: 1,
		targetRevision: 2,
		beforeDigest: digest("b"),
		afterDigest: digest("c"),
		resourceAdditions: [],
		resourceReleases: [],
		operations: [{ kind: "move-page", key: key("1"), index: 1 }],
	});
	assert.deepEqual(root.children, [second, first]);
	assert.equal(mount.nodeForKey(key("1")), first);
	assert.equal(mount.nodeForKey(key("3")), second);
});

test("invalid and hostile patches perform no mutation and request recovery", async () => {
	const changed = { ...snapshot().pages[0].nodes[0], text: "changed" };
	const unsafe = { ...changed, key: key("5"), link: "javascript:alert(1)" };
	const cases = [
		["patch-schema", renderPatch({ schemaVersion: 2 })],
		["session-mismatch", renderPatch({ sessionId: key("f") })],
		["stale-base", renderPatch({ baseRevision: 0 })],
		["stale-base", renderPatch({ beforeDigest: digest("f") })],
		["target", renderPatch({ targetRevision: 3 })],
		["operation-budget", renderPatch({ operations: null })],
		["resources", renderPatch({ resourceAdditions: null })],
		[
			"unknown-operation",
			renderPatch({ operations: [{ kind: "execute-script" }] }),
		],
		[
			"missing-key",
			renderPatch({ operations: [{ kind: "remove-page", key: key("f") }] }),
		],
		[
			"index",
			renderPatch({
				operations: [
					{ kind: "move-page", key: key("1"), index: Number.MAX_SAFE_INTEGER },
				],
			}),
		],
		[
			"duplicate-key",
			renderPatch({
				operations: [
					{
						kind: "insert-node",
						page: key("1"),
						index: 1,
						node: changed,
					},
				],
			}),
		],
		[
			"node-kind-change",
			renderPatch({
				operations: [
					{
						kind: "update-node",
						page: key("1"),
						node: { key: key("2"), kind: "math-end" },
					},
				],
			}),
		],
		[
			"unsafe-link",
			renderPatch({
				operations: [
					{
						kind: "insert-node",
						page: key("1"),
						index: 1,
						node: unsafe,
					},
				],
			}),
		],
		["string-nul", renderPatch({ title: "bad\0title" })],
		["resource-release", renderPatch({ resourceReleases: ["bad"] })],
		["unknown-resource", renderPatch({ resourceReleases: [digest("e")] })],
		[
			"duplicate-resource",
			renderPatch({
				resourceAdditions: [
					{
						identity: digest("e"),
						kind: "font",
						family: "umber-test",
						bytes: new Uint8Array(),
					},
					{
						identity: digest("e"),
						kind: "font",
						family: "umber-test",
						bytes: new Uint8Array(),
					},
				],
			}),
		],
		[
			"special-payload",
			renderPatch({
				operations: [
					{
						kind: "insert-node",
						page: key("1"),
						index: 1,
						node: {
							key: key("5"),
							kind: "special",
							xSp: 0,
							ySp: 0,
							class: "literal",
							payloadHex: "<script>",
							action: "inert",
						},
					},
				],
			}),
		],
	];

	for (const [code, patch] of cases) {
		const document = new FakeDocument();
		const root = new FakeNode(document, "main");
		const mount = new HtmlPatchMount(root, { document });
		await mount.mountSnapshot(snapshot());
		const identity = mount.nodeForKey(key("2"));
		await assert.rejects(
			mount.applyPatch(patch),
			(error) => error instanceof HtmlPatchError && error.code === code,
			code,
		);
		assert.equal(mount.revision, 1, code);
		assert.equal(mount.nodeForKey(key("2")), identity, code);
		assert.equal(root.replaceCount, 1, code);
		assert.equal(mount.needsResync, true, code);
	}
});

test("host limits can only tighten browser validation budgets", () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	assert.throws(
		() =>
			new HtmlPatchMount(root, {
				document,
				limits: { maxOperations: Number.POSITIVE_INFINITY },
			}),
		RangeError,
	);
});

test("browser validation enforces structural and aggregate string budgets", async () => {
	const applyWithLimits = async (limits, patch, code) => {
		const document = new FakeDocument();
		const root = new FakeNode(document, "main");
		const mount = new HtmlPatchMount(root, { document, limits });
		await mount.mountSnapshot(snapshot());
		await assert.rejects(
			mount.applyPatch(patch),
			(error) => error instanceof HtmlPatchError && error.code === code,
		);
		assert.equal(root.replaceCount, 1);
		assert.equal(mount.needsResync, true);
	};
	const changed = { ...snapshot().pages[0].nodes[0], text: "changed" };
	await applyWithLimits(
		{ maxOperations: 0 },
		renderPatch({
			operations: [{ kind: "update-node", page: key("1"), node: changed }],
		}),
		"operation-budget",
	);
	await applyWithLimits(
		{ maxResources: 0 },
		renderPatch({
			resourceAdditions: [
				{
					identity: digest("e"),
					kind: "asset",
					bytes: new Uint8Array(),
				},
			],
		}),
		"resources",
	);
	await applyWithLimits(
		{ maxStringBytes: 40 },
		renderPatch({ title: "x".repeat(41) }),
		"string-budget",
	);
	await applyWithLimits(
		{ maxTotalStringBytes: 90 },
		renderPatch({
			operations: [
				{
					kind: "update-node",
					page: key("1"),
					node: { ...changed, text: "x".repeat(20) },
				},
			],
		}),
		"string-total-budget",
	);

	for (const [limits, code] of [
		[{ maxPages: 1 }, "page-budget"],
		[{ maxNodes: 1 }, "node-budget"],
	]) {
		const document = new FakeDocument();
		const root = new FakeNode(document, "main");
		const mount = new HtmlPatchMount(root, { document, limits });
		await assert.rejects(
			mount.mountSnapshot(snapshot()),
			(error) => error instanceof HtmlPatchError && error.code === code,
		);
		assert.equal(root.replaceCount, 0);
	}
});

test("publication exceptions restore the validated base tree before resync", async () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	const mount = new HtmlPatchMount(root, { document });
	await mount.mountSnapshot(snapshot());
	mount.nodeForKey(key("2")).replaceWith = () => {
		throw new Error("injected DOM failure");
	};
	const patch = {
		kind: "patch",
		schemaVersion: 1,
		sessionId: key("a"),
		baseRevision: 1,
		targetRevision: 2,
		beforeDigest: digest("b"),
		afterDigest: digest("c"),
		resourceAdditions: [],
		resourceReleases: [],
		operations: [
			{
				kind: "update-node",
				page: key("1"),
				node: { ...snapshot().pages[0].nodes[0], text: "candidate" },
			},
		],
	};

	await assert.rejects(
		mount.applyPatch(patch),
		(error) => error instanceof HtmlPatchError && error.code === "apply-failed",
	);
	assert.equal(mount.revision, 1);
	assert.equal(mount.needsResync, true);
	assert.equal(root.children.length, 2);
	assert.equal(mount.nodeForKey(key("2")).children[1].textContent, "one");
});

test("resource leases deduplicate, reject conflicts, and reclaim after acknowledgement", async () => {
	const document = new FakeDocument();
	const registry = new HtmlResourceRegistry({
		document,
		verify: async () => true,
		FontFace: class FakeFontFace {
			async load() {
				return this;
			}
		},
	});
	const resource = {
		identity: digest("d"),
		kind: "font",
		family: "umber-test",
		bytes: new Uint8Array([1, 2, 3]),
	};
	const first = await registry.stage([resource, resource]);
	await first.commit([], [resource.identity]);
	assert.deepEqual(registry.metrics, { count: 1, bytes: 3, churnBytes: 3 });
	await assert.rejects(
		registry.stage([{ ...resource, bytes: new Uint8Array([9]) }]),
		/resource identity has conflicting bytes/,
	);
	const release = await registry.stage([]);
	await release.commit([resource.identity], []);
	assert.deepEqual(registry.metrics, { count: 0, bytes: 0, churnBytes: 3 });
	await registry.dispose();
});

test("resource cancellation, font failure, lifetime checks, and churn budgets fail closed", async () => {
	const document = new FakeDocument();
	const registry = new HtmlResourceRegistry({
		document,
		verify: async () => true,
		maxResourceBytes: 3,
		maxBytes: 4,
		maxChurnBytes: 5,
		FontFace: class FakeFontFace {
			async load() {
				return this;
			}
		},
	});
	const firstResource = {
		identity: digest("e"),
		kind: "font",
		family: "umber-first",
		bytes: new Uint8Array([1, 2, 3]),
	};
	const cancelled = await registry.stage([firstResource]);
	await cancelled.rollback();
	assert.deepEqual(registry.metrics, { count: 0, bytes: 0, churnBytes: 3 });
	const secondResource = {
		identity: digest("f"),
		kind: "asset",
		bytes: new Uint8Array([4, 5]),
	};
	const accepted = await registry.stage([secondResource]);
	await accepted.commit([], [secondResource.identity]);
	assert.deepEqual(registry.metrics, { count: 1, bytes: 2, churnBytes: 5 });
	await assert.rejects(
		registry.stage([], [secondResource.identity, secondResource.identity], []),
		(error) =>
			error instanceof HtmlPatchError && error.code === "duplicate-release",
	);
	const release = await registry.stage([]);
	await assert.rejects(
		release.commit([secondResource.identity], [secondResource.identity]),
		/cannot release a live resource/,
	);
	assert.equal(registry.metrics.count, 1);
	await assert.rejects(
		registry.stage([
			{
				identity: digest("9"),
				kind: "asset",
				bytes: new Uint8Array([6]),
			},
		]),
		/cumulative resource churn exceeded/,
	);
	await registry.dispose();
	assert.equal(registry.metrics.count, 0);

	const failing = new HtmlResourceRegistry({
		document,
		verify: async () => true,
		FontFace: class FailingFontFace {
			async load() {
				throw new Error("font rejected");
			}
		},
	});
	await assert.rejects(failing.stage([firstResource]), /font rejected/);
	assert.equal(failing.metrics.count, 0);

	const corrupt = new HtmlResourceRegistry({
		document,
		verify: async () => false,
		FontFace: null,
	});
	await assert.rejects(
		corrupt.stage([firstResource]),
		(error) =>
			error instanceof HtmlPatchError && error.code === "resource-digest",
	);
	assert.equal(corrupt.metrics.count, 0);
});
