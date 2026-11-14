// -- WP HUBADMIN -- Settings > Installer edits the copy in force: this
// console's own on the demo (the console-copy line, editable), and the HUB's
// over DSPi Link against `uconsole --demo hub` (DSPI_SMOKE_UCONSOLE): an
// Admin (open hub) edits the hub's limits, PIN (the recovery code shown once
// with the hub's row), what its lock covers and the pinned modes, and the page
// follows another Admin's change by the hub's event; a Control client (paired
// with the hub's PIN) sees the page read-only with the hub_role row.
// Screenshots in <out>/hubadmin/.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "LimitsController.h"
#include "LinkController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDir>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <atomic>
#include <memory>
#include <thread>

namespace {

constexpr const char *kPin = "2468";

// One `uconsole --demo hub` process (a real hub with the demo DSPi)
class DemoHub {
public:
    bool start(const QString &store, const QStringList &extra, bool pairing = false) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        QDir(store).removeRecursively();
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--loopback", "--json", "--no-keep-awake", "--store", store, "--port", "0" } + extra);
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        int port = 0;
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                const QString event = e.value(QLatin1String("event")).toString();
                if (event == QLatin1String("status")) port = e.value(QLatin1String("port")).toInt();
                if (event == QLatin1String("pairing")) pin = e.value(QLatin1String("pin")).toString();
            }
            if (port > 0 && (!pairing || !pin.isEmpty())) {
                url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
                return true;
            }
        }
        return false;
    }
    void stop() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    ~DemoHub() { stop(); }
    QString pin, url;

private:
    QProcess m_proc;
};

bool shown(QQuickItem *item) {
    for (QQuickItem *i = item; i; i = i->parentItem())
        if (!i->isVisible()) return false;
    return item != nullptr;
}

QJsonObject reply(const QByteArray &buf) { return QJsonDocument::fromJson(buf.constData()).object(); }

// Another Admin on the open hub, through the C ABI on a thread of its own:
// pins the Viewer's mode on the hub (the hub then sends `mode.changed`)
bool pinElsewhere(SmokeDriver &d, const QString &url, const char *role, const char *mode) {
    auto ok = std::make_shared<std::atomic<int>>(-1);
    const QByteArray u = url.toUtf8();
    const QByteArray r(role), m(mode);
    std::thread([ok, u, r, m] {
        FfiCore *other = dspi_core_new();
        bool done = false;
        if (other) {
            QByteArray buf(256 * 1024, '\0');
            if (dspi_link_connect(other, u.constData(), "hubadmin-other", "0", "", "", buf.data(), uint32_t(buf.size())) > 0
                && reply(buf).value(QLatin1String("ok")).toBool()) {
                buf.fill('\0');
                dspi_link_devices(other, buf.data(), uint32_t(buf.size()));
                const QJsonObject dev = reply(buf).value(QLatin1String("value")).toArray().first().toObject();
                const QByteArray serial = dev.value(QLatin1String("serial")).toString().toUtf8();
                buf.fill('\0');
                dspi_link_select_device(other, uint32_t(dev.value(QLatin1String("handle")).toInt()), serial.constData(), buf.data(),
                                        uint32_t(buf.size()));
                if (reply(buf).value(QLatin1String("ok")).toBool()) {
                    buf.fill('\0');
                    dspi_hub_mode_pin(other, r.constData(), m.constData(), buf.data(), uint32_t(buf.size()));
                    done = reply(buf).value(QLatin1String("ok")).toBool();
                }
            }
            dspi_link_close(other);
            dspi_core_free(other);
        }
        ok->store(done ? 1 : 0);
    }).detach();
    d.waitFor(40000, [&] { return ok->load() >= 0; });
    return ok->load() == 1;
}

QString pinnedMode(LimitsController *lc, const char *role) {
    for (const QVariant &v : lc->modePins()) {
        const QVariantMap p = v.toMap();
        if (p.value(QStringLiteral("role")).toString() == QLatin1String(role)) return p.value(QStringLiteral("mode")).toString();
    }
    return QStringLiteral("?");
}

} // namespace

static void hubAdminSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *lc = Registrar::instance<LimitsController>();
    auto *link = Registrar::instance<LinkController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    auto *settings = Registrar::instance<SettingsController>();
    d.check("hubadmin: the controllers", lc && link && notices && settings);
    if (!lc || !link || !notices || !settings) return;
    const QString out = d.outDir() + QStringLiteral("/hubadmin");
    QDir().mkpath(out);
    const auto settled = [&] {
        d.waitIdle();
        const bool ok = d.waitFor(15000, [&] { return !lc->busy() && !link->busy(); });
        d.waitIdle();
        d.spin(60);
        return ok;
    };

    // ---- The demo: this console's copy ------------------------------------------
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("hubadmin: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && settled());
    d.waitFor(5000, [&] { return lc->copy() == QLatin1String("console") && !lc->copyText().isEmpty(); });
    d.check("hubadmin: the demo is this console's copy, editable",
            lc->copy() == QLatin1String("console") && lc->editable() && lc->readOnlyText().isEmpty() && lc->modePins().isEmpty());
    d.check("hubadmin: the console-copy line is the core's row",
            !lc->copyText().isEmpty() && lc->copyText() == notices->line(QStringLiteral("installer.console_copy")));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check("hubadmin: the settings window opens", w && d.waitFor(3000, [&] { return w->isVisible(); }));
    if (!w) return;
    settings->open(QStringLiteral("installer"));
    d.spin(300);
    const auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), w->contentItem()); };
    d.check("hubadmin: the page shows the console-copy line",
            shown(item("Installer.copyNotice")) && item("Installer.copyNotice")->property("text").toString() == lc->copyText());
    d.check("hubadmin: no read-only line, no pinned modes on the demo",
            !shown(item("Installer.readOnly")) && !shown(item("Installer.modePins")));
    d.check("hubadmin: the demo's fields are editable", item("Installer.masterVolumeMax") && item("Installer.masterVolumeMax")->isEnabled());
    d.grab(w, QStringLiteral("hubadmin/console-copy.png"));

    // ---- An open hub: Admin edits the hub's copy ---------------------------------
    if (qEnvironmentVariable("DSPI_SMOKE_UCONSOLE").isEmpty()) {
        d.notRun("hubadmin: the hub's copy", QStringLiteral("no DSPI_SMOKE_UCONSOLE"));
        w->close();
        return;
    }
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    const auto bind = [&] {
        if (!d.waitFor(15000, [&] { return phaseIs("choose_device") && !link->devices().isEmpty(); })) return false;
        const QVariantMap m = link->devices().first().toMap();
        link->bindDevice(m.value(QStringLiteral("handle")).toInt(), m.value(QStringLiteral("serial")).toString());
        return settled() && d.waitFor(15000, [&] { return phaseIs("bound") && device->remote() && device->connected(); }) && d.waitIdle(20000);
    };
    const auto leave = [&] {
        link->closeSession();
        settled();
        d.waitFor(10000, [&] { return !device->remote(); });
        d.waitIdle();
    };
    {
        DemoHub hub;
        const bool up = hub.start(out + QStringLiteral("/hub-open"), { QStringLiteral("--open"), QStringLiteral("--forget-clients") });
        d.check("hubadmin: uconsole --demo hub --open runs (DSPI_SMOKE_UCONSOLE)", up);
        if (!up) {
            w->close();
            return;
        }
        link->connectToHub(hub.url);
        d.check("hubadmin: on the open hub, its device bound", settled() && bind());
        d.check("hubadmin: the hub's copy is read", d.waitFor(15000, [&] {
            return lc->copy() == QLatin1String("hub") && !lc->busy() && lc->modePins().size() == 3;
        }));
        d.check("hubadmin: an Admin of the open hub may edit it", lc->editable() && lc->readOnlyText().isEmpty());
        d.check("hubadmin: the hub-copy line is the core's row",
                !lc->copyText().isEmpty() && lc->copyText() == notices->line(QStringLiteral("installer.hub_copy")));
        d.spin(200);
        d.check("hubadmin: the page shows the hub-copy line and the pinned modes",
                shown(item("Installer.copyNotice")) && item("Installer.copyNotice")->property("text").toString() == lc->copyText()
                    && shown(item("Installer.modePins")) && !shown(item("Installer.readOnly")));
        d.check("hubadmin: the PIN group edits the hub's PIN", shown(item("Installer.pinGroup")) && !shown(item("Installer.hubGroup")));

        // A limit on the hub, read back from the hub
        lc->setOutputLimit(0, QStringLiteral("maxGainDb"), -3.0);
        settled();
        d.check("hubadmin: a limit set on the hub is the hub's profile", lc->active() && lc->profileState() == QLatin1String("ok")
                                                                          && lc->outputs().value(0).toMap().value(QStringLiteral("maxGainDb")).toDouble() == -3.0);
        // The PIN: its recovery code, once, with the hub's row
        lc->setPin(QString::fromLatin1(kPin), QString::fromLatin1(kPin));
        settled();
        d.check("hubadmin: a PIN set on the hub", lc->pinSet() && lc->pinError().isEmpty());
        d.check("hubadmin: its recovery code with the hub's row",
                lc->recoveryCode().size() >= 20 && lc->recoveryKey() == QLatin1String("installer_lock.hub_recovery"));
        d.spin(200);
        d.check("hubadmin: the page shows the code and the hub_recovery line",
                shown(item("Installer.recovery")) && item("Installer.recoveryNotice")
                    && item("Installer.recoveryNotice")->property("text").toString()
                           == notices->line(QStringLiteral("installer_lock.hub_recovery")));
        d.grab(w, QStringLiteral("hubadmin/hub-admin.png"));
        lc->dismissRecovery();
        // What the lock covers: the hub's profile's lockedPages
        lc->setCovers(true);
        settled();
        d.check("hubadmin: the lock's cover is the hub profile's", lc->covers() && lc->active());
        lc->setCovers(false);
        settled();
        d.check("hubadmin: and off again", !lc->covers());
        // A pinned mode, from the page's chooser
        QQuickItem *control = item("Installer.modePin.control");
        d.check("hubadmin: a chooser per role", control && item("Installer.modePin.viewer") && item("Installer.modePin.admin"));
        if (control) QMetaObject::invokeMethod(control, "chosen", Q_ARG(QVariant, QStringLiteral("simple")));
        settled();
        d.check("hubadmin: Control pinned to Simple on the hub", d.waitFor(5000, [&] { return pinnedMode(lc, "control") == QLatin1String("simple"); }));
        d.check("hubadmin: the chooser shows it",
                item("Installer.modePin.control.combo") && item("Installer.modePin.control.combo")->property("currentIndex").toInt() == 1);
        // Another Admin pins the Viewer's: the hub's event, and the page reads again
        d.check("hubadmin: another Admin pins the Viewer's mode", pinElsewhere(d, hub.url, "viewer", "advanced"));
        d.check("hubadmin: the page follows the hub's event",
                d.waitFor(15000, [&] { return pinnedMode(lc, "viewer") == QLatin1String("advanced"); }));
        // Back as it was on the hub
        lc->setModePin(QStringLiteral("control"), QString());
        settled();
        lc->setModePin(QStringLiteral("viewer"), QString());
        settled();
        d.check("hubadmin: unpinned", pinnedMode(lc, "control").isEmpty() && pinnedMode(lc, "viewer").isEmpty());
        lc->clearPin();
        settled();
        d.check("hubadmin: the hub's PIN removed", !lc->pinSet());
        lc->clearLimits();
        settled();
        d.check("hubadmin: the hub's profile removed", !lc->active());
        leave();
        hub.stop();
    }

    // ---- A PIN hub: Control sees it read-only ---------------------------------------
    {
        DemoHub pinHub;
        const bool up = pinHub.start(out + QStringLiteral("/hub-pin"), { QStringLiteral("--pair"), QStringLiteral("--forget-clients") }, true);
        d.check("hubadmin: a PIN hub runs", up);
        if (up) {
            link->connectToHub(pinHub.url);
            d.check("hubadmin: the PIN hub asks for its PIN", settled() && d.waitFor(5000, [&] { return phaseIs("needs_pairing"); }));
            link->pair(pinHub.pin);
            d.check("hubadmin: paired, its device bound", settled() && bind());
            d.check("hubadmin: Control: the hub's copy, read-only", d.waitFor(15000, [&] {
                return lc->copy() == QLatin1String("hub") && !lc->busy() && !lc->editable();
            }));
            d.check("hubadmin: why, in the hub_role row",
                    lc->readOnlyText() == notices->line(QStringLiteral("installer.hub_role")) && !lc->readOnlyText().isEmpty());
            d.spin(200);
            d.check("hubadmin: the page shows the read-only line, its fields disabled",
                    shown(item("Installer.readOnly")) && item("Installer.masterVolumeMax") && !item("Installer.masterVolumeMax")->isEnabled());
            d.check("hubadmin: Control sees its own role's pinned mode only, not to be changed",
                    lc->modePins().size() <= 1
                        && (lc->modePins().isEmpty() || (item("Installer.modePin.control") && !item("Installer.modePin.control")->isEnabled())));
            d.grab(w, QStringLiteral("hubadmin/hub-control.png"));
            leave();
            pinHub.stop();
        }
    }
    w->close();
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    notices->clear();
}

static const SmokeDriver::Section hubAdmin("hubadmin-installer", 161, &hubAdminSection);
