// -- WP QP19 -- Every window, in the dark scheme and the light: the main window,
// each tool window a menu row opens (Registrar::Window), every Settings page
// shown for the demo, and the device list. The PNGs go to <out>/gallery/, and
// run.sh writes <out>/index.html over them and the other sections' PNGs.
// Last of the main run, so it shows what the run left: the demo, as it opened.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DevicesController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QDir>
#include <QQuickItem>
#include <QQuickWindow>
#include <QWindow>

namespace {

struct Scheme {
    Qt::ColorScheme scheme;
    const char *name;
};
constexpr Scheme kSchemes[] = { { Qt::ColorScheme::Dark, "dark" }, { Qt::ColorScheme::Light, "light" } };

// "SettingsWindow" -> "settings", "AutoEqWindow" -> "autoeq"
QString shortName(const char *type) {
    QString name = QString::fromLatin1(type);
    if (name.endsWith(QLatin1String("Window"))) name.chop(6);
    return name.toLower();
}

QStringList settingsPages(SettingsController *settings) {
    QStringList ids;
    for (const QVariant &row : settings->sidebarRows(QString())) {
        const QString id = row.toMap().value(QStringLiteral("id")).toString();
        if (!id.isEmpty()) ids << id;
    }
    return ids;
}

void gallerySection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    DeviceModel *device = d.app().device;
    QQuickWindow *main = d.mainWindow();
    auto *settings = Registrar::instance<SettingsController>();
    auto *devices = Registrar::instance<DevicesController>();
    d.check("qp19-gallery: the main window and the controllers", main && settings && devices);
    if (!main || !settings || !devices) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("qp19-gallery: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    QDir(d.outDir()).mkpath(QStringLiteral("gallery"));
    // Nothing an earlier section opened is still over the main window
    QStringList left;
    if (auto *overlay = main->property("overlay").value<QQuickItem *>()) {
        for (QQuickItem *child : overlay->childItems())
            if (child->isVisible()) left << (child->parent() ? child->parent()->objectName() : child->objectName());
    }
    if (!left.isEmpty()) std::printf("smoke: qp19-gallery: left open: %s\n", qPrintable(left.join(QStringLiteral(", "))));
    d.check("qp19-gallery: no popup or dialog is left open by an earlier section", left.isEmpty());

    const Qt::ColorScheme before = shell->colorSchemeOverride();
    int shots = 0;
    const auto shoot = [&](QQuickWindow *w, const QString &name, const char *scheme) {
        const bool ok = d.grab(w, QStringLiteral("gallery/%1-%2.png").arg(name, QLatin1String(scheme)));
        d.check(QStringLiteral("qp19-gallery: %1, %2").arg(name, QLatin1String(scheme)), ok);
        shots += ok;
    };

    for (const Scheme &s : kSchemes) {
        shell->setColorSchemeOverride(s.scheme);
        d.spin(400);
        shoot(main, QStringLiteral("main"), s.name);

        // The device list, over the main window
        QQuickItem *host = devices->host();
        QObject *popup = host ? host->findChild<QObject *>(QStringLiteral("DevicesPopup")) : nullptr;
        shell->trigger(QStringLiteral("device.devices"));
        if (popup && d.waitFor(2000, [&] { return popup->property("opened").toBool(); })) {
            d.spin(300);
            shoot(main, QStringLiteral("devices"), s.name);
            QMetaObject::invokeMethod(popup, "close");
            d.waitFor(2000, [&] { return !popup->property("opened").toBool(); });
        } else {
            d.check(QStringLiteral("qp19-gallery: the device list opens, %1").arg(QLatin1String(s.name)), false);
        }

        for (const Registrar::WindowEntry &entry : Registrar::windows()) {
            const QString type = QString::fromLatin1(entry.type);
            if (type.startsWith(QLatin1String("Smoke"))) continue;   // the base's probe window
            const QString action = QString::fromLatin1(entry.action);
            shell->trigger(action);
            QWindow *window = nullptr;
            const bool open = d.waitFor(3000, [&] {
                window = shell->openWindow(action);
                return window && window->isVisible();
            });
            auto *quick = qobject_cast<QQuickWindow *>(window);
            d.check(QStringLiteral("qp19-gallery: %1 opens from %2").arg(type, action), open && quick);
            if (!open || !quick) continue;
            d.waitIdle();
            d.spin(400);
            if (type == QLatin1String("SettingsWindow")) {
                for (const QString &page : settingsPages(settings)) {
                    settings->open(page);
                    d.spin(300);
                    shoot(quick, QStringLiteral("settings-%1").arg(page), s.name);
                }
            } else {
                shoot(quick, shortName(entry.type), s.name);
            }
            window->close();
            d.spin(100);
        }
    }
    shell->setColorSchemeOverride(before);
    d.spin(200);
    std::printf("smoke: qp19-gallery: %d screenshots in gallery/\n", shots);
    std::fflush(stdout);
}

const SmokeDriver::Section gallery("qp19-gallery", 900, &gallerySection);

} // namespace
