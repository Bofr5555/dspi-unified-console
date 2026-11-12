// -- WP QP18 -- Polish across the tree: the shortcut table against the Mac's,
// the keyboard rows (presets, mute, bypass), undo, redo and the History
// window, notices instead of success alerts, the demo banner, the Mode entry,
// the host audio format's strips, the Pro layer's components and the channel
// colours as small tags.
#include "SmokeDriver.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DemoController.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "HostAudioFormat.h"
#include "ModeController.h"
#include "NoticeCenter.h"
#include "Poller.h"
#include "PresetModel.h"
#include "Qp18Notices.h"
#include "Registrar.h"
#include "Shell.h"
#include "ShortcutController.h"
#include "SidebarController.h"

#include <QAccessible>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonObject>
#include <QtQml/QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <cmath>

namespace {

// The Mac console's menu shortcuts (DSPi_ConsoleApp.swift's CommandGroups and
// the system's Quit), Command as Ctrl and Option as Alt, then the rows this
// app adds (owner, 2026-11-12). Every row of the table with a shortcut must be
// here, and every entry must be in the table.
const struct {
    const char *id;
    const char *keys;
} kShortcuts[] = {
    // The Mac's
    { "file.importFilters", "Ctrl+I" },
    { "file.exportFilters", "Ctrl+E" },
    { "file.quit", "Ctrl+Q" },
    { "edit.cut", "Ctrl+X" },
    { "edit.copy", "Ctrl+C" },
    { "edit.paste", "Ctrl+V" },
    { "edit.selectAll", "Ctrl+A" },
    { "edit.settings", "Ctrl+," },
    { "autoeq.browse", "Ctrl+Shift+B" },
    { "tools.matrix", "Ctrl+Shift+M" },
    { "tools.loudness", "Ctrl+Shift+L" },
    { "tools.crossfeed", "Ctrl+Shift+X" },
    { "tools.psybass", "Ctrl+Shift+P" },
    { "tools.subharmonic", "Ctrl+Shift+S" },
    { "tools.tube", "Ctrl+Shift+D" },
    { "tools.upmixer", "Ctrl+Shift+U" },
    { "tools.leveller", "Ctrl+Shift+V" },
    { "tools.signalGenerator", "Ctrl+Shift+G" },
    { "tools.spectrum", "Ctrl+Shift+A" },
    { "tools.stats", "Ctrl+Shift+T" },
    { "tools.interruptMonitor", "Ctrl+Shift+I" },
    { "window.minimiseToTray", "Ctrl+Alt+M" },
    // This app's
    { "edit.undo", "Ctrl+Z" },
    { "edit.redo", "Ctrl+Shift+Z" },
    { "edit.history", "Ctrl+Shift+H" },
    { "device.commit", "Ctrl+S" },
    { "device.revert", "Ctrl+Shift+R" },
    { "device.preset1", "Ctrl+1" },
    { "device.preset2", "Ctrl+2" },
    { "device.preset3", "Ctrl+3" },
    { "device.preset4", "Ctrl+4" },
    { "device.preset5", "Ctrl+5" },
    { "device.preset6", "Ctrl+6" },
    { "device.preset7", "Ctrl+7" },
    { "device.preset8", "Ctrl+8" },
    { "device.preset9", "Ctrl+9" },
    { "device.mute", "Ctrl+M" },
    { "device.bypass", "Ctrl+B" },
};

QVariant jsCall(QObject *o, const char *method, const QVariant &a = QVariant(), const QVariant &b = QVariant()) {
    QVariant r;
    if (!o) return r;
    if (b.isValid()) QMetaObject::invokeMethod(o, method, Q_RETURN_ARG(QVariant, r), Q_ARG(QVariant, a), Q_ARG(QVariant, b));
    else if (a.isValid()) QMetaObject::invokeMethod(o, method, Q_RETURN_ARG(QVariant, r), Q_ARG(QVariant, a));
    else QMetaObject::invokeMethod(o, method, Q_RETURN_ARG(QVariant, r));
    if (r.canConvert<QJSValue>()) r = r.value<QJSValue>().toVariant();
    return r;
}

float outputGain(DeviceModel *device, int output) {
    const DspState *s = device->state();
    return s ? s->output_gain_db[output] : NAN;
}

// A QML object of module UConsole made for a test, parented to the main window
QObject *make(SmokeDriver &d, const char *type, const QVariantMap &props = {}) {
    QQmlComponent c(d.app().engine, QStringLiteral("UConsole"), QString::fromLatin1(type));
    QVariantMap p = props;
    p.insert(QStringLiteral("parent"), QVariant::fromValue(d.mainWindow()->contentItem()));
    QObject *o = c.createWithInitialProperties(p);
    if (!o) std::printf("smoke: qp18: %s did not load: %s\n", type, qPrintable(c.errorString()));
    return o;
}

void keyToMain(SmokeDriver &d, int key, Qt::KeyboardModifiers mods) {
    QQuickWindow *w = d.mainWindow();
    w->requestActivate();
    d.waitFor(1000, [&]() { return QGuiApplication::focusWindow() == w; });
    QTest::keyClick(w, Qt::Key(key), mods);
    d.spin(50);
}

void checkShortcutTable(SmokeDriver &d) {
    QObject *table = d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable");
    const QVariantList rows = jsCall(table, "shortcutRows").toList();
    QHash<QString, QString> got;
    QSet<QString> keys;
    bool unique = true;
    for (const QVariant &v : rows) {
        const QVariantMap r = v.toMap();
        const QString id = r.value(QStringLiteral("id")).toString(), k = r.value(QStringLiteral("shortcut")).toString();
        got.insert(id, k);
        if (keys.contains(k)) unique = false;
        keys.insert(k);
    }
    d.check(QStringLiteral("qp18: no two rows share a shortcut"), unique);
    QStringList wrong;
    for (const auto &s : kShortcuts)
        if (got.value(QString::fromLatin1(s.id)) != QLatin1String(s.keys)) wrong << QString::fromLatin1(s.id);
    d.check(QStringLiteral("qp18: every shortcut of the Mac's and of this app is in the table") +
                (wrong.isEmpty() ? QString() : QStringLiteral(" (wrong: ") + wrong.join(QLatin1Char(' ')) + QLatin1Char(')')),
            wrong.isEmpty());
    d.check(QStringLiteral("qp18: the table has no shortcut the list does not know"),
            got.size() == int(sizeof(kShortcuts) / sizeof(kShortcuts[0])));
    for (const auto &s : kShortcuts)
        d.check(QStringLiteral("qp18: menu row %1 exists").arg(QLatin1String(s.id)),
                d.object(QStringLiteral("Menu.") + QLatin1String(s.id)) != nullptr);
}

void checkKeyboardRows(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    ShortcutController *keys = Registrar::instance<ShortcutController>();
    d.check(QStringLiteral("qp18: the keyboard rows have a handler"),
            keys && d.app().shell->handles(QStringLiteral("device.preset1")) && d.app().shell->handles(QStringLiteral("device.mute"))
                && d.app().shell->handles(QStringLiteral("device.bypass")));
    if (!keys) return;

    // Ctrl+M mutes and unmutes the volume
    const bool muted = keys->muted();
    keyToMain(d, Qt::Key_M, Qt::ControlModifier);
    d.check(QStringLiteral("qp18: Ctrl+M mutes"), d.waitFor(3000, [&]() { return keys->muted() != muted; }));
    keyToMain(d, Qt::Key_M, Qt::ControlModifier);
    d.check(QStringLiteral("qp18: Ctrl+M again unmutes"), d.waitFor(3000, [&]() { return keys->muted() == muted; }));

    // Ctrl+B switches the master EQ bypass
    const bool bypassed = keys->bypassed();
    keyToMain(d, Qt::Key_B, Qt::ControlModifier);
    d.check(QStringLiteral("qp18: Ctrl+B switches the bypass"), d.waitFor(3000, [&]() { return keys->bypassed() != bypassed; }));
    keyToMain(d, Qt::Key_B, Qt::ControlModifier);
    d.check(QStringLiteral("qp18: Ctrl+B again switches it back"), d.waitFor(3000, [&]() { return keys->bypassed() == bypassed; }));
    d.waitIdle();

    // Ctrl+2 recalls preset 2, Ctrl+1 preset 1 (no unsaved changes: the
    // bypass went back, so nothing is asked)
    PresetModel *presets = device->presets();
    // The unsaved question, if the preset has changes: discard them
    SidebarController *sidebar = Registrar::instance<SidebarController>();
    int asked = 0;
    const auto answer = QObject::connect(sidebar, &SidebarController::unsavedQuestion, sidebar,
                                         [&](const QString &op, int slot, const QString &) {
                                             ++asked;
                                             sidebar->resolveUnsaved(op, slot, QStringLiteral("alternative"));
                                         });
    const int start = presets->activeSlot();
    // An occupied slot other than the active one (an empty one fails to load)
    int other = -1;
    for (int slot = 0; slot < 9 && other < 0; ++slot)
        if (slot != start && sidebar->presetOccupied(slot)) other = slot;
    // None stored: copy the active one to another slot for the test, cleared after
    bool made = false;
    if (other < 0 && start >= 0) {
        const int slot = start == 1 ? 0 : 1;
        sidebar->requestPresetCopy(slot);
        made = d.waitFor(5000, [&]() { return sidebar->presetOccupied(slot) && !sidebar->presetBusy(); });
        if (made) other = slot;
        d.waitIdle();
    }
    std::printf("smoke: qp18: presets: active %d, recalling %d%s\n", start, other, made ? " (copied for the test)" : "");
    QString reached;
    const auto seen = QObject::connect(d.app().shell, &Shell::triggered, d.app().shell, [&](const QString &id) { reached = id; });
    const int key = other >= 0 ? other : qMax(0, start);
    keyToMain(d, Qt::Key_1 + key, Qt::ControlModifier);
    d.check(QStringLiteral("qp18: Ctrl+%1 reaches Recall Preset > Preset %1").arg(key + 1),
            d.waitFor(2000, [&]() { return reached == QStringLiteral("device.preset%1").arg(key + 1); }));
    QObject::disconnect(seen);
    d.waitIdle();
    if (other < 0) other = start;   // nothing else to recall on this device
    const bool switched = d.waitFor(5000, [&]() { return presets->activeSlot() == other; });
    d.check(QStringLiteral("qp18: Ctrl+%1 recalls preset %1").arg(other + 1), switched);
    if (switched && start >= 0) {
        keyToMain(d, Qt::Key_1 + start, Qt::ControlModifier);
        d.check(QStringLiteral("qp18: Ctrl+%1 recalls preset %1 again").arg(start + 1),
                d.waitFor(5000, [&]() { return presets->activeSlot() == start; }));
    }
    if (made) {
        sidebar->clearPreset(other);
        d.waitFor(5000, [&]() { return !sidebar->presetOccupied(other) && !sidebar->presetBusy(); });
    }
    QObject::disconnect(answer);
    std::printf("smoke: qp18: the preset keys asked about unsaved changes %d times\n", asked);
    d.waitIdle();
}

void checkUndo(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    HistoryController *history = Registrar::instance<HistoryController>();
    d.check(QStringLiteral("qp18: the history controller is made"), history != nullptr);
    if (!history) return;
    const float start = outputGain(device, 0);
    const float target = start > -5.0f ? -7.5f : -3.0f;
    device->write(QStringLiteral("output gain 0"), [target](FfiCore *c) { return dspi_set_output_gain(c, 0, target); });
    d.waitIdle();
    d.check(QStringLiteral("qp18: a change can be undone"), d.waitFor(3000, [&]() { return history->canUndo(); }));
    const QVariantList entries = history->entries();
    const QVariantMap newest = entries.isEmpty() ? QVariantMap() : entries.first().toMap();
    d.check(QStringLiteral("qp18: the history lists it in the glossary's words"),
            newest.value(QStringLiteral("title")).toString() == QLatin1String("Output gain, OUT1")
                && newest.value(QStringLiteral("detail")).toString().endsWith(QStringLiteral(" dB")));
    QObject *undoRow = d.object(QStringLiteral("Menu.edit.undo"));
    QObject *undoAction = undoRow ? undoRow->property("action").value<QObject *>() : nullptr;
    d.check(QStringLiteral("qp18: Edit > Undo is enabled"), undoAction && undoAction->property("enabled").toBool());

    d.app().shell->trigger(QStringLiteral("edit.undo"));
    d.check(QStringLiteral("qp18: Undo puts the gain back"), d.waitFor(3000, [&]() { return outputGain(device, 0) == start; }));
    d.check(QStringLiteral("qp18: then Redo can"), d.waitFor(3000, [&]() { return history->canRedo(); }));
    keyToMain(d, Qt::Key_Z, Qt::ControlModifier | Qt::ShiftModifier);
    d.check(QStringLiteral("qp18: Ctrl+Shift+Z redoes it"), d.waitFor(3000, [&]() { return outputGain(device, 0) == target; }));
    keyToMain(d, Qt::Key_Z, Qt::ControlModifier);
    d.check(QStringLiteral("qp18: Ctrl+Z undoes it"), d.waitFor(3000, [&]() { return outputGain(device, 0) == start; }));
    d.waitIdle();

    // The History window
    QWindow *window = d.app().shell->openWindow(QStringLiteral("edit.history"));
    auto *historyWindow = qobject_cast<QQuickWindow *>(window);
    d.check(QStringLiteral("qp18: Edit > History opens the window"), historyWindow && historyWindow->isVisible());
    if (historyWindow) {
        QObject *list = d.item(QStringLiteral("HistoryWindow.list"), historyWindow->contentItem());
        d.check(QStringLiteral("qp18: it lists the changes"), d.waitFor(3000, [&]() { return list && list->property("count").toInt() >= 2; }));
        std::printf("smoke: qp18: the History window lists %d changes (%lld in the controller)\n",
                    list ? list->property("count").toInt() : -1, qlonglong(history->entries().size()));
        d.spin(200);
        d.check(QStringLiteral("qp18: screenshot of the History window"), d.grab(historyWindow, QStringLiteral("window-history.png")));
        historyWindow->close();
    }

    // The journal's paths in words
    d.check(QStringLiteral("qp18: describe() names a crosspoint"),
            HistoryController::describe(QJsonObject{ { QStringLiteral("path"), QStringLiteral("mix") },
                                                     { QStringLiteral("indices"), QJsonArray{ 0, 2 } } })
                    .value(QStringLiteral("title")).toString() == QLatin1String("Crosspoint, IN1 to OUT3"));
    d.check(QStringLiteral("qp18: describe() keeps a path it has no words for"),
            HistoryController::describe(QJsonObject{ { QStringLiteral("path"), QStringLiteral("raw") } })
                    .value(QStringLiteral("title")).toString() == QLatin1String("raw"));
}

void checkNotices(SmokeDriver &d) {
    NoticeCenter *notices = Registrar::instance<NoticeCenter>();
    QQuickItem *banner = d.item(QStringLiteral("NoticeBanner"));
    d.check(QStringLiteral("qp18: the notice banner is in the stack"), notices && banner);
    if (!notices || !banner) return;
    notices->clear();
    d.check(QStringLiteral("qp18: no notice, no banner"), !banner->isVisible());
    notices->post(QStringLiteral("Filters exported successfully"));
    d.check(QStringLiteral("qp18: a notice shows in the banner stack"),
            d.waitFor(1000, [&]() { return banner->isVisible(); }) && banner->property("text").toString() == QLatin1String("Filters exported successfully"));
    QAccessibleInterface *a = QAccessible::queryAccessibleInterface(banner);
    d.check(QStringLiteral("qp18: it is announced as an alert"), a && a->role() == QAccessible::AlertMessage);
    d.check(QStringLiteral("qp18: an info notice goes by itself"), notices->current().value(QStringLiteral("kind")) == QLatin1String("info"));
    notices->post(QStringLiteral("A second one"), QStringLiteral("warning"));
    d.check(QStringLiteral("qp18: a second notice waits its turn"), notices->waiting() == 1);
    d.grab(d.mainWindow(), QStringLiteral("window-notice.png"));
    notices->dismiss();
    d.check(QStringLiteral("qp18: dismissing shows the next"),
            notices->current().value(QStringLiteral("kind")) == QLatin1String("warning") && banner->isVisible());
    notices->dismiss();
    d.check(QStringLiteral("qp18: then the banner goes"), d.waitFor(1000, [&]() { return !banner->isVisible(); }));

    // A success through the file host's alert is a notice now, not a modal
    QObject *dialog = d.object(QStringLiteral("FilesHost.notice"));
    qp18NoticeMark();
    if (dialog) QMetaObject::invokeMethod(dialog, "show", Q_ARG(QVariant, QStringLiteral("Success")),
                                          Q_ARG(QVariant, QStringLiteral("Preset saved successfully")), Q_ARG(QVariant, false));
    d.check(QStringLiteral("qp18: a success opens no modal alert"), dialog && !dialog->property("opened").toBool() && qp18NoticeNew());
    d.check(QStringLiteral("qp18: it becomes a notice"), qp18TakeNotice() == QLatin1String("Preset saved successfully"));
    if (dialog) QMetaObject::invokeMethod(dialog, "show", Q_ARG(QVariant, QStringLiteral("Error")),
                                          Q_ARG(QVariant, QStringLiteral("Failed to load parameters")), Q_ARG(QVariant, true));
    d.check(QStringLiteral("qp18: an error is still an alert to read"), d.waitFor(2000, [&]() { return dialog && dialog->property("opened").toBool(); }));
    if (dialog) QMetaObject::invokeMethod(dialog, "close");
    d.waitFor(2000, [&]() { return !dialog || !dialog->property("opened").toBool(); });
}

void checkDemo(SmokeDriver &d) {
    DemoController *demo = Registrar::instance<DemoController>();
    QQuickItem *banner = d.item(QStringLiteral("DemoBanner"));
    d.check(QStringLiteral("qp18: the demo banner shows on the demo"),
            demo && banner && demo->demoOpen() && !demo->nothingConnected() && banner->isVisible());
    QQuickItem *open = d.item(QStringLiteral("DemoBanner.open"));
    d.check(QStringLiteral("qp18: it has no button then"), open && !open->isVisible());
    d.check(QStringLiteral("qp18: screenshot of the demo banner"), d.grab(d.mainWindow(), QStringLiteral("window-demo-banner.png")));
}

void checkMode(SmokeDriver &d) {
    ModeController *mode = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("qp18: a desktop opens in Advanced"), mode && mode->mode() == QLatin1String("advanced"));
    if (!mode) return;
    const int setters = device->settersRun();
    QWindow *window = d.app().shell->openWindow(QStringLiteral("window.mode"));
    auto *w = qobject_cast<QQuickWindow *>(window);
    d.check(QStringLiteral("qp18: Window > Mode... opens the choice"), w && w->isVisible());
    mode->choose(QStringLiteral("simple"), false);
    d.check(QStringLiteral("qp18: Simple, remembered for this device"),
            mode->mode() == QLatin1String("simple") && mode->rememberedForDevice()
                && QSettings().value(QStringLiteral("mode/device/") + device->serial()).toString() == QLatin1String("simple"));
    if (w) {
        d.spin(150);
        d.check(QStringLiteral("qp18: screenshot of the Mode window"), d.grab(w, QStringLiteral("window-mode.png")));
    }
    mode->choose(QStringLiteral("advanced"), true);
    d.check(QStringLiteral("qp18: Advanced for every device forgets the device's own"),
            mode->mode() == QLatin1String("advanced") && !mode->rememberedForDevice()
                && QSettings().value(QStringLiteral("mode/default")).toString() == QLatin1String("advanced"));
    mode->choose(QStringLiteral("installer"), true);
    d.check(QStringLiteral("qp18: no other mode is taken (no PIN, no Installer yet)"), mode->mode() == QLatin1String("advanced"));
    d.check(QStringLiteral("qp18: switching wrote nothing to the device"), device->settersRun() == setters);
    if (w) w->close();
    QSettings().remove(QStringLiteral("mode"));
}

void checkHostFormat(SmokeDriver &d) {
    using namespace HostAudio;
    d.check(QStringLiteral("qp18: a WAVEFORMATEX's channel count"),
            waveFormatChannels("\xfe\xff\x08\x00", 4) == 8 && waveFormatChannels("\x01\x00", 2) == 0);
    d.check(QStringLiteral("qp18: the serial above a Windows endpoint"),
            usbSerialFromInstanceIds({ QStringLiteral("SWD\\MMDEVAPI\\{0.0.0}"), QStringLiteral("USB\\VID_2E8A&PID_FEAA&MI_00\\7&1&0"),
                                       QStringLiteral("USB\\VID_2E8A&PID_FEAA\\DSPI0001") }) == QLatin1String("DSPI0001")
                && usbSerialFromInstanceIds({ QStringLiteral("USB\\VID_2E8A&PID_FEAA\\5&2B&0&3") }).isEmpty());
    std::vector<Endpoint> eps{ { QStringLiteral("Speakers Realtek"), 2, QString() },
                               { QStringLiteral("Speakers DSPi"), 8, QStringLiteral("DSPI0001") },
                               { QStringLiteral("Speakers DSPi"), 4, QStringLiteral("DSPI0002") } };
    d.check(QStringLiteral("qp18: the open board's endpoint by its serial"),
            channelsFor(eps, QStringLiteral("dspi0002")) == 4 && channelsFor(eps, QStringLiteral("NONE")) == 0);
    d.check(QStringLiteral("qp18: by name when no serial is known, if it is the only one"),
            channelsFor({ { QStringLiteral("Out DSPi"), 6, QString() } }, QString()) == 6
                && channelsFor({ { QStringLiteral("DSPi A"), 6, QString() }, { QStringLiteral("DSPi B"), 4, QString() } }, QString()) == 0);

    HostAudioFormat *host = Registrar::instance<HostAudioFormat>();
    DeviceModel *device = d.app().device;
    const DspState *s = device->state();
    const bool usb = s && s->input_source == 0;
    d.check(QStringLiteral("qp18: the demo gives no OS format"), host && host->channels() == 0);
    if (!host || !usb) return;
    const int before = device->channels()->inputCount();
    host->setOverride(4);
    d.check(QStringLiteral("qp18: an OS format of 4 channels shows 4 USB inputs"),
            d.waitFor(5000, [&]() { return device->channels()->inputCount() == 4; }));
    d.check(QStringLiteral("qp18: screenshot with 4 USB inputs"), d.grab(d.mainWindow(), QStringLiteral("hostaudio-4-strips.png")));
    host->setOverride(0);
    d.check(QStringLiteral("qp18: no answer from the OS: the device's count again"),
            d.waitFor(5000, [&]() { return device->channels()->inputCount() == before; }));
    d.waitIdle();
}

void checkComponents(SmokeDriver &d) {
    // NumericField: the glossary's units, typed and relative entry
    QObject *f = make(d, "NumericField", { { QStringLiteral("unit"), QStringLiteral("Hz") }, { QStringLiteral("from"), 10 },
                                           { QStringLiteral("to"), 24000 }, { QStringLiteral("decimals"), 0 } });
    auto parsed = [&](const QString &text, double current) {
        const QVariantMap r = jsCall(f, "parse", text, current).toMap();
        return r.value(QStringLiteral("ok")).toBool() ? r.value(QStringLiteral("value")).toDouble() : NAN;
    };
    d.check(QStringLiteral("qp18: NumericField reads Hz, kHz and k"),
            parsed(QStringLiteral("120"), 0) == 120 && parsed(QStringLiteral("2.5 kHz"), 0) == 2500 && parsed(QStringLiteral("2k"), 0) == 2000
                && parsed(QStringLiteral("440hz"), 0) == 440);
    d.check(QStringLiteral("qp18: NumericField takes a change of the value"),
            parsed(QStringLiteral("+100"), 1000) == 1100 && parsed(QStringLiteral("-=50"), 1000) == 950);
    d.check(QStringLiteral("qp18: NumericField refuses another unit and nonsense"),
            std::isnan(parsed(QStringLiteral("3 dB"), 0)) && std::isnan(parsed(QStringLiteral("abc"), 0)));
    const QVariantMap held = jsCall(f, "parse", QStringLiteral("30000"), 0).toMap();
    d.check(QStringLiteral("qp18: NumericField holds a value to its range and says so"),
            held.value(QStringLiteral("value")).toDouble() == 24000 && held.value(QStringLiteral("limited")).toBool());
    d.check(QStringLiteral("qp18: NumericField writes kHz from 1000 up"), jsCall(f, "format", 2500).toString() == QLatin1String("2.50 kHz"));
    delete f;
    QObject *g = make(d, "NumericField", { { QStringLiteral("unit"), QStringLiteral("dB") }, { QStringLiteral("from"), -24 },
                                           { QStringLiteral("to"), 24 }, { QStringLiteral("decimals"), 1 } });
    const QVariantMap neg = jsCall(g, "parse", QStringLiteral("-3"), 2).toMap();
    const QVariantMap rel = jsCall(g, "parse", QStringLiteral("+=1.5"), 2).toMap();
    const QVariantMap comma = jsCall(g, "parse", QStringLiteral("-6,5 dBFS"), 0).toMap();
    d.check(QStringLiteral("qp18: NumericField: a gain's -3 is the gain, += changes it"),
            neg.value(QStringLiteral("value")).toDouble() == -3 && rel.value(QStringLiteral("value")).toDouble() == 3.5
                && comma.value(QStringLiteral("value")).toDouble() == -6.5);
    auto *gi = qobject_cast<QQuickItem *>(g);
    QAccessibleInterface *ga = gi ? QAccessible::queryAccessibleInterface(gi) : nullptr;
    d.check(QStringLiteral("qp18: NumericField is a spin box to assistive technology"), ga && ga->role() == QAccessible::SpinBox);
    delete g;

    // LevelMeter: the core's hold line and clip hold, the scale in dBFS
    auto *m = qobject_cast<QQuickItem *>(make(d, "LevelMeter", { { QStringLiteral("level"), 0.5 }, { QStringLiteral("hold"), 0.9 },
                                                                 { QStringLiteral("clipped"), true }, { QStringLiteral("width"), 200 },
                                                                 { QStringLiteral("height"), 8 } }));
    QQuickItem *holdLine = m ? m->findChild<QQuickItem *>(QStringLiteral("LevelMeter.hold")) : nullptr;
    QQuickItem *clip = m ? m->findChild<QQuickItem *>(QStringLiteral("LevelMeter.clip")) : nullptr;
    d.check(QStringLiteral("qp18: LevelMeter shows the hold line and the clip latch"),
            holdLine && holdLine->isVisible() && clip && clip->isVisible() && holdLine->x() > 150);
    d.check(QStringLiteral("qp18: LevelMeter: -6 dBFS sits at 90 % of a 60 dB scale"),
            m && std::abs(m->property("filled").toDouble() - (60 - 20 * std::log10(2.0)) / 60) < 1e-6);
    QAccessibleInterface *ma = m ? QAccessible::queryAccessibleInterface(m) : nullptr;
    d.check(QStringLiteral("qp18: LevelMeter reads its level out"), ma && ma->role() == QAccessible::ProgressBar
                && ma->text(QAccessible::Description).contains(QStringLiteral("dBFS")));
    if (m) m->setProperty("channel", 0);
    d.check(QStringLiteral("qp18: LevelMeter follows the meter poll's hold"),
            d.waitFor(2000, [&]() { return m && m->property("hold").toDouble() >= 0 && !Poller::instance()->peakHold().isEmpty(); }));
    delete m;

    // CustomSlider (QD41): Up and Down step a horizontal slider and commit
    auto *slider = qobject_cast<QQuickItem *>(make(d, "CustomSlider", { { QStringLiteral("from"), 0 }, { QStringLiteral("to"), 10 },
                                                                        { QStringLiteral("stepSize"), 1 }, { QStringLiteral("value"), 5 },
                                                                        { QStringLiteral("label"), QStringLiteral("Test") } }));
    if (slider) {
        QSignalSpy commits(slider, SIGNAL(committed(double)));
        d.mainWindow()->requestActivate();
        d.waitFor(1000, [&]() { return QGuiApplication::focusWindow() == d.mainWindow(); });
        slider->forceActiveFocus();
        QTest::keyClick(d.mainWindow(), Qt::Key_Up);
        QTest::keyClick(d.mainWindow(), Qt::Key_Up);
        QTest::keyClick(d.mainWindow(), Qt::Key_Down);
        d.check(QStringLiteral("qp18: Up and Down step a horizontal slider, each step committed"),
                slider->property("value").toDouble() == 6 && commits.count() == 3);
        QAccessibleInterface *sa = QAccessible::queryAccessibleInterface(slider);
        QAccessibleActionInterface *act = sa ? sa->actionInterface() : nullptr;
        if (act) act->doAction(QAccessibleActionInterface::increaseAction());
        d.check(QStringLiteral("qp18: the accessibility increase steps and commits"),
                act && slider->property("value").toDouble() == 7 && commits.count() == 4);
        delete slider;
    }

    // StatusBar
    auto *bar = qobject_cast<QQuickItem *>(make(d, "StatusBar", { { QStringLiteral("connectionText"), QStringLiteral("Demo DSPi") },
                                                                  { QStringLiteral("connectionState"), QStringLiteral("online") },
                                                                  { QStringLiteral("modeText"), QStringLiteral("Advanced") } }));
    QAccessibleInterface *ba = bar ? QAccessible::queryAccessibleInterface(bar) : nullptr;
    d.check(QStringLiteral("qp18: StatusBar is a status bar with its slots"),
            ba && ba->role() == QAccessible::StatusBar && bar->findChild<QObject *>(QStringLiteral("StatusBar.mode")));
    delete bar;
}

void checkChannelTags(SmokeDriver &d) {
    QObject *theme = d.app().engine->singletonInstance<QObject *>("UConsole", "Theme");
    const QColor level = theme ? theme->property("meterLevel").value<QColor>() : QColor();
    // The sidebar's meter fill and the dashboard's meter: one level colour
    bool sidebar = false;
    if (QQuickItem *meter = d.item(QStringLiteral("Sidebar.meter.0")))
        for (QQuickItem *child : meter->childItems())
            if (child->property("color").value<QColor>() == level) sidebar = true;
    d.check(QStringLiteral("qp18: the sidebar's meters are in the level colour, not the channel's"), level.isValid() && sidebar);
    QQuickItem *pill = d.item(QStringLiteral("Sidebar.pill.0"));
    d.check(QStringLiteral("qp18: the channel's colour stays on its pill"), pill && pill->property("tint").value<QColor>() != level);
}

void polishSection(SmokeDriver &d) {
    d.waitIdle();
    // A question an earlier section left open would hold the keys: say which, and close it
    QStringList open;
    for (QObject *o : d.mainWindow()->findChildren<QObject *>())
        if (o->inherits("QQuickPopup") && o->property("opened").toBool()) {
            open << o->objectName();
            QMetaObject::invokeMethod(o, "close");
        }
    if (!open.isEmpty()) std::printf("smoke: qp18: closed what was left open: %s\n", qPrintable(open.join(QStringLiteral(", "))));
    checkShortcutTable(d);
    checkDemo(d);
    checkNotices(d);
    checkComponents(d);
    checkChannelTags(d);
    checkMode(d);
    checkUndo(d);
    checkKeyboardRows(d);
    checkHostFormat(d);
    d.mainWindow()->requestActivate();
}

} // namespace

static const SmokeDriver::Section qp18Polish("qp18-polish", 180, &polishSection);
