// -- WP QP0 -- The base's section: the app starts on the demo device, the
// sidebar shows the state's channel names, a write goes through the write path
// and comes back, a rename made elsewhere arrives as a notification, the menu
// table and its shortcuts reach the shell, and the plug-in points (controller,
// banner, tool window, action handler) work. Screenshots in both schemes.
#include "SmokeDriver.h"
#include "SmokeProbe.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "DeviceModel.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QImage>
#include <QJSValue>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <cstdio>

// ---- The probe (SmokeProbe.h) ----------------------------------------------

SmokeProbe::SmokeProbe(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_message(QStringLiteral("Smoke probe banner")) {
    ctx.shell->addHandler(QStringLiteral("smoke.action"), this, [this]() { ++m_actions; });
}

void SmokeProbe::setBannerShown(bool shown) {
    if (shown == m_bannerShown) return;
    m_bannerShown = shown;
    emit messageChanged();
}

static const Registrar::Controller<SmokeProbe> probeController;
static const Registrar::Banner probeBanner("SmokeProbeBanner", 1000);
static const Registrar::Window probeWindow("smoke.window", "SmokeProbeWindow");

// ---- The section -------------------------------------------------------------

namespace {

QString shownName(QObject *o) { return o ? o->property("shownName").toString() : QString(); }

// The sidebar row of wire channel `channel`
QObject *sidebarRow(SmokeDriver &d, int channel) {
    return d.item(QStringLiteral("Sidebar.channel.%1").arg(channel));
}

void checkSidebarNames(SmokeDriver &d) {
    ChannelListModel *rows = d.app().device->channels();
    const DspState *state = d.app().device->state();
    d.check(QStringLiteral("qp0: the sidebar has rows"), rows->rowCount() > 2 && state);
    if (!state) return;
    d.check(QStringLiteral("qp0: the sidebar's inputs are the live inputs"),
            rows->inputCount() == d.app().device->numInputs());
    for (int r = 0; r < rows->rowCount(); ++r) {
        const int channel = rows->get(r, QStringLiteral("channel")).toInt();
        const bool output = rows->get(r, QStringLiteral("kind")).toString() == QLatin1String("output");
        const QString label = rows->get(r, QStringLiteral("label")).toString();
        if (output && !rows->get(r, QStringLiteral("outputEnabled")).toBool()) continue;
        d.check(QStringLiteral("qp0: channel %1 has a label").arg(channel), !label.isEmpty());
        // An input's label is the device's name for it (the state's record)
        if (!output) {
            const char *raw = reinterpret_cast<const char *>(state->channel_names[channel]);
            const QString stored = QString::fromUtf8(raw, int(qstrnlen(raw, CHANNEL_NAME_LEN)));
            if (!stored.isEmpty())
                d.check(QStringLiteral("qp0: input %1's label is the state's name").arg(channel), label == stored);
        }
        d.check(QStringLiteral("qp0: the sidebar shows channel %1's label").arg(channel),
                shownName(sidebarRow(d, channel)) == label);
    }
}

void checkWritePath(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    const QString before = device->channels()->get(0, QStringLiteral("name")).toString();
    const QString name = QStringLiteral("QP0 Smoke");
    d.check(QStringLiteral("qp0: a too long name is refused before sending"),
            !device->setChannelName(0, QString(40, QLatin1Char('x'))));
    d.check(QStringLiteral("qp0: setChannelName sends"), device->setChannelName(0, name));
    d.check(QStringLiteral("qp0: the written name reaches the sidebar"),
            d.waitFor(5000, [&]() { return shownName(sidebarRow(d, 0)) == name; }));
    device->setChannelName(0, before);
    d.check(QStringLiteral("qp0: the name is restored"),
            d.waitFor(5000, [&]() { return device->channels()->get(0, QStringLiteral("name")).toString() == before; }));
}

void checkNotification(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    uint notified = 0;
    const QMetaObject::Connection c =
        QObject::connect(device, &DeviceModel::stateChanged, [&notified](uint flags) { notified |= flags; });
    const QByteArray name("QP0 Elsewhere");
    // Another console renames the device: the demo device notifies, as the
    // firmware does, and nothing in this app asked for it
    d.app().core->worker()->post([name](FfiCore *core) { dspi_demo_rename_elsewhere(core, 0xFF, name.constData()); });
    QObject *button = d.item(QStringLiteral("DeviceStatus.button"));
    d.check(QStringLiteral("qp0: the device button exists"), button != nullptr);
    d.check(QStringLiteral("qp0: a rename made elsewhere reaches the device button"),
            d.waitFor(5000, [&]() { return shownName(button) == QString::fromUtf8(name); }));
    d.check(QStringLiteral("qp0: it came as a notification"), notified != 0);
    QObject::disconnect(c);
    d.app().core->worker()->post([](FfiCore *core) { dspi_demo_rename_elsewhere(core, 0xFF, ""); });
    d.waitFor(5000, [&]() { return device->deviceName().isEmpty(); });
    d.waitIdle();
}

void checkMenus(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    QObject *table = d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable");
    QVariant ids;
    if (table) QMetaObject::invokeMethod(table, "allIds", Q_RETURN_ARG(QVariant, ids));
    // A JavaScript array comes back wrapped
    if (ids.canConvert<QJSValue>()) ids = ids.value<QJSValue>().toVariant();
    const QStringList list = ids.toStringList();
    printf("smoke: qp0: %lld menu rows (table %s, %s)\n", qlonglong(list.size()), table ? "found" : "missing", ids.typeName());
    d.check(QStringLiteral("qp0: the menu table has the Mac's rows"), list.size() >= 42);
    d.check(QStringLiteral("qp0: menu ids are unique"), QSet<QString>(list.begin(), list.end()).size() == list.size());
    for (const QString &id : list)
        d.check(QStringLiteral("qp0: menu row %1 exists").arg(id), d.object(QStringLiteral("Menu.") + id) != nullptr);

    // A row's action, as a click on it runs it
    // -- WP QP7 -- the shell's triggered signal: the row has a window now (QP7)
    QString reached;
    const auto seen = QObject::connect(shell, &Shell::triggered, shell, [&](const QString &id) { reached = id; });
    QObject *row = d.object(QStringLiteral("Menu.tools.loudness"));
    QObject *action = row ? row->property("action").value<QObject *>() : nullptr;
    if (action) QMetaObject::invokeMethod(action, "trigger");
    d.check(QStringLiteral("qp0: a menu row reaches the shell"),
            d.waitFor(2000, [&]() { return reached == QLatin1String("tools.loudness"); }));
    QObject::disconnect(seen);
    if (QWindow *opened = shell->openWindow(QStringLiteral("tools.loudness"))) opened->close();
    d.mainWindow()->requestActivate();
    d.spin(100);

    // A shortcut, as a key press
    QQuickWindow *w = d.mainWindow();
    w->requestActivate();
    d.spin(100);
    // -- WP QP6 -- tools.matrix is handled now (the matrix window): the
    // shortcut is checked by the shell's triggered signal, and the window it
    // opens is closed again
    QString triggered;
    QMetaObject::Connection seen = QObject::connect(shell, &Shell::triggered, [&](const QString &a) { triggered = a; });
    QTest::keyClick(w, Qt::Key_M, Qt::ControlModifier | Qt::ShiftModifier);
    d.check(QStringLiteral("qp0: Ctrl+Shift+M reaches the shell as tools.matrix"),
            d.waitFor(2000, [&]() { return triggered == QLatin1String("tools.matrix"); }));
    QObject::disconnect(seen);
    if (QWindow *opened = shell->handles(QStringLiteral("tools.matrix")) ? shell->openWindow(QStringLiteral("tools.matrix")) : nullptr)
        opened->close();
}

void checkPlugPoints(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    SmokeProbe *probe = Registrar::instance<SmokeProbe>();
    d.check(QStringLiteral("qp0: a registered controller is made"), probe != nullptr);
    if (!probe) return;

    shell->trigger(QStringLiteral("smoke.action"));
    d.check(QStringLiteral("qp0: a controller's action handler runs"), probe->actions() == 1);
    d.check(QStringLiteral("qp0: the shell knows the handled ids"),
            shell->handles(QStringLiteral("smoke.action")) && shell->handles(QStringLiteral("smoke.window")) &&
                !shell->handles(QStringLiteral("smoke.nothing")));   // -- WP QP6 -- (was tools.matrix, now handled)

    d.check(QStringLiteral("qp0: a registered banner is in the host"),
            shell->banners().contains(QStringLiteral("SmokeProbeBanner")));
    QQuickItem *banner = d.item(QStringLiteral("SmokeProbeBanner"));
    d.check(QStringLiteral("qp0: the banner is made and hidden"), banner && !banner->isVisible());
    probe->setBannerShown(true);
    d.check(QStringLiteral("qp0: the banner shows when its controller says so"),
            d.waitFor(2000, [&]() { return banner && banner->isVisible() && banner->height() > 0; }));
    d.spin(200);
    d.check(QStringLiteral("qp0: screenshot with the banner"), d.grab(d.mainWindow(), QStringLiteral("window-main-banner.png")));
    probe->setBannerShown(false);
    d.waitFor(2000, [&]() { return !banner->isVisible(); });

    shell->trigger(QStringLiteral("smoke.window"));
    QWindow *window = nullptr;
    d.waitFor(3000, [&]() {
        window = shell->openWindow(QStringLiteral("smoke.window"));
        return window && window->isVisible();
    });
    auto *quick = qobject_cast<QQuickWindow *>(window);
    d.check(QStringLiteral("qp0: a registered tool window opens"), quick != nullptr);
    d.spin(400);
    d.check(QStringLiteral("qp0: screenshot of the tool window"), d.grab(quick, QStringLiteral("window-smoke-probe.png")));
    if (window) window->close();
}

// The window's background where nothing covers it: the right pane's margin
QColor background(const QImage &image) { return image.pixelColor(image.width() - 3, image.height() - 3); }

void checkSchemes(SmokeDriver &d) {
    QQuickWindow *w = d.mainWindow();
    Shell *shell = d.app().shell;
    shell->setColorSchemeOverride(Qt::ColorScheme::Dark);
    d.spin(400);
    const QImage dark = w->grabWindow();
    d.check(QStringLiteral("qp0: screenshot, dark"), !dark.isNull() && dark.save(d.outDir() + QStringLiteral("/window-main.png")));
    shell->setColorSchemeOverride(Qt::ColorScheme::Light);
    d.spin(400);
    const QImage light = w->grabWindow();
    d.check(QStringLiteral("qp0: screenshot, light"),
            !light.isNull() && light.save(d.outDir() + QStringLiteral("/window-main-light.png")));
    if (!dark.isNull() && !light.isNull())
        d.check(QStringLiteral("qp0: the light scheme is lighter"), background(light).lightness() > background(dark).lightness() + 100);
    shell->setColorSchemeOverride(Qt::ColorScheme::Unknown);
    d.spin(200);
}

void baseSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("qp0: the demo device opens"),
            d.waitFor(15000, [&]() { return device->connected() && device->channels()->rowCount() > 0; }));
    d.check(QStringLiteral("qp0: the core goes idle"), d.waitIdle());
    d.check(QStringLiteral("qp0: it is the demo"), device->demo() && !device->readOnly());
    d.check(QStringLiteral("qp0: the main window is up"), d.mainWindow() && d.mainWindow()->isVisible());
    d.spin(300);
    checkSidebarNames(d);
    checkWritePath(d);
    checkNotification(d);
    checkMenus(d);
    checkPlugPoints(d);
    checkSchemes(d);
}

const SmokeDriver::Section base("qp0-base", 0, &baseSection);

} // namespace
