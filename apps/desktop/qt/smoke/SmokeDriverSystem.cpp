// WP QD13: open at login and keep awake while sharing. The autostart entry is the
// real backend's, written under this run's XDG_CONFIG_HOME; the Run key's writer runs
// against an INI file; the inhibit calls go to a fake portal and logind on a private
// D-Bus server. What a real desktop and Windows do with them is QD35's hand check.
#include "SmokeDriver.h"

#include "SystemIntegration.h"
#include "TrayController.h"

#include <QCoreApplication>
#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QDBusServer>
#include <QDBusUnixFileDescriptor>
#include <QDBusVirtualObject>
#include <QDir>
#include <QFile>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QStandardPaths>
#include <fcntl.h>
#include <memory>
#include <unistd.h>

namespace {

class FakeHub : public TrayHub {
public:
    using TrayHub::TrayHub;
    State state;
    State trayState() const override { return state; }
    void setSharing(bool on) override {
        state.sharing = state.running = on;
        emit trayStateChanged();
    }
    void allowNewClient() override {}
    void cancelPairing() override {}
};

// org.freedesktop.portal.Inhibit and org.freedesktop.login1.Manager.Inhibit, as far
// as the inhibitor uses them
class FakePower : public QDBusVirtualObject {
public:
    struct Call {
        QString path, interface, member;
        QVariantList args;
    };
    QList<Call> calls;
    bool portalFails = false;
    bool logindFails = false;
    bool holdPortalReply = false;
    QDBusMessage heldReply;
    QDBusConnection heldOn { QString() };
    int lockReadEnd = -1;           // the other end of the fd logind handed out
    int requests = 0;

    QString introspect(const QString &) const override { return QString(); }

    bool handleMessage(const QDBusMessage &m, const QDBusConnection &c) override {
        QDBusConnection connection(c);
        calls.append({ m.path(), m.interface(), m.member(), m.arguments() });
        if (m.member() == QLatin1String("Inhibit") && m.interface() == QLatin1String("org.freedesktop.portal.Inhibit")) {
            if (portalFails) {
                connection.send(m.createErrorReply(QStringLiteral("org.freedesktop.DBus.Error.ServiceUnknown"), QStringLiteral("no portal")));
                return true;
            }
            QDBusMessage reply = m.createReply(QVariant::fromValue(QDBusObjectPath(
                QStringLiteral("/org/freedesktop/portal/desktop/request/1/qd13_%1").arg(++requests))));
            if (holdPortalReply) {
                m.setDelayedReply(true);
                heldReply = reply;
                heldOn = connection;
            } else {
                connection.send(reply);
            }
            return true;
        }
        if (m.member() == QLatin1String("Inhibit") && m.interface() == QLatin1String("org.freedesktop.login1.Manager")) {
            if (logindFails) {
                connection.send(m.createErrorReply(QStringLiteral("org.freedesktop.DBus.Error.AccessDenied"), QStringLiteral("no logind")));
                return true;
            }
            int fds[2];
            if (::pipe2(fds, O_NONBLOCK | O_CLOEXEC) != 0) return false;
            if (lockReadEnd >= 0) ::close(lockReadEnd);
            lockReadEnd = fds[0];
            connection.send(m.createReply(QVariant::fromValue(QDBusUnixFileDescriptor(fds[1]))));
            ::close(fds[1]);    // the descriptor holds a copy; the app's is then the only one
            return true;
        }
        if (m.member() == QLatin1String("Close") && m.interface() == QLatin1String("org.freedesktop.portal.Request")) {
            connection.send(m.createReply());
            return true;
        }
        return false;
    }

    int count(const char *interface, const char *member, const QString &path = QString()) const {
        int n = 0;
        for (const Call &call : calls)
            if (call.interface == QLatin1String(interface) && call.member == QLatin1String(member) && (path.isEmpty() || call.path == path))
                n++;
        return n;
    }
    Call last(const char *interface, const char *member) const {
        for (int i = calls.size() - 1; i >= 0; i--)
            if (calls[i].interface == QLatin1String(interface) && calls[i].member == QLatin1String(member)) return calls[i];
        return {};
    }
    // Whether every write end of the lock's pipe is closed, as logind sees a lock end;
    // the read end does not block
    bool lockClosed() const {
        if (lockReadEnd < 0) return false;
        char byte;
        return ::read(lockReadEnd, &byte, 1) == 0;
    }
};

QQuickItem *findNamed(QQuickItem *in, const QString &name) {
    if (!in) return nullptr;
    if (in->objectName() == name) return in;
    for (QQuickItem *child : in->childItems())
        if (QQuickItem *found = findNamed(child, name)) return found;
    return nullptr;
}

QString fileText(const QString &path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? QString::fromUtf8(f.readAll()) : QString();
}

} // namespace

void SmokeDriver::testSystemIntegration() {
    section("QD13: open at login, keep awake");
    auto *system = qobject_cast<SystemIntegration *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("system")).value<QObject *>());
    check("QD13: controller registered as system", system != nullptr);
    if (!system) return;
    check("QD13: both supported here (Linux)", system->loginSupported() && system->keepAwakeSupported());
    check("QD13: it found the hub among the controllers",
          system->hub() && system->hub() == m_.engine->rootContext()->contextProperty(QStringLiteral("hub")).value<QObject *>());

    // ── XDG autostart, the real backend ──
    const QString autostart = QDir(QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)).filePath(QStringLiteral("autostart"));
    const QString entry = autostart + QStringLiteral("/dspi-console.desktop");
    check("QD13: under this run's XDG_CONFIG_HOME", autostart.startsWith(qEnvironmentVariable("XDG_CONFIG_HOME")));
    check("QD13: off to start with", !system->openAtLogin() && !QFile::exists(entry));
    system->setOpenAtLogin(true);
    const QString program = SystemIntegration::programPath();
    const QString text = fileText(entry);
    check("QD13: on writes the entry", system->openAtLogin() && system->loginError().isEmpty());
    check("QD13: the entry is an application", text.startsWith(QLatin1String("[Desktop Entry]\nType=Application\nName=DSPi Console\n"))
          && text.contains(QLatin1String("\nIcon=dspi-console\n")) && text.contains(QLatin1String("\nX-GNOME-Autostart-enabled=true\n")));
    check("QD13: it starts this program, quoted", text.contains(QStringLiteral("\nExec=\"%1\"\n").arg(program)));
    check("QD13: the program is this executable outside an AppImage", program == QCoreApplication::applicationFilePath());

    // A desktop's own switch turns it off by editing it
    {
        QFile f(entry);
        if (f.open(QIODevice::Append)) f.write("Hidden=true\n");
    }
    check("QD13: Hidden=true reads as off", !system->openAtLogin());
    system->setOpenAtLogin(true);
    check("QD13: on again rewrites it", system->openAtLogin() && !fileText(entry).contains(QLatin1String("Hidden")));

    // An entry for a program that has moved is pointed here at start
    {
        QFile f(entry);
        if (f.open(QIODevice::WriteOnly)) f.write("[Desktop Entry]\nType=Application\nExec=\"/old/place/dspi-console\"\n");
    }
    system->setLoginItem(makeXdgLoginItem(autostart));
    check("QD13: a moved program is rewritten", fileText(entry).contains(QStringLiteral("\nExec=\"%1\"\n").arg(program)));

    system->setOpenAtLogin(false);
    check("QD13: off removes the entry", !system->openAtLogin() && !QFile::exists(entry) && system->loginError().isEmpty());
    system->setOpenAtLogin(false);
    check("QD13: off twice is no error", system->loginError().isEmpty());

    // The page's two switches
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    check("QD13: settings window", settings != nullptr);
    if (settings) {
        const bool wasVisible = settings->isVisible();
        settings->setVisible(true);
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QStringLiteral("networking")));
        spin(400);
        QQuickItem *page = findNamed(settings->contentItem(), QStringLiteral("networkingPage"));
        QQuickItem *loginRow = findNamed(page, QStringLiteral("networkingOpenAtLogin"));
        QQuickItem *awakeRow = findNamed(page, QStringLiteral("networkingKeepAwake"));
        check("QD13: both rows on the Networking page", loginRow && awakeRow && loginRow->isVisible() && awakeRow->isVisible());
        if (loginRow && awakeRow) {
            QMetaObject::invokeMethod(loginRow, "toggled", Q_ARG(bool, true));
            check("QD13: the login switch writes the entry", system->openAtLogin() && QFile::exists(entry)
                  && loginRow->property("checked").toBool());
            QMetaObject::invokeMethod(loginRow, "toggled", Q_ARG(bool, false));
            check("QD13: and removes it", !system->openAtLogin() && !QFile::exists(entry) && !loginRow->property("checked").toBool());
            QMetaObject::invokeMethod(awakeRow, "toggled", Q_ARG(bool, true));
            check("QD13: the awake switch sets the choice", system->keepAwake() && awakeRow->property("checked").toBool());
            QMetaObject::invokeMethod(awakeRow, "toggled", Q_ARG(bool, false));
            check("QD13: and clears it", !system->keepAwake() && !awakeRow->property("checked").toBool());
        }
        settings->setVisible(wasVisible);
    }

    // Exec quoting (Desktop Entry spec): ", `, $ and \ escaped in quotes, % doubled,
    // then each backslash written twice as a string value
    const QString odd = QDir(m_out).filePath(QStringLiteral("qd13-odd"));
    auto oddItem = makeXdgLoginItem(odd);
    oddItem->set(true, QStringLiteral("/opt/My Apps/a\"b`c$d\\e%f/dspi"));
    check("QD13: Exec quotes the reserved characters",
          fileText(odd + QStringLiteral("/dspi-console.desktop")).contains(QLatin1String("\nExec=\"/opt/My Apps/a\\\\\"b\\\\`c\\\\$d\\\\\\\\e%%f/dspi\"\n")));
    check("QD13: and reads back as the same program", oddItem->matches(QStringLiteral("/opt/My Apps/a\"b`c$d\\e%f/dspi")));

    // A directory that cannot be made
    {
        QFile blocker(QDir(m_out).filePath(QStringLiteral("qd13-blocker")));
        check("QD13: a file where the directory would go", blocker.open(QIODevice::WriteOnly));
    }
    auto blocked = makeXdgLoginItem(QDir(m_out).filePath(QStringLiteral("qd13-blocker/autostart")));
    check("QD13: a failure is a sentence, not a crash", !blocked->set(true, program).isEmpty() && !blocked->isOn());

    // ── Windows' Run key, written to an INI file ──
    const QString ini = QDir(m_out).filePath(QStringLiteral("qd13-run.ini"));
    QFile::remove(ini);
    auto run = makeRunKeyLoginItem(std::make_unique<QSettings>(ini, QSettings::IniFormat));
    const QString exe = QStringLiteral("C:\\Program Files\\DSPi Console\\DSPiConsole.exe");
    check("QD13: Run key: off to start with", !run->isOn());
    check("QD13: Run key: on", run->set(true, exe).isEmpty() && run->isOn() && run->matches(exe));
    check("QD13: Run key: the value is the quoted exe",
          QSettings(ini, QSettings::IniFormat).value(QStringLiteral("DSPi Console")).toString() == QLatin1Char('"') + exe + QLatin1Char('"'));
    check("QD13: Run key: another exe does not match", !run->matches(QStringLiteral("C:\\Other\\DSPiConsole.exe")));
    check("QD13: Run key: off removes the value", run->set(false, exe).isEmpty() && !run->isOn()
          && !QSettings(ini, QSettings::IniFormat).contains(QStringLiteral("DSPi Console")));

    // ── Keep awake: the portal, then logind ──
    const QString scratch = QDir(m_out).filePath(QStringLiteral("qd13"));
    QDir().mkpath(scratch);
    QDBusServer server(QStringLiteral("unix:path=") + scratch + QStringLiteral("/power.sock"));
    server.setAnonymousAuthenticationAllowed(true);
    check("QD13: a D-Bus server for the fake services", server.isConnected());
    auto fake = std::make_shared<FakePower>();
    QObject::connect(&server, &QDBusServer::newConnection, &server, [fake](const QDBusConnection &c) {
        QDBusConnection(c).registerVirtualObject(QStringLiteral("/"), fake.get(), QDBusConnection::SubPath);
    });
    QDBusConnection peer = QDBusConnection::connectToPeer(server.address(), QStringLiteral("qd13-power"));
    check("QD13: connected to the fake services", waitFor(3000, [&] { return peer.isConnected(); }));

    FakeHub hub;
    TrayHub *realHub = system->hub();
    const bool keptAwake = system->keepAwake();
    system->setKeepAwake(false);
    system->setSleepInhibitor(makeDBusSleepInhibitor(peer, QString(), peer, QString()));
    system->setHub(&hub);
    const char *portal = "org.freedesktop.portal.Inhibit";
    const char *logind = "org.freedesktop.login1.Manager";
    const char *request = "org.freedesktop.portal.Request";

    hub.setSharing(true);
    spin(200);
    check("QD13: off by default: sharing asks for nothing", fake->calls.isEmpty() && !system->holdingAwake());
    system->setKeepAwake(true);
    check("QD13: the choice is kept", QSettings().value(QStringLiteral("system/keepAwake")).toBool());
    check("QD13: on while sharing: the portal holds it", waitFor(3000, [&] { return system->holdingAwake(); }));
    const FakePower::Call inhibit = fake->last(portal, "Inhibit");
    check("QD13: the portal call: no window, Suspend, a reason", inhibit.path == QLatin1String("/org/freedesktop/portal/desktop")
          && inhibit.args.size() == 3 && inhibit.args.at(0).toString().isEmpty() && inhibit.args.at(1).toUInt() == 4
          && qdbus_cast<QVariantMap>(inhibit.args.at(2)).value(QStringLiteral("reason")).toString() == QLatin1String("Sharing DSPi devices on the local network"));
    check("QD13: one call, no logind", fake->count(portal, "Inhibit") == 1 && fake->count(logind, "Inhibit") == 0);

    hub.setSharing(false);
    check("QD13: sharing off closes the request", waitFor(3000, [&] {
        return fake->count(request, "Close", QStringLiteral("/org/freedesktop/portal/desktop/request/1/qd13_1")) == 1;
    }) && !system->holdingAwake() && system->awakeError().isEmpty());

    // No portal: logind's idle lock, dropped by closing its fd
    fake->portalFails = true;
    hub.setSharing(true);
    check("QD13: no portal: logind holds it", waitFor(3000, [&] { return system->holdingAwake(); }));
    const FakePower::Call lock = fake->last(logind, "Inhibit");
    check("QD13: the logind call: idle, block", lock.path == QLatin1String("/org/freedesktop/login1") && lock.args.size() == 4
          && lock.args.at(0).toString() == QLatin1String("idle") && lock.args.at(1).toString() == QLatin1String("DSPi Console")
          && lock.args.at(3).toString() == QLatin1String("block"));
    check("QD13: the lock is open while held", !fake->lockClosed());
    system->setKeepAwake(false);
    check("QD13: keep awake off closes the lock", waitFor(1000, [&] { return fake->lockClosed(); }) && !system->holdingAwake());

    // Neither: a sentence, and nothing held
    fake->logindFails = true;
    system->setKeepAwake(true);
    check("QD13: neither service: says so", waitFor(3000, [&] { return !system->awakeError().isEmpty(); }) && !system->holdingAwake());
    system->setKeepAwake(false);
    check("QD13: off clears the error", system->awakeError().isEmpty());

    // Sharing stops before the portal answers: the late handle is given back
    fake->portalFails = fake->logindFails = false;
    fake->holdPortalReply = true;
    system->setKeepAwake(true);
    check("QD13: the call is on its way", waitFor(3000, [&] { return fake->heldReply.type() == QDBusMessage::ReplyMessage; }));
    hub.setSharing(false);
    const QString late = fake->heldReply.arguments().value(0).value<QDBusObjectPath>().path();
    fake->heldOn.send(fake->heldReply);
    check("QD13: a late answer is closed at once", waitFor(3000, [&] { return fake->count(request, "Close", late) == 1; })
          && !system->holdingAwake());

    // As it was: the app's hub and backend, the user's choice
    system->setHub(realHub);
    system->setKeepAwake(keptAwake);
    QSettings().remove(QStringLiteral("system/keepAwake"));
    if (fake->lockReadEnd >= 0) ::close(fake->lockReadEnd);
    QDBusConnection::disconnectFromPeer(QStringLiteral("qd13-power"));
}
