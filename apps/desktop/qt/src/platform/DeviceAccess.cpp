// -- WP QP16 -- Help for a DSPi that cannot be opened (DeviceAccess.h)
#include "DeviceAccess.h"

#include "Core.h"
#include "DeviceModel.h"

#include <QDir>
#include <QJSEngine>
#include <QProcess>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSaveFile>
#include <QStandardPaths>

static const Registrar::Controller<DeviceAccess> deviceAccess;

namespace {
constexpr int kSettleMs = 400;
// pkexec's own exit codes: the authentication was dismissed, or not given
constexpr int kPkexecDismissed = 126;
constexpr int kPkexecNotAuthorised = 127;

// The rule file goes in place and udev reads it, as root, in one pkexec call.
// The file's path is the script's first argument, never part of the script.
const char *const kInstallScript =
    "install -m 0644 \"$1\" /etc/udev/rules.d/70-dspi.rules && udevadm control --reload-rules && "
    "udevadm trigger --action=add --subsystem-match=usb";
} // namespace

DeviceAccess::DeviceAccess(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_settle.setSingleShot(true);
    m_settle.setInterval(kSettleMs);
    connect(&m_settle, &QTimer::timeout, this, &DeviceAccess::check);
    connect(ctx.core, &Core::deviceEvent, this, [this] { m_settle.start(); });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        if (!m_ctx.device->connected()) m_settle.start();
        else if (m_offered) applyCount(0);   // something opened: nothing left to help with
    });
    if (auto *app = qobject_cast<QQmlApplicationEngine *>(ctx.engine)) {
        connect(app, &QQmlApplicationEngine::objectCreated, this, [this](QObject *object, const QUrl &) {
            if (auto *window = qobject_cast<QQuickWindow *>(object)) wire(window);
        });
    }
}

DeviceAccess *DeviceAccess::create(QQmlEngine *, QJSEngine *) {
    DeviceAccess *self = Registrar::instance<DeviceAccess>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QString DeviceAccess::system() {
#if defined(Q_OS_LINUX)
    return QStringLiteral("linux");
#elif defined(Q_OS_WIN)
    return QStringLiteral("windows");
#else
    return QStringLiteral("other");
#endif
}

QString DeviceAccess::ruleText() {
    return QStringLiteral("# uConsole for DSPi: the person signed in at this seat may open a DSPi\n"
                          "# (USB vendor 2e8b), without root\n"
                          "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"2e8b\", TAG+=\"uaccess\"\n");
}

// The dialog lives in the main window, made once the window exists
void DeviceAccess::wire(QQuickWindow *window) {
    if (m_dialog) return;
    QQmlComponent component(m_ctx.engine, QStringLiteral("UConsole"), QStringLiteral("UdevDialog"));
    QObject *made = component.createWithInitialProperties({ { QStringLiteral("parent"), QVariant::fromValue(window->contentItem()) } });
    if (!made) {
        qWarning("DeviceAccess: the dialog did not load: %s", qPrintable(component.errorString()));
        return;
    }
    made->setParent(window->contentItem());
    m_dialog = made;
}

void DeviceAccess::setCountForTest(int count) { m_countForTest = count; }

void DeviceAccess::setInstallerForTest(Installer installer) { m_installer = std::move(installer); }

void DeviceAccess::check() {
    if (m_checking || m_ctx.device->connected() || m_ctx.device->remote()) return;
    m_checking = true;   // one question at a time
    const int fake = m_countForTest;
    m_ctx.core->worker()->call(
        this,
        [fake](FfiCore *c) {
            if (fake >= 0) return fake;
            dspi_scan_devices(c, nullptr, 0);
            return int(dspi_inaccessible_devices(c));
        },
        [this](int count) {
            m_checking = false;
            if (!m_ctx.device->connected()) applyCount(count);
        });
}

void DeviceAccess::applyCount(int count) {
    const bool rose = m_count == 0 && count > 0;
    m_count = count;
    if (count == 0) m_offered = false;
    else if (rose) {
        m_offered = true;
        m_installed = false;
        m_error.clear();
    }
    emit changed();
}

void DeviceAccess::dismiss() {
    if (!m_offered) return;
    m_offered = false;
    emit changed();
}

void DeviceAccess::offer() {
    // While a unit cannot be opened and nothing is open
    const bool helps = m_count > 0 && !m_offered && !m_ctx.device->connected();
    if (!helps) return;
    m_error = QString();
    m_offered = true;
    emit changed();
}

void DeviceAccess::rescan() {
    const int fake = m_countForTest;
    DeviceModel *device = m_ctx.device;
    m_ctx.core->worker()->call(
        this,
        [fake](FfiCore *c) {
            // The first unit the scan opens, if any
            DeviceInfo first{};
            QString serial;
            if (dspi_scan_devices(c, &first, 1) > 0) {
                const QByteArray raw(reinterpret_cast<const char *>(first.serial), int(sizeof first.serial));
                serial = QString::fromUtf8(raw.left(int(qMin<uint32_t>(first.serial_len, uint32_t(raw.size())))));
            }
            return std::make_pair(fake >= 0 ? fake : int(dspi_inaccessible_devices(c)), serial);
        },
        [this, device](std::pair<int, QString> r) {
            if (!device->connected() && !device->remote() && !r.second.isEmpty()) device->selectDevice(r.second);
            if (!device->connected()) applyCount(r.first);
        });
}

void DeviceAccess::installRule() {
    if (m_installing || system() != QLatin1String("linux")) return;
    // The rule file, written where only this account writes (never a shared temporary directory)
    const QDir cache(QStandardPaths::writableLocation(QStandardPaths::CacheLocation));
    cache.mkpath(QStringLiteral("."));
    const QString path = cache.filePath(QStringLiteral("70-dspi.rules"));
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly) || file.write(ruleText().toUtf8()) < 0 || !file.commit()) {
        m_error = tr("The rule was not installed: its file could not be written (%1).").arg(file.errorString());
        emit changed();
        return;
    }
    m_installing = true;
    m_error.clear();
    emit changed();
    const auto done = [this](int code, const QString &detail) { finished(code, detail); };
    if (m_installer) {
        m_installer(path, done);
        return;
    }
    // One outcome per run: how it exited, or that it never started
    auto *pkexec = new QProcess(this);
    connect(pkexec, &QProcess::finished, this, [this, pkexec](int code, QProcess::ExitStatus how) {
        const QByteArray said = pkexec->readAllStandardError();
        pkexec->deleteLater();
        finished(how == QProcess::CrashExit ? -1 : code, QString::fromLocal8Bit(said).simplified());
    });
    connect(pkexec, &QProcess::errorOccurred, this, [this, pkexec](QProcess::ProcessError e) {
        if (e == QProcess::FailedToStart) {
            pkexec->deleteLater();
            finished(-1, QString());
        }
    });
    pkexec->start(QStringLiteral("pkexec"), { QStringLiteral("/bin/sh"), QStringLiteral("-c"),
                                              QString::fromLatin1(kInstallScript), QStringLiteral("sh"), path });
}

void DeviceAccess::finished(int exitCode, const QString &detail) {
    m_installing = false;
    if (exitCode == 0) {
        m_installed = true;
        m_error.clear();
        emit changed();
        // udev applies the tag to the device as it is triggered: look again
        QTimer::singleShot(1500, this, &DeviceAccess::rescan);
        return;
    }
    if (exitCode == -1) m_error = tr("The rule was not installed: pkexec could not be started.");
    else if (exitCode == kPkexecDismissed) m_error = tr("The rule was not installed: the authentication was cancelled.");
    else if (exitCode == kPkexecNotAuthorised) m_error = tr("The rule was not installed: the authentication failed.");
    else if (!detail.isEmpty()) m_error = tr("The rule was not installed: %1").arg(detail);
    else m_error = tr("The rule was not installed (exit code %1).").arg(exitCode);
    emit changed();
}
