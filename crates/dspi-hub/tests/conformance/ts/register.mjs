// WP HB7: lets Node 24 run upstream's TypeScript source as it is, with no build and
// no npm download: `--experimental-transform-types` strips the types (and turns its
// enums into objects), and this resolves its `./x.js` imports to the `./x.ts` files
// that tsc would have compiled them from.
import { register } from "node:module";

register("data:text/javascript," + encodeURIComponent(`
export async function resolve(specifier, context, next) {
  try {
    return await next(specifier, context);
  } catch (error) {
    if (/^\\.\\.?\\//.test(specifier) && specifier.endsWith(".js")) {
      return next(specifier.slice(0, -3) + ".ts", context);
    }
    throw error;
  }
}
`));
