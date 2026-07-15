import { ot1TextEncoding } from "./cm-fonts.js";

const DEFAULT_ENCODING = ot1TextEncoding();

/** Converts a browser point in canonical Umber HTML into a rendered-source key. */
export function renderedSourceKeyFromPoint(document, x, y, options = {}) {
	if (!document || typeof document.caretPositionFromPoint !== "function") {
		throw new TypeError("document.caretPositionFromPoint is unavailable");
	}
	const caret = document.caretPositionFromPoint(x, y);
	const text = caret?.offsetNode?.parentElement?.closest?.(".umber-run-text");
	const run = text?.closest?.(".umber-run");
	const page = run?.closest?.(".umber-page");
	if (!text || !run || !page || caret.offsetNode.parentElement !== text) {
		return null;
	}

	const pageOrdinal = unsignedInteger(page.dataset?.umberPage, 1);
	const event = unsignedInteger(run.dataset?.umberEvent, 0);
	const revision = unsignedInteger(page.dataset?.umberRevision, 1);
	const output = outputIdentity(page.dataset?.umberOutput);
	const codes = parseCodes(run.dataset?.umberCodes);
	const encoding = selectEncoding(run.dataset?.umberFont, options);
	const unit = unitAtOffset(codes, encoding, caret.offset);
	if (
		pageOrdinal === null ||
		event === null ||
		revision === null ||
		output === null ||
		unit === null
	) {
		return null;
	}
	return { page: pageOrdinal, event, unit, output, revision };
}

/** Resolves the source location at a browser point with one session query. */
export function renderedSourceLocationFromPoint(
	session,
	document,
	x,
	y,
	options = {},
) {
	const key = renderedSourceKeyFromPoint(document, x, y, options);
	return key === null
		? null
		: session.renderedSourceLocation(
				key.page,
				key.event,
				key.unit,
				key.output,
				key.revision,
			);
}

function selectEncoding(font, options) {
	const encodings = options.encodings;
	const selected =
		encodings instanceof Map ? encodings.get(font) : encodings?.[font];
	return selected ?? options.encoding ?? DEFAULT_ENCODING;
}

function parseCodes(value) {
	if (typeof value !== "string" || value.length === 0) return null;
	const codes = [];
	for (const token of value.split(",")) {
		if (token === "space") {
			codes.push(null);
		} else if (/^0x[0-9a-f]{2}$/i.test(token)) {
			codes.push(Number.parseInt(token.slice(2), 16));
		} else {
			return null;
		}
	}
	return codes;
}

function unitAtOffset(codes, encoding, offset) {
	if (!Array.isArray(codes) || !Array.isArray(encoding)) return null;
	if (!Number.isInteger(offset) || offset < 0) return null;
	let end = 0;
	let last = null;
	for (let unit = 0; unit < codes.length; unit += 1) {
		const mapped = codes[unit] === null ? " " : encoding[codes[unit]];
		if (typeof mapped !== "string" || mapped.length === 0) return null;
		end += mapped.length;
		last = unit;
		if (offset < end) return unit;
	}
	return offset === end ? last : null;
}

function unsignedInteger(value, minimum) {
	if (typeof value !== "string" || !/^\d+$/.test(value)) return null;
	const number = Number(value);
	return Number.isSafeInteger(number) && number >= minimum ? number : null;
}

function outputIdentity(value) {
	return typeof value === "string" && /^[0-9a-f]{32}$/.test(value)
		? value
		: null;
}
