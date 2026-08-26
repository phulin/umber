export const HTML_NS = "http://www.w3.org/1999/xhtml";
export const SVG_NS = "http://www.w3.org/2000/svg";
export const KEY = /^[0-9a-f]{32}$/;

import { deterministicAhash64Hex } from "./manifest-resolver.js";

export const DIGEST = /^[0-9a-f]{16}$/;
export const SESSION = /^[0-9a-f]{32}$/;

export const DEFAULT_LIMITS = Object.freeze({
	maxPages: 16_384,
	maxNodes: 1_000_000,
	maxOperations: 250_000,
	maxResources: 65_536,
	maxStrings: 1_000_000,
	maxStringBytes: 16 * 1024 * 1024,
	maxTotalStringBytes: 64 * 1024 * 1024,
	maxResourceBytes: 256 * 1024 * 1024,
});

export function resolveLimits(overrides = {}) {
	const limits = { ...DEFAULT_LIMITS, ...overrides };
	for (const [name, ceiling] of Object.entries(DEFAULT_LIMITS)) {
		const value = limits[name];
		if (!Number.isSafeInteger(value) || value < 0 || value > ceiling) {
			throw new RangeError(
				`${name} must be a nonnegative safe integer no greater than ${ceiling}`,
			);
		}
	}
	return Object.freeze(limits);
}

export class HtmlPatchError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "HtmlPatchError";
		this.code = code;
	}
}

export function modelPage(pages, key) {
	return pages[indexByKey(pages, key)];
}

export function indexByKey(values, key) {
	const index = values.findIndex((value) => value.key === key);
	if (index < 0) fail("missing-key");
	return index;
}

export function removeByKey(values, key) {
	values.splice(indexByKey(values, key), 1);
}

export function moveByKey(values, key, index) {
	const [value] = values.splice(indexByKey(values, key), 1);
	validateIndex(index, values.length, true);
	values.splice(index, 0, value);
}

export function validateIndex(index, length, allowEnd) {
	if (
		!Number.isSafeInteger(index) ||
		index < 0 ||
		index > length ||
		(!allowEnd && index === length)
	) {
		fail("index");
	}
}

export function required(map, key) {
	const value = map.get(key);
	if (!value) fail("missing-key");
	return value;
}

export function cloneState(value) {
	return structuredCloneValue(value);
}

export function structuredCloneValue(value) {
	if (typeof structuredClone === "function") return structuredClone(value);
	return cloneFallback(value);
}

export function cloneFallback(value) {
	if (value instanceof Uint8Array) return value.slice();
	if (Array.isArray(value)) return value.map(cloneFallback);
	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value).map(([key, item]) => [key, cloneFallback(item)]),
		);
	}
	return value;
}

export function boundedString(value, limits) {
	if (typeof value !== "string") fail("string");
	if (new TextEncoder().encode(value).byteLength > limits.maxStringBytes)
		fail("string-budget");
	if (value.includes("\0")) fail("string-nul");
	return value;
}

export function safeLink(link) {
	return (
		(/^#[A-Za-z0-9_.:-]{1,128}$/u.test(link) ||
			/^https:\/\/[^\s"'<>\\]+$/u.test(link)) &&
		![...link].some((character) => {
			const code = character.codePointAt(0);
			return code <= 31 || code === 127;
		})
	);
}

export function exactInteger(value) {
	if (
		!Number.isSafeInteger(value) ||
		value < -2_147_483_648 ||
		value > 2_147_483_647
	) {
		fail("coordinate");
	}
	return String(value);
}

export function exactUnsigned(value) {
	if (!Number.isSafeInteger(value) || value < 0 || value > 4_294_967_295)
		fail("unsigned-integer");
	return String(value);
}

export function cssPx(sp, mag) {
	exactInteger(sp);
	if (!Number.isSafeInteger(mag) || mag <= 0) fail("magnification");
	return `${(sp * mag * 48) / (65_536 * 5 * 7_227)}px`;
}

export function cssNumber(sp, mag) {
	exactInteger(sp);
	if (!Number.isSafeInteger(mag) || mag <= 0) fail("magnification");
	return (sp * mag * 48) / (65_536 * 5 * 7_227);
}

export function cssScale(sp, mag, unitsPerEm) {
	if (!Number.isSafeInteger(unitsPerEm) || unitsPerEm <= 0)
		fail("units-per-em");
	return cssNumber(sp, mag) / unitsPerEm;
}

export function safeColor(value) {
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

export function settingStyle(settings, signed) {
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

export function safeDestination(value) {
	return (
		typeof value === "string" &&
		value.startsWith("umber-dest-") &&
		value.length <= 139 &&
		/^umber-dest-[A-Za-z0-9_.:-]+$/u.test(value)
	);
}

export async function verifyAhash64(identity, bytes) {
	return deterministicAhash64Hex(bytes, 24) === identity;
}

export function sameBytes(left, right) {
	return (
		left.byteLength === right.byteLength &&
		left.every((value, index) => value === right[index])
	);
}

export function freshMetrics() {
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

export function now() {
	return globalThis.performance?.now?.() ?? Date.now();
}

export function fail(code) {
	throw new HtmlPatchError(code, `invalid incremental HTML data: ${code}`);
}
