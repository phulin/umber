import {
	boundedString,
	cssNumber,
	cssPx,
	cssScale,
	DEFAULT_LIMITS,
	exactInteger,
	exactUnsigned,
	fail,
	HTML_NS,
	modelPage,
	SVG_NS,
	safeColor,
	safeDestination,
	safeLink,
	settingStyle,
} from "./html-patch-shared.js";

export function stageInsertions(document, operations, pages) {
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

export function buildPage(document, page, nodes, pageContent, staged = nodes) {
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
			element.setAttribute("class", "umber-run");
			element.setAttribute("role", "text");
			element.style.position = "absolute";
			element.style.left = "0";
			element.style.top = "0";
			element.style.width = "0";
			element.style.height = "0";
			element.style.overflow = "visible";
			const baseline = document.createElementNS(SVG_NS, "rect");
			baseline.setAttribute("class", "umber-baseline");
			baseline.setAttribute("x", cssPx(node.xSp, mag));
			baseline.setAttribute("y", cssPx(node.baselineSp, mag));
			baseline.setAttribute("width", "1");
			baseline.setAttribute("height", "1");
			baseline.setAttribute("fill", "transparent");
			element.append(baseline);
			const text = document.createElementNS(SVG_NS, "text");
			text.setAttribute("class", "umber-run-text");
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
			element.setAttribute("class", "umber-math");
			element.setAttribute("aria-hidden", "true");
			zeroSizedSvg(element);
			break;
		case "math-glyph": {
			element = document.createElementNS(SVG_NS, "svg");
			element.setAttribute("class", "umber-math");
			element.setAttribute("aria-hidden", "true");
			zeroSizedSvg(element);
			const glyph = document.createElementNS(SVG_NS, "g");
			glyph.setAttribute("class", "umber-math-glyph");
			glyph.setAttribute("data-umber-glyph-id", exactUnsigned(node.glyphId));
			glyph.setAttribute("data-umber-font-instance", node.fontInstance);
			glyph.setAttribute("data-umber-ssty", exactUnsigned(node.ssty));
			if (node.drawing === "text") {
				const text = document.createElementNS(SVG_NS, "text");
				text.setAttribute("class", "umber-math-text");
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
				path.setAttribute("class", "umber-math-outline");
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
			element.setAttribute("class", "umber-math");
			element.setAttribute("aria-hidden", "true");
			zeroSizedSvg(element);
			const rule = document.createElementNS(SVG_NS, "rect");
			rule.setAttribute("class", "umber-math-rule");
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

export function updatePage(element, page, content) {
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

export function captureUserState(document, root, nodes) {
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

export function restoreUserState(state, nodes, root) {
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

export function insertAt(parent, node, index) {
	if (node.parentNode === parent) node.remove();
	const before = parent.children?.[index] ?? null;
	parent.insertBefore(node, before);
}
