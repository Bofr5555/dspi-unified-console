// -- WP UB12 -- (UXP B12, owner D12) The demo shows what a DSPi does:
// - "ub12-demo": the product's demo (dspi_open_demo; the smoke build opens the
//   blank one otherwise, SessionModel::smokeDemoExample) is the tuned example:
//   outputs 1-4 named Left/Right woofer/tweeter, LR4 crossovers, the roles
//   confirmed, the rest unused, nothing unsaved. The demo strip's Reset Demo
//   and Device > Reset Demo put back a changed level and name with nothing
//   unsaved; on a real device's place the row does nothing. Screenshots of the
//   blank demo (before) and the example (after), Modern, dark and light, go to
//   <out>/ub12/. The section ends on the blank demo, as it began.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"

#include <QDir>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <functional>

namespace {

QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

QVariant rowValue(DeviceModel *device, int channel, const char *role) {
    ChannelListModel *rows = device->channels();
    const int row = rows ? rows->rowOfChannel(channel) : -1;
    return row >= 0 ? rows->get(row, QString::fromLatin1(role)) : QVariant();
}

// A core call on the active device's worker, waited for (SmokeUa1's)
template <typename T> T onWorker(SmokeDriver &d, std::function<T(FfiCore *)> job) {
    T out{};
    bool done = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, std::move(job), [&](const T &r) {
        out = r;
        done = true;
    });
    d.waitFor(5000, [&] { return done; });
    return out;
}

QJsonObject speakerLayout(SmokeDriver &d) {
    const QByteArray text = onWorker<QByteArray>(d, [](FfiCore *c) {
        QByteArray buf(1 << 16, '\0');
        const uint32_t n = dspi_speaker_layout(c, buf.data(), uint32_t(buf.size()));
        if (n == 0 || n > uint32_t(buf.size())) return QByteArray();
        buf.truncate(int(n) - 1);
        return buf;
    });
    // The layout's fields beside "ok"; `{ok, value: null}` with none
    QJsonObject reply = QJsonDocument::fromJson(text).object();
    if (!reply.contains(QLatin1String("outputs"))) return {};
    reply.remove(QLatin1String("ok"));
    return reply;
}

void demoSection(SmokeDriver &d) {
    d.allowSectionTime(180);
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *cc = Registrar::instance<ChannelController>();
    auto *sc = Registrar::instance<SidebarController>();
    QQuickWindow *w = d.mainWindow();
    d.check("ub12: the controllers and the window", cc && sc && w);
    if (!cc || !sc || !w) return;
    const auto reopen = [&](bool example) {
        SessionModel::smokeDemoExample = example;
        device->openDemo();
        return d.waitFor(10000, [&] {
            return device->connected() && device->demo()
                && (rowValue(device, cc->outputChannel(0), "displayName").toString() == QLatin1String("Left woofer")) == example;
        }) && d.waitIdle();
    };
    const auto dirty = [&] { return onWorker<bool>(d, [](FfiCore *c) { return dspi_preset_dirty(c); }); };
    const auto state = [&] { return device->state(); };

    // Before: the blank demo, as every client opened it until 0.16
    d.check("ub12: the blank demo first", reopen(false));
    d.check("ub12: the blank demo has no roles", speakerLayout(d).isEmpty());

    QDir(d.outDir()).mkpath(QStringLiteral("ub12"));
    const QSize original = w->size();
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    if (!d.switchLayout("modern")) {
        d.check("ub12: in Modern", false);
        return;
    }
    w->resize(QSize(1600, 1000));
    d.waitFor(1000, [&] { return w->contentItem()->width() == 1600; });
    const auto shoot = [&](const char *when) {
        for (const char *schemeName : { "dark", "light" }) {
            shell->setColorSchemeOverride(QLatin1String(schemeName) == QLatin1String("dark") ? Qt::ColorScheme::Dark
                                                                                          : Qt::ColorScheme::Light);
            if (auto *notices = Registrar::instance<NoticeCenter>()) notices->clear();
            for (const char *page : { "overview/overview", "equaliser/crossover" }) {
                d.goTo(QLatin1String(page), 400);
                sc->select(cc->outputChannel(0));
                d.spin(400);
                const QString file = QStringLiteral("ub12/%1-%2-%3.png")
                                         .arg(QLatin1String(when), QString::fromLatin1(page).section(QLatin1Char('/'), 1),
                                              QLatin1String(schemeName));
                d.check(QStringLiteral("ub12: screenshot %1").arg(file), d.grab(w, file));
            }
        }
    };
    shoot("before");

    // After: the product's demo
    d.check("ub12: the product's demo opens on the example", reopen(true));
    const int n = cc->numOutputs();
    d.check(QStringLiteral("ub12: nine outputs (%1)").arg(n), n >= 5);
    if (n < 5 || !state()) return;
    const char *names[] = { "Left woofer", "Right woofer", "Left tweeter", "Right tweeter" };
    for (int o = 0; o < 4; ++o) {
        const QString shown = rowValue(device, cc->outputChannel(o), "displayName").toString();
        d.check(QStringLiteral("ub12: output %1 is %2 (%3)").arg(o + 1).arg(QLatin1String(names[o]), shown), shown == QLatin1String(names[o]));
        const int type = state()->xover[cc->outputChannel(o)][0].filter_type;
        d.check(QStringLiteral("ub12: output %1 is crossed over with an LR4 at 2.2 kHz (%2)").arg(o + 1).arg(type),
                type == (o < 2 ? 34 : 35) && qAbs(state()->xover[cc->outputChannel(o)][0].freq - 2200.0f) < 1.0f);
    }
    for (int o = 4; o < n; ++o)
        d.check(QStringLiteral("ub12: output %1 is unused").arg(o + 1), rowValue(device, cc->outputChannel(o), "unused").toBool());
    const QJsonObject layout = speakerLayout(d);
    d.check("ub12: its roles are confirmed", layout.value(QLatin1String("confirmedByUser")).toBool());
    d.check("ub12: nothing unsaved, after the blank demo", !dirty());
    shoot("after");

    // The strip's Reset Demo puts a changed level and name back
    d.goTo(QStringLiteral("overview/overview"), 300);
    QQuickItem *reset = shownItem(w->contentItem(), QStringLiteral("DemoBanner.reset"));
    d.check("ub12: the demo strip offers Reset Demo", reset && reset->property("text").toString() == QStringLiteral("Reset Demo"));
    d.check("ub12: and not Open Demo", !shownItem(w->contentItem(), QStringLiteral("DemoBanner.open")));
    const auto changeThenReset = [&](const QString &how, const std::function<void()> &doReset) {
        cc->setOutputGain(2, -9.0);
        device->setChannelName(cc->outputChannel(0), QStringLiteral("Mine"));
        d.waitIdle();
        d.check(QStringLiteral("ub12: %1: changed and unsaved").arg(how), d.waitFor(3000, [&] {
            return state() && state()->output_gain_db[2] == -9.0f;
        }) && dirty());
        doReset();
        d.check(QStringLiteral("ub12: %1 puts the example back").arg(how), d.waitFor(5000, [&] {
            return state() && state()->output_gain_db[2] == -3.0f
                && rowValue(device, cc->outputChannel(0), "displayName").toString() == QLatin1String("Left woofer");
        }) && d.waitIdle());
        d.check(QStringLiteral("ub12: %1 leaves nothing unsaved").arg(how), !dirty());
        d.check(QStringLiteral("ub12: %1 keeps the roles confirmed").arg(how),
                speakerLayout(d).value(QLatin1String("confirmedByUser")).toBool());
        d.check(QStringLiteral("ub12: %1 keeps the demo open").arg(how), device->connected() && device->demo());
    };
    changeThenReset(QStringLiteral("the strip's button"), [&] {
        if (QQuickItem *r = shownItem(w->contentItem(), QStringLiteral("DemoBanner.reset"))) SmokeDriver::press(r);
    });
    d.check("ub12: Device > Reset Demo is the shell's", shell->handles(QStringLiteral("device.resetDemo")));
    changeThenReset(QStringLiteral("Device > Reset Demo"), [&] { shell->trigger(QStringLiteral("device.resetDemo")); });

    // As it was: the blank demo, Classic, the window's size and scheme
    shell->setColorSchemeOverride(scheme);
    w->resize(original);
    d.goTo(QStringLiteral("overview/overview"));
    d.switchLayout("classic");
    d.check("ub12: back on the blank demo", reopen(false));
}

const SmokeDriver::Section demo("ub12-demo", 996, &demoSection);

} // namespace
