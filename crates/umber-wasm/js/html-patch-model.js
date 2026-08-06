import {
	boundedString,
	cloneState,
	DIGEST,
	exactInteger,
	fail,
	indexByKey,
	KEY,
	modelPage,
	moveByKey,
	removeByKey,
	SESSION,
	safeColor,
	safeLink,
	settingStyle,
	structuredCloneValue,
	validateIndex,
} from "./html-patch-shared.js";

export function validateSnapshot(snapshot, limits) {
	if (snapshot?.kind !== "snapshot" || snapshot.schemaVersion !== 1)
		fail("snapshot-schema");
	validateIdentity(snapshot);
	const strings = stringBudget(limits);
	strings.add(snapshot.title);
	strings.add(snapshot.language);
	if (!Array.isArray(snapshot.pages) || snapshot.pages.length > limits.maxPages)
		fail("page-budget");
	if (
		!Array.isArray(snapshot.resources) ||
		snapshot.resources.length > limits.maxResources
	)
		fail("resources");
	const resourceIdentities = new Set();
	for (const resource of snapshot.resources) {
		validateResource(resource, strings);
		if (resourceIdentities.has(resource.identity)) fail("duplicate-resource");
		resourceIdentities.add(resource.identity);
	}
	let nodes = 0;
	const keys = new Set();
	for (const page of snapshot.pages) {
		validatePage(page, keys, limits, strings);
		nodes += page.nodes.length;
	}
	if (nodes > limits.maxNodes) fail("node-budget");
	return cloneState(snapshot);
}

export function simulatePatch(base, patch, limits) {
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
	if (
		!Array.isArray(patch.resourceAdditions) ||
		!Array.isArray(patch.resourceReleases) ||
		patch.resourceAdditions.length > limits.maxResources ||
		patch.resourceReleases.length > limits.maxResources
	)
		fail("resources");
	const strings = stringBudget(limits);
	const candidate = cloneState(base);
	candidate.revision = patch.targetRevision;
	candidate.digest = patch.afterDigest;
	if (patch.title !== undefined)
		candidate.title = boundedString(patch.title, limits);
	if (patch.language !== undefined)
		candidate.language = boundedString(patch.language, limits);
	candidate.resources = updateResources(base.resources, patch);
	if (candidate.resources.length > limits.maxResources) fail("resources");
	strings.add(candidate.title);
	strings.add(candidate.language);
	for (const resource of candidate.resources)
		validateResource(resource, strings);
	for (const operation of patch.operations)
		simulateOperation(candidate.pages, operation, limits);
	const keys = new Set();
	let nodes = 0;
	for (const page of candidate.pages) {
		validatePage(page, keys, limits, strings);
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

function validateIdentity(value) {
	if (!SESSION.test(value.sessionId) || !DIGEST.test(value.digest))
		fail("identity");
	if (!Number.isSafeInteger(value.revision) || value.revision < 1)
		fail("revision");
}

function validatePage(page, keys, limits, strings) {
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
		for (const value of [
			node.text,
			node.link,
			node.color,
			node.class,
			node.family,
			node.language,
			node.script,
			node.path,
			node.payloadHex,
			node.actionValue,
		]) {
			if (value !== undefined && value !== null) strings.add(value);
		}
		if (node.kind === "text") {
			if (!/^umber-font-[0-9a-f]{24}$/u.test(node.family)) fail("font-family");
			exactInteger(node.fontSizeSp);
			if (
				!Array.isArray(node.positionsSp) ||
				node.positionsSp.length > limits.maxStrings
			)
				fail("positions");
			for (const position of node.positionsSp) exactInteger(position);
			if (
				node.features?.length > limits.maxStrings ||
				node.variations?.length > limits.maxStrings
			)
				fail("font-settings");
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
	const releases = new Set();
	for (const identity of patch.resourceReleases) {
		if (!DIGEST.test(identity)) fail("resource-release");
		if (releases.has(identity)) fail("duplicate-release");
		releases.add(identity);
		if (!resources.delete(identity)) fail("unknown-resource");
	}
	for (const resource of patch.resourceAdditions) {
		validateResource(resource);
		if (resources.has(resource.identity)) fail("duplicate-resource");
		resources.set(resource.identity, structuredCloneValue(resource));
	}
	return [...resources.values()].sort((left, right) =>
		left.identity.localeCompare(right.identity),
	);
}

export function validateResource(resource, strings) {
	if (
		!DIGEST.test(resource?.identity) ||
		!(resource.bytes instanceof Uint8Array)
	)
		fail("resource");
	if (resource.kind !== "font" && resource.kind !== "asset")
		fail("resource-kind");
	if (resource.kind === "font" && typeof resource.family !== "string")
		fail("font-family");
	if (strings) {
		if (resource.family !== undefined) strings.add(resource.family);
		if (resource.provenance !== undefined) strings.add(resource.provenance);
	}
}

function stringBudget(limits) {
	let count = 0;
	let bytes = 0;
	return {
		add(value) {
			boundedString(value, limits);
			count += 1;
			bytes += new TextEncoder().encode(value).byteLength;
			if (count > limits.maxStrings) fail("string-count-budget");
			if (bytes > limits.maxTotalStringBytes) fail("string-total-budget");
			return value;
		},
	};
}
