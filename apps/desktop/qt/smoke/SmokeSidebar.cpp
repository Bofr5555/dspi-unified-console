// -- WP QP1 -- The sidebar's section: rows, selection with linked pairs, the
// curve pills, rename by Alt+click, the row menu, the channel clipboard,
// identify, the quick strip, presets with the unsaved question, the source,
// the volume modes and tapers, the CPU loads and the device button's notice.
// Every change is undone before the section ends.
#include "SmokeDriver.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "ChannelController.h"
#include "Poller.h"
#include "PresetModel.h"
#include "Shell.h"
#include "SidebarController.h"

#include "CoreApi.h"

#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>
#include <cmath>
#include <cstdio>

namespace {

QString shownName(QObject *o) { return o ? o->property("shownName").toString() : QString(); }

void click(SmokeDriver &d, QQuickItem *item, Qt::MouseButton button = Qt::LeftButton,
           Qt::KeyboardModifiers modifiers = Qt::NoModifier) {
    if (!item) return;
    const QPoint at = item->mapToScene(QPointF(item->width() / 2, item->height() / 2)).toPoint();
    QTest::mouseClick(d.mainWindow(), button, modifiers, at);
    d.spin(80);
}

bool near(double a, double b) { return std::fabs(a - b) < 1e-6; }

// The first and second wire channels of the outputs the sidebar shows
QList<int> shownOutputs(SmokeDriver &d) {
    QList<int> out;
    const ChannelListModel *rows = d.app().device->channels();
    for (int r = 0; r < rows->rowCount(); ++r)
        if (rows->get(r, QStringLiteral("kind")).toString() == QLatin1String("output") &&
            rows->get(r, QStringLiteral("outputEnabled")).toBool())
            out.append(rows->get(r, QStringLiteral("channel")).toInt());
    return out;
}

// Field by field: the record has padding bytes
bool sameBand(const FilterParams &x, const FilterParams &y) {
    return x.filter_type == y.filter_type && x.bypass == y.bypass && x.freq == y.freq && x.q == y.q && x.gain == y.gain &&
           x.qp == y.qp;
}

bool sameBands(const DspState *s, int a, int b) {
    if (!s) return false;
    for (int band = 0; band < BANDS_PER_CHANNEL; ++band)
        if (!sameBand(s->filters[a][band], s->filters[b][band])) return false;
    return true;
}

bool bandsAre(const DspState *s, int channel, const QVector<FilterParams> &bands) {
    if (!s || bands.size() != BANDS_PER_CHANNEL) return false;
    for (int band = 0; band < BANDS_PER_CHANNEL; ++band)
        if (!sameBand(s->filters[channel][band], bands.at(band))) return false;
    return true;
}

// Writes a channel's ten bands back, as the section found them
void restoreBands(SmokeDriver &d, int channel, const QVector<FilterParams> &bands) {
    d.app().device->write(QStringLiteral("qp1 restore %1").arg(channel), [channel, bands](FfiCore *c) {
        bool ok = true;
        for (int b = 0; b < bands.size(); ++b) ok = dspi_set_filter(c, uint8_t(channel), uint8_t(b), bands.at(b)) && ok;
        return ok;
    });
}

QVector<FilterParams> bandsOf(const DspState *s, int channel) {
    QVector<FilterParams> v;
    for (int b = 0; s && b < BANDS_PER_CHANNEL; ++b) v.append(s->filters[channel][b]);
    return v;
}

void checkTapers(SmokeDriver &d) {
    using S = SidebarController;
    d.check(QStringLiteral("qp1: user taper ends"), near(S::userPositionToDb(0), -60) && near(S::userPositionToDb(1), 0));
    d.check(QStringLiteral("qp1: user taper is a square root (0.25 is -30 dB)"),
            near(S::userPositionToDb(0.25), -30) && near(S::userDbToPosition(-30), 0.25));
    const double break1 = 1.0 - 100.0 / 248.0, break2 = break1 - 60.0 / 248.0;
    d.check(QStringLiteral("qp1: master taper breaks at -10 and -40 dB"),
            near(S::masterPositionToDb(break1), -10) && near(S::masterPositionToDb(break2), -40));
    d.check(QStringLiteral("qp1: master taper ends at mute and 0 dB"),
            S::masterPositionToDb(0) == -128 && S::masterPositionToDb(1) == 0 && S::masterDbToPosition(-128) == 0);
    bool roundTrip = true;
    for (double db : { -0.5, -9.9, -12.5, -39.0, -77.0, -127.0 })
        roundTrip = roundTrip && std::fabs(S::masterPositionToDb(S::masterDbToPosition(db)) - db) < 1e-6;
    d.check(QStringLiteral("qp1: master taper round-trips"), roundTrip);
    d.check(QStringLiteral("qp1: master readout snaps to the firmware's steps"),
            S::formatMasterDb(-7.46) == QStringLiteral("-7.5 dB") && S::formatMasterDb(-12.3) == QStringLiteral("-12.5 dB") &&
                S::formatMasterDb(-52.4) == QStringLiteral("-52 dB") && S::formatMasterDb(-0.04) == QStringLiteral("0 dB"));
    d.check(QStringLiteral("qp1: master readout at mute is minus infinity"), S::formatMasterDb(-128) == QStringLiteral("−∞"));
    d.check(QStringLiteral("qp1: user readout"), S::formatUserDb(-12) == QStringLiteral("-12.0 dB"));
    d.check(QStringLiteral("qp1: source names keep printable ASCII, 31 at most"),
            S::sanitizeSourceName(QStringLiteral("Opt\tical ü")) == QStringLiteral("Optical ") &&
                S::sanitizeSourceName(QString(40, QLatin1Char('a'))).size() == 31);
}

void checkRows(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    const int expected = device->numInputs() >= 8 ? 23 : device->numInputs() >= 6 ? 27 : 28;
    QQuickItem *row0 = d.item(QStringLiteral("Sidebar.channel.0"));
    d.check(QStringLiteral("qp1: input rows are as tall as the Mac's for %1 inputs").arg(device->numInputs()),
            row0 && int(row0->height()) == expected);
    const QList<int> outs = shownOutputs(d);
    d.check(QStringLiteral("qp1: the sidebar shows outputs"), outs.size() >= 2);
    for (int ch : outs) {
        d.check(QStringLiteral("qp1: output row %1 has a pill").arg(ch), d.item(QStringLiteral("Sidebar.pill.%1").arg(ch)) != nullptr);
        d.check(QStringLiteral("qp1: output row %1 has a meter").arg(ch), d.item(QStringLiteral("Sidebar.meter.%1").arg(ch)) != nullptr);
    }
    d.check(QStringLiteral("qp1: slot names come from the core"), !sc->slotNames().isEmpty() && !sc->slotNames().first().isEmpty());
}

void checkSelection(SmokeDriver &d, SidebarController *sc) {
    QObject *sidebar = d.item(QStringLiteral("Sidebar"));
    d.check(QStringLiteral("qp1: the overview is selected at start"), sc->selectedChannel() == -1 && sc->selectedKind() == QLatin1String("overview"));
    ChannelController *cc = Registrar::instance<ChannelController>();
    GraphController *g = Registrar::instance<GraphController>();
    d.check(QStringLiteral("qp1: inputs 1/2 are linked by default"), cc->isLinked(0) && sc->linkedPartner(0) == 1 && sc->linkedPartner(1) == 0);

    click(d, d.item(QStringLiteral("Sidebar.channel.0")));
    d.check(QStringLiteral("qp1: a click selects input 1"), sc->selectedChannel() == 0 && sc->selectedKind() == QLatin1String("input") && sc->selectedIndex() == 0);
    d.check(QStringLiteral("qp1: the main window follows the selection"), sidebar && sidebar->property("selectedChannel").toInt() == 0);
    d.check(QStringLiteral("qp1: the linked partner is highlighted too"), sc->highlightedMask() == 0b11);
    // -- WP QPI1 -- the right pane shows the input's page, the graph the pair
    d.check(QStringLiteral("qpi1: the right pane shows input 1's page"), d.waitFor(2000, [&]() {
        QQuickItem *page = d.item(QStringLiteral("InputDetail"));
        return page && page->isVisible() && page->property("input").toInt() == 0;
    }));
    d.check(QStringLiteral("qpi1: the dashboard gives way to the page"), !d.item(QStringLiteral("Dashboard")) || !d.item(QStringLiteral("Dashboard"))->isVisible());
    d.check(QStringLiteral("qpi1: the graph shows the linked pair"), g->visibleChannels() == QList<int>({ 0, 1 }));
    d.check(QStringLiteral("qp1: screenshot of an input page"), d.grab(d.mainWindow(), QStringLiteral("window-main-input.png")));
    click(d, d.item(QStringLiteral("Sidebar.channel.1")));
    d.check(QStringLiteral("qp1: a click on the partner goes back to the overview"), sc->selectedChannel() == -1 && sc->highlightedMask() == 0);

    cc->unlinkPair(0);
    d.spin(50);
    d.check(QStringLiteral("qpi1: unlinking in the input header ends the highlight"), sc->linkedPartner(0) == -1);
    click(d, d.item(QStringLiteral("Sidebar.channel.0")));
    click(d, d.item(QStringLiteral("Sidebar.channel.1")));
    d.check(QStringLiteral("qp1: unlinked, the other input is selected instead"), sc->selectedChannel() == 1 && sc->highlightedMask() == 0b10);
    d.check(QStringLiteral("qpi1: unlinked, the graph shows the one input"), d.waitFor(1000, [&]() { return g->visibleChannels() == QList<int>({ 1 }); }));
    d.check(QStringLiteral("qpi1: and the page is input 2's"), d.item(QStringLiteral("InputDetail")) && d.item(QStringLiteral("InputDetail"))->property("input").toInt() == 1);
    d.check(QStringLiteral("qp1: the link choice is kept per device"),
            QSettings().value(QStringLiteral("inputLinks/") + d.app().device->serial()).toInt() == 0);
    cc->linkPair(0, -1);
    d.spin(50);
    d.check(QStringLiteral("qpi1: linked again, the graph shows the pair"), d.waitFor(1000, [&]() { return g->visibleChannels() == QList<int>({ 0, 1 }); }));
    click(d, d.item(QStringLiteral("Sidebar.channel.1")));

    const QList<int> outs = shownOutputs(d);
    if (!outs.isEmpty()) {
        click(d, d.item(QStringLiteral("Sidebar.channel.%1").arg(outs.first())));
        d.check(QStringLiteral("qp1: a click selects an output"), sc->selectedChannel() == outs.first() && sc->selectedKind() == QLatin1String("output"));
        d.check(QStringLiteral("qpi1: the right pane shows the output's page"), d.waitFor(2000, [&]() {
            QQuickItem *page = d.item(QStringLiteral("OutputDetail"));
            return page && page->isVisible() && page->property("output").toInt() == sc->selectedIndex();
        }));
        d.check(QStringLiteral("qp1: screenshot of an output page"), d.grab(d.mainWindow(), QStringLiteral("window-main-output.png")));
        click(d, d.item(QStringLiteral("Sidebar.channel.%1").arg(outs.first())));
        d.check(QStringLiteral("qp1: a second click goes back to the overview"), sc->selectedChannel() == -1);
        d.check(QStringLiteral("qpi1: and the dashboard shows again"), d.waitFor(2000, [&]() {
            QQuickItem *dash = d.item(QStringLiteral("Dashboard"));
            return dash && dash->isVisible() && !d.item(QStringLiteral("OutputDetail"));
        }));
    }
}

// -- WP QPI1 -- the pills show and set GraphController's visibility
void checkPills(SmokeDriver &d, SidebarController *sc) {
    GraphController *g = Registrar::instance<GraphController>();
    QQuickItem *pill = d.item(QStringLiteral("Sidebar.pill.2"));
    QStringList shown;
    for (int ch : g->visibleChannels()) shown << QString::number(ch);
    std::printf("smoke: qpi1 graph curves at the pills' check: %s\n", qPrintable(shown.join(QLatin1Char(' '))));
    const bool start = g->isCurveVisible(2);
    d.check(QStringLiteral("qpi1: the pill shows the graph's curve state"), pill && pill->property("checked").toBool() == start);
    click(d, pill);
    d.check(QStringLiteral("qpi1: the pill toggles the graph's curve"), g->isCurveVisible(2) == !start && g->visibleChannels().contains(2) == !start);
    d.check(QStringLiteral("qp1: the pill does not select its row"), sc->selectedChannel() == -1);
    d.check(QStringLiteral("qpi1: the pill shows the new state"), pill && pill->property("checked").toBool() == !start);
    g->toggleCurve(2);
    d.spin(50);
    d.check(QStringLiteral("qpi1: the pill follows a change made elsewhere"),
            g->isCurveVisible(2) == start && pill && pill->property("checked").toBool() == start);
}

void checkRename(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    const QString before = device->channels()->get(device->channels()->rowOfChannel(3), QStringLiteral("name")).toString();
    click(d, d.item(QStringLiteral("Sidebar.channel.3")), Qt::LeftButton, Qt::AltModifier);
    QQuickItem *field = d.item(QStringLiteral("Sidebar.rename.3"));
    d.check(QStringLiteral("qp1: Alt+click opens the rename field"), field && field->isVisible() && field->hasActiveFocus());
    d.check(QStringLiteral("qp1: Alt+click does not select the row"), Registrar::instance<SidebarController>()->selectedChannel() == -1);
    if (!field) return;
    field->setProperty("text", QStringLiteral("  QP1 Rename  "));
    QTest::keyClick(d.mainWindow(), Qt::Key_Return);
    d.check(QStringLiteral("qp1: Enter renames the channel, trimmed"),
            d.waitFor(5000, [&]() { return shownName(d.item(QStringLiteral("Sidebar.channel.3"))) == QLatin1String("QP1 Rename"); }));
    d.check(QStringLiteral("qp1: the field closes"), !field->isVisible());

    // Escape leaves the name alone
    click(d, d.item(QStringLiteral("Sidebar.channel.3")), Qt::LeftButton, Qt::AltModifier);
    field->setProperty("text", QStringLiteral("Not this"));
    QTest::keyClick(d.mainWindow(), Qt::Key_Escape);
    d.spin(300);
    d.check(QStringLiteral("qp1: Escape keeps the name"), shownName(d.item(QStringLiteral("Sidebar.channel.3"))) == QLatin1String("QP1 Rename"));
    device->setChannelName(3, before);
    d.waitIdle();
}

void checkRowMenu(SmokeDriver &d, SidebarController *sc) {
    const QList<int> outs = shownOutputs(d);
    if (outs.isEmpty()) return;
    const int out = outs.first();
    QQuickItem *row = d.item(QStringLiteral("Sidebar.channel.%1").arg(out));
    click(d, row, Qt::RightButton);
    // A delegate's popups are its QObject children, outside the window's tree
    QObject *menu = row ? row->findChild<QObject *>(QStringLiteral("Sidebar.rowMenu.%1").arg(out)) : nullptr;
    d.check(QStringLiteral("qp1: a right-click opens the row's menu"), menu && d.waitFor(1000, [&]() { return menu->property("opened").toBool(); }));
    QObject *identify = row ? row->findChild<QObject *>(QStringLiteral("Sidebar.menu.identify.%1").arg(out)) : nullptr;
    d.check(QStringLiteral("qp1: an output's menu offers Identify on the demo"), identify && identify->property("visible").toBool() == sc->identifySupported());
    QObject *paste = row ? row->findChild<QObject *>(QStringLiteral("Sidebar.menu.paste.%1").arg(out)) : nullptr;
    d.check(QStringLiteral("qp1: Paste waits for a copy"), paste && paste->property("enabled").toBool() == sc->hasClipboard());
    // -- WP QPI1 -- the last row sits clear of the menu's bottom edge
    d.check(QStringLiteral("qpi1: the row menu has room below its last row"),
            menu && menu->property("bottomPadding").toDouble() >= 4 && menu->property("topPadding").toDouble() >= 4);
    d.check(QStringLiteral("qp1: screenshot of the row menu"), d.grab(d.mainWindow(), QStringLiteral("window-main-sidebar-menu.png")));
    QTest::keyClick(d.mainWindow(), Qt::Key_Escape);
    d.check(QStringLiteral("qp1: Escape closes the menu"), d.waitFor(1000, [&]() { return menu && !menu->property("opened").toBool(); }));

    QQuickItem *input = d.item(QStringLiteral("Sidebar.channel.0"));
    QObject *inputMenuIdentify = input ? input->findChild<QObject *>(QStringLiteral("Sidebar.menu.identify.0")) : nullptr;
    d.check(QStringLiteral("qp1: an input's menu has no Identify"), inputMenuIdentify && !inputMenuIdentify->property("visible").toBool());
}

void checkClipboard(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    const QList<int> outs = shownOutputs(d);
    if (outs.size() < 2 || !device->state()) return;
    const int a = outs.at(0), b = outs.at(1);
    // Make the source differ: band 0 of output A, a bell
    const QVector<FilterParams> savedA = bandsOf(device->state(), a), savedB = bandsOf(device->state(), b);
    const QVector<FilterParams> saved0 = bandsOf(device->state(), 0), saved1 = bandsOf(device->state(), 1);
    FilterParams bell = savedA.at(0);
    bell.filter_type = 1;
    bell.freq = 777.0f;
    bell.gain = 3.0f;
    bell.q = 0.9f;
    device->write(QStringLiteral("qp1 seed"), [a, bell](FfiCore *c) { return dspi_set_filter(c, uint8_t(a), 0, bell); });
    d.waitIdle();

    sc->select(a);
    d.app().shell->trigger(QStringLiteral("edit.copy"));
    d.check(QStringLiteral("qp1: Edit > Copy takes the selected channel"),
            sc->hasClipboard() && sc->clipboardName() == device->channels()->get(device->channels()->rowOfChannel(a), QStringLiteral("label")).toString());
    sc->pasteChannel(b);
    d.check(QStringLiteral("qp1: Paste copies the bands to another output"),
            d.waitFor(5000, [&]() { return sameBands(device->state(), a, b); }));
    sc->pasteChannel(0);
    d.check(QStringLiteral("qp1: Paste onto a linked input mirrors to its partner"),
            d.waitFor(5000, [&]() { return sameBands(device->state(), a, 0) && sameBands(device->state(), a, 1); }));
    sc->select(-1);
    restoreBands(d, a, savedA);
    restoreBands(d, b, savedB);
    restoreBands(d, 0, saved0);
    restoreBands(d, 1, saved1);
    d.waitIdle();
    d.check(QStringLiteral("qp1: the bands are restored"),
            bandsAre(device->state(), a, savedA) && bandsAre(device->state(), b, savedB) &&
                bandsAre(device->state(), 0, saved0) && bandsAre(device->state(), 1, saved1));
}

void checkIdentify(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("qp1: the demo's generator can identify outputs"), d.waitFor(3000, [&]() { return sc->identifySupported(); }));
    bool failed = false;
    const auto c = QObject::connect(device, &DeviceModel::writeFailed, [&failed](const QString &) { failed = true; });
    const int before = device->settersRun();
    sc->identifyOutput(0);
    d.waitIdle();
    d.check(QStringLiteral("qp1: Identify starts the generator"), device->settersRun() == before + 1 && !failed);
    QObject::disconnect(c);
    d.app().core->worker()->post([](FfiCore *core) { dspi_siggen_control(core, SIGGEN_CTL_STOP_NOW); });
    d.waitIdle();
}

void checkQuickStrip(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    QStringList triggered;
    const auto c = QObject::connect(d.app().shell, &Shell::triggered, [&triggered](const QString &a) { triggered.append(a); });
    QWindowList shown;
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->isVisible()) shown.append(w);

    QQuickItem *crossfeed = d.item(QStringLiteral("QuickStrip.crossfeed"));
    const bool was = sc->crossfeedOn();
    click(d, crossfeed);
    d.check(QStringLiteral("qp1: the crossfeed button switches crossfeed"), d.waitFor(3000, [&]() { return sc->crossfeedOn() == !was; }));
    d.check(QStringLiteral("qp1: the button shows it"), crossfeed && crossfeed->property("checked").toBool() == !was);
    click(d, crossfeed);
    d.check(QStringLiteral("qp1: and back"), d.waitFor(3000, [&]() { return sc->crossfeedOn() == was; }));
    click(d, crossfeed, Qt::RightButton);
    d.check(QStringLiteral("qp1: a right-click opens the crossfeed window"), triggered.contains(QStringLiteral("tools.crossfeed")));
    d.check(QStringLiteral("qp1: a right-click switches nothing"), sc->crossfeedOn() == was);

    for (const char *name : { "loudness", "leveller", "bypass" }) {
        QQuickItem *b = d.item(QStringLiteral("QuickStrip.%1").arg(QLatin1String(name)));
        const auto state = [&]() { return QLatin1String(name) == QLatin1String("loudness") ? sc->loudnessOn()
                                          : QLatin1String(name) == QLatin1String("leveller") ? sc->levellerOn() : sc->bypassOn(); };
        const bool on = state();
        click(d, b);
        d.check(QStringLiteral("qp1: the %1 button switches it").arg(QLatin1String(name)), d.waitFor(3000, [&]() { return state() == !on; }));
        click(d, b);
        d.waitFor(3000, [&]() { return state() == on; });
    }
    QQuickItem *psybass = d.item(QStringLiteral("QuickStrip.psybass"));
    d.check(QStringLiteral("qp1: the bass button shows where the firmware has it"),
            psybass && psybass->isVisible() == ((device->snapshot().wireFeatures & WIRE_FEAT_PSYBASS) != 0));

    click(d, d.item(QStringLiteral("QuickStrip.matrix")));
    click(d, d.item(QStringLiteral("QuickStrip.stats")));
    click(d, d.item(QStringLiteral("QuickStrip.settings")));
    d.check(QStringLiteral("qp1: the window buttons run their menu actions"),
            triggered.contains(QStringLiteral("tools.matrix")) && triggered.contains(QStringLiteral("tools.stats")) &&
                triggered.contains(QStringLiteral("edit.settings")));
    QObject::disconnect(c);
    // -- WP QPF1 -- The windows these buttons opened took the focus. Close them and make the main
    // window active again, as a person's next click there would: Qt Quick drops a
    // window's active focus item when it loses the focus, so without this the keys
    // the checks below send to the main window's dialogs would reach nothing
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->isVisible() && !shown.contains(w)) w->close();
    d.mainWindow()->requestActivate();
    d.check(QStringLiteral("qp1: the main window is active again"),
            d.waitFor(1000, [&]() { return QGuiApplication::focusWindow() == d.mainWindow(); }));
    d.waitIdle();
}

void checkPresets(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    PresetModel *presets = device->presets();
    QObject *box = d.item(QStringLiteral("GlobalControls.preset"));
    const int start = presets->activeSlot();
    d.check(QStringLiteral("qp1: a preset is active"), start >= 0);
    if (start < 0) return;
    if (sc->unsaved()) {
        sc->savePreset();
        d.waitIdle();
    }
    d.check(QStringLiteral("qp1: nothing unsaved after a save"), d.waitFor(3000, [&]() { return !sc->unsaved(); }));
    d.check(QStringLiteral("qp1: the popup shows the active slot"), shownName(box) == sc->presetTitle(start));
    d.check(QStringLiteral("qp1: a saved slot has a title"), sc->presetOccupied(start) && sc->presetTitle(start) != QLatin1String("Empty"));

    int spare = -1;
    for (int s = 0; s < MAX_PRESETS && spare < 0; ++s)
        if (!sc->presetOccupied(s)) spare = s;
    d.check(QStringLiteral("qp1: the demo has an empty slot"), spare >= 0);
    if (spare < 0) return;
    d.check(QStringLiteral("qp1: an empty slot reads Empty"), sc->presetTitle(spare) == QLatin1String("Empty") &&
                                                                 sc->presetLabel(spare) == QStringLiteral("%1: Empty").arg(spare + 1));

    // Copy to the empty slot: it gets the default name and stays inactive
    sc->requestPresetCopy(spare);
    d.check(QStringLiteral("qp1: Copy to fills the slot"), d.waitFor(8000, [&]() { return !sc->presetBusy() && sc->presetOccupied(spare); }));
    d.check(QStringLiteral("qp1: the copy is named Preset N"), sc->presetTitle(spare) == QStringLiteral("Preset %1").arg(spare + 1));
    d.check(QStringLiteral("qp1: the source stays active"), presets->activeSlot() == start);

    // An unsaved change: the "*" and the question before a switch
    const float gain = device->state()->output_gain_db[0];
    device->write(QStringLiteral("qp1 dirty"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 3.0f); });
    d.check(QStringLiteral("qp1: an edit marks the preset unsaved"), d.waitFor(3000, [&]() { return sc->unsaved() && !sc->unsavedChanges().isEmpty(); }));
    d.check(QStringLiteral("qp1: the popup's title carries the star"), d.waitFor(1000, [&]() { return shownName(box).endsWith(QLatin1Char('*')); }));

    QObject *question = d.object(QStringLiteral("GlobalControls.unsavedQuestion"));
    sc->requestPresetSwitch(spare);
    d.check(QStringLiteral("qp1: switching asks about the unsaved changes"),
            question && d.waitFor(3000, [&]() { return question->property("opened").toBool(); }));
    d.check(QStringLiteral("qp1: the question lists the changes"), question && question->property("text").toString().contains(QStringLiteral("• ")));
    d.spin(250);
    d.check(QStringLiteral("qp1: screenshot of the unsaved question"), d.grab(d.mainWindow(), QStringLiteral("window-main-unsaved-question.png")));
    QTest::keyClick(d.mainWindow(), Qt::Key_Escape);
    d.check(QStringLiteral("qp1: Escape closes the question"), d.waitFor(2000, [&]() { return !question->property("opened").toBool(); }));
    d.waitIdle();
    d.check(QStringLiteral("qp1: Cancel stays on the slot"), presets->activeSlot() == start && sc->unsaved());

    sc->requestPresetSwitch(spare);
    d.waitFor(3000, [&]() { return question->property("opened").toBool(); });
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("alternative")));
    d.check(QStringLiteral("qp1: Discard loads the other slot"), d.waitFor(8000, [&]() { return presets->activeSlot() == spare && !sc->presetBusy(); }));
    d.check(QStringLiteral("qp1: the loaded slot has nothing unsaved"), d.waitFor(3000, [&]() { return !sc->unsaved(); }));

    // Back without a question; the discarded edit is gone
    sc->requestPresetSwitch(start);
    d.check(QStringLiteral("qp1: a clean switch needs no question"),
            d.waitFor(8000, [&]() { return presets->activeSlot() == start && !sc->presetBusy(); }) && !question->property("opened").toBool());
    d.check(QStringLiteral("qp1: the discarded edit is gone"), device->state() && device->state()->output_gain_db[0] == gain);

    // Rename, then clear the spare slot again
    sc->renamePreset(spare, QStringLiteral("QP1 Spare"));
    d.check(QStringLiteral("qp1: Rename names the slot"), d.waitFor(3000, [&]() { return sc->presetTitle(spare) == QLatin1String("QP1 Spare"); }));
    sc->clearPreset(spare);
    d.check(QStringLiteral("qp1: Clear empties the slot"), d.waitFor(8000, [&]() { return !sc->presetBusy() && !sc->presetOccupied(spare); }));
    d.check(QStringLiteral("qp1: the active slot is untouched"), presets->activeSlot() == start);

    // Set as Default, then back to how the device started
    const int mode = presets->startupMode(), def = presets->defaultSlot();
    sc->setDefaultPreset();
    d.check(QStringLiteral("qp1: Set as Default boots into this slot"),
            d.waitFor(3000, [&]() { return presets->startupMode() == 0 && presets->defaultSlot() == start; }));
    device->write(QStringLiteral("qp1 startup"), [mode, def](FfiCore *c) { return dspi_set_preset_startup(c, uint8_t(mode), uint8_t(def)); });
    d.waitIdle();

    // The slot menu opens on a right-click
    click(d, qobject_cast<QQuickItem *>(box), Qt::RightButton);
    QObject *menu = d.object(QStringLiteral("GlobalControls.presetMenu"));
    d.check(QStringLiteral("qp1: a right-click opens the slot menu"), menu && d.waitFor(1000, [&]() { return menu->property("opened").toBool(); }));
    d.check(QStringLiteral("qp1: screenshot of the slot menu"), d.grab(d.mainWindow(), QStringLiteral("window-main-preset-menu.png")));
    QTest::keyClick(d.mainWindow(), Qt::Key_Escape);
    d.waitFor(1000, [&]() { return menu && !menu->property("opened").toBool(); });
}

void checkSource(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    const QVariantList sources = sc->sources();
    d.check(QStringLiteral("qp1: the source popup lists USB, S/PDIF and I2S"),
            sources.size() >= 3 && sources.at(0).toMap().value(QStringLiteral("value")).toInt() == 0 &&
                sources.at(0).toMap().value(QStringLiteral("title")).toString() == QLatin1String("USB"));
    const int start = sc->source();
    sc->setSource(1);
    d.check(QStringLiteral("qp1: choosing a source switches it"), d.waitFor(3000, [&]() { return device->state()->input_source == 1; }));
    d.check(QStringLiteral("qp1: the popup shows it"), shownName(d.item(QStringLiteral("GlobalControls.source"))) == sc->sourceTitle(1));
    if (sc->sourceNamesSupported()) {
        sc->renameSource(1, QStringLiteral("QP1 Optical"));
        d.check(QStringLiteral("qp1: a source can be renamed"), d.waitFor(3000, [&]() { return sc->sourceTitle(1) == QLatin1String("QP1 Optical"); }));
        sc->renameSource(1, QString());
        d.check(QStringLiteral("qp1: an empty name gives the built-in one back"), d.waitFor(3000, [&]() { return sc->sourceName(1).isEmpty(); }));
    }
    sc->setSource(start);
    d.waitFor(3000, [&]() { return device->state()->input_source == start; });
}

void checkVolume(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    QObject *mode = d.item(QStringLiteral("GlobalControls.volumeMode"));
    QQuickItem *slider = d.item(QStringLiteral("GlobalControls.volumeSlider"));
    QObject *readout = d.item(QStringLiteral("GlobalControls.volumeReadout"));
    const QString startMode = sc->volumeMode();
    const double user = sc->userVolumeDb(), master = sc->masterVolumeDb();

    sc->setVolumeMode(QStringLiteral("user"));
    sc->setUserVolume(-30);
    d.check(QStringLiteral("qp1: the user volume is set"), d.waitFor(3000, [&]() { return std::fabs(sc->userVolumeDb() + 30) < 0.05; }));
    d.check(QStringLiteral("qp1: the slider follows the device (taper)"),
            slider && d.waitFor(1000, [&]() { return std::fabs(slider->property("value").toDouble() - 0.25) < 1e-3; }));
    d.check(QStringLiteral("qp1: the readout shows it"), shownName(readout) == QLatin1String("-30.0 dB"));
    click(d, slider, Qt::RightButton);
    d.check(QStringLiteral("qp1: a right-click resets the user volume to 0 dB"), d.waitFor(3000, [&]() { return std::fabs(sc->userVolumeDb()) < 0.05; }));

    sc->setVolumeMode(QStringLiteral("master"));
    d.check(QStringLiteral("qp1: the mode is remembered"), QSettings().value(QStringLiteral("sidebar/showMaster")).toBool());
    d.check(QStringLiteral("qp1: the heading names the master volume"), shownName(mode).startsWith(QLatin1String("Master Volume")));
    sc->setMasterVolume(-20);
    d.check(QStringLiteral("qp1: the master volume is set"), d.waitFor(3000, [&]() { return std::fabs(sc->masterVolumeDb() + 20) < 0.05; }));
    d.check(QStringLiteral("qp1: the master readout"), d.waitFor(1000, [&]() { return shownName(readout) == QLatin1String("-20 dB"); }));
    d.check(QStringLiteral("qp1: screenshot, master volume"), d.grab(d.mainWindow(), QStringLiteral("window-main-master-volume.png")));
    click(d, slider, Qt::RightButton);
    d.check(QStringLiteral("qp1: a right-click resets the master volume to 0 dB"), d.waitFor(3000, [&]() { return std::fabs(sc->masterVolumeDb()) < 0.05; }));

    sc->setUserVolume(user);
    sc->setMasterVolume(master);
    sc->setVolumeMode(startMode);
    d.waitIdle();
    d.check(QStringLiteral("qp1: the volumes are restored"), std::fabs(sc->userVolumeDb() - user) < 0.05 && std::fabs(sc->masterVolumeDb() - master) < 0.05 && device->connected());
}

// -- WP QPI1 -- the sidebar keeps the width it was given (sidebar/width)
void checkWidth(SmokeDriver &d, SidebarController *sc) {
    QQuickItem *bar = d.item(QStringLiteral("Sidebar"));
    const int before = bar ? int(bar->width()) : 0;
    sc->setSidebarWidth(280);
    d.spin(100);
    d.check(QStringLiteral("qpi1: the sidebar takes its stored width"), bar && int(bar->width()) == 280);
    d.check(QStringLiteral("qpi1: and the width is kept under sidebar/width"), QSettings().value(QStringLiteral("sidebar/width")).toInt() == 280);
    sc->setSidebarWidth(before);
    d.spin(100);
}

void checkStatus(SmokeDriver &d, SidebarController *) {
    QObject *cpu = d.item(QStringLiteral("Sidebar.cpu0"));
    d.check(QStringLiteral("qp1: the C0 load shows a percentage"),
            cpu && d.waitFor(3000, [&]() { return shownName(cpu).endsWith(QLatin1Char('%')); }));
    d.check(QStringLiteral("qp1: the C1 load exists"), d.item(QStringLiteral("Sidebar.cpu1")) != nullptr);
    // The lock notice on this line comes from the Link client: qp13-devices checks it on the fake hub
    QObject *text = d.item(QStringLiteral("DeviceStatus.text"));
    d.check(QStringLiteral("qp1: the device line names the device"), text && text->property("text").toString() == d.app().device->deviceTitle());
}

void checkFallback(SmokeDriver &d, SidebarController *sc) {
    DeviceModel *device = d.app().device;
    const QList<int> outs = shownOutputs(d);
    if (outs.size() < 2) return;
    const int ch = outs.at(1);
    const int output = ch - device->state()->num_input_channels;
    sc->select(ch);
    device->write(QStringLiteral("qp1 output off"), [output](FfiCore *c) { return dspi_set_output_enable(c, uint8_t(output), false) >= 0; });
    d.waitIdle();
    const bool off = !device->state()->output_enabled[output];
    d.check(QStringLiteral("qp1: an output switched off leaves its page for the overview"), !off || sc->selectedChannel() == -1);
    device->write(QStringLiteral("qp1 output on"), [output](FfiCore *c) { return dspi_set_output_enable(c, uint8_t(output), true) >= 0; });
    d.waitIdle();
    d.check(QStringLiteral("qp1: the output is back"), device->state()->output_enabled[output]);
    sc->select(-1);
}

void sidebarSection(SmokeDriver &d) {
    auto *sc = Registrar::instance<SidebarController>();
    d.check(QStringLiteral("qp1: the sidebar controller is made"), sc != nullptr);
    if (!sc || !d.app().device->connected()) return;
    d.waitIdle();
    sc->select(-1);
    d.spin(200);
    d.check(QStringLiteral("qp1: screenshot of the sidebar"), d.grab(d.mainWindow(), QStringLiteral("window-main-sidebar.png")));
    checkTapers(d);
    checkRows(d, sc);
    checkSelection(d, sc);
    checkPills(d, sc);
    checkRename(d);
    checkRowMenu(d, sc);
    checkClipboard(d, sc);
    checkIdentify(d, sc);
    checkQuickStrip(d, sc);
    checkPresets(d, sc);
    checkSource(d, sc);
    checkVolume(d, sc);
    checkStatus(d, sc);
    checkWidth(d, sc);
    checkFallback(d, sc);
    d.waitIdle();
}

const SmokeDriver::Section sidebar("qp1-sidebar", 10, &sidebarSection);

} // namespace
