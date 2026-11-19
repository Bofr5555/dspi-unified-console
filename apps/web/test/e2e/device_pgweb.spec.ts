// WP PGWEB e2e: every firmware generation over the WebUSB mock (test/pages/usb.html), each
// device the core's fake of that generation answering the probe itself (PLAN-PG section 1.1):
// G1s, G1b with and without PAGEGUARD, G2s, G2b, the DSPi-Zero on 1.1.6 and 1.1.7, the alink
// sender (guarded and dual-stack). Per device: the device screen's generation, pages, extension
// envelope and board, read once per connection; on a 1.1.6-generation device no paged request
// but the probe in the mock's log. The probe's refusals (a newer protocol, an unrecognised
// device) as their notices, with nothing sent after the refusal. Raw requests by id on a paged
// device (a read, a gated SET refused by the core, an input that would truncate refused before
// anything is sent), and the update check's verdict (lineage, no BOOTSEL, the generation
// change). Every project: Chromium, Firefox (the mock stands in for WebUSB there too), and
// Chromium at 375 px without sideways scroll.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";

type Setup = { requestType: string; recipient: string; request: number; value: number; index: number };
type Mock = { usbMock: { log(serial: string): Promise<Setup[]> } };

const stockImage = fileURLToPath(new URL("../../../desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2", import.meta.url));

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

/** Open the mock DSPi `serial` running `kind`; its session. */
async function connect(page: Page, serial: string, kind: string): Promise<Locator> {
  await page.goto(`/test/usb.html?usb=${serial}&permitted=${serial}&gen=${serial}:${kind}`);
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: `DSPi on USB · ${serial}` }).getByRole("button", { name: "Connect" }).click();
  return page.locator("article.session");
}

const log = (page: Page, serial: string) => page.evaluate((s) => (globalThis as unknown as Mock).usbMock.log(s), serial);
const paged = (s: Setup) => s.index >> 8 !== 0;
const isProbe = (s: Setup) => s.request === 0x10 && s.index === 0x0102;

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

async function deviceScreen(session: Locator): Promise<Locator> {
  await expect(session.locator(".volume-value")).toBeVisible();
  await session.getByRole("tab", { name: "Device", exact: true }).click();
  return session.getByRole("region", { name: "Device information" });
}

const fact = (info: Locator, cls: string) => info.locator(`.${cls} dd`);

const GENERATIONS: { kind: string; label: string; legacy: boolean; board: string; protocol: RegExp; pages: RegExp; ext: RegExp }[] = [
  { kind: "g1s", label: "G1s", legacy: true, board: "DSPi", protocol: /1\.1\.6 or older/, pages: /no pages/, ext: /None \(upstream firmware\)/ },
  { kind: "g1b", label: "G1b (PAGEGUARD)", legacy: true, board: "DSPi", protocol: /1\.1\.6 or older/, pages: /no pages/, ext: /1\.1\.6 extension request/ },
  { kind: "g1b-noguard", label: "G1b (no guard)", legacy: true, board: "DSPi", protocol: /1\.1\.6 or older/, pages: /no pages/, ext: /1\.1\.6 extension request/ },
  { kind: "g2s", label: "G2s", legacy: false, board: "DSPi", protocol: /paged protocol 1\.\d+/, pages: /^0x01 SYS rev \d+, .*0x05 ROUTE/, ext: /None \(upstream firmware\)/ },
  { kind: "g2b", label: "G2b", legacy: false, board: "DSPi", protocol: /paged protocol 1\.\d+/, pages: /0xE0 EXT rev \d+/, ext: /page 0xE0/ },
  { kind: "zero", label: "Zero (1.1.6)", legacy: true, board: "DSPi Zero", protocol: /1\.1\.6 or older/, pages: /no pages/, ext: /None/ },
  { kind: "zero-paged", label: "Zero (1.1.7)", legacy: false, board: "DSPi Zero", protocol: /paged protocol/, pages: /0x01 SYS/, ext: /None/ },
];

for (const g of GENERATIONS) {
  test(`PGWEB ${g.label}: the device screen shows its generation; ${g.legacy ? "no paged request but the probe" : "paged only"}`, async ({ page }) => {
    const errors = errorsOf(page);
    // 8 characters: the picker shows a serial's last 8
    const serial = `PG${g.kind.replace(/[^a-z0-9]/g, "").toUpperCase()}`.slice(0, 8);
    const session = await connect(page, serial, g.kind);
    const info = await deviceScreen(session);
    await expect(fact(info, "device-board")).toHaveText(g.board);
    await expect(fact(info, "device-kind")).toHaveText("DSP");
    await expect(fact(info, "device-generation")).toHaveText(g.protocol);
    await expect(fact(info, "device-pages")).toHaveText(g.pages);
    await expect(fact(info, "device-envelope")).toHaveText(g.ext);
    const raw = session.getByRole("region", { name: "Raw request by id" });
    if (g.legacy) await expect(session.locator(".raw-none")).toBeVisible();
    else await expect(raw).toBeVisible();
    await noSidewaysScroll(page);
    const seen = await log(page, serial);
    expect(seen.length).toBeGreaterThan(1);
    expect(isProbe(seen[0]!), "the probe is the first request").toBe(true);
    if (g.legacy) {
      // PLAN-PG 1.1: never a paged request other than the probe to a 1.1.6-generation device
      expect(seen.filter(paged).filter((s) => !isProbe(s))).toEqual([]);
    } else {
      // and never a page-0 request from this client to a paged device
      expect(seen.filter((s) => !paged(s))).toEqual([]);
    }
    // The protocol is read once per connection: switching screens reads nothing more
    const before = seen.length;
    await session.getByRole("tab", { name: "Main", exact: true }).click();
    await session.getByRole("tab", { name: "Device", exact: true }).click();
    await expect(fact(info, "device-board")).toHaveText(g.board);
    const platformReads = (l: Setup[]) => l.filter((s) => (s.request === 0x20 && s.index === 0x0102) || (s.request === 0x7f && s.index === 2)).length;
    expect(platformReads(await log(page, serial))).toBe(platformReads((await log(page, serial)).slice(0, before)));
    expect(errors).toEqual([]);
  });
}

// The guarded sender, PLAN-PG section 1.11's row, and (-- WP PGSENDER --) the dual-stack sender
// after FWPG4 on its paged stack (PLAN-PG section 1.9's rule): the same page, no DSP screen.
for (const kind of ["sender", "sender-dual"]) {
  test(`PGWEB alink sender (${kind}): its own page, no DSP screen`, async ({ page }) => {
    const errors = errorsOf(page);
    const serial = "PGSENDER";
    const session = await connect(page, serial, kind);
    const sender = session.getByRole("region", { name: "Device information" });
    await expect(sender).toBeVisible();
    await expect(fact(sender, "device-kind")).toHaveText("Audio link sender");
    await expect(session.locator(".sender-note")).toContainText("no DSP");
    await expect(session.getByRole("region", { name: "Audio link" }).locator(".alink-role dd")).toHaveText(/sender/i);
    await expect(session.getByRole("region", { name: "Firmware update check" })).toBeVisible();
    // No DSP screen, no meters
    await expect(session.getByRole("tab", { name: "Main", exact: true })).toHaveCount(0);
    await expect(session.getByRole("tab", { name: "EQ", exact: true })).toHaveCount(0);
    await expect(session.locator(".volume-value")).toHaveCount(0);
    await noSidewaysScroll(page);
    const seen = await log(page, serial);
    if (kind === "sender") {
      expect(seen.filter(paged).filter((s) => !isProbe(s))).toEqual([]);
    } else {
      // -- WP PGSENDER -- paged only: its SYS reads and the tunnel, nothing on page 0 or a DSP page
      const own = (s: Setup) => s.index >> 8 === 0x01 || (s.index >> 8 === 0xe0 && s.request === 0x10);
      expect(seen.filter((s) => !own(s))).toEqual([]);
    }
    expect(errors).toEqual([]);
  });
}

const NOTICES = [
  { kind: "newer", key: "transfer.newer_protocol", title: "Newer protocol", after: 1 },
  { kind: "garbled", key: "transfer.unrecognised_device", title: "Unrecognised device", after: 1 },
  // the probe STALLs, then GET_PLATFORM STALLs too (the core asks it again within its busy window)
  { kind: "silent", key: "transfer.unrecognised_device", title: "Unrecognised device", after: 0 },
];

for (const n of NOTICES) {
  test(`PGWEB notice ${n.key} (${n.kind}): the open is refused in its words; nothing more is sent`, async ({ page }) => {
    const serial = `PG${n.kind.toUpperCase()}`.slice(0, 8);
    const session = await connect(page, serial, n.kind);
    const alert = session.getByRole("alert");
    await expect(alert).toHaveAttribute("data-key", n.key);
    await expect(alert).toContainText(n.title);
    await expect(session.locator(".volume-value")).toHaveCount(0);
    await noSidewaysScroll(page);
    const seen = await log(page, serial);
    expect(isProbe(seen[0]!)).toBe(true);
    if (n.after) expect(seen.length, JSON.stringify(seen)).toBe(n.after);
    // after a STALLed probe: GET_PLATFORM only, and no other paged request
    else expect(seen.slice(1).filter((s) => s.request !== 0x7f || s.index !== 2), JSON.stringify(seen)).toEqual([]);
  });
}

test("PGWEB raw by id on a paged device: a read, the core's refusal in words, input that would truncate refused", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page, "PGRAW", "g2s");
  await deviceScreen(session);
  const raw = session.getByRole("region", { name: "Raw request by id" });
  const said = raw.getByRole("status", { name: "Raw result" });
  // A read: ROUTE.MASTER_VOLUME_SAVED
  await raw.getByLabel("Command id").fill("0x0550");
  await raw.getByLabel("Length").fill("4");
  await raw.getByRole("button", { name: "Send" }).click();
  await expect(said).toHaveText(/^0x0550: \d+ bytes?: ([0-9A-F]{2} ?)+$/);
  // An id that would be truncated to 16 bits: refused, nothing sent (to 0x0550, what it would become)
  const to0550 = async () => (await log(page, "PGRAW")).filter((s) => s.request === 0x50 && s.index === 0x0502).length;
  const count = await to0550();
  await raw.getByLabel("Command id").fill("0x10550");
  await raw.getByRole("button", { name: "Send" }).click();
  await expect(said).toHaveText(/^Not sent\. Command id: 0x10550 is outside .* not cut to fit\.$/);
  // A SET's byte past 0xFF: refused, nothing sent
  await raw.getByLabel("Direction").selectOption("set");
  await raw.getByLabel("Command id").fill("0x0550");
  await raw.getByLabel("Data").fill("0x00 0x100");
  await raw.getByRole("button", { name: "Send" }).click();
  await expect(said).toHaveText(/^Not sent\. Data: 0x100 is outside 0x00\.\.0xFF/);
  expect(await to0550()).toBe(count);
  // The bootloader by its paged id: gated by the core, nothing sent
  await raw.getByLabel("Command id").fill("0x0130");
  await raw.getByLabel("Data").fill("");
  await raw.getByRole("button", { name: "Send" }).click();
  await expect(said).toHaveText(/^0x0130 was not sent: /);
  expect((await log(page, "PGRAW")).filter((s) => s.request === 0x30 && s.index === 0x0102)).toEqual([]);
  // A page-0 id is a 1.1.6 opcode, refused here
  await raw.getByLabel("Command id").fill("0x0050");
  await raw.getByRole("button", { name: "Send" }).click();
  await expect(said).toHaveText(/^Not sent\. Command id: 0x0050 is on page 0/);
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

const VERDICTS = [
  // our firmware on a Pico, the stock image: the lineage rule asks the typed phrase
  { kind: "g1b", rule: "lineageLoss", decision: "needsConfirmation", head: "Needs a typed confirmation", generation: false },
  // a board without BOOTSEL: refused whatever is typed
  { kind: "zero", rule: "noBootsel", decision: "refused", head: "Refused", generation: false },
  // stock 1.1.7 on the demo's board (it needs 1.20 V), the unstamped 1.1.6 image: the voltage
  // rule asks, and the generation change is said
  { kind: "g2s", rule: "none", decision: "needsConfirmation", head: "Needs a typed confirmation", generation: true },
];

for (const v of VERDICTS) {
  test(`PGWEB update check on ${v.kind}: ${v.rule}, ${v.decision}`, async ({ page }) => {
    const errors = errorsOf(page);
    const serial = `PGUP${v.kind.toUpperCase()}`.slice(0, 8);
    const session = await connect(page, serial, v.kind);
    await deviceScreen(session);
    const card = session.getByRole("region", { name: "Firmware update check" });
    await card.getByLabel("Firmware image to check").setInputFiles(stockImage);
    const verdict = card.getByRole("status", { name: "Update verdict" });
    await expect(verdict).toHaveAttribute("data-rule", v.rule);
    await expect(verdict).toHaveAttribute("data-decision", v.decision);
    await expect(verdict.locator(".update-verdict-head")).toHaveText(new RegExp(`^${v.head}`));
    if (v.rule === "noBootsel") await expect(verdict.locator(".update-rule")).toHaveText(/^No BOOTSEL/);
    if (v.rule === "lineageLoss") await expect(verdict.locator(".update-rule")).toHaveText(/^Lineage/);
    if (v.generation) await expect(verdict.locator(".update-generation")).toHaveText(/^Generation change: .+/);
    else await expect(verdict.locator(".update-generation")).toHaveCount(0);
    // Nothing that enters the bootloader went out (0xF0, SYS.ENTER_BOOTLOADER)
    expect((await log(page, serial)).filter((s) => (s.request === 0xf0 && s.index === 2) || (s.request === 0x30 && s.index === 0x0102))).toEqual([]);
    await noSidewaysScroll(page);
    expect(errors).toEqual([]);
  });
}
