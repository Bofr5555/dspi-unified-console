#include "SystemIntegration.h"
#include "Registrar.h"
#include "TrayController.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QSaveFile>
#include <QStandardPaths>
#include <QTimer>
#include <utility>

#ifdef Q_OS_LINUX
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusUnixFileDescriptor>
#endif

#ifdef Q_OS_WIN
#include <windows.h>
#endif

static const Registrar::Controller<SystemIntegration> systemRegistered("system");

namespace {

const char *const kKeepAwakeKey = "system/keepAwake";
const char *const kReason = "Sharing DSPi devices on the local network";
// The Run key's value name, and the autostart file's; the latter matches the
// installed launcher's, so a desktop sees one app
const char *const kRunName = "DSPi uConsole";
const char *const kDesktopFile = "dspi-uconsole.desktop";

// ── XDG autostart ──

// One argument of an Exec line: quoted, with the characters the Desktop Entry
// spec reserves inside quotes escaped, % doubled, then escaped again as a string
// value (a backslash is written twice)
QString execArgument(const QString &path) {
    QString quoted;
    for (const QChar c : path) {
        if (c == QLatin1Char('"') || c == QLatin1Char('`') || c == QLatin1Char('$') || c == QLatin1Char('\\'))
            quoted += QLatin1Char('\\');
        quoted += c;
    }
    quoted = QLatin1Char('"') + quoted + QLatin1Char('"');
    quoted.replace(QLatin1Char('%'), QStringLiteral("%%"));
    quoted.replace(QLatin1Char('\\'), QStringLiteral("\\\\"));
    return quoted;
}

// -- WP QD13b -- The first argument of an Exec value, as execArgument's reverse:
// the string value's escapes, then quoting, then %%
QString execProgram(const QString &value) {
    QString unescaped;
    for (int i = 0; i < value.size(); i++) {
        if (value[i] != QLatin1Char('\\') || i + 1 == value.size()) { unescaped += value[i]; continue; }
        const QChar next = value[++i];
        unescaped += next == QLatin1Char('s') ? QChar(u' ') : next == QLatin1Char('n') ? QChar(u'\n')
                   : next == QLatin1Char('t') ? QChar(u'\t') : next == QLatin1Char('r') ? QChar(u'\r') : next;
    }
    const QString line = unescaped.trimmed();
    QString arg;
    if (line.startsWith(QLatin1Char('"'))) {
        for (int i = 1; i < line.size() && line[i] != QLatin1Char('"'); i++) {
            if (line[i] == QLatin1Char('\\') && i + 1 < line.size()) i++;
            arg += line[i];
        }
    } else {
        arg = line.section(QLatin1Char(' '), 0, 0);
    }
    arg.replace(QStringLiteral("%%"), QStringLiteral("%"));
    // A bare name, as a desktop's own startup tool writes it
    if (!arg.isEmpty() && !arg.contains(QLatin1Char('/'))) {
        const QString found = QStandardPaths::findExecutable(arg);
        if (!found.isEmpty()) return found;
    }
    return arg;
}

class XdgLoginItem : public LoginItem
{
public:
    explicit XdgLoginItem(const QString &dir) : m_dir(dir) {}

    QString file() const { return QDir(m_dir).filePath(QLatin1String(kDesktopFile)); }

    // The [Desktop Entry] keys of the file, "" values for keys it doesn't have
    QString key(const char *name) const {
        QFile f(file());
        if (!f.open(QIODevice::ReadOnly | QIODevice::Text)) return QString();
        bool inEntry = false;
        const QString prefix = QLatin1String(name) + QLatin1Char('=');
        while (!f.atEnd()) {
            const QString line = QString::fromUtf8(f.readLine()).trimmed();
            if (line.startsWith(QLatin1Char('['))) inEntry = line == QLatin1String("[Desktop Entry]");
            else if (inEntry && line.startsWith(prefix)) return line.mid(prefix.size());
        }
        return QString();
    }

    bool isOn() const override {
        // A desktop's own switch turns an entry off with one of these
        return QFile::exists(file()) && key("Hidden") != QLatin1String("true")
               && key("X-GNOME-Autostart-enabled") != QLatin1String("false");
    }

    bool matches(const QString &program) const override {
        return isOn() && key("Exec") == execArgument(program);
    }

    QString program() const override { return QFile::exists(file()) ? execProgram(key("Exec")) : QString(); }

    QString set(bool on, const QString &program) override {
        if (!on) {
            if (QFile::exists(file()) && !QFile::remove(file()))
                return QCoreApplication::translate("SystemIntegration", "Could not remove %1.").arg(file());
            return QString();
        }
        if (!QDir().mkpath(m_dir)) return QCoreApplication::translate("SystemIntegration", "Could not create %1.").arg(m_dir);
        QSaveFile f(file());
        if (!f.open(QIODevice::WriteOnly | QIODevice::Text))
            return QCoreApplication::translate("SystemIntegration", "Could not write %1: %2").arg(file(), f.errorString());
        const QString entry = QStringLiteral("[Desktop Entry]\n"
                                             "Type=Application\n"
                                             "Name=DSPi uConsole\n"
                                             "Comment=Configure and tune a DSPi audio processor\n"
                                             "Exec=%1\n"
                                             "Icon=dspi-uconsole\n"
                                             "Terminal=false\n"
                                             "X-GNOME-Autostart-enabled=true\n")
                                  .arg(execArgument(program));
        f.write(entry.toUtf8());
        if (!f.commit()) return QCoreApplication::translate("SystemIntegration", "Could not write %1: %2").arg(file(), f.errorString());
        return QString();
    }

private:
    QString m_dir;
};

// ── Windows' Run key ──

QString runCommand(const QString &program) {
    return QLatin1Char('"') + QDir::toNativeSeparators(program) + QLatin1Char('"');
}

class RunKeyLoginItem : public LoginItem
{
public:
    explicit RunKeyLoginItem(std::unique_ptr<QSettings> key) : m_key(std::move(key)) {}

    bool isOn() const override { return !m_key->value(QLatin1String(kRunName)).toString().isEmpty(); }

    bool matches(const QString &program) const override {
        return m_key->value(QLatin1String(kRunName)).toString() == runCommand(program);
    }

    // -- WP QD13b -- "path" args, or an unquoted path: up to .exe, else the first space
    QString program() const override {
        const QString value = m_key->value(QLatin1String(kRunName)).toString().trimmed();
        if (value.startsWith(QLatin1Char('"'))) return value.mid(1).section(QLatin1Char('"'), 0, 0);
        const int exe = value.indexOf(QLatin1String(".exe"), 0, Qt::CaseInsensitive);
        return exe >= 0 ? value.left(exe + 4) : value.section(QLatin1Char(' '), 0, 0);
    }

    QString set(bool on, const QString &program) override {
        if (on) m_key->setValue(QLatin1String(kRunName), runCommand(program));
        else m_key->remove(QLatin1String(kRunName));
        m_key->sync();
        if (m_key->status() != QSettings::NoError)
            return on ? QCoreApplication::translate("SystemIntegration", "Could not add the Console to the programs that start at sign-in.")
                      : QCoreApplication::translate("SystemIntegration", "Could not remove the Console from the programs that start at sign-in.");
        return QString();
    }

private:
    std::unique_ptr<QSettings> m_key;
};

// ── Windows' power request ──

#ifdef Q_OS_WIN
class PowerRequestInhibitor : public SleepInhibitor
{
public:
    ~PowerRequestInhibitor() override { release(); }

    void acquire(const QString &reason, std::function<void(const QString &)> done) override {
        release();
        std::wstring text = reason.toStdWString();
        REASON_CONTEXT context {};
        context.Version = POWER_REQUEST_CONTEXT_VERSION;
        context.Flags = POWER_REQUEST_CONTEXT_SIMPLE_STRING;
        context.Reason.SimpleReasonString = text.data();
        HANDLE request = PowerCreateRequest(&context);
        if (request == INVALID_HANDLE_VALUE) {
            done(QCoreApplication::translate("SystemIntegration", "Windows did not take the request (error %1).").arg(GetLastError()));
            return;
        }
        if (!PowerSetRequest(request, PowerRequestSystemRequired)) {
            const DWORD error = GetLastError();
            CloseHandle(request);
            done(QCoreApplication::translate("SystemIntegration", "Windows did not take the request (error %1).").arg(error));
            return;
        }
        m_request = request;
        done(QString());
    }

    void release() override {
        if (!m_request) return;
        PowerClearRequest(m_request, PowerRequestSystemRequired);
        CloseHandle(m_request);
        m_request = nullptr;
    }

private:
    HANDLE m_request = nullptr;
};
#endif

} // namespace

// ── The desktop portal, or logind ──

#ifdef Q_OS_LINUX
namespace {

const char *const kPortalPath = "/org/freedesktop/portal/desktop";
const char *const kLogindPath = "/org/freedesktop/login1";
constexpr uint kPortalInhibitSuspend = 4;

class DBusSleepInhibitor : public SleepInhibitor
{
public:
    DBusSleepInhibitor(const QDBusConnection &portalBus, const QString &portal, const QDBusConnection &logindBus,
                       const QString &logind)
        : m_bus(portalBus), m_portal(portal), m_logindBus(logindBus), m_logind(logind) {}
    ~DBusSleepInhibitor() override { release(); }

    void acquire(const QString &reason, std::function<void(const QString &)> done) override {
        release();
        const int generation = ++m_generation;
        QDBusMessage call = QDBusMessage::createMethodCall(m_portal, QLatin1String(kPortalPath),
                                                           QStringLiteral("org.freedesktop.portal.Inhibit"),
                                                           QStringLiteral("Inhibit"));
        call << QString() << kPortalInhibitSuspend << QVariantMap { { QStringLiteral("reason"), reason } };
        watch(m_bus.asyncCall(call), [this, generation, reason, done](QDBusPendingCallWatcher *w) {
            QDBusPendingReply<QDBusObjectPath> reply = *w;
            if (generation != m_generation) {
                // Released meanwhile: give back what came
                if (!reply.isError()) closeRequest(reply.value().path());
                return;
            }
            if (!reply.isError()) {
                m_request = reply.value().path();
                done(QString());
                return;
            }
            acquireLogind(generation, reason, reply.error().message(), done);
        });
    }

    void release() override {
        ++m_generation;
        if (!m_request.isEmpty()) closeRequest(m_request);
        m_request.clear();
        m_lock = QDBusUnixFileDescriptor();     // closing the fd ends logind's lock
    }

private:
    void acquireLogind(int generation, const QString &reason, const QString &portalError,
                       std::function<void(const QString &)> done) {
        QDBusMessage call = QDBusMessage::createMethodCall(m_logind, QLatin1String(kLogindPath),
                                                           QStringLiteral("org.freedesktop.login1.Manager"),
                                                           QStringLiteral("Inhibit"));
        call << QStringLiteral("idle") << QStringLiteral("DSPi uConsole") << reason << QStringLiteral("block");
        watch(m_logindBus.asyncCall(call), [this, generation, portalError, done](QDBusPendingCallWatcher *w) {
            QDBusPendingReply<QDBusUnixFileDescriptor> reply = *w;
            if (generation != m_generation) return;     // the fd closes with the reply
            if (!reply.isError() && reply.value().isValid()) {
                m_lock = reply.value();
                done(QString());
                return;
            }
            done(QCoreApplication::translate("SystemIntegration", "Neither the desktop portal nor logind took the request (%1; %2).")
                     .arg(portalError, reply.isError() ? reply.error().message() : QCoreApplication::translate("SystemIntegration", "no lock")));
        });
    }

    void closeRequest(const QString &path) {
        m_bus.asyncCall(QDBusMessage::createMethodCall(m_portal, path, QStringLiteral("org.freedesktop.portal.Request"),
                                                       QStringLiteral("Close")));
    }

    void watch(const QDBusPendingCall &call, std::function<void(QDBusPendingCallWatcher *)> then) {
        auto *w = new QDBusPendingCallWatcher(call, &m_context);
        QObject::connect(w, &QDBusPendingCallWatcher::finished, &m_context, [w, then]() {
            then(w);
            w->deleteLater();
        });
    }

    QDBusConnection m_bus;      // the session bus, the portal's
    QString m_portal;
    QDBusConnection m_logindBus;
    QString m_logind;
    QObject m_context;          // the watchers' parent: none outlives this object
    int m_generation = 0;
    QString m_request;          // the portal's request handle
    QDBusUnixFileDescriptor m_lock;
};

} // namespace

std::unique_ptr<SleepInhibitor> makeDBusSleepInhibitor(const QDBusConnection &portalBus, const QString &portalService,
                                                       const QDBusConnection &logindBus, const QString &logindService) {
    return std::make_unique<DBusSleepInhibitor>(portalBus, portalService, logindBus, logindService);
}
#endif

std::unique_ptr<LoginItem> makeXdgLoginItem(const QString &dir) { return std::make_unique<XdgLoginItem>(dir); }

std::unique_ptr<LoginItem> makeRunKeyLoginItem(std::unique_ptr<QSettings> key) {
    return std::make_unique<RunKeyLoginItem>(std::move(key));
}

// ── The controller ──

SystemIntegration::SystemIntegration(DSPiBridge *, QObject *parent) : QObject(parent)
{
#if defined(Q_OS_WIN)
    setLoginItem(makeRunKeyLoginItem(std::make_unique<QSettings>(
        QStringLiteral("HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"), QSettings::NativeFormat)));
    m_inhibitor = std::make_unique<PowerRequestInhibitor>();
#elif defined(Q_OS_LINUX)
    setLoginItem(makeXdgLoginItem(
        QDir(QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)).filePath(QStringLiteral("autostart"))));
    m_inhibitor = makeDBusSleepInhibitor(QDBusConnection::sessionBus(), QStringLiteral("org.freedesktop.portal.Desktop"),
                                         QDBusConnection::systemBus(), QStringLiteral("org.freedesktop.login1"));
#endif
    m_keepAwake = QSettings().value(QLatin1String(kKeepAwakeKey), false).toBool();

    // The hub registers alongside, in no set order: look once all are made
    QTimer::singleShot(0, this, [this]() {
        if (m_hub || !this->parent()) return;
        const auto hubs = this->parent()->findChildren<TrayHub *>(QString(), Qt::FindDirectChildrenOnly);
        if (!hubs.isEmpty()) setHub(hubs.first());
    });
}

SystemIntegration::~SystemIntegration() {
    if (m_inhibitor) m_inhibitor->release();
}

QString SystemIntegration::programPath() {
    const QString appImage = qEnvironmentVariable("APPIMAGE");
    return appImage.isEmpty() ? QCoreApplication::applicationFilePath() : appImage;
}

void SystemIntegration::setLoginItem(std::unique_ptr<LoginItem> item) {
    m_login = std::move(item);
    m_loginError.clear();
    m_loginOther.clear();
    // -- WP QD13b -- On, but for another program. Repointed here only when that one is
    // gone (an AppImage moved, an old folder removed): an entry that starts nothing
    // helps nobody. Another copy that is still there is the user's choice, and stays.
    if (m_login && m_login->isOn() && !m_login->matches(programPath())) {
        const QString recorded = m_login->program();
        // -- WP QD37 -- (review c4s2 N1) This very program spelled otherwise (unquoted,
        // a symlink) is this copy: nothing to say and nothing to rewrite
        const QString canonical = QFileInfo(recorded).canonicalFilePath();
        const bool thisCopy = !canonical.isEmpty() && canonical == QFileInfo(programPath()).canonicalFilePath();
        if (recorded.isEmpty() || !QFileInfo(recorded).isFile()) m_loginError = m_login->set(true, programPath());
        else if (!thisCopy) m_loginOther = recorded;
    }
    emit loginChanged();
}

void SystemIntegration::pointLoginHere() {
    if (!m_login) return;
    m_loginError = m_login->set(true, programPath());
    if (m_loginError.isEmpty()) m_loginOther.clear();
    emit loginChanged();
}

bool SystemIntegration::openAtLogin() const { return m_login && m_login->isOn(); }

void SystemIntegration::setOpenAtLogin(bool on) {
    if (!m_login) return;
    m_loginError = m_login->set(on, programPath());
    if (m_loginError.isEmpty()) m_loginOther.clear();
    emit loginChanged();
}

void SystemIntegration::setKeepAwake(bool on) {
    if (on == m_keepAwake) return;
    m_keepAwake = on;
    QSettings().setValue(QLatin1String(kKeepAwakeKey), on);
    emit awakeChanged();
    reconcileAwake();
}

void SystemIntegration::setSleepInhibitor(std::unique_ptr<SleepInhibitor> inhibitor) {
    if (m_inhibitor) m_inhibitor->release();
    m_inhibitor = std::move(inhibitor);
    m_wanted = m_held = false;
    ++m_generation;
    reconcileAwake();
}

void SystemIntegration::setHub(TrayHub *hub) {
    if (m_hub) m_hub->disconnect(this);
    m_hub = hub;
    if (m_hub) connect(m_hub, &TrayHub::trayStateChanged, this, &SystemIntegration::reconcileAwake);
    reconcileAwake();
}

// The hold follows the hub: in place exactly while it runs and the user asked
void SystemIntegration::reconcileAwake() {
    const bool want = m_inhibitor && m_keepAwake && m_hub && m_hub->trayState().running;
    if (want == m_wanted) return;
    m_wanted = want;
    const int generation = ++m_generation;
    if (!want) {
        m_inhibitor->release();
        m_held = false;
        m_awakeError.clear();
        emit awakeChanged();
        return;
    }
    m_inhibitor->acquire(QLatin1String(kReason), [this, generation](const QString &error) {
        if (generation != m_generation) return;
        m_held = error.isEmpty();
        m_awakeError = error;
        emit awakeChanged();
    });
}
