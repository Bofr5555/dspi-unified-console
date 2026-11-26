// -- WP UFW -- (UXP A8, A10, C8) Finding things and one word for one thing, in
// Modern on the demo with a stereo two-way speaker layout (outputs 1-4: Left
// woofer, Left tweeter, Right woofer, Right tweeter):
// - "ufw-words": the command palette finds a speaker by its role ("tweeter":
//   "Left tweeter · 2"), the Crossover by "high-pass" and the manual's answer
//   by "refused"; the app menu has the User Manual; Save ▾ has no Factory
//   Reset, a device tab's menu has it; What's New opens with the app's notes.
//   Screenshots of each, dark and light, go to <out>/ufw/.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "LayoutController.h"   // -- FIX016 --
#include "NoticeCenter.h"
#include "PaletteController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QDir>
#include <QGuiApplication>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <functional>

namespace {

// The shown item named `name` under `from`
QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A core call on the active device's worker, waited for (SmokeUa1's)
QJsonObject onWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
    QByteArray out;
    bool done = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, std::move(job), [&](const QByteArray &r) {
        out = r;
        done = true;
    });
    d.waitFor(5000, [&] { return done; });
    return QJsonDocument::fromJson(out).object();
}

struct Route {
    bool on = false;
    double gain = 0;
    bool invert = false;
};

void wordsSection(SmokeDriver &d) {
    d.allowSectionTime(180);
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *cc = Registrar::instance<ChannelController>();
    auto *pc = Registrar::instance<PaletteController>();
    QQuickWindow *w = d.mainWindow();
    d.check("ufw: the controllers and the window", cc && pc && w);
    if (!cc || !pc || !w) return;
    d.check("ufw: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    const int inputs = cc->numInputs();
    const int outputs = cc->numOutputs();
    if (inputs < 2 || outputs <= 4) return;

    // L to outputs 1-2, R to 3-4, and a stereo two-way layout the user set (SmokeUa1's)
    const auto s = [&] { return device->state(); };
    QVector<QVector<Route>> was(outputs, QVector<Route>(inputs));
    for (int o = 0; o < outputs && s(); ++o)
        for (int i = 0; i < inputs; ++i)
            was[o][i] = { s()->matrix_routing[i][o], double(s()->matrix_gain[i][o]), s()->matrix_invert[i][o] };
    const auto wanted = [](int i, int o) { return (i == 0 && o < 2) || (i == 1 && (o == 2 || o == 3)); };
    for (int o = 0; o < outputs; ++o)
        for (int i = 0; i < inputs; ++i)
            if (was[o][i].on != wanted(i, o)) cc->setRoute(i, o, wanted(i, o), 0, false);
    d.waitIdle();
    const QJsonValue stored = onWorker(d, [](FfiCore *c) {
                                  return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); });
                              }).value(QLatin1String("value"));
    const auto row = [](int o, const char *role, const char *speaker, int group) {
        return QJsonObject{ { QStringLiteral("output"), o }, { QStringLiteral("role"), QLatin1String(role) },
                            { QStringLiteral("speaker"), QLatin1String(speaker) }, { QStringLiteral("group"), group },
                            { QStringLiteral("source"), QStringLiteral("user") } };
    };
    const QJsonObject layout{ { QStringLiteral("outputs"),
                                QJsonArray{ row(0, "low", "L", 0), row(1, "high", "L", 0), row(2, "low", "R", 1), row(3, "high", "R", 1) } },
                              { QStringLiteral("confirmedByUser"), true } };
    const auto setLayout = [&](const QJsonObject &l) {
        const QByteArray text = QJsonDocument(l).toJson(QJsonDocument::Compact);
        return onWorker(d, [text](FfiCore *c) {
            return fill([c, &text](char *b, uint32_t n) { return dspi_speaker_layout_set(c, text.constData(), b, n); });
        });
    };
    setLayout(layout);
    device->refresh();
    d.waitIdle();

    QDir(d.outDir()).mkpath(QStringLiteral("ufw"));
    const QSize original = w->size();
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    const bool modernWas = Registrar::instance<LayoutController>()->modern();   // -- FIX016 -- (R1-QT)
    d.check("ufw: in Modern", d.switchLayout("modern"));
    if (auto *notices = Registrar::instance<NoticeCenter>()) notices->clear();
    w->resize(QSize(1280, 800));
    d.waitFor(1000, [&] { return w->contentItem()->width() == 1280; });
    d.goTo(QStringLiteral("overview/overview"), 300);

    const auto shoot = [&](const QString &name, const char *schemeName) {
        d.spin(250);
        d.check(QStringLiteral("ufw: screenshot %1-%2").arg(name, QLatin1String(schemeName)),
                d.grab(w, QStringLiteral("ufw/%1-%2.png").arg(name, QLatin1String(schemeName))));
    };
    const auto click = [&](const QString &name, Qt::MouseButton button = Qt::LeftButton) {
        QQuickItem *it = shownItem(w->contentItem(), name);
        if (!it) return false;
        QTest::mouseClick(w, button, Qt::NoModifier, it->mapToScene(QPointF(it->width() / 2, it->height() / 2)).toPoint());
        d.spin(250);
        return true;
    };
    const auto closeMenu = [&](const QString &name) {
        if (QObject *m = d.object(name)) QMetaObject::invokeMethod(m, "close");
        d.spin(200);
    };
    QObject *palette = d.object(QStringLiteral("CommandPalette"));
    const auto rows = [&] {
        const QVariant v = palette ? palette->property("rows") : QVariant();
        return v.canConvert<QJSValue>() ? v.value<QJSValue>().toVariant().toList() : v.toList();
    };
    const auto type = [&](const QString &text) {
        QTest::keyClick(w, Qt::Key_A, Qt::ControlModifier);
        QTest::keyClick(w, Qt::Key_Backspace);
        for (const QChar c : text) QTest::keyClick(w, c.toLatin1());
        d.spin(150);
    };
    const auto titles = [&](int n) {
        QStringList out;
        for (const QVariant &r : rows().mid(0, n)) out << r.toMap().value(QStringLiteral("title")).toString();
        return out;
    };

    for (const char *schemeName : { "dark", "light" }) {
        const bool first = QLatin1String(schemeName) == QLatin1String("dark");
        shell->setColorSchemeOverride(first ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
        d.spin(300);

        // The palette: a role finds the speaker, a filter's word the Crossover, a question the manual
        w->requestActivate();
        QTest::keyClick(w, Qt::Key_K, Qt::ControlModifier);
        d.check("ufw: Ctrl+K opens the palette", d.waitFor(1500, [&] { return palette && palette->property("opened").toBool(); }));
        type(QStringLiteral("tweeter"));
        if (first)
            d.check(QStringLiteral("ufw: \"tweeter\" finds the tweeters by name (%1)").arg(titles(4).join(QLatin1String(" | "))),
                    titles(4).contains(QStringLiteral("Left tweeter · 2")) && titles(4).contains(QStringLiteral("Right tweeter · 4")));
        shoot(QStringLiteral("palette-tweeter"), schemeName);
        type(QStringLiteral("high-pass"));
        if (first) d.check("ufw: \"high-pass\" finds the Crossover", rows().value(0).toMap().value(QStringLiteral("id")).toString() == QLatin1String("page:equaliser/crossover"));
        shoot(QStringLiteral("palette-high-pass"), schemeName);
        type(QStringLiteral("refused"));
        if (first) d.check("ufw: \"refused\" finds the manual's answer", rows().value(0).toMap().value(QStringLiteral("id")).toString() == QLatin1String("help:refused"));
        shoot(QStringLiteral("palette-refused"), schemeName);
        if (palette) QMetaObject::invokeMethod(palette, "close");
        d.spin(250);

        // The app menu: the User Manual
        click(QStringLiteral("AppMenuButton"));
        if (first) d.check("ufw: the app menu has the User Manual", shownItem(w->contentItem(), QStringLiteral("AppMenu.help.manual")) || d.object(QStringLiteral("AppMenu.help.manual")));
        shoot(QStringLiteral("app-menu"), schemeName);
        closeMenu(QStringLiteral("AppMenu"));

        // Save ▾ without Factory Reset; the device tab's menu with it
        click(QStringLiteral("TopBar.save.more"));
        shoot(QStringLiteral("save-menu"), schemeName);
        closeMenu(QStringLiteral("TopBar.save.menu"));
        const QString id = DeviceRegistry::instance()->activeId();
        click(QStringLiteral("DeviceTabs.tab.") + id, Qt::RightButton);
        if (first) d.check("ufw: the device tab's menu has Factory Reset", d.object(QStringLiteral("DeviceTabs.menu.%1.device.factoryReset").arg(id)) != nullptr);
        shoot(QStringLiteral("tab-menu"), schemeName);
        closeMenu(QStringLiteral("DeviceTabs.menu.") + id);

        // What's New: the app's notes first
        shell->trigger(QStringLiteral("help.whatsNew"));
        d.spin(400);
        shoot(QStringLiteral("whats-new"), schemeName);
        d.goTo(QStringLiteral("overview/overview"), 300);
    }

    // As it was
    shell->setColorSchemeOverride(scheme);
    w->resize(original);
    if (!modernWas) d.switchLayout("classic");   // -- FIX016 -- (R1-QT) the layout as it found it
    if (stored.isObject()) setLayout(stored.toObject());
    for (int o = 0; o < outputs; ++o)
        for (int i = 0; i < inputs; ++i)
            if (was[o][i].on != wanted(i, o) || was[o][i].on) cc->setRoute(i, o, was[o][i].on, was[o][i].gain, was[o][i].invert);
    device->refresh();
    d.waitIdle();
}

const SmokeDriver::Section words("ufw-words", 992, &wordsSection);

} // namespace
