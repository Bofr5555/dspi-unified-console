<!-- Copied from DESREV's spec (~/.cache/dspi-build/work/wp-DESREV/DESIGN-LANGUAGE.md) by WP DL0, with §19 added: the component map. The packages DL1-DL10 and MX1 work from this copy. -->

# The Modern design language (Qt desktop, uConsole for DSPi)

Derived from the approved mockup (`wp-MODLAY/mockup.html`, variant B: rail + page tabs, Grey tone) and from the best
Modern screens as built today (Equaliser › PEQ and Delay & Gain, the Settings pages, the Signal Generator, the AI
popover and sheets). Every rule names the `Theme.qml` token or the shared component that carries it, or says that the
component is missing. Written 2026-11-24 for the DESREV review; the owner's two rules come first.

**Two rules above all the others**
1. Surfaces are separated by tone and spacing, not by lines. A 1 px outline is allowed only where it means something: a
   focus ring, a matrix crosspoint that is off, the accent ring on a chosen shape or slot, and the edge of a floating
   surface (popover, menu, dialog) in the dark scheme.
2. The accent (blue) says "on" or "chosen" or "the one primary action". It never fills a row of options. Options that can
   be picked are chips or segments on the control tone; the chosen one gets the selection tone and accent text.

---

## 1. Colour tokens

The Grey tone of `Theme.greys` is the Modern palette and already matches the mockup to the hex. Darker grey is the
option. Values below are Grey; the Darker set is in `Theme.qml`.

| Role | Token | Dark | Light | Use |
|---|---|---|---|---|
| Window | `Theme.window` | `#202020` | `#f3f3f3` | the page background behind everything |
| Panel | `Theme.panel` | `#2a2a2a` | `#fbfbfb` | cards, tables, the inspector sheet, popovers, dialogs |
| Sidebar | `Theme.sidebar` | `#1c1c1c` | `#ebebeb` | the rail, the device-tab row, the dock, the settings sub-list |
| Control | `Theme.control` | `#343434` | `#e8e8e8` | buttons, fields, chips, segments, slider tracks |
| Control hover / pressed | `Theme.controlHover` / `controlPressed` | `#3c3c3c` / `#464646` | `#e2e2e2` / `#d8d8d8` | |
| Selection | `Theme.selection` | `#3b3b3b` | `#e1e1e1` | the chosen row, tab, rail item, chip; never blue |
| Text selection | `Theme.textSelection` | blue tint | blue tint | selected text inside a field only |
| Separator | `Theme.separator` | `#303030` | `#e2e2e2` | the few lines that remain (table header rule, popover edge in dark) |
| Border | `Theme.border` | `#484848` | `#cdcdcd` | focus-less outlines where allowed (off crosspoint ring) |
| Graph grid | `Theme.graphGrid` | `#3a3a3a` | `#d3d3d3` | |
| Text | `Theme.text` | `#e8e8e8` | `#1b1b1b` | |
| Muted | `Theme.textMuted` | `#a7a7a7` | `#5e5e5e` | labels, captions, section labels, inactive tabs |
| Disabled | `Theme.textDisabled` | `#868686` | `#7d7d7d` | |
| Accent | `Theme.accent` / `accentText` | `#2563eb` / `#7aaeff` | `#2563eb` / `#1d5bd0` | fills: the primary button, a switch that is on, the tab underline, the slider fill; text: links, the chosen chip's label |
| Danger | `Theme.danger` / `dangerText` | `#c42b1c` / `#ff7468` | `#c42b1c` / `#b42318` | destructive button fill, muted state |
| Warning | `Theme.warning` | `#e0a030` | `#875200` | inverted polarity, a conflict ring, a warning notice icon. Never body text |
| Success | `Theme.success` | `#34c759` | `#1a6b2e` | connected dot, "Active" badge |
| Meter | `meterTrack/Level/Low/Mid/High/Hold` | as `Theme.qml` | | |
| Banner fills | `Theme.bannerFill(kind)` | | | status strip and notice rows, see §12 |
| Stripe | missing: `Theme.stripe` | `rgba(255,255,255,.025)` | `rgba(0,0,0,.035)` | every second table row |
| Shadow | missing: `Theme.shadow` | `0 12 34 rgba(0,0,0,.5)` | `0 10 30 rgba(0,0,0,.16)` | floating surfaces |
| Rail selection mark | `Theme.accent`, 3×16 px | | | the bar at the left of the current rail item |

Channel colours stay `ChannelPalette`; text on them is `Theme.textOnTint`.

What this forbids (seen today): accent-filled option buttons ("1 … 9" outputs on every Effects page, "Detector / Apply"
rows on Leveller, "Claude Desktop / Claude Code / Other" on AI Agents), warning-orange used as body text (Measure, Live,
Auto-tune, Several units, Files' template descriptions, the empty Measure page), and bluish `#1e1f22` greys (Classic only).

## 2. Type scale

System UI font; numbers in `Shell.monospaceFamily`. Sizes in px, weights as Qt names.

| Token | Size / weight | Use |
|---|---|---|
| missing: `Theme.fontPageTitle` | 15 DemiBold | the page title at the left of the tab row ("Equaliser") |
| `Theme.fontTitle` | 15 DemiBold | a dialog or popover title, a tool's name in its intro row |
| `Theme.fontBody` | 13 | everything that is read |
| missing: `Theme.fontTable` | 12.5 | table cells, list rows |
| `Theme.fontMono` | 12 mono | numbers in fields, readouts, timestamps, codes |
| `Theme.fontSmall` | 11 | captions under a control, units after a field, tab-row tools' secondary text |
| `Theme.fontHeading` | 11 DemiBold, all caps, letter spacing 0.06 em | section labels ("MICROPHONE", "OUTPUT PAIRS") |
| `Theme.fontCaption` | 10 | the rail labels, the dock's small labels, graph axis labels |

Rules: one page title per page, never a second heading that repeats the tab ("Live response" under the Live tab). A
section label is the only all-caps text. Bold is for the one value that matters in a row (a chip's label, a title),
never for whole sentences.

## 3. Spacing grid, sizes, density

4 px grid. `Theme.spacingSmall` 8, `Theme.spacing` 12, `Theme.spacingLarge` 16; a 24 px gap between sections (missing:
`Theme.spacingSection`).

| Element | Size | Token |
|---|---|---|
| Top bar | 40 | missing: `Theme.topBarHeight` |
| Device-tab row | 38 | as built |
| Page tab row (title + tabs + tools) | 36 | missing: `Theme.tabBarHeight` |
| Status strip | 32 | as built |
| Dock | 44 | missing: `Theme.dockHeight` |
| Rail | 72 wide, icon 20 + label 10 | as built |
| Inspector | 320 wide, collapsible | `InspectorState` |
| Control (button, field, chip) | 24 high (`Theme.controlHeight`); 28 in the top bar and page tools | |
| Table row | 28 (`Theme.rowHeight` is 26: raise to 28 in Modern) | compact tables 24 (`Theme.density("compact")`) |
| Settings / form row | 36 | as built in `SettingsValueRow` |
| Page padding | 12 all round; the graph may bleed to 0 | |
| Content max width | 760 for forms and lists, left-aligned, never centred | missing: `Theme.contentMaxWidth` |

The last line is the one most pages break: Leveller, Upmixer, Firmware, Presets, Files, Devices, What's New all centre a
column sized like their Classic window. In Modern a form starts at the left margin of the page and stops at 760 px; the
room on the right is for an aside (preview graph, chain, inspector) or stays empty.

## 4. Radii

| Token | Value | Use |
|---|---|---|
| `Theme.radius` | 8 | cards, the inspector sheet, popovers, menus, dialogs, tiles |
| missing: `Theme.radiusControl` | 6 | buttons, fields, chips (non-pill), segments, split buttons |
| `Theme.radiusSmall` | 4 | table cells, the tab underline ends, tiny badges |
| pill | height / 2 | channel pills, status chips ("Active"), counts |

## 5. Outlines and elevation

- Cards, sections, tables, the band list, graphs: no border (`Card.border.width` must be 0 in Modern; today it is 1 and
  every Measure, Hubs, Speakers, Snapshots, Events and Files card shows it).
- Buttons, fields, chips, segments: no border; the control tone separates them (`AppButton.border.width` 1 and
  `BorderedField` must go to 0 in Modern; keep the focus ring at 2 px accent).
- Floating surfaces (menus, popovers, the device switcher, the command palette, dialogs): `Theme.panel`, radius 8, a
  shadow (`Theme.shadow`, via `MultiEffect` or a pre-rendered nine-patch), plus a 1 px `Theme.separator` edge in the
  dark scheme only, because panel and window are close in tone there.
- The inspector and the notification drawer are sheets, not floating: same tone as the page (`Theme.window`) with a 1 px
  `Theme.separator` at their edge, no shadow.
- Keep a 1 px ring on: an off crosspoint (`Theme.border`), a conflict (`Theme.warning`), the chosen shape or slot
  (`Theme.accent`), focus (`Theme.focusRing` 2 px).

## 6. Page header and the page-tab bar

One 36 px row (`PageTabs.qml`): the page title (`fontPageTitle`), then the underline tabs, then at the right the page's
tools and the Inspector toggle and "Open in Window". Content begins directly under it with 12 px padding. No intro text,
no second title. A panel that needs a name and a one-line description gets a **page intro row** (`ToolHeader.qml`, to be
restyled): 20 px icon in `accentText`, the name in `fontTitle`, the description in `fontSmall` muted, the on/off switch
or primary action at the right, 44 px high, no background, a 24 px gap under it.

Tabs: label in `textMuted`, the selected one in `text` DemiBold with a 2 px `accent` underline; a 6 px dot before the
label for on/off state (Effects) in `success` when on and `textMuted` when off; a quiet dot in `accent` for "running"
(Measure, Live). Tabs are never pills in variant B.

## 7. Toolbars, buttons, split buttons

| Kind | Look | Component |
|---|---|---|
| Primary | `accent` fill, `textOnAccent`, radius 6; one per view (Save to Device; Start; Measure; Allow) | `AppButton { primary: true }` |
| Secondary | `control` fill, no border, `text`; hover `controlHover` | `AppButton` |
| Destructive | `danger` fill only inside a confirmation sheet; elsewhere a secondary button with `dangerText` | `AppButton { destructive: true }` |
| Quiet / icon | no fill until hover; 16 px Lucide icon in `textMuted` | `IconButton` |
| Split | primary part + a 24 px `more` part separated by 1 px of window tone, radius 6 at the outer corners | `SplitButton` |
| Toggle (RTA, Phase, All Curves) | `control` fill; on = `selection` fill + `accentText` | missing: `ToggleChip` (today each page draws its own) |
| Segments (Curves / Bars / Both, Basic / Advanced, Slow / Medium / Fast) | one `control` bar, the chosen segment `selection` + `accentText`, sized to content, never stretched across the page | `ToolSegments` (restyle; today the chosen segment is accent-filled and the bar fills the width) |

Page tools (the buttons at the right of the tab row) are secondary buttons with text; at most four, the rest in a
"More ▾" menu. Buttons with an ellipsis open a dialog or a picker; a button without one acts at once.

## 8. Cards and sections

- A **section** is a section label (`fontHeading`, muted, 24 px above, 8 px below) followed by its content. It has no box.
  This is the default grouping on a page.
- A **card** (`Card.qml`, `Theme.panel`, radius 8, 12 px padding, no border) is used when the content is a self-contained
  unit that is read as one thing: an Overview tile, a settings group of rows, a tool's preview. Never nest a card in a
  card; never put one card alone on a page as a frame around everything (Snapshots, Events, the AI Agents activity).
- A card's title, when it has one, is a section label inside the card at the top left.

## 9. Tables and lists

`fontTable` 12.5; header row = section labels, 24 px, a 1 px `separator` under it (the one line that stays); rows 28 px
with `Theme.stripe` on every second row instead of row rules; the selected row `selection`, hover `controlHover`.
Numbers right-aligned in `fontMono` with the unit in `fontSmall` muted after them. Channel rows start with the channel's
pill. Row actions (Recall, Save, ×) are quiet buttons that appear on hover and on the selected row. The band table under
the graph and Delay & Gain already follow this; Presets, Snapshots, Events, Documents and the AutoEQ list should use the
same row (missing: a shared `ListRow` delegate; today each list draws its own).

## 10. Form rows and number fields

A form row (`SettingsValueRow`, `SettingsToggle`, `SettingsPickerRow`, `SettingsSliderRow`, to be shared outside
Settings) is 36 px: label in `text` at the left with an optional caption in `fontSmall` muted under it; the control at the
right edge of the 760 px column. Number fields (`NumericField`, `BorderedField`, `MatrixField`, `ChannelValueField`):
`control` fill, no border, radius 6, 24 high, `fontMono` right-aligned, width 72 (gain, Q) or 88 (frequency, delay),
the unit after the field in `fontSmall` muted; select all on focus; wheel steps; Enter commits, Escape restores; the
focus ring is the only outline. A stepper ("−  0  +") is used only for integers (passes, group numbers).

## 11. Sliders and meters

`CustomSlider`: 4 px track in `control`, the filled part in `accent`, a 14 px handle in `text` on `panel`; a value
field beside the slider, never a value baked into the label ("Line Width: 1.5pt" and "Grid Opacity: 50%" in Graphing are
Classic). `LevelMeter`: `meterTrack` with the three segments and the hold line; the dock's 16 small meters at 3 px.

## 12. Graph framing

The graph is the brightest thing on screen and never sits in a box: no border, no card, the plot fills its area to the
page padding; axis labels in `fontCaption` muted drawn inside the plot with 6 px inset so the top and bottom labels and
the last frequency label are never clipped (today "+25", "−25" and "20k" are cut on every PEQ graph); grid `graphGrid`;
overlays (RTA, Phase, All Curves, Pop out) as toggle chips at the top right at 0.92 opacity; a hint sentence at the
bottom in `fontSmall` muted. The same framing serves the Effects previews (today four different boxed mini-graphs with
7 px mono labels), the Measure graphs, Live and AutoEQ (missing: a shared `GraphFrame` that gives axes, padding and the
empty "Disabled / No curve yet" label; the plot stays each page's).

## 13. Chips and pills

- Channel pill (`Pill`): tint fill, `textOnTint`, height 18, pill radius; the checkable form shows a ring when off.
- Option chip (missing: `OptionChip` / `ChipGroup`): `control` fill, radius 12, 24 high, label `fontSmall`; chosen =
  `selection` fill + `accentText`, a checkable set with "All / None" quiet buttons at the right of its section label.
  This replaces the accent-filled "1 … 9" output buttons on all seven Effects pages, the Detector/Apply rows, the
  Signal Generator's output rectangles and the "Outputs" pair buttons of Crossfeed.
- Status chip: pill, `control` fill with a 6 px dot (`success` / `textMuted` / `warning`) and `fontSmall` text
  ("Active", "Idle", "Connected since").

## 14. Banners, notices, toasts

- **Status strip** (`StatusStrip`): only states that limit what can be done (demo device, read-only firmware, installer
  lock, hub lost, safe-apply countdown, sender, offline file) and the AI working line. One line, 32 px, `bannerFill(kind)`
  with a 3 px left edge in `bannerEdge(kind)`, no full outline, icon 16, text, actions at the right as secondary buttons.
  The demo banner is `info` and should read as calm as the rest (today its blue fill is the loudest surface on screen).
- **Notice row** (missing: `NoticeRow`): inline in a page where a condition blocks an action: 32 px, `bannerFill(kind)`,
  icon, one sentence, an optional action at the right. Replaces every bare orange sentence (Measure, Live, Auto-tune,
  Several units, the empty Measure page). One notice per place; never a notice under a heading that says the same.
- **Toast** (`ModernShell.toast`): bottom right, `panel`, radius 8, shadow, 5 s, for results ("Preset saved").
- Help text under a control: `fontSmall` muted, never warning colour.

## 15. Dialogs, inline panels, the inspector, popovers

| Where | What goes there | Look |
|---|---|---|
| Modal sheet (`QuestionDialog`, `AiSheet`, `UnlockDialog`, `PairingDialog`) | destructive confirmations, PIN, an agent's question, unsaved changes, pairing | centred, 420–460 wide, `panel`, radius 8, shadow, scrim; title `fontTitle`, body `fontBody`, buttons at the right with the primary last; Escape cancels |
| Inline (a page) | everything that is worked in: tools, lists, forms, wizards | never a Cancel/OK row at the bottom of a page (AutoEQ, What's New, Firmware have one) |
| Inspector (`Inspector`, `InspectorSection`) | details of the current selection: band, channel, the effects chain | 320 px sheet, collapsible sections with the section label and a value summary at the right; one primary action at most |
| Popover (`PopoverPanel`, `AiPopover`, `DeviceSwitcher`, menus) | pickers and status that are read and closed: devices, AI, notifications, slot menu, split menus | anchored under their button, `panel`, radius 8, shadow, no heading unless it has actions (then title left, "Done" right) |
| Window | "Open in Window" of any page, the graph pop-out | the page's own content, 12 px padding, no extra header |
| File picking | import / export / open / save | the platform's dialog (portal on Linux). The Qt widgets fallback seen in the container is not acceptable in a release build: verify that the AppImage and the Fedora package get the native one |

## 16. Empty states

One component (missing: `EmptyState`) used everywhere a page has nothing to show: centred in the content area, a 32 px
Lucide icon in `textMuted`, a title in `fontTitle` (`text`), one sentence in `fontBody` muted, and one action (a primary
button when it is the next step, a secondary one otherwise). Never a bare sentence (System, Measure › Results, Documents,
Several units, Snapshots, the AI activity list), never a warning-coloured sentence, and never the page's skeleton with
nothing in it (Routing with no device). The page tools stay enabled only when they make sense without content.

## 17. Icons

Lucide through `Icon`, 16 px in buttons and rows, 20 px in the rail and intro rows, stroke 1.75, `textMuted` unless the
icon carries state (`accentText` on, `warning`, `dangerText`). Every icon button has a tooltip and an `Accessible.name`.
The channel-role and settings badges (`Theme.settingsTints`) are the Mac's and stay, at 18 px, radius 4.

## 18. Where the language lives today, and what is missing

Exists and is right: the Grey/Darker tone sets in `Theme.qml`; `PageTabs`, `NavRail`, `TopBar`, `Dock`, `SplitButton`,
`CommandPalette`, `Inspector`/`InspectorSection`, `ChannelStrip`, `EqualiserDelayRow`, `BandRow`, `AiSheet`,
`QuestionDialog`, `Pill`, `IconButton`, `AppSwitch`, the Settings row components, `SiggenTile`.

To add or change in the shared layer (the first work package, DL0):
- Tokens: `stripe`, `shadow`, `radiusControl`, `fontPageTitle`, `fontTable`, `spacingSection`, `topBarHeight`,
  `tabBarHeight`, `dockHeight`, `contentMaxWidth`; `rowHeight` 28 in Modern.
- `Card`: border off in Modern. `AppButton`, `BorderedField`, `NumericField`, `AppComboBox`, `AppSpinBox`: border off in
  Modern, control fill, radius 6. `ToolSegments`: selection tone, content-sized. `ToolHeader`: the intro row of §6.
- New: `EmptyState`, `NoticeRow`, `OptionChip` + `ChipGroup`, `ToggleChip`, `ListRow`, `GraphFrame`, `FormRow`
  (the Settings rows generalised).
- `GraphPlot`: axis-label insets.

## 19. Component map (DL0, 2026-11-24)

DL0 added every token and component marked "missing" above (`wp/DL0`). Everything that would change Classic is gated on
`LayoutController.modern`; Classic keeps its look. Each file's header comment has its full API and an example.

**Tokens** (`qml/Theme/Theme.qml`, under `-- WP DL0 --`)

| Token | Value | Notes |
|---|---|---|
| `lineWidth` | 0 in Modern, 1 in Classic | the outline of a card, button, field, combo box, spin box, segment bar, the response graph |
| `focusRingWidth` | 2 | with `focusRing` (accent): the one outline that stays in Modern |
| `stripe` | `#06ffffff` dark / `#09000000` light | every second row |
| `shadowColor`, `shadowOffset`, `shadowBlur` | `#80000000`, 12, 34 dark; `#29000000`, 10, 30 light | floating surfaces (for DL5's popovers; not yet drawn anywhere) |
| `radiusControl` | 6 in Modern (Classic: `radiusSmall`, 4) | buttons, fields, segments, toggle chips, notices |
| `radiusChip` | 12 | option chips |
| `spacingSection` | 24 | between sections; under an intro row |
| `topBarHeight`, `tabBarHeight`, `dockHeight` | 40, 36, 44 | the shell's bars (tokens only; the shell is not migrated) |
| `formRowHeight`, `introRowHeight` | 36, 44 | `FormRow`, `ToolHeader` |
| `contentMaxWidth` | 760 | `ContentColumn` |
| `rowHeight` | 28 in Modern (26 Classic) | |
| `fontPageTitle`, `fontTable` | 15 DemiBold; 12 | the spec's 12.5 is not a whole pixel size: 12 |

`tools/contrast.py` now also checks `accentText` on `selection` (a chosen chip or segment): 4.6 to 5.9 in every tone.

**Changed components**

| Component | Modern now | API change |
|---|---|---|
| `Card` | no outline | none |
| `AppButton` | no outline, radius 6, hover/pressed in `controlHover`/`controlPressed` | none |
| `BorderedField`, `NumericField`, `AppSpinBox`, `AppComboBox` | no outline, radius 6; focus ring 2 px; `NumericField` keeps the warning ring when held to its range | none |
| `ToolSegments` | one control-tone bar, no outline; the chosen segment `selection` + `accentText`; the bar as wide as its segments, at the left of the room it gets, even under `Layout.fillWidth: true` | root is now an `Item` (was a `Rectangle`); new `contentSized` (default: Modern) |
| `ToolHeader` | the intro row: no background, 44 high, 20 px icon, `fontSmall` description | new `showSwitch` (default true) |
| `GraphPlot` | no outline; axis labels by `AxisLabels` 6 px inside (BodePlot's labels layer stays for Classic): "+25", "−25", "20k" whole; a label that would collide after the inset is dropped, the end labels kept | new child `GraphHost.plot.axisLabels` |
| `BandRow` | `stripe` token, hover tone | none |

**New components** (`qml/components/`)

| Component | Use for | Key API |
|---|---|---|
| `OptionChip` | one option that can be picked | `text`, `checked` (owner-bound), `clicked`, `iconName`, `toolTip`, `exclusive`, `chipRadius` |
| `ChipGroup` | §13: output "1 … 9", Detector/Apply, pairs, tube types | `label`, `model` (strings or `{text, toolTip, iconName, enabled}`), many: `selected` (index list) + `toggled(index, on)` + `setAll(on)` (All/None, shown with `allNone`); one: `exclusive`, `current` + `chosen(index)`. Names: `<name>.chip.<i>`, `.all`, `.none` |
| `ToggleChip` | §7 toggles: RTA, Phase, All Curves, Solo | `checked`, `toggled()` (flips itself), `overlay` (0.92 opacity) |
| `EmptyState` | §16 | `iconName`, `title`, `text`, `actionText`, `actionPrimary`, `actionEnabled`, `action()`; button `<name>.action` |
| `NoticeRow` | §14 notice rows (replaces orange sentences) | `kind` ("info" / "warning" / "error"), `text`, `actionText`, `action()`, extra buttons as children; button `<name>.action` |
| `ListRow` | §9 the shared row | `rowIndex` (stripe), `title`, `subtitle`, `value`, `monoValue`, `iconName`, `selected`, `leading` (pill, dot), children = hover actions, `actionsAlways`; `clicked(modifiers)`, `activated()`, `menuRequested(x, y)`; the focus ring while it is a focused ListView's current item |
| `FormRow` | §10 the 36 px form row | `title`, `caption`, children = the control(s) at the right, `controlFills`, `titleWidth` |
| `ContentColumn` | §3 the 760 px left-aligned column | a `ColumnLayout` with `maximumWidth` and `spacingSection` |
| `GraphFrame` | §12 previews, Measure, Live, AutoEQ | `xTicks`/`yTicks`/`rightTicks` (`{at, text}`), `grid`, `filled`, `empty` + `emptyText`, `hint`, `tools` (ToggleChips), the plot as its child (fills `plotArea`) |
| `AxisLabels` | labels over any plot, whole inside | `xTicks`, `yTicks`, `rightTicks`, `inset`, `font`, `color`, `rightColor` |

Not done in DL0 (left to the packages that own the places): the floating-surface shadow (DL5), the page title in
`PageTabs` and the status strip (DL4/DL5), `SectionHeading`'s 24/8 spacing (a page's layout), Settings rows on `FormRow`
(DL7), `Pill` and `IconButton` (already as the spec says).
