// -- WP SWAPFIX -- A re-read keeps unsaved changes unsaved (owner, 2026-11-13):
// after a refresh on the demo, and after a DSPi Link session is lost and comes
// back (the device read again), the device switch and Quit still ask. The
// question lists an output's delay once. Every change is put back.
#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"
#include "DevicesController.h"
#include "FilesController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "Registrar.h"
#include "Shell.h"
#include "UnsavedGuard.h"

#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <cmath>
#include <cstring>

namespace {
using namespace linksmoke;

// The firmware's PARAM_SRC_UART (notify.h): the change's writer
constexpr int kSourceUart = 8;

bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }

struct Rig {
    SmokeDriver &d;
    DeviceModel *device;
    UnsavedGuard *guard;
    DevicesController *devices;
    Shell *shell;
    QObject *question;

    bool write(const QString &name, std::function<bool(FfiCore *)> fn) {
        int outcome = -1;
        device->write(name, std::move(fn), [&outcome](bool ok) { outcome = ok ? 1 : 0; });
        return d.waitFor(10000, [&] { return outcome >= 0; }) && outcome == 1 && d.waitIdle();
    }
    bool dirty() { return dspi_preset_dirty(d.app().core->handle()); }
    // The question `op` brings up, its text, then Cancel; empty when none came
    QString ask(const char *op) {
        if (op == QLatin1String("device")) devices->chooseUsb(QStringLiteral("SWAPFIX-NONE"));
        else shell->trigger(QStringLiteral("file.quit"));
        if (!d.waitFor(3000, [&] { return opened(question); })) return {};
        const QString text = question->property("text").toString();
        QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
        d.waitFor(2000, [&] { return !guard->asking(); });
        d.waitIdle();
        return text;
    }
};

void swapfixSection(SmokeDriver &d) {
    FilesController *files = Registrar::instance<FilesController>();
    Rig r{ d, d.app().device, Registrar::instance<UnsavedGuard>(), Registrar::instance<DevicesController>(), d.app().shell,
           files && files->host() ? files->host()->findChild<QObject *>(QStringLiteral("FilesHost.unsavedQuestion")) : nullptr };
    d.check("swapfix: the guard and its question exist", r.guard && r.devices && r.question);
    if (!r.guard || !r.devices || !r.question) return;
    d.waitIdle();
    const QString demoSerial = r.device->serial();
    const float delay0 = r.device->state()->output_delay_ms[0];
    const float gain0 = r.device->state()->output_gain_db[0];
    // Earlier sections may leave other changes unsaved on the demo
    const bool dirtyBefore = r.dirty();

    // F3: an output's delay is one line in the question
    d.check("swapfix: an output delay edit", r.write(QStringLiteral("output delay 0"), [delay0](FfiCore *c) {
        return dspi_set_output_delay(c, 0, delay0 + 1.5f);
    }) && r.dirty());
    QString text = r.ask("device");
    d.check("swapfix-f3: the switch asks", !text.isEmpty());
    const int delayLines = int(text.count(QRegularExpression(QStringLiteral("delay: "))));
    std::printf("smoke: swapfix-f3: %s\n", qPrintable(QString(text).replace(QLatin1Char('\n'), QLatin1String(" | "))));
    d.check(QStringLiteral("swapfix-f3: the output's delay is listed once (%1)").arg(delayLines), delayLines == 1);
    d.check("swapfix: Cancel stays on the device", r.device->connected() && r.device->serial() == demoSerial);

    // F2: a refresh (dspi_fetch_all) keeps it unsaved, and both questions ask
    r.device->refresh();
    d.check("swapfix-f2: refreshed", d.waitIdle(20000));
    d.check("swapfix-f2: still unsaved after a refresh", r.dirty());
    d.check("swapfix-f2: the switch asks after a refresh", !r.ask("device").isEmpty());
    d.check("swapfix-f2: Quit asks after a refresh", !r.ask("quit").isEmpty());
    d.check("swapfix-f2: Cancel keeps the app", d.mainWindow()->isVisible());
    d.check("swapfix: the delay is put back", r.write(QStringLiteral("output delay 0"), [delay0](FfiCore *c) {
        return dspi_set_output_delay(c, 0, delay0);
    }) && r.dirty() == dirtyBefore);

    // F2 over a reconnect: a hub's session is lost; meanwhile someone else
    // changes OUT1's gain at the hub (unheard); the session comes back, the
    // device is read again, and the change is unsaved against the same
    // baseline. The fake hub keeps only the image changes it is told of
    // ("change"), so both the change and the restore go through it.
    auto *link = Registrar::instance<LinkController>();
    FakeHubProcess hub;
    const bool hubUp = link && hub.start({ "--auth", "none" });
    d.check("swapfix-f2: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    link->connectToHub(hub.url);
    d.check("swapfix-f2: session", settled() && d.waitFor(3000, [&] { return phaseIs("choose_device") && link->devices().size() == 1; }));
    const QVariantMap hubDevice = link->devices().value(0).toMap();
    const QString serial = hubDevice.value(QStringLiteral("serial")).toString();
    link->bindDevice(hubDevice.value(QStringLiteral("handle")).toInt(), serial);
    d.check("swapfix-f2: bound", settled() && d.waitFor(5000, [&] {
        return phaseIs("bound") && r.device->remote() && r.device->connected() && r.device->serial() == serial;
    }) && d.waitIdle());
    const float hubGain = r.device->state()->output_gain_db[0];
    const float edited = hubGain > -20.0f ? hubGain - 2.0f : hubGain + 2.0f;
    d.check("swapfix-f2: not unsaved at first", !r.dirty());
    // OUT1's gain in the image: the outputs' records, gain at byte 4
    const auto change = [&hub](float db) {
        QByteArray v(4, '\0');
        std::memcpy(v.data(), &db, 4);
        return hub.fault(QStringLiteral("change %1 %2 %3 0").arg(OFF_OUTPUTS + 4).arg(QString::fromLatin1(v.toHex())).arg(kSourceUart).toUtf8().constData());
    };

    int back = 0;
    const QMetaObject::Connection counting = QObject::connect(link, &LinkController::reconnected, link, [&] { back++; });
    d.check("swapfix-f2: the hub takes refuse 503", hub.fault("refuse 503"));
    d.check("swapfix-f2: the hub takes drop", hub.fault("drop"));
    d.check("swapfix-f2: lost", d.waitFor(3000, [&] { return phaseIs("reconnecting"); }));
    d.check("swapfix-f2: someone else changes OUT1's gain meanwhile", change(edited));
    d.check("swapfix-f2: the hub takes refuse ok", hub.fault("refuse ok"));
    link->retryNow();
    d.check("swapfix-f2: back and bound", d.waitFor(15000, [&] { return back >= 1 && phaseIs("bound"); }));
    QObject::disconnect(counting);
    r.device->refresh();
    d.check("swapfix-f2: the device read again", d.waitIdle(20000) && r.device->connected());
    std::printf("smoke: swapfix-f2: after the reconnect: gain %.2f (changed to %.2f, was %.2f)\n",
                r.device->state()->output_gain_db[0], edited, hubGain);
    d.check("swapfix-f2: the re-read shows the change", std::fabs(r.device->state()->output_gain_db[0] - edited) < 0.01f);
    d.check("swapfix-f2: unsaved after the reconnect", r.dirty());
    d.check("swapfix-f2: the switch asks after the reconnect", !r.ask("device").isEmpty());
    d.check("swapfix-f2: Quit asks after the reconnect", !r.ask("quit").isEmpty());
    d.check("swapfix-f2: Cancel keeps the hub's device", r.device->remote() && r.device->serial() == serial);
    d.check("swapfix-f2: the hub takes the gain back", change(hubGain));
    r.device->refresh();
    d.check("swapfix-f2: and nothing is unsaved then", d.waitIdle(20000)
                                                         && std::fabs(r.device->state()->output_gain_db[0] - hubGain) < 0.01f && !r.dirty());

    link->closeSession();
    d.check("swapfix: the session ends", settled() && d.waitFor(5000, [&] { return phaseIs("idle") && !r.device->remote(); }));
    d.waitIdle();
    if (!r.device->connected() || r.device->serial() != demoSerial) {
        r.devices->chooseDemo();
        d.waitFor(8000, [&] { return r.device->connected() && r.device->demo(); });
        d.waitIdle();
    }
    d.check("swapfix: the demo is back as it was", r.device->demo() && r.device->state()->output_gain_db[0] == gain0
                                                       && r.device->state()->output_delay_ms[0] == delay0);
}

const SmokeDriver::Section swapfix("swapfix", 152, &swapfixSection);

} // namespace
