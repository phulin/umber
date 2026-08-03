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
const digest = (digit) => digit.repeat(64);

function page(pageKey, ordinal, nodeKey, text) {
	return {
		key: pageKey,
		ordinal,
		widthSp: 1_000,
		heightSp: 2_000,
		mag: 1_000,
		nodes: [
			{
				key: nodeKey,
				kind: "text",
				xSp: 10,
				baselineSp: 20,
				text,
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

test("invalid and hostile patches perform no mutation and request recovery", async () => {
	const document = new FakeDocument();
	const root = new FakeNode(document, "main");
	const mount = new HtmlPatchMount(root, { document });
	await mount.mountSnapshot(snapshot());
	const identity = mount.nodeForKey(key("2"));
	const invalid = {
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
				kind: "insert-node",
				page: key("1"),
				index: 1,
				node: {
					key: key("5"),
					kind: "text",
					text: "x",
					link: "javascript:alert(1)",
				},
			},
		],
	};

	await assert.rejects(
		mount.applyPatch(invalid),
		(error) => error instanceof HtmlPatchError && error.code === "unsafe-link",
	);
	assert.equal(mount.revision, 1);
	assert.equal(mount.nodeForKey(key("2")), identity);
	assert.equal(root.replaceCount, 1);
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
	assert.deepEqual(registry.metrics, { count: 1, bytes: 3 });
	await assert.rejects(
		registry.stage([{ ...resource, bytes: new Uint8Array([9]) }]),
		/resource identity has conflicting bytes/,
	);
	const release = await registry.stage([]);
	await release.commit([resource.identity], []);
	assert.deepEqual(registry.metrics, { count: 0, bytes: 0 });
	await registry.dispose();
});
