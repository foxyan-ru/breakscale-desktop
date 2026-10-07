/**
 * Read the colour tokens out of a stylesheet's theme blocks.
 *
 * Ported from src/theme/tokens.ts. Parsing the CSS rather than restating the
 * palette in TypeScript is deliberate: a second copy is a second thing to
 * keep in sync, and the point of a contrast check is to check the values
 * that actually ship in app.css. Not used at runtime by the app itself --
 * `apply-theme.ts` only ever sets a `data-theme` attribute and lets the
 * stylesheet do the colour-swapping -- but ported anyway for parity with the
 * web app and because a future test suite for this app will want the same
 * tool to check app.css's tokens the way theme.contrast.test.ts does there.
 *
 * No file reading here, so this stays usable from any context (Node test,
 * browser, etc.); the caller supplies the CSS text.
 */

export type Tokens = Record<string, string>;

/**
 * Read one block's custom properties.
 *
 * `selector` is matched literally, so ':root' and ":root[data-theme='dark']"
 * select different blocks. The blocks contain no nested braces, so the first
 * closing brace at the start of a line ends one.
 */
export function readTokens(css: string, selector: string): Tokens {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`no block for ${selector}`);
  const end = css.indexOf('\n}', start);
  const body = css.slice(start, end < 0 ? undefined : end);
  const out: Tokens = {};
  for (const m of body.matchAll(/^\s*(--[a-z0-9-]+):\s*([^;]+);/gim)) {
    out[m[1]!] = m[2]!.trim();
  }
  return out;
}

/** Resolve a token that may be a `var(--other)` alias, one hop deep. */
export function resolve(tokens: Tokens, name: string): string | null {
  const v = tokens[name];
  if (!v) return null;
  const alias = /^var\((--[a-z0-9-]+)\)$/i.exec(v);
  if (alias) return tokens[alias[1]!] ?? null;
  return v;
}
