// WP QD14: Settings > Networking on a demo hub, through the real page and HubController:
// sharing on loopback, the Terminal (DSPI_SMOKE_TERMINAL) pairing with the code the page
// shows, roles, revoking, the access warning, a restart on rename, a port in use, the
// tray's view of it, and browsing only while the page shows.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "DemoDevice.h"
#include "HubController.h"
#include "TrayController.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QProcessEnvironment>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTcpServer>
#include <cstdio>

extern "C" {
#include "dspi_core.h"
}

namespace {

QJsonObject hubJson(FfiHub *hub, uint32_t (*call)(FfiHub *, char *, uint32_t)) {
    QByteArray buf(int(HUB_REPLY_MIN), '\0');
    uint32_t needed = call(hub, buf.data(), HUB_REPLY_MIN);
    if (needed > HUB_REPLY_MIN) {
        buf.resize(int(needed));
        needed = call(hub, buf.data(), needed);
    }
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    return QJsonDocument::fromJson(QByteArray(buf.constData(), int(needed) - 1)).object();
}

struct Run {
    int code = -1;
    QString out;
};

// The Terminal as a client of the hub; the PIN in its environment, never its arguments
Run terminal(const QString &binary, const QString &address, const QString &pin) {
    QProcess proc;
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.remove(QStringLiteral("DSPI_PIN"));
    if (!pin.isEmpty()) env.insert(QStringLiteral("DSPI_PIN"), pin);
    proc.setProcessEnvironment(env);
    proc.setProcessChannelMode(QProcess::MergedChannels);
    proc.setStandardInputFile(QProcess::nullDevice());
    proc.start(binary, { QStringLiteral("--link"), address, QStringLiteral("dump") });
    if (!proc.waitForStarted(5000) || !proc.waitForFinished(30000)) {
        proc.kill();
        proc.waitForFinished(2000);
        return {};
    }
    return { proc.exitStatus() == QProcess::NormalExit ? proc.exitCode() : -1, QString::fromUtf8(proc.readAll()) };
}

// Pages are built anew on each visit, under the Loader's item tree
QQuickItem *findItem(QQuickItem *in, const QString &name) {
    if (in->objectName() == name) return in;
    for (QQuickItem *child : in->childItems())
        if (QQuickItem *found = findItem(child, name)) return found;
    return nullptr;
}

// QD12's pass brings its own hub and starts from none, however this pass ends
struct LeaveTray {
    ~LeaveTray() { TrayController::setHub(nullptr); }
};

QVariantMap firstClient(HubController *hub) {
    return hub->clients().isEmpty() ? QVariantMap() : hub->clients().first().toMap();
}

} // namespace

void SmokeDriver::testNetworking() {
    section("QD14: networking");
    LeaveTray leaveTray;
    auto *hub = qobject_cast<HubController *>(m_.engine->rootContext()->contextProperty(QStringLiteral("hub")).value<QObject *>());
    check("QD14: controller registered as hub", hub != nullptr);
    if (!hub) return;
    const QString binary = qEnvironmentVariable("DSPI_SMOKE_TERMINAL");
    check("QD14: the Terminal is there (DSPI_SMOKE_TERMINAL)", !binary.isEmpty());

    hub->setStoreDir(QDir(m_out).filePath(QStringLiteral("networking-store")));
    hub->setExtraOptions(HUB_OPT_LOOPBACK);
    check("QD14: sharing is off by default, with PIN access", !hub->sharing() && !hub->running() && hub->access() == QLatin1String("pin"));
    check("QD14: the tray has the hub", hub->trayState().pinAccess && !hub->trayState().running);
    check("QD14: a default port from the store", waitFor(5000, [&]() { return hub->port() > 0; }));
    check("QD14: a hub name to start with", !hub->hubName().isEmpty());

    // The page, through the settings window's registry
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    check("QD14: settings window", settings != nullptr);
    if (!settings) return;
    settings->setVisible(true);
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("networking")));
    spin(400);
    QQuickItem *page = findItem(settings->contentItem(), QStringLiteral("networkingPage"));
    check("QD14: the Networking page loads", page != nullptr);
    if (!page) return;
    const auto item = [&](const char *name) { return findItem(page, QString::fromLatin1(name)); };
    check("QD14: browsing while the page shows", hub->watching() && hub->lookForHubs());
    check("QD14: browsing started, or says why not", hub->browsing() || !hub->browseError().isEmpty());
    settings->grabWindow().save(m_out + QStringLiteral("/settings-networking.png"));

    // Sharing on, from the page's switch
    QQuickItem *share = item("networkingShare");
    check("QD14: the share switch", share != nullptr);
    if (share) QMetaObject::invokeMethod(share, "toggled", Q_ARG(bool, true));
    check("QD14: the switch starts a hub", waitFor(15000, [&]() { return hub->running() && !hub->busy(); }));
    check("QD14: the switch is kept", QSettings().value(QStringLiteral("hub/sharing")).toBool());
    check("QD14: no error", hub->lastError().isEmpty());
    if (!hub->lastError().isEmpty()) fprintf(stderr, "hub: %s\n", qPrintable(hub->lastError()));
    const QString address = hub->address();
    check("QD14: the address to dial", address == QStringLiteral("127.0.0.1:%1").arg(hub->boundPort()) && hub->boundPort() == hub->port());
    const QJsonObject status = hubJson(hub->handle(), dspi_hub_status);
    check("QD14: the hub has the page's name", status.value("name").toString() == hub->hubName());
    const QString serial = hubJson(hub->handle(), dspi_hub_devices).value("value").toArray().at(0).toObject().value("serial").toString();
    // -- WP QD36 -- The hub shares a demo of its own (attach_demo), not the app's: the
    // page says so, and the hub's one device is a demo
    const QJsonArray hubDevices = hubJson(hub->handle(), dspi_hub_devices).value("value").toArray();
    check("QD36: the hub shares its own demo device", hubDevices.size() == 1 && serial == QLatin1String(DemoDevice::kSerial));
    QQuickItem *demoNote = item("networkingDemoNote");
    check("QD36: the page says the hub's demo is its own", m_.bridge->selectedSerial() == QLatin1String(DemoDevice::kSerial)
              && demoNote && demoNote->isVisible() && demoNote->property("detail").toString().contains(QLatin1String("separate demo device")));
    check("QD14: the tray sees it running", hub->trayState().running && hub->trayState().address == address);
    check("QD14: the hub knows its id, and browsing leaves it out", !hub->hubId().isEmpty());

    if (!binary.isEmpty()) {
        Run run = terminal(binary, address, QString());
        check("QD14: with PIN access, no code, no way in", run.code > 0);
    }

    // Pair: the page's button opens a window, and the code is what the client types
    QQuickItem *pair = item("networkingPair");
    check("QD14: Pair a New Client is on while running", pair && pair->isVisible() && pair->isEnabled());
    if (pair) QMetaObject::invokeMethod(pair, "clicked");
    check("QD14: a pairing code shows", waitFor(5000, [&]() { return hub->pairingCode().size() == 6; })
              && hub->pairingSecondsLeft() > 0 && hub->trayState().pairingCode == hub->pairingCode());
    spin(200);
    QQuickItem *code = item("networkingPairingCode");
    check("QD14: the code is on the page, and Pair hides", code && code->isVisible() && pair && !pair->isVisible());
    settings->grabWindow().save(m_out + QStringLiteral("/settings-networking-pairing.png"));

    if (!binary.isEmpty()) {
        Run run = terminal(binary, address, hub->pairingCode());
        check("QD14: a real client pairs with the page's code", run.code == 0 && run.out.contains(serial));
        if (run.code != 0) fprintf(stderr, "dspi: exit %d: %s\n", run.code, qPrintable(run.out));
        check("QD14: the client is listed, and the code is gone",
              waitFor(5000, [&]() { return hub->clients().size() == 1 && hub->pairingCode().isEmpty(); }));
        spin(200);
        settings->grabWindow().save(m_out + QStringLiteral("/settings-networking-client.png"));
        const int cid = firstClient(hub).value("cid").toInt();
        hub->setRole(cid, QStringLiteral("viewer"));
        check("QD14: a role changes", waitFor(5000, [&]() { return firstClient(hub).value("role") == QLatin1String("viewer"); }));

        // The access warning: going open would cost this client
        hub->setSharing(false);
        check("QD14: sharing off stops the hub", waitFor(15000, [&]() { return !hub->running() && !hub->busy(); }));
        check("QD14: stopped, the clients come from the store", waitFor(5000, [&]() {
            return hub->clients().size() == 1 && hub->revokeCost(QStringLiteral("open")) == 1 && hub->revokeCost(QStringLiteral("pin")) == 0;
        }));
        QQuickItem *accessRow = item("networkingAccess");
        if (accessRow) QMetaObject::invokeMethod(accessRow, "chosen", Q_ARG(QVariant, QStringLiteral("open")));
        spin(200);
        QObject *dialog = settings->findChild<QObject *>(QStringLiteral("networkingAccessDialog"));
        if (!dialog) dialog = page->findChild<QObject *>(QStringLiteral("networkingAccessDialog"));
        check("QD14: going open asks first, with the cost", dialog && dialog->property("opened").toBool()
                  && dialog->property("message").toString().contains(QLatin1String("must pair again")));
        if (dialog) QMetaObject::invokeMethod(dialog, "chosen", Q_ARG(QString, QStringLiteral("cancel")));
        if (dialog) QMetaObject::invokeMethod(dialog, "close");
        check("QD14: cancelled, access is unchanged", hub->access() == QLatin1String("pin"));

        // Port: refused while running, taken while stopped
        check("QD14: a port below 1024 is refused", !hub->setPort(80));
        const int port = hub->port();
        check("QD14: a port is taken while stopped", hub->setPort(port));
        hub->dismissError();

        hub->setSharing(true);
        check("QD14: sharing on again", waitFor(15000, [&]() { return hub->running() && !hub->busy(); }));
        check("QD14: the port is refused while running", !hub->setPort(port == 40000 ? 40001 : 40000) && !hub->lastError().isEmpty());
        hub->dismissError();
        run = terminal(binary, hub->address(), QString());
        check("QD14: the client comes back on its token", run.code == 0 && run.out.contains(serial));

        // A rename restarts the hub under the new name; the client still gets in
        check("QD14: a rename is taken", hub->setHubName(QStringLiteral("Smoke studio")));
        check("QD14: the hub restarts with it", waitFor(15000, [&]() {
            return hub->running() && !hub->busy() && hubJson(hub->handle(), dspi_hub_status).value("name") == QLatin1String("Smoke studio");
        }));
        run = terminal(binary, hub->address(), QString());
        check("QD14: the client comes back after the restart", run.code == 0);

        hub->revoke(firstClient(hub).value("cid").toInt());
        check("QD14: revoking empties the list", waitFor(5000, [&]() { return hub->clients().isEmpty(); }));
        run = terminal(binary, hub->address(), QString());
        check("QD14: a revoked client is refused", run.code > 0);
    }

    // Cancel a pairing window from the tray's side
    hub->allowNewClient();
    check("QD14: a window from the tray", waitFor(5000, [&]() { return !hub->trayState().pairingCode.isEmpty(); }));
    hub->cancelPairing();
    check("QD14: cancelled", hub->pairingCode().isEmpty()
              && waitFor(5000, [&]() { return !hubJson(hub->handle(), dspi_hub_status).value("pairingOpen").toBool(); }));

    hub->setSharing(false);
    check("QD14: sharing off", waitFor(15000, [&]() { return !hub->running() && !hub->busy(); }));

    // A port in use: the switch stays on and the page says why
    QTcpServer squatter;
    if (squatter.listen(QHostAddress::LocalHost, 0)) {
        check("QD14: a free port is taken", hub->setPort(int(squatter.serverPort())));
        hub->setSharing(true);
        check("QD14: a port in use fails the start", waitFor(15000, [&]() { return !hub->busy() && !hub->lastError().isEmpty(); })
                  && !hub->running() && hub->sharing());
        spin(300);
        QQuickItem *error = item("networkingError");
        check("QD14: the page shows it, with the switch still on", error && error->isVisible() && share && share->property("checked").toBool());
        settings->grabWindow().save(m_out + QStringLiteral("/settings-networking-error.png"));
        hub->setSharing(false);
        squatter.close();
        check("QD14: switched off after the failure", waitFor(15000, [&]() { return !hub->sharing() && !hub->busy(); }));
    }

    // Leaving the page stops browsing
    QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("about")));
    spin(300);
    check("QD14: no browsing once the page is gone", !hub->watching() && !hub->browsing());
    settings->setVisible(false);
    QSettings().setValue(QStringLiteral("hub/sharing"), false);
}
