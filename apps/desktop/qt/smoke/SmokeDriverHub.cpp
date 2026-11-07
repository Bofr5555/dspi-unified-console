// WP HB6: the hub's C ABI from the app's side: a demo hub on loopback, with the Terminal
// (DSPI_SMOKE_TERMINAL, the `dspi` binary) as a real client over a real socket. The UI on
// top of this is QD14's. All of it blocks: the app keeps these calls off its GUI thread.
#include "SmokeDriver.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QProcessEnvironment>
#include <QStringList>

#include <atomic>
#include <functional>

extern "C" {
#include "dspi_core.h"
}

namespace {

QJsonObject reply(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(HUB_REPLY_MIN, Qt::Uninitialized);
    uint32_t needed = call(buf.data(), HUB_REPLY_MIN);
    if (needed > HUB_REPLY_MIN) {
        buf.resize(int(needed));
        needed = call(buf.data(), needed);
    }
    if (needed == 0) return {};
    return QJsonDocument::fromJson(QByteArray(buf.constData(), int(needed) - 1)).object();
}

struct Events {
    std::atomic<int> online { 0 };
    std::atomic<int> sessions { 0 };
    std::atomic<int> stopped { 0 };
    // What the module says a listener may do, done from the hub's thread
    std::atomic<int> readsOk { 0 };
    std::atomic<int> stopRefused { 0 };
    FfiHub *hub = nullptr;
};

void onHubEvent(uint8_t event, int32_t, const char *, void *userData) {
    auto *e = static_cast<Events *>(userData);
    if (event == HUB_EVENT_DEVICE_ONLINE) {
        e->online++;
        // A listener may read, and may not stop the hub it is called from.
        if (reply([e](char *b, uint32_t l) { return dspi_hub_status(e->hub, b, l); }).value("running").toBool())
            e->readsOk++;
        if (!dspi_hub_stop(e->hub)) e->stopRefused++;
    } else if (event == HUB_EVENT_SESSION_OPENED) {
        e->sessions++;
    } else if (event == HUB_EVENT_STOPPED) {
        e->stopped++;
    }
}

// One run of the Terminal against the hub: its exit code, and what it printed.
struct Run {
    int code = -1;
    QString out;
};

Run terminal(const QString &binary, int port, const QString &pin) {
    QProcess proc;
    // No PIN in the arguments, where `ps` would show it: DSPI_PIN is the Terminal's way.
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.remove(QStringLiteral("DSPI_PIN"));
    if (!pin.isEmpty()) env.insert(QStringLiteral("DSPI_PIN"), pin);
    proc.setProcessEnvironment(env);
    proc.setProcessChannelMode(QProcess::MergedChannels);
    proc.setStandardInputFile(QProcess::nullDevice());
    proc.start(binary, { QStringLiteral("--link"), QStringLiteral("127.0.0.1:%1").arg(port), QStringLiteral("dump") });
    if (!proc.waitForStarted(5000) || !proc.waitForFinished(30000)) {
        proc.kill();
        proc.waitForFinished(2000);
        return {};
    }
    return { proc.exitStatus() == QProcess::NormalExit ? proc.exitCode() : -1, QString::fromUtf8(proc.readAll()) };
}

} // namespace

void SmokeDriver::testHub() {
    const QString binary = qEnvironmentVariable("DSPI_SMOKE_TERMINAL");
    check("HB6: the Terminal is there (DSPI_SMOKE_TERMINAL)", !binary.isEmpty());
    if (binary.isEmpty()) return;

    const QByteArray store = QDir(m_out).filePath(QStringLiteral("hub-store")).toUtf8();
    const auto settings = [&]() {
        return reply([&](char *b, uint32_t l) { return dspi_hub_saved_settings(store.constData(), b, l); });
    };
    check("HB6: no store before the first start", settings().value("exists") == false);

    Events events;
    FfiHub *hub = dspi_hub_new();
    events.hub = hub;
    check("HB6: the handle", hub != nullptr);
    check("HB6: the listener", dspi_hub_set_listener(hub, onHubEvent, &events));

    const auto start = [&](uint8_t access) {
        return reply([&](char *b, uint32_t l) {
            return dspi_hub_start(hub, "Smoke hub", store.constData(), 0, access, HUB_OPT_LOOPBACK, b, l);
        });
    };

    // A buffer too short for the answer starts nothing
    char small[8];
    check("HB6: a short buffer starts nothing",
          dspi_hub_start(hub, "x", store.constData(), 0, HUB_ACCESS_PIN, HUB_OPT_LOOPBACK, small, sizeof small) == HUB_REPLY_MIN
              && reply([&](char *b, uint32_t l) { return dspi_hub_status(hub, b, l); }).value("running") == false);

    QJsonObject started = start(HUB_ACCESS_PIN);
    check("HB6: a demo hub starts", started.value("ok").toBool() && started.value("access") == QLatin1String("pin"));
    int port = started.value("port").toInt();
    check("HB6: on a port of the OS's choosing", port > 0);
    check("HB6: a demo device on it", dspi_hub_attach_demo(hub) == 0);
    const QString serial = reply([&](char *b, uint32_t l) { return dspi_hub_devices(hub, b, l); })
                               .value("value").toArray().at(0).toObject().value("serial").toString();
    check("HB6: the device is listed", !serial.isEmpty());
    check("HB6: the listener heard it, read, and could not stop the hub",
          waitFor(5000, [&]() { return events.stopRefused.load() == 1; }) && events.readsOk.load() == 1);

    // Sharing is PIN by default: no PIN, no way in
    Run run = terminal(binary, port, QString());
    check("HB6: the Terminal is refused with no PIN", run.code > 0);

    const auto clients = [&]() {
        return reply([&](char *b, uint32_t l) { return dspi_hub_clients(hub, b, l); }).value("value").toArray();
    };
    const QJsonObject window = reply([&](char *b, uint32_t l) { return dspi_hub_begin_pairing(hub, b, l); });
    const QString pin = window.value("pin").toString();
    check("HB6: a pairing window", pin.size() == 6 && window.value("expiresInMs").toInt() > 0);

    run = terminal(binary, port, pin);
    check("HB6: the Terminal connects with the PIN and reads the demo device", run.code == 0 && run.out.contains(serial));
    if (run.code != 0) fprintf(stderr, "dspi: exit %d: %s\n", run.code, qPrintable(run.out));
    check("HB6: the hub saw its session", waitFor(5000, [&]() { return events.sessions.load() > 0; }));
    check("HB6: the Terminal is a paired client", clients().size() == 1);
    check("HB6: a right PIN ends the window",
          reply([&](char *b, uint32_t l) { return dspi_hub_status(hub, b, l); }).value("pairingOpen") == false);

    const int cid = clients().at(0).toObject().value("cid").toInt();
    run = terminal(binary, port, QString());
    check("HB6: the Terminal comes back on its token", run.code == 0 && run.out.contains(serial));
    check("HB6: its role can change", dspi_hub_set_role(hub, uint32_t(cid), "viewer")
              && clients().at(0).toObject().value("role") == QLatin1String("viewer"));

    check("HB6: a client can be revoked", dspi_hub_revoke(hub, uint32_t(cid)) && clients().isEmpty());
    run = terminal(binary, port, QString());
    check("HB6: a revoked Terminal is refused", run.code > 0);

    // The warning a UI gives before changing the access mode, and the count after
    const QJsonObject again = reply([&](char *b, uint32_t l) { return dspi_hub_begin_pairing(hub, b, l); });
    run = terminal(binary, port, again.value("pin").toString());
    check("HB6: the Terminal pairs again", run.code == 0);
    check("HB6: the hub stops", dspi_hub_stop(hub));
    check("HB6: the listener heard the stop", events.stopped.load() == 1);
    check("HB6: no hub after the stop",
          reply([&](char *b, uint32_t l) { return dspi_hub_status(hub, b, l); }).value("running") == false);
    const QJsonObject saved = settings();
    check("HB6: going open would cost the paired client", saved.value("tokensRevokedIfOpen").toInt() == 1
              && saved.value("tokensRevokedIfPin").toInt() == 0 && saved.value("clients").toArray().size() == 1);

    started = start(HUB_ACCESS_OPEN);
    check("HB6: going open says what it cost", started.value("ok").toBool() && started.value("tokensRevoked").toInt() == 1);
    port = started.value("port").toInt();
    check("HB6: a demo device on the open hub", dspi_hub_attach_demo(hub) == 0);
    run = terminal(binary, port, QString());
    check("HB6: an open hub takes the Terminal with no PIN", run.code == 0 && run.out.contains(serial));

    dspi_hub_free(hub);
    check("HB6: free stops a running hub", events.stopped.load() == 2);
}
