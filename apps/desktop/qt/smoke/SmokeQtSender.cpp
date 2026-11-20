// -- WP QTSENDER -- The alink sender in the main window (owner, 2026-11-19:
// "do the controls actually do anything useful with a sender connected? If
// no, grey them out and put up a banner"). On the legacy sender (fake
// generation 6) and the dual-stack sender (8, paged 1.1.7 and 1.1.6): the
// banner shows; every menu row outside DeviceInfoController::senderKeeps is
// greyed (not hidden) and the kept rows are not; a greyed row's shortcut and
// Shell.trigger do nothing; a DSP tool window opened before is greyed under its
// banner; Settings greys its System and Control pages; the update window
// greys Export Configuration; the sender gets no request it must not get.
// Back on the demo DSP all of it is undone.
#include "SmokeDriver.h"

#include "BootloaderLocator.h"
#include "Core.h"
#include "DeviceInfoController.h"
#include "DeviceModel.h"
#include "FirmwareController.h"
#include "IdentifyController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QJSValue>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <atomic>
#include <memory>

namespace {

QStringList menuIds(SmokeDriver &d) {
    QObject *table = d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable");
    QVariant ids;
    if (table) QMetaObject::invokeMethod(table, "allIds", Q_RETURN_ARG(QVariant, ids));
    if (ids.canConvert<QJSValue>()) ids = ids.value<QJSValue>().toVariant();
    return ids.toStringList();
}

// The enabled state of menu row `id`'s action; -1 when the row is missing
int rowEnabled(SmokeDriver &d, const QString &id) {
    QObject *row = d.object(QStringLiteral("Menu.") + id);
    QObject *action = row ? row->property("action").value<QObject *>() : nullptr;
    return action ? int(action->property("enabled").toBool()) : -1;
}

// The Edit rows that act on a focused text field first: they stay
const QStringList kTextEdits{ QStringLiteral("edit.cut"), QStringLiteral("edit.copy"), QStringLiteral("edit.paste"),
                              QStringLiteral("edit.delete"), QStringLiteral("edit.selectAll") };

bool open(SmokeDriver &d, DeviceInfoController *info, int code) {
    auto ok = std::make_shared<std::atomic<bool>>(false);
    d.app().core->worker()->post([code, ok](FfiCore *c) { ok->store(dspi_demo_open_generation(c, uint8_t(code))); });
    d.app().device->refresh();
    const bool in = d.waitFor(6000, [&]() { return info->known() && info->sender(); });
    d.waitIdle();
    d.spin(100);
    return ok->load() && in;
}

uint32_t violations(SmokeDriver &d) {
    auto n = std::make_shared<std::atomic<uint32_t>>(UINT32_MAX);
    d.app().core->worker()->post([n](FfiCore *c) { n->store(dspi_demo_generation_violations(c)); });
    d.waitIdle();
    return n->load();
}

QQuickItem *visibleItem(QQuickItem *scope, const QString &name) {
    if (!scope || !scope->isVisible()) return nullptr;
    if (scope->objectName() == name) return scope;
    for (QQuickItem *child : scope->childItems())
        if (QQuickItem *found = visibleItem(child, name)) return found;
    return nullptr;
}

QVariantMap settingsRow(SettingsController *s, const QString &id) {
    for (const QVariant &v : s->sidebarRows(QString())) {
        const QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("id")).toString() == id) return m;
    }
    return {};
}

void onSender(SmokeDriver &d, DeviceInfoController *info, QWindow *matrix, int code, const QString &name,
              const char *generation) {
    Shell *shell = d.app().shell;
    d.check(QStringLiteral("qtsender: %1 opens as the sender").arg(name), open(d, info, code));
    d.check(QStringLiteral("qtsender: %1: generation %2").arg(name, QLatin1String(generation)),
            info->generation() == QLatin1String(generation));
    auto *settingsNow = Registrar::instance<SettingsController>();
    d.check(QStringLiteral("qtsender: %1: Settings is on no System page").arg(name),
            settingsNow && d.waitFor(1000, [&]() { return settingsNow->currentPage() == QLatin1String("about"); }));

    // ---- The banner and the sender's page ----
    QQuickItem *banner = d.item(QStringLiteral("SenderBanner"));
    d.check(QStringLiteral("qtsender: %1: the banner shows, in words").arg(name),
            d.waitFor(2000, [&]() { return banner && banner->isVisible() && banner->height() > 0; }) &&
                banner->property("text").toString().contains(QLatin1String("It has no DSP")));
    QQuickItem *senderPage = d.item(QStringLiteral("SenderPage"));
    QQuickItem *rightPane = d.item(QStringLiteral("RightPane"));
    d.check(QStringLiteral("qtsender: %1: the sender's page, not the DSP screens").arg(name),
            senderPage && senderPage->isVisible() && rightPane && !rightPane->isVisible());

    // ---- Every menu row: greyed, not hidden, unless it does something ----
    const QStringList ids = menuIds(d);
    QStringList greyed, kept, wrong;
    for (const QString &id : ids) {
        const int on = rowEnabled(d, id);
        const bool keeps = info->senderKeeps(id) || kTextEdits.contains(id);
        if (on == 1) kept << id;
        else greyed << id;
        // Identify follows its own availability (whether the sender answers
        // EXT_IDENTIFY), Clear Favorites whether there is a favourite to clear
        if (id == QLatin1String("device.identify") || id == QLatin1String("autoeq.clearFavorites")) continue;
        if (on == -1 || bool(on) != keeps) wrong << id;
    }
    qInfo("smoke: qtsender %s kept: %s", qPrintable(name), qPrintable(kept.join(QLatin1Char(' '))));
    qInfo("smoke: qtsender %s greyed: %s", qPrintable(name), qPrintable(greyed.join(QLatin1Char(' '))));
    d.check(QStringLiteral("qtsender: %1: every DSP row is greyed and every useful row is not (wrong: %2)")
                .arg(name, wrong.join(QLatin1Char(' '))),
            ids.size() >= 42 && wrong.isEmpty());
    // Greyed, not hidden: the gate never touches a row's visible (Menus.qml)
    d.check(QStringLiteral("qtsender: %1: the greyed rows are still in the menu").arg(name),
            rowEnabled(d, QStringLiteral("tools.matrix")) == 0 && rowEnabled(d, QStringLiteral("device.commit")) == 0);
    auto *identify = Registrar::instance<IdentifyController>();
    d.check(QStringLiteral("qtsender: %1: Identify follows whether the sender has it").arg(name),
            identify && rowEnabled(d, QStringLiteral("device.identify")) == int(identify->available()));
    QObject *favourite = d.object(QStringLiteral("Menu.autoeq.favorite.0"));
    d.check(QStringLiteral("qtsender: %1: no AutoEQ favourite applies").arg(name),
            !favourite || !favourite->property("enabled").toBool());

    // ---- A greyed row does nothing, by shortcut or by trigger ----
    QStringList reached;
    const auto seen = QObject::connect(shell, &Shell::triggered, shell, [&](const QString &id) { reached << id; });
    QQuickWindow *w = d.mainWindow();
    w->requestActivate();
    d.waitFor(1000, [&]() { return QGuiApplication::focusWindow() == w; });
    QTest::keyClick(w, Qt::Key_M, Qt::ControlModifier | Qt::ShiftModifier);   // tools.matrix
    QTest::keyClick(w, Qt::Key_S, Qt::ControlModifier);                        // device.commit
    QTest::keyClick(w, Qt::Key_1, Qt::ControlModifier);                        // device.preset1
    QTest::keyClick(w, Qt::Key_M, Qt::ControlModifier);                        // device.mute
    d.spin(200);
    shell->trigger(QStringLiteral("tools.signalGenerator"));
    shell->trigger(QStringLiteral("device.factoryReset"));
    d.spin(100);
    d.check(QStringLiteral("qtsender: %1: a greyed row's shortcut and trigger reach nothing (%2)")
                .arg(name, reached.join(QLatin1Char(' '))),
            reached.isEmpty() && !shell->allows(QStringLiteral("tools.matrix")) &&
                shell->allows(QStringLiteral("device.firmwareUpdate")));

    // ---- A DSP window opened before is greyed under its banner ----
    QQuickWindow *mw = qobject_cast<QQuickWindow *>(matrix);
    // The window's root holds the header; ApplicationWindow.contentItem the content
    QQuickItem *mBanner = mw ? d.item(QStringLiteral("ToolWindow.senderBanner"), mw->contentItem()) : nullptr;
    QQuickItem *mContent = mw ? mw->property("contentItem").value<QQuickItem *>() : nullptr;
    d.check(QStringLiteral("qtsender: %1: the open matrix window is greyed under its banner").arg(name),
            mw && mw->property("senderLocked").toBool() && mw->property("actionId").toString() == QLatin1String("tools.matrix") &&
                mContent && !mContent->isEnabled() && mBanner && mBanner->isVisible());

    // ---- Settings: System and Control greyed, About opens ----
    auto *settings = Registrar::instance<SettingsController>();
    shell->trigger(QStringLiteral("edit.settings"));
    auto *sw = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qtsender: %1: Settings opens").arg(name),
            reached.contains(QLatin1String("edit.settings")) && sw && d.waitFor(3000, [&]() { return sw->isVisible(); }));
    if (settings && sw) {
        settings->open(QStringLiteral("about"));
        settings->open(QStringLiteral("outputs"));
        // The sidebar's delegates are made again for the sender's rows; the
        // ones made before go at the next deferred delete
        d.waitFor(2000, [&]() {
            QQuickItem *b = visibleItem(sw->contentItem(), QStringLiteral("Settings.button.outputs"));
            return b && !b->isEnabled();
        });
        const QVariantMap outputs = settingsRow(settings, QStringLiteral("outputs"));
        const QVariantMap about = settingsRow(settings, QStringLiteral("about"));
        QQuickItem *outputsButton = visibleItem(sw->contentItem(), QStringLiteral("Settings.button.outputs"));
        qInfo("smoke: qtsender settings: outputs row %s device page %d, about device page %d, page %s, button %d enabled %d",
              outputs.isEmpty() ? "missing" : "found", int(outputs.value(QStringLiteral("devicePage")).toBool()),
              int(about.value(QStringLiteral("devicePage")).toBool()), qPrintable(settings->currentPage()), int(outputsButton != nullptr),
              int(outputsButton && outputsButton->isEnabled()));
        d.check(QStringLiteral("qtsender: %1: Settings shows System greyed and does not open it").arg(name),
                outputs.value(QStringLiteral("devicePage")).toBool() &&
                    !about.value(QStringLiteral("devicePage")).toBool() && settings->currentPage() == QLatin1String("about") &&
                    outputsButton && !outputsButton->isEnabled());
        d.check(QStringLiteral("qtsender: %1: screenshot of Settings").arg(name),
                d.grab(sw, QStringLiteral("qtsender-settings-%1.png").arg(code)));
        sw->close();
    }

    // ---- The update window: works, Export Configuration greyed ----
    auto *fw = Registrar::instance<FirmwareController>();
    if (fw) {
        fw->locator()->setScanForTest([]() { return BootloaderLocator::Scan{}; });
        fw->resetRun();
    }
    reached.clear();
    shell->trigger(QStringLiteral("device.firmwareUpdate"));
    auto *fwWindow = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.firmwareUpdate")));
    d.waitFor(3000, [&]() { return fwWindow && fwWindow->isVisible(); });
    d.waitIdle();
    QQuickItem *exportButton = fwWindow ? visibleItem(fwWindow->contentItem(), QStringLiteral("FirmwareWindow.export")) : nullptr;
    QQuickItem *fwContent = fwWindow ? fwWindow->property("contentItem").value<QQuickItem *>() : nullptr;
    d.check(QStringLiteral("qtsender: %1: Firmware Update opens, not greyed, Export Configuration greyed").arg(name),
            reached.contains(QLatin1String("device.firmwareUpdate")) && fwWindow && !fwWindow->property("senderLocked").toBool() &&
                fwContent && fwContent->isEnabled() && exportButton && !exportButton->isEnabled());
    if (fw) fw->resetRun();
    if (fwWindow) fwWindow->close();
    QObject::disconnect(seen);
    d.spin(50);

    d.check(QStringLiteral("qtsender: %1: screenshot of the main window with the banner").arg(name),
            d.grab(d.mainWindow(), QStringLiteral("qtsender-main-%1.png").arg(code)));
    d.check(QStringLiteral("qtsender: %1: no request the sender must not get").arg(name), violations(d) == 0);
}

void senderSection(SmokeDriver &d) {
    auto *info = Registrar::instance<DeviceInfoController>();
    d.check(QStringLiteral("qtsender: the device info controller is registered"), info != nullptr);
    if (!info) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.waitFor(3000, [&]() { return device->connected() && info->known() && !info->sender(); });
    d.waitIdle();
    QQuickItem *banner = d.item(QStringLiteral("SenderBanner"));
    d.check(QStringLiteral("qtsender: the demo DSP: no banner, the matrix row enabled"),
            banner && !banner->isVisible() && rowEnabled(d, QStringLiteral("tools.matrix")) == 1);
    QWindow *matrix = d.app().shell->openWindow(QStringLiteral("tools.matrix"));
    d.check(QStringLiteral("qtsender: the matrix window opens on the demo, not greyed"),
            matrix && d.waitFor(2000, [&]() { return matrix->isVisible(); }) && !matrix->property("senderLocked").toBool());

    // Settings left on a System page: the sender's connect leaves it
    auto *settings = Registrar::instance<SettingsController>();
    if (settings) settings->open(QStringLiteral("outputs"));
    d.check(QStringLiteral("qtsender: the demo DSP: Settings opens a System page"),
            settings && settings->currentPage() == QLatin1String("outputs"));
    onSender(d, info, matrix, 6, QStringLiteral("legacy sender"), "legacy");
    onSender(d, info, matrix, 8, QStringLiteral("dual-stack sender"), "paged");

    if (auto *fw = Registrar::instance<FirmwareController>()) fw->locator()->setScanForTest(nullptr);
    device->openDemo();
    d.waitFor(3000, [&]() { return device->connected() && info->known() && !info->sender(); });
    d.waitIdle();
    d.check(QStringLiteral("qtsender: back on the demo: no banner, rows and the matrix window enabled again"),
            d.waitFor(2000, [&]() { return banner && !banner->isVisible(); }) && rowEnabled(d, QStringLiteral("tools.matrix")) == 1 &&
                rowEnabled(d, QStringLiteral("device.commit")) == 1 && d.app().shell->allows(QStringLiteral("tools.matrix")) &&
                matrix && !matrix->property("senderLocked").toBool());
    if (matrix) matrix->close();
    d.spin(50);
}

} // namespace

static const SmokeDriver::Section qtSender("qtsender", 216, &senderSection);
