import type { RenderedSourceResult } from "./umber_wasm.js";

export type FontEncoding = Array<string | null>;
export interface RenderedSourceKey {
	page: number;
	event: number;
	unit: number;
	output: string;
	revision: number;
}
export interface RenderedSourcePointOptions {
	encoding?: FontEncoding;
	encodings?: ReadonlyMap<string, FontEncoding> | Record<string, FontEncoding>;
}

/** Converts a browser point in canonical Umber HTML into a session query key. */
export function renderedSourceKeyFromPoint(
	document: Document,
	x: number,
	y: number,
	options?: RenderedSourcePointOptions,
): RenderedSourceKey | null;

/** Converts and resolves a browser point with one CompilerSession query. */
export function renderedSourceLocationFromPoint(
	session: {
		renderedSourceLocation(
			page: number,
			event: number,
			unit: number | undefined,
			output: string,
			revision: number,
		): RenderedSourceResult | undefined;
	},
	document: Document,
	x: number,
	y: number,
	options?: RenderedSourcePointOptions,
): RenderedSourceResult | null | undefined;
