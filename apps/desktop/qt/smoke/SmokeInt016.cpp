// -- INT016 -- The integrator's folds of the 0.16 reports' leftovers, in Modern:
// Measure and Auto-tune fold the outputs nothing is routed to and nothing plays
// on (UA1's C5, the `hidden` role) under one "N unused" row, a ticked one stays;
// Settings › Installer's "No limiter on …" row names the outputs as Modern's
// pages do (unprotectedTextOf, an empty name is the label). (The menu row
// "Agent Activity…", C4, is in assets/actions.json, which lay3b-actions compares.)
#include "SmokeDriver.h"

#include "ChannelListModel.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "ToolsController.h"
#include "TuneController.h"

#include <QQuickItem>
#include <QQuickWindow>
#include <cstdio>

namespace {

QQuickItem *int016Shown(QQuickItem *from, const QString &name) {
    if (!from || !from->isVisible()) return nullptr;
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *f = int016Shown(c, name)) return f;
    return nullptr;
}

// The outputs (from 0) the channel list marks `hidden`: nothing routed, no test signal
QList<int> int016Hidden(DeviceModel *device) {
    QList<int> out;
    ChannelListModel *ch = device->channels();
    for (int r = 0; r < ch->rowCount(); ++r)
        if (ch->get(r, QStringLiteral("kind")).toString() == QLatin1String("output") && ch->get(r, QStringLiteral("hidden")).toBool())
            out << ch->get(r, QStringLiteral("number")).toInt();
    return out;
}

bool int016On(const QVariantList &rows, int output) {
    for (const QVariant &v : rows)
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap().value(QStringLiteral("on")).toBool();
    return false;
}

// One tab's fold: the unticked hidden outputs folded, the row counts them, a click shows them and folds them again
void int016Fold(SmokeDriver &d, const char *tab, QQuickItem *page, const QString &prefix, const QVariantList &rows, const QList<int> &hidden) {
    auto check = [&](const QString &what, bool ok) { d.check(QStringLiteral("int016: %1: %2").arg(QLatin1String(tab), what), ok); };
    QList<int> folded;
    for (int o : hidden)
        if (!int016On(rows, o)) folded << o;
    QQuickItem *row = int016Shown(page, prefix + QStringLiteral(".unused"));
    std::printf("smoke: int016: %s: hidden %d, folded %d, row %p\n", tab, int(hidden.size()), int(folded.size()), (void *)row);
    if (folded.isEmpty()) {
        check(QStringLiteral("no unused row with nothing to fold"), !row);
        return;
    }
    const auto box = [&](int o) { return int016Shown(page, prefix + QStringLiteral(".output.%1").arg(o)); };
    check(QStringLiteral("the unused row counts them"), row && row->property("count").toInt() == folded.size());
    check(QStringLiteral("a folded output has no tick box"), !box(folded.first()));
    SmokeDriver::press(row);
    d.spin(150);
    check(QStringLiteral("a click shows them"), box(folded.first()) != nullptr);
    SmokeDriver::press(int016Shown(page, prefix + QStringLiteral(".unused")));
    d.spin(150);
    check(QStringLiteral("and folds them again"), !box(folded.first()));
}

void int016Section(SmokeDriver &d) {
    auto check = [&](const char *what, bool ok) { d.check(QStringLiteral("int016: ") + QString::fromUtf8(what), ok); };
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo()) device->openDemo();
    check("the demo DSPi is open", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    auto *mc = Registrar::instance<MeasureController>();
    auto *tc = Registrar::instance<TuneController>();
    auto *tools = Registrar::instance<ToolsController>();
    check("the controllers exist", mc && tc && tools);
    if (!mc || !tc || !tools) return;
    check("Modern", d.switchLayout("modern"));
    const QList<int> hidden = int016Hidden(device);

    // ---- Measure and Auto-tune: the unused outputs folded -----------------------------------
    d.goTo(QStringLiteral("measure/measure"), 400);
    QQuickItem *root = d.mainWindow()->contentItem();
    int016Fold(d, "measure", int016Shown(root, QStringLiteral("MeasureSetup.outputs"))
                                 ? int016Shown(root, QStringLiteral("MeasureSetup.outputs"))->parentItem() : root,
               QStringLiteral("MeasureSetup"), mc->setupOutputs(), hidden);
    d.goTo(QStringLiteral("measure/tune"), 400);
    int016Fold(d, "tune", int016Shown(root, QStringLiteral("MeasureTune.page")), QStringLiteral("MeasureTune"), tc->layout(), hidden);

    // ---- Settings › Installer: the unprotected outputs by Modern's names ---------------------
    const QVariantList unprotected = tools->unprotectedOutputs();
    std::printf("smoke: int016: unprotected %d: %s\n", int(unprotected.size()), qPrintable(tools->unprotectedText()));
    if (!unprotected.isEmpty()) {
        const QString named = tools->unprotectedTextOf({ QStringLiteral("Left woofer · 1") });
        check("the row takes the names given", named.startsWith(QStringLiteral("No limiter on Left woofer · 1")));
        check("an output with no name given keeps its label", tools->unprotectedTextOf({}) == tools->unprotectedText());
    }
}

} // namespace

const SmokeDriver::Section int016("int016-folds", 206, &int016Section);
