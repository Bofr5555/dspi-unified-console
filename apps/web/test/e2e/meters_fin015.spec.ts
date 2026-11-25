// -- WP FIN015 -- (REVF R1, code-regr P4) e2e on the core's demo device: an output nothing is routed to has
// no meter (owner 2026-11-24), but while the signal generator plays on it, it is loud and keeps one: the
// generator writes after the matrix. The views read the core's `hidden`, not `unused`.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { rail } from "../nav";

type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as G).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}

/** Out N as the page names it: "Out 3 · R", or "3 · None" while nothing is routed to it. */
const OUT = (o: number) => new RegExp(`^(?:Out ${o + 1}(?: · [^·]+){0,2}|${o + 1} · None)$`);

async function connect(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(rail(session)).toBeVisible();
  return session;
}

test.afterEach(async ({ page }) => {
  await call(page, "siggenControl", { action: "stopNow" }).catch(() => undefined);
});

test("FIN015: an unused output the generator plays on keeps its meter", async ({ page }) => {
  const session = await connect(page);
  // The demo routes to Out 1 and 2 only: Out 3 is "3 · None" and has no meter
  const out3 = session.getByRole("meter", { name: OUT(2) });
  await expect(session.getByRole("meter", { name: OUT(0) })).toBeVisible();
  await expect(out3).toHaveCount(0);
  // Pink noise on Out 3 alone
  await call(page, "siggenSet", { config: { signalType: 3, channelMask: 1 << 2, levelDb: -60, p: [0, 0, 0, 0] } });
  await call(page, "siggenControl", { action: "start" });
  await expect(out3).toBeVisible();
  await expect(out3).toHaveAccessibleName("3 · None");
  // Stopped: it goes again
  await call(page, "siggenControl", { action: "stopNow" });
  await expect(out3).toHaveCount(0);
});
