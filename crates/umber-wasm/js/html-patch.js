const HTML_NS = "http://www.w3.org/1999/xhtml";
const SVG_NS = "http://www.w3.org/2000/svg";
const KEY = /^[0-9a-f]{32}$/;
const DIGEST = /^[0-9a-f]{64}$/;
const SESSION = /^[0-9a-f]{32}$/;

const DEFAULT_LIMITS = Object.freeze({
	maxPages: 16_384,
	maxNodes: 1_000_000,
	maxOperations: 250_000,
	maxStringBytes: 16 * 1024 * 1024,
	maxResourceBytes: 256 * 1024 * 1024,
});

export class HtmlPatchError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "HtmlPatchError";
		this.code = code;
	}
}

/** Mounts typed render snapshots and applies schema-1 patches in place. */
export class HtmlPatchMount {
	#root;
	#document;
	#limits;
	#resources;
	#state = null;
	#nodes = new Map();
	#pageContent = new Map();
	#needsResync = false;
	#disposed = false;
	#metrics = freshMetrics();

	constructor(root, options = {}) {
		if (!root || typeof root.replaceChildren !== "function") {
			throw new TypeError("HTML patch root must be a DOM-like element");
		}
		this.#root = root;
		this.#document =
			options.document ?? root.ownerDocument ?? globalThis.document;
		if (!this.#document?.createElement || !this.#document?.createElementNS) {
			throw new TypeError("HTML patch mount requires DOM constructors");
		}
		this.#limits = { ...DEFAULT_LIMITS, ...options.limits };
		this.#resources =
			options.resources ??
			new HtmlResourceRegistry({
				document: this.#document,
				verify: options.verifyResource,
				FontFace: options.FontFace,
				maxBytes: this.#limits.maxResourceBytes,
			});
	}

	get revision() {
		return this.#state?.revision ?? 0;
	}

	get digest() {
		return this.#state?.digest ?? null;
	}

	get needsResync() {
		return this.#needsResync;
	}

	get metrics() {
		return Object.freeze({
			...this.#metrics,
			resources: this.#resources.metrics,
		});
	}

	/** Initial mount or explicit resynchronization. */
	async mountSnapshot(snapshot) {
		this.#assertLive();
		const state = validateSnapshot(snapshot, this.#limits);
		const retained = new Set(
			state.resources.map((resource) => resource.identity),
		);
		const releases = (this.#state?.resources ?? [])
			.map((resource) => resource.identity)
			.filter((identity) => !retained.has(identity));
		const retainedIdentities = state.resources.map(
			(resource) => resource.identity,
		);
		const lease = await this.#resources.stage(
			state.resources,
			releases,
			retainedIdentities,
		);
		const nodes = new Map();
		const pageContent = new Map();
		const fragment = this.#document.createDocumentFragment();
		try {
			for (const page of state.pages) {
				const built = buildPage(this.#document, page, nodes, pageContent);
				fragment.append(built);
			}
			this.#root.replaceChildren(fragment);
			await lease.commit(releases, retainedIdentities);
		} catch (error) {
			await lease.rollback();
			throw new HtmlPatchError(
				"snapshot-apply",
				"snapshot could not be mounted",
				{
					cause: error,
				},
			);
		}
		this.#nodes = nodes;
		this.#pageContent = pageContent;
		this.#state = state;
		this.#needsResync = false;
		this.#metrics.snapshots += 1;
		return this.acknowledgement();
	}

	async applyPatch(patch) {
		this.#assertLive();
		if (!this.#state)
			throw new HtmlPatchError("missing-base", "mount a snapshot first");
		if (this.#needsResync) {
			throw new HtmlPatchError(
				"resync-required",
				"mount requires a full resynchronization",
			);
		}
		if (
			patch?.sessionId === this.#state.sessionId &&
			patch?.targetRevision === this.#state.revision &&
			patch?.afterDigest === this.#state.digest
		) {
			this.#metrics.duplicates += 1;
			return this.acknowledgement();
		}
		let candidate;
		try {
			candidate = simulatePatch(this.#state, patch, this.#limits);
		} catch (error) {
			this.#metrics.resyncs += 1;
			throw error;
		}
		const retainedIdentities = candidate.resources.map(
			(resource) => resource.identity,
		);
		const releases = patch.resourceReleases ?? [];
		const lease = await this.#resources.stage(
			patch.resourceAdditions ?? [],
			releases,
			retainedIdentities,
		);
		let staged;
		try {
			staged = stageInsertions(
				this.#document,
				patch.operations,
				candidate.pages,
			);
		} catch (error) {
			await lease.rollback();
			throw error;
		}
		const preserved = captureUserState(this.#document, this.#root, this.#nodes);
		const started = now();
		try {
			for (const operation of patch.operations) this.#apply(operation, staged);
			restoreUserState(preserved, this.#nodes, this.#root);
			await lease.commit(releases, retainedIdentities);
		} catch (error) {
			await lease.rollback();
			this.#restoreValidatedState();
			this.#needsResync = true;
			this.#metrics.resyncs += 1;
			throw new HtmlPatchError("apply-failed", "patch publication failed", {
				cause: error,
			});
		}
		this.#state = candidate;
		this.#metrics.patches += 1;
		this.#metrics.operations += patch.operations.length;
		this.#metrics.applyMilliseconds += now() - started;
		return this.acknowledgement();
	}

	acknowledgement() {
		this.#assertLive();
		if (!this.#state) return null;
		return Object.freeze({
			kind: "ack",
			schemaVersion: 1,
			sessionId: this.#state.sessionId,
			revision: this.#state.revision,
			digest: this.#state.digest,
		});
	}

	nodeForKey(key) {
		return this.#nodes.get(key) ?? null;
	}

	async dispose() {
		if (this.#disposed) return;
		this.#disposed = true;
		await this.#resources.dispose();
		this.#root.replaceChildren();
		this.#nodes.clear();
		this.#pageContent.clear();
		this.#state = null;
	}

	#assertLive() {
		if (this.#disposed)
			throw new HtmlPatchError("disposed", "HTML patch mount is disposed");
	}

	#restoreValidatedState() {
		const nodes = new Map();
		const pageContent = new Map();
		const fragment = this.#document.createDocumentFragment();
		for (const page of this.#state.pages) {
			fragment.append(buildPage(this.#document, page, nodes, pageContent));
		}
		this.#root.replaceChildren(fragment);
		this.#nodes = nodes;
		this.#pageContent = pageContent;
	}

	#apply(operation, staged) {
		switch (operation.kind) {
			case "remove-node": {
				const node = required(this.#nodes, operation.key);
				node.remove();
				this.#nodes.delete(operation.key);
				this.#metrics.removed += 1;
				break;
			}
			case "remove-page": {
				const model = modelPage(this.#state.pages, operation.key);
				const page = required(this.#nodes, operation.key);
				for (const node of model.nodes) this.#nodes.delete(node.key);
				page.remove();
				this.#nodes.delete(operation.key);
				this.#pageContent.delete(operation.key);
				this.#metrics.removed += model.nodes.length + 1;
				break;
			}
			case "insert-page": {
				const record = staged.get(operation.page.key);
				insertAt(this.#root, record.element, operation.index);
				for (const [key, node] of record.nodes) this.#nodes.set(key, node);
				this.#pageContent.set(operation.page.key, record.content);
				this.#metrics.inserted += operation.page.nodes.length + 1;
				break;
			}
			case "move-page": {
				insertAt(
					this.#root,
					required(this.#nodes, operation.key),
					operation.index,
				);
				this.#metrics.moved += 1;
				break;
			}
			case "insert-node": {
				const content = required(this.#pageContent, operation.page);
				const node = staged.get(operation.node.key).element;
				insertAt(content, node, operation.index);
				this.#nodes.set(operation.node.key, node);
				this.#metrics.inserted += 1;
				break;
			}
			case "move-node": {
				const content = required(this.#pageContent, operation.page);
				insertAt(
					content,
					required(this.#nodes, operation.key),
					operation.index,
				);
				this.#metrics.moved += 1;
				break;
			}
			case "update-page":
				updatePage(
					required(this.#nodes, operation.page.key),
					operation.page,
					required(this.#pageContent, operation.page.key),
				);
				this.#metrics.updated += 1;
				break;
			case "update-node": {
				const old = required(this.#nodes, operation.node.key);
				const replacement = staged.get(operation.node.key).element;
				old.replaceWith(replacement);
				this.#nodes.set(operation.node.key, replacement);
				this.#metrics.updated += 1;
				break;
			}
			default:
				throw new HtmlPatchError(
					"unknown-operation",
					"unknown patch operation",
				);
		}
	}
}

/** Content-addressed FontFace/object ownership behind an acknowledgement barrier. */
export class HtmlResourceRegistry {
	#document;
	#verify;
	#FontFace;
	#maxBytes;
	#maxResourceBytes;
	#maxChurnBytes;
	#churnBytes = 0;
	#entries = new Map();
	#disposed = false;

	constructor(options = {}) {
		this.#document = options.document ?? globalThis.document;
		this.#verify = options.verify ?? verifySha256;
		this.#FontFace = options.FontFace ?? globalThis.FontFace;
		this.#maxBytes = options.maxBytes ?? DEFAULT_LIMITS.maxResourceBytes;
		this.#maxResourceBytes = options.maxResourceBytes ?? 64 * 1024 * 1024;
		this.#maxChurnBytes = options.maxChurnBytes ?? 1024 * 1024 * 1024;
	}

	get metrics() {
		let bytes = 0;
		for (const entry of this.#entries.values()) bytes += entry.bytes.byteLength;
		return Object.freeze({
			count: this.#entries.size,
			bytes,
			churnBytes: this.#churnBytes,
		});
	}

	async stage(additions, releases = [], retained = []) {
		if (this.#disposed)
			throw new HtmlPatchError("disposed", "resource registry is disposed");
		validateReleaseSet(this.#entries, releases, retained);
		let projected = this.metrics.bytes;
		const staged = new Map();
		for (const resource of additions) {
			validateResource(resource);
			if (resource.bytes.byteLength > this.#maxResourceBytes) {
				throw new HtmlPatchError(
					"resource-size",
					"individual resource budget exceeded",
				);
			}
			const existing =
				this.#entries.get(resource.identity) ?? staged.get(resource.identity);
			if (existing) {
				if (!sameBytes(existing.bytes, resource.bytes)) {
					throw new HtmlPatchError(
						"resource-conflict",
						"resource identity has conflicting bytes",
					);
				}
				continue;
			}
			projected += resource.bytes.byteLength;
			if (projected > this.#maxBytes) {
				throw new HtmlPatchError(
					"resource-budget",
					"resident resource budget exceeded",
				);
			}
			if (!(await this.#verify(resource.identity, resource.bytes))) {
				throw new HtmlPatchError(
					"resource-digest",
					"resource digest does not match bytes",
				);
			}
			if (this.#churnBytes + resource.bytes.byteLength > this.#maxChurnBytes) {
				throw new HtmlPatchError(
					"resource-churn",
					"cumulative resource churn exceeded",
				);
			}
			this.#churnBytes += resource.bytes.byteLength;
			const entry = { ...resource, bytes: resource.bytes.slice(), face: null };
			if (resource.kind === "font" && this.#FontFace) {
				entry.face = new this.#FontFace(
					resource.family,
					entry.bytes,
					resource.descriptors ?? {},
				);
				await entry.face.load();
			}
			staged.set(resource.identity, entry);
		}
		let settled = false;
		return {
			commit: async (releases, retained) => {
				if (settled)
					throw new HtmlPatchError(
						"resource-lease",
						"resource lease already settled",
					);
				validateReleaseSet(this.#entries, releases, retained);
				settled = true;
				const installed = [];
				try {
					for (const entry of staged.values()) {
						this.#entries.set(entry.identity, entry);
						if (entry.face) this.#document?.fonts?.add(entry.face);
						installed.push(entry.identity);
					}
					for (const identity of releases) this.#remove(identity);
				} catch (error) {
					for (const identity of installed.reverse()) this.#remove(identity);
					throw error;
				}
			},
			rollback: async () => {
				settled = true;
			},
		};
	}

	async dispose() {
		if (this.#disposed) return;
		this.#disposed = true;
		for (const identity of [...this.#entries.keys()]) this.#remove(identity);
	}

	#remove(identity) {
		const entry = this.#entries.get(identity);
		if (!entry)
			throw new HtmlPatchError(
				"unknown-resource",
				"cannot release an unknown resource",
			);
		if (entry.face) this.#document?.fonts?.delete(entry.face);
		this.#entries.delete(identity);
	}
}

function validateReleaseSet(entries, releases, retained) {
	const live = new Set(retained);
	const uniqueReleases = new Set(releases);
	if (uniqueReleases.size !== releases.length) {
		throw new HtmlPatchError(
			"duplicate-release",
			"resource released more than once",
		);
	}
	for (const identity of releases) {
		if (live.has(identity)) {
			throw new HtmlPatchError(
				"live-resource-release",
				"cannot release a live resource",
			);
		}
		if (!entries.has(identity)) {
			throw new HtmlPatchError(
				"unknown-resource",
				"cannot release an unknown resource",
			);
		}
	}
}

function validateSnapshot(snapshot, limits) {
	if (snapshot?.kind !== "snapshot" || snapshot.schemaVersion !== 1)
		fail("snapshot-schema");
	validateIdentity(snapshot);
	if (!Array.isArray(snapshot.pages) || snapshot.pages.length > limits.maxPages)
		fail("page-budget");
	if (!Array.isArray(snapshot.resources)) fail("resources");
	let nodes = 0;
	const keys = new Set();
	for (const page of snapshot.pages) {
		validatePage(page, keys, limits);
		nodes += page.nodes.length;
	}
	if (nodes > limits.maxNodes) fail("node-budget");
	return cloneState(snapshot);
}

function simulatePatch(base, patch, limits) {
	if (patch?.kind !== "patch" || patch.schemaVersion !== 1)
		fail("patch-schema");
	if (patch.sessionId !== base.sessionId) fail("session-mismatch");
	if (
		patch.baseRevision !== base.revision ||
		patch.beforeDigest !== base.digest
	)
		fail("stale-base");
	if (
		patch.targetRevision !== base.revision + 1 ||
		!DIGEST.test(patch.afterDigest)
	)
		fail("target");
	if (
		!Array.isArray(patch.operations) ||
		patch.operations.length > limits.maxOperations
	) {
		fail("operation-budget");
	}
	const candidate = cloneState(base);
	candidate.revision = patch.targetRevision;
	candidate.digest = patch.afterDigest;
	if (patch.title !== undefined)
		candidate.title = boundedString(patch.title, limits);
	if (patch.language !== undefined)
		candidate.language = boundedString(patch.language, limits);
	candidate.resources = updateResources(base.resources, patch);
	for (const operation of patch.operations)
		simulateOperation(candidate.pages, operation, limits);
	const keys = new Set();
	let nodes = 0;
	for (const page of candidate.pages) {
		validatePage(page, keys, limits);
		nodes += page.nodes.length;
	}
	if (candidate.pages.length > limits.maxPages || nodes > limits.maxNodes)
		fail("node-budget");
	return candidate;
}

function simulateOperation(pages, operation, limits) {
	switch (operation?.kind) {
		case "remove-node": {
			const page = modelPage(pages, operation.page);
			removeByKey(page.nodes, operation.key);
			break;
		}
		case "remove-page":
			removeByKey(pages, operation.key);
			break;
		case "insert-page":
			validateIndex(operation.index, pages.length, true);
			pages.splice(operation.index, 0, structuredCloneValue(operation.page));
			break;
		case "move-page":
			moveByKey(pages, operation.key, operation.index);
			break;
		case "insert-node": {
			const nodes = modelPage(pages, operation.page).nodes;
			validateIndex(operation.index, nodes.length, true);
			nodes.splice(operation.index, 0, structuredCloneValue(operation.node));
			break;
		}
		case "move-node":
			moveByKey(
				modelPage(pages, operation.page).nodes,
				operation.key,
				operation.index,
			);
			break;
		case "update-page": {
			const page = modelPage(pages, operation.page.key);
			const nodes = page.nodes;
			Object.assign(page, structuredCloneValue(operation.page), { nodes });
			break;
		}
		case "update-node": {
			const nodes = modelPage(pages, operation.page).nodes;
			const index = indexByKey(nodes, operation.node.key);
			if (nodes[index].kind !== operation.node.kind) fail("node-kind-change");
			nodes[index] = structuredCloneValue(operation.node);
			break;
		}
		default:
			fail("unknown-operation");
	}
	void limits;
}

function stageInsertions(document, operations, pages) {
	const staged = new Map();
	for (const operation of operations) {
		if (operation.kind === "insert-page") {
			const nodes = new Map();
			const pageContent = new Map();
			const element = buildPage(document, operation.page, nodes, pageContent);
			staged.set(operation.page.key, {
				element,
				nodes,
				content: pageContent.get(operation.page.key),
			});
		} else if (
			operation.kind === "insert-node" ||
			operation.kind === "update-node"
		) {
			const page = modelPage(pages, operation.page);
			const element = buildNode(document, operation.node, page.mag);
			staged.set(operation.node.key, { element });
		}
	}
	return staged;
}

function buildPage(document, page, nodes, pageContent, staged = nodes) {
	const section = document.createElementNS(HTML_NS, "section");
	section.className = "umber-page";
	section.tabIndex = -1;
	section.setAttribute("role", "group");
	section.setAttribute("aria-label", `Page ${page.ordinal}`);
	const content = document.createElementNS(HTML_NS, "div");
	content.className = "umber-page-content";
	updatePage(section, page, content);
	for (const node of page.nodes) {
		const element = buildNode(document, node, page.mag);
		content.append(element);
		nodes.set(node.key, element);
	}
	section.append(content);
	nodes.set(page.key, section);
	staged.set(page.key, section);
	pageContent.set(page.key, content);
	return section;
}

function buildNode(document, node, mag) {
	let element;
	switch (node.kind) {
		case "box": {
			element = document.createElementNS(HTML_NS, "div");
			element.className = "umber-box";
			element.setAttribute("aria-hidden", "true");
			element.style.pointerEvents = "none";
			element.setAttribute("data-umber-box-id", exactUnsigned(node.boxId));
			if (node.boxKind !== "hbox" && node.boxKind !== "vbox") fail("box-kind");
			element.setAttribute("data-umber-box-kind", node.boxKind);
			positionGeometry(element, node, mag);
			break;
		}
		case "rule":
			element = document.createElementNS(HTML_NS, "div");
			element.className = "umber-rule";
			element.setAttribute("aria-hidden", "true");
			positionGeometry(element, node, mag);
			element.style.background = "currentColor";
			applyColor(element, node.color);
			break;
		case "special": {
			element = document.createElementNS(HTML_NS, "span");
			element.className = "umber-special";
			element.setAttribute("aria-hidden", "true");
			element.style.position = "absolute";
			element.style.left = cssPx(node.xSp, mag);
			element.style.top = cssPx(node.ySp, mag);
			element.setAttribute("data-umber-special-class", node.class);
			if (!/^(?:[0-9a-f]{2})*$/u.test(node.payloadHex)) fail("special-payload");
			element.setAttribute("data-umber-special-hex", node.payloadHex);
			if (node.action === "destination") {
				if (!safeDestination(node.actionValue)) fail("special-destination");
				element.id = node.actionValue;
			}
			element.setAttribute(
				"data-umber-special-policy",
				node.action === "inert" ? "inert" : "applied",
			);
			break;
		}
		case "text": {
			element = document.createElementNS(SVG_NS, "svg");
			element.className = "umber-run";
			element.setAttribute("role", "text");
			element.style.position = "absolute";
			element.style.left = "0";
			element.style.top = "0";
			element.style.width = "0";
			element.style.height = "0";
			element.style.overflow = "visible";
			const baseline = document.createElementNS(SVG_NS, "rect");
			baseline.className = "umber-baseline";
			baseline.setAttribute("x", cssPx(node.xSp, mag));
			baseline.setAttribute("y", cssPx(node.baselineSp, mag));
			baseline.setAttribute("width", "1");
			baseline.setAttribute("height", "1");
			baseline.setAttribute("fill", "transparent");
			element.append(baseline);
			const text = document.createElementNS(SVG_NS, "text");
			text.className = "umber-run-text";
			text.style.fill = "currentColor";
			text.style.whiteSpace = "pre";
			text.textContent = boundedString(node.text, DEFAULT_LIMITS);
			if (!/^umber-font-[0-9a-f]{24}$/u.test(node.family)) fail("font-family");
			text.style.fontFamily = node.family;
			text.style.fontSize = cssPx(node.fontSizeSp, mag);
			text.style.fontFeatureSettings = settingStyle(node.features, false);
			text.style.fontVariationSettings = settingStyle(node.variations, true);
			applyColor(element, node.color);
			if (node.direction !== "ltr" && node.direction !== "rtl")
				fail("direction");
			text.setAttribute("direction", node.direction);
			if (node.language !== undefined) text.setAttribute("lang", node.language);
			if (!Array.isArray(node.positionsSp)) fail("positions");
			text.setAttribute(
				"x",
				(node.positionsSp.length > 0 ? node.positionsSp : [node.xSp])
					.map((position) => cssPx(position, mag))
					.join(" "),
			);
			text.setAttribute("y", cssPx(node.baselineSp, mag));
			if (node.link !== undefined && node.link !== null) {
				if (!safeLink(node.link)) fail("unsafe-link");
				const anchor = document.createElementNS(SVG_NS, "a");
				anchor.setAttribute("href", node.link);
				anchor.setAttribute("rel", "noreferrer noopener");
				anchor.append(text);
				element.append(anchor);
			} else {
				element.append(text);
			}
			break;
		}
		case "math-start":
			element = document.createElementNS(SVG_NS, "svg");
			element.className = "umber-math";
			element.setAttribute("aria-hidden", "true");
			zeroSizedSvg(element);
			break;
		case "math-glyph": {
			element = document.createElementNS(SVG_NS, "svg");
			element.className = "umber-math";
			element.setAttribute("aria-hidden", "true");
			zeroSizedSvg(element);
			const glyph = document.createElementNS(SVG_NS, "g");
			glyph.className = "umber-math-glyph";
			glyph.setAttribute("data-umber-glyph-id", exactUnsigned(node.glyphId));
			glyph.setAttribute("data-umber-font-instance", node.fontInstance);
			glyph.setAttribute("data-umber-ssty", exactUnsigned(node.ssty));
			if (node.drawing === "text") {
				const text = document.createElementNS(SVG_NS, "text");
				text.className = "umber-math-text";
				text.style.fill = "currentColor";
				text.textContent = node.text;
				text.setAttribute("x", cssPx(node.xSp, mag));
				text.setAttribute("y", cssPx(node.baselineSp, mag));
				if (!/^umber-font-[0-9a-f]{24}$/u.test(node.family))
					fail("font-family");
				text.style.fontFamily = node.family;
				text.style.fontSize = cssPx(node.fontSizeSp, mag);
				text.style.fontFeatureSettings = `'ssty' ${exactUnsigned(node.ssty)}`;
				text.style.fontVariationSettings = settingStyle(node.variations, true);
				glyph.append(text);
			} else if (node.drawing === "outline") {
				if (!/^[MLQCZ0-9.,+\-\s]+$/u.test(node.path)) fail("outline-path");
				const path = document.createElementNS(SVG_NS, "path");
				path.className = "umber-math-outline";
				path.style.fill = "currentColor";
				path.setAttribute("d", node.path);
				const scale = cssScale(node.fontSizeSp, mag, node.unitsPerEm);
				path.setAttribute(
					"transform",
					`translate(${cssNumber(node.xSp, mag)} ${cssNumber(node.baselineSp, mag)}) scale(${scale} ${-scale})`,
				);
				glyph.append(path);
			} else {
				fail("math-drawing");
			}
			element.append(glyph);
			break;
		}
		case "math-rule": {
			element = document.createElementNS(SVG_NS, "svg");
			element.className = "umber-math";
			element.setAttribute("aria-hidden", "true");
			zeroSizedSvg(element);
			const rule = document.createElementNS(SVG_NS, "rect");
			rule.className = "umber-math-rule";
			rule.style.fill = "currentColor";
			rule.setAttribute("x", cssPx(node.xSp, mag));
			rule.setAttribute("y", cssPx(node.ySp, mag));
			rule.setAttribute("width", cssPx(node.widthSp, mag));
			rule.setAttribute("height", cssPx(node.heightSp, mag));
			element.append(rule);
			break;
		}
		case "math-end":
			element = document.createElementNS(HTML_NS, "span");
			element.hidden = true;
			break;
		default:
			fail("unknown-node-kind");
	}
	element.setAttribute("data-umber-key", node.key);
	element.setAttribute("data-umber-kind", node.kind);
	setGeometry(element, node);
	return element;
}

function updatePage(element, page, content) {
	element.setAttribute("data-umber-key", page.key);
	element.setAttribute("data-umber-page", String(page.ordinal));
	element.setAttribute("aria-label", `Page ${page.ordinal}`);
	element.setAttribute("data-umber-width-sp", exactInteger(page.widthSp));
	element.setAttribute("data-umber-height-sp", exactInteger(page.heightSp));
	element.style.width = cssPx(page.widthSp, page.mag);
	element.style.height = cssPx(page.heightSp, page.mag);
	element.style.position = "relative";
	if (content) {
		content.style.position = "absolute";
		content.style.left = cssPx(page.originXSp, page.mag);
		content.style.top = cssPx(page.originYSp, page.mag);
		content.style.width = "0";
		content.style.height = "0";
		content.style.overflow = "visible";
	}
}

function positionGeometry(element, node, mag) {
	element.style.position = "absolute";
	element.style.left = cssPx(node.xSp, mag);
	element.style.top = cssPx(node.ySp, mag);
	element.style.width = cssPx(node.widthSp, mag);
	element.style.height = cssPx(node.heightSp, mag);
}

function zeroSizedSvg(element) {
	element.style.position = "absolute";
	element.style.left = "0";
	element.style.top = "0";
	element.style.width = "0";
	element.style.height = "0";
	element.style.overflow = "visible";
}

function applyColor(element, color) {
	if (color === undefined || color === null) return;
	if (!safeColor(color)) fail("color");
	element.style.color = color;
}

function settingStyle(settings, signed) {
	if (!Array.isArray(settings)) fail("font-settings");
	return settings
		.map((setting) => {
			if (!/^[A-Za-z0-9 ]{4}$/u.test(setting?.tag)) fail("font-setting-tag");
			const value = signed
				? exactInteger(setting.value)
				: exactUnsigned(setting.value);
			return `'${setting.tag}' ${value}`;
		})
		.join(",");
}

function setGeometry(element, node) {
	for (const [name, value] of [
		["x", node.xSp],
		["y", node.ySp],
		["width", node.widthSp],
		["height", node.heightSp],
		["baseline", node.baselineSp],
	]) {
		if (value !== undefined)
			element.setAttribute(`data-umber-${name}-sp`, exactInteger(value));
	}
}

function validateIdentity(value) {
	if (!SESSION.test(value.sessionId) || !DIGEST.test(value.digest))
		fail("identity");
	if (!Number.isSafeInteger(value.revision) || value.revision < 1)
		fail("revision");
}

function validatePage(page, keys, limits) {
	if (!KEY.test(page?.key) || keys.has(page.key)) fail("duplicate-key");
	keys.add(page.key);
	for (const value of [
		page.ordinal,
		page.widthSp,
		page.heightSp,
		page.originXSp,
		page.originYSp,
		page.mag,
	]) {
		exactInteger(value);
	}
	if (page.mag <= 0) fail("magnification");
	if (!Array.isArray(page.nodes)) fail("nodes");
	for (const node of page.nodes) {
		if (!KEY.test(node?.key) || keys.has(node.key)) fail("duplicate-key");
		keys.add(node.key);
		if (
			![
				"box",
				"rule",
				"text",
				"special",
				"math-start",
				"math-glyph",
				"math-rule",
				"math-end",
			].includes(node.kind)
		) {
			fail("node-kind");
		}
		for (const value of [node.text, node.link, node.color, node.class]) {
			if (value !== undefined && value !== null) boundedString(value, limits);
		}
		if (node.kind === "text") {
			if (!/^umber-font-[0-9a-f]{24}$/u.test(node.family)) fail("font-family");
			exactInteger(node.fontSizeSp);
			if (!Array.isArray(node.positionsSp)) fail("positions");
			for (const position of node.positionsSp) exactInteger(position);
			settingStyle(node.features, false);
			settingStyle(node.variations, true);
		}
		if (node.color !== undefined && !safeColor(node.color)) fail("color");
		if (node.link !== undefined && node.link !== null && !safeLink(node.link))
			fail("unsafe-link");
	}
}

function updateResources(base, patch) {
	const resources = new Map(
		base.map((resource) => [resource.identity, resource]),
	);
	for (const identity of patch.resourceReleases ?? []) {
		if (!resources.delete(identity)) fail("unknown-resource");
	}
	for (const resource of patch.resourceAdditions ?? []) {
		validateResource(resource);
		if (resources.has(resource.identity)) fail("duplicate-resource");
		resources.set(resource.identity, structuredCloneValue(resource));
	}
	return [...resources.values()].sort((left, right) =>
		left.identity.localeCompare(right.identity),
	);
}

function validateResource(resource) {
	if (
		!DIGEST.test(resource?.identity) ||
		!(resource.bytes instanceof Uint8Array)
	)
		fail("resource");
	if (resource.kind !== "font" && resource.kind !== "asset")
		fail("resource-kind");
	if (resource.kind === "font" && typeof resource.family !== "string")
		fail("font-family");
}

function captureUserState(document, root, nodes) {
	const active = document.activeElement;
	const activeKey = [...nodes].find(([, node]) => node === active)?.[0] ?? null;
	const selection = document.getSelection?.();
	return {
		activeKey,
		scrollTop: root.scrollTop,
		scrollLeft: root.scrollLeft,
		selection:
			selection && selection.rangeCount > 0
				? {
						anchor: domAddress(
							selection.anchorNode,
							selection.anchorOffset,
							nodes,
						),
						focus: domAddress(
							selection.focusNode,
							selection.focusOffset,
							nodes,
						),
					}
				: null,
	};
}

function restoreUserState(state, nodes, root) {
	const active = state.activeKey ? nodes.get(state.activeKey) : null;
	if (active?.focus) active.focus({ preventScroll: true });
	else if (state.activeKey) root.focus?.({ preventScroll: true });
	if (state.selection) {
		const selection = root.ownerDocument?.getSelection?.();
		const anchor = resolveDomAddress(state.selection.anchor, nodes);
		const focus = resolveDomAddress(state.selection.focus, nodes);
		if (anchor && focus) {
			selection?.setBaseAndExtent(
				anchor.node,
				anchor.offset,
				focus.node,
				focus.offset,
			);
		} else {
			selection?.removeAllRanges();
		}
	}
	root.scrollTop = state.scrollTop;
	root.scrollLeft = state.scrollLeft;
}

function domAddress(node, offset, nodes) {
	if (!node) return null;
	for (const [key, root] of nodes) {
		const path = descendantPath(root, node);
		if (path) return { key, path, offset };
	}
	return null;
}

function descendantPath(root, target, path = []) {
	if (root === target) return path;
	for (
		let index = 0;
		index < (root.childNodes ?? root.children ?? []).length;
		index += 1
	) {
		const child = (root.childNodes ?? root.children)[index];
		const found = descendantPath(child, target, [...path, index]);
		if (found) return found;
	}
	return null;
}

function resolveDomAddress(address, nodes) {
	if (!address) return null;
	let node = nodes.get(address.key);
	for (const index of address.path) {
		node = (node?.childNodes ?? node?.children)?.[index];
		if (!node) return null;
	}
	const length =
		node.nodeType === 3 ? node.data.length : (node.childNodes?.length ?? 0);
	return { node, offset: Math.min(address.offset, length) };
}

function insertAt(parent, node, index) {
	if (node.parentNode === parent) node.remove();
	const before = parent.children?.[index] ?? null;
	parent.insertBefore(node, before);
}

function modelPage(pages, key) {
	return pages[indexByKey(pages, key)];
}

function indexByKey(values, key) {
	const index = values.findIndex((value) => value.key === key);
	if (index < 0) fail("missing-key");
	return index;
}

function removeByKey(values, key) {
	values.splice(indexByKey(values, key), 1);
}

function moveByKey(values, key, index) {
	const [value] = values.splice(indexByKey(values, key), 1);
	validateIndex(index, values.length, true);
	values.splice(index, 0, value);
}

function validateIndex(index, length, allowEnd) {
	if (
		!Number.isSafeInteger(index) ||
		index < 0 ||
		index > length ||
		(!allowEnd && index === length)
	) {
		fail("index");
	}
}

function required(map, key) {
	const value = map.get(key);
	if (!value) fail("missing-key");
	return value;
}

function cloneState(value) {
	return structuredCloneValue(value);
}

function structuredCloneValue(value) {
	if (typeof structuredClone === "function") return structuredClone(value);
	return cloneFallback(value);
}

function cloneFallback(value) {
	if (value instanceof Uint8Array) return value.slice();
	if (Array.isArray(value)) return value.map(cloneFallback);
	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value).map(([key, item]) => [key, cloneFallback(item)]),
		);
	}
	return value;
}

function boundedString(value, limits) {
	if (typeof value !== "string") fail("string");
	if (new TextEncoder().encode(value).byteLength > limits.maxStringBytes)
		fail("string-budget");
	if (value.includes("\0")) fail("string-nul");
	return value;
}

function safeLink(link) {
	return (
		(/^#[A-Za-z0-9_.:-]{1,128}$/u.test(link) ||
			/^https:\/\/[^\s"'<>\\]+$/u.test(link)) &&
		![...link].some((character) => {
			const code = character.codePointAt(0);
			return code <= 31 || code === 127;
		})
	);
}

function exactInteger(value) {
	if (
		!Number.isSafeInteger(value) ||
		value < -2_147_483_648 ||
		value > 2_147_483_647
	) {
		fail("coordinate");
	}
	return String(value);
}

function exactUnsigned(value) {
	if (!Number.isSafeInteger(value) || value < 0 || value > 4_294_967_295)
		fail("unsigned-integer");
	return String(value);
}

function cssPx(sp, mag) {
	exactInteger(sp);
	if (!Number.isSafeInteger(mag) || mag <= 0) fail("magnification");
	return `${(sp * mag * 48) / (65_536 * 5 * 7_227)}px`;
}

function cssNumber(sp, mag) {
	exactInteger(sp);
	if (!Number.isSafeInteger(mag) || mag <= 0) fail("magnification");
	return (sp * mag * 48) / (65_536 * 5 * 7_227);
}

function cssScale(sp, mag, unitsPerEm) {
	if (!Number.isSafeInteger(unitsPerEm) || unitsPerEm <= 0)
		fail("units-per-em");
	return cssNumber(sp, mag) / unitsPerEm;
}

function safeColor(value) {
	return (
		[
			"black",
			"red",
			"green",
			"blue",
			"cyan",
			"magenta",
			"yellow",
			"gray",
		].includes(value) || /^#[0-9a-f]{6}$/u.test(value)
	);
}

function safeDestination(value) {
	return (
		typeof value === "string" &&
		value.startsWith("umber-dest-") &&
		value.length <= 139 &&
		/^umber-dest-[A-Za-z0-9_.:-]+$/u.test(value)
	);
}

async function verifySha256(identity, bytes) {
	if (!globalThis.crypto?.subtle) {
		throw new HtmlPatchError(
			"crypto-unavailable",
			"Web Crypto SHA-256 is required",
		);
	}
	const digest = new Uint8Array(
		await globalThis.crypto.subtle.digest("SHA-256", bytes),
	);
	return (
		[...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("") ===
		identity
	);
}

function sameBytes(left, right) {
	return (
		left.byteLength === right.byteLength &&
		left.every((value, index) => value === right[index])
	);
}

function freshMetrics() {
	return {
		snapshots: 0,
		patches: 0,
		duplicates: 0,
		resyncs: 0,
		operations: 0,
		inserted: 0,
		removed: 0,
		moved: 0,
		updated: 0,
		applyMilliseconds: 0,
	};
}

function now() {
	return globalThis.performance?.now?.() ?? Date.now();
}

function fail(code) {
	throw new HtmlPatchError(code, `invalid incremental HTML data: ${code}`);
}
