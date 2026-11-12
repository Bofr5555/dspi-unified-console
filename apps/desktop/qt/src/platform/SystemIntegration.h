#ifndef SYSTEMINTEGRATION_H
#define SYSTEMINTEGRATION_H

#include <QObject>
#include <QPointer>
#include <QSettings>
#include <QString>
#include <QtQml/qqmlregistration.h>
#include <functional>
#include <memory>

#include "Registrar.h"

class HubController;
class QQmlEngine;
class QJSEngine;

// WP QD13: open at login, and keep awake while sharing (MenuBarMode.swift's
// SMAppService item, LinkService.swift's idleSystemSleepDisabled). -- WP QP14 --
// QML sees it as the singleton `SystemIntegration`; the two rows are in
// Settings > Networking > Running in the Background (behaviour.md §12.2, §12.3).

// Where the login entry lives. Each backend writes one entry and reads it back;
// the controller never caches what the system holds.
class LoginItem
{
public:
    virtual ~LoginItem() = default;
    virtual bool isOn() const = 0;
    // Writes (or rewrites) the entry for `program`, or removes it. "" on success,
    // else a sentence.
    virtual QString set(bool on, const QString &program) = 0;
    // On, and starting `program`
    virtual bool matches(const QString &program) const = 0;
    // The program the entry starts, as recorded (a bare name found on PATH is made
    // absolute); "" when there is no entry -- WP QD13b --
    virtual QString program() const = 0;
};

// XDG autostart: <dir>/dspi-uconsole.desktop, dir being $XDG_CONFIG_HOME/autostart
std::unique_ptr<LoginItem> makeXdgLoginItem(const QString &dir);
// Windows' Run key; `key` is HKCU\...\Run in the registry, an INI file in the tests
std::unique_ptr<LoginItem> makeRunKeyLoginItem(std::unique_ptr<QSettings> key);

// Holds the machine out of idle sleep. acquire() may answer later, on the UI
// thread: `done` gets "" when the hold is in place, else why not. release() drops
// a hold or one on its way, and never blocks.
class SleepInhibitor
{
public:
    virtual ~SleepInhibitor() = default;
    virtual void acquire(const QString &reason, std::function<void(const QString &error)> done) = 0;
    virtual void release() = 0;
};

#ifdef Q_OS_LINUX
#include <QDBusConnection>
// The desktop portal's Inhibit (Suspend) on the session bus, and logind's idle
// block lock on the system bus when there is no portal. Services are empty for a
// direct peer connection (the smoke driver's fake).
std::unique_ptr<SleepInhibitor> makeDBusSleepInhibitor(const QDBusConnection &portalBus, const QString &portalService,
                                                       const QDBusConnection &logindBus, const QString &logindService);
#endif

class SystemIntegration : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Open at login: read from the entry itself, so a user who removes it sees off
    Q_PROPERTY(bool loginSupported READ loginSupported CONSTANT)
    Q_PROPERTY(bool openAtLogin READ openAtLogin WRITE setOpenAtLogin NOTIFY loginChanged)
    Q_PROPERTY(QString loginError READ loginError NOTIFY loginChanged)
    // -- WP QD13b -- On, but for another copy of the app that is still there: its
    // path. Left alone until the user picks this copy (pointLoginHere). An entry for
    // a copy that is gone is repointed here at start.
    Q_PROPERTY(QString loginOtherProgram READ loginOtherProgram NOTIFY loginChanged)
    Q_PROPERTY(QString thisProgram READ programPath CONSTANT)
    // Keep awake while sharing: the user's choice (QSettings system/keepAwake), and
    // whether a hold is in place now
    Q_PROPERTY(bool keepAwakeSupported READ keepAwakeSupported CONSTANT)
    Q_PROPERTY(bool keepAwake READ keepAwake WRITE setKeepAwake NOTIFY awakeChanged)
    Q_PROPERTY(bool holdingAwake READ holdingAwake NOTIFY awakeChanged)
    Q_PROPERTY(QString awakeError READ awakeError NOTIFY awakeChanged)

public:
    SystemIntegration(const AppContext &ctx, QObject *parent);
    ~SystemIntegration() override;
    static SystemIntegration *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<SystemIntegration>(); }

    bool loginSupported() const { return m_login != nullptr; }
    bool openAtLogin() const;
    void setOpenAtLogin(bool on);
    QString loginError() const { return m_loginError; }
    QString loginOtherProgram() const { return m_loginOther; }
    Q_INVOKABLE void pointLoginHere();

    bool keepAwakeSupported() const { return m_inhibitor != nullptr; }
    bool keepAwake() const { return m_keepAwake; }
    void setKeepAwake(bool on);
    bool holdingAwake() const { return m_held; }
    QString awakeError() const { return m_awakeError; }

    // What the entry starts: the AppImage when run from one, else this executable
    static QString programPath();

    // For the smoke driver: other backends, and what the hold follows. The app's
    // HubController is followed from the first turn of the event loop; null stops
    // following, and setHubRunning() then stands in for it.
    void setLoginItem(std::unique_ptr<LoginItem> item);
    void setSleepInhibitor(std::unique_ptr<SleepInhibitor> inhibitor);
    void followHub(HubController *hub);
    HubController *hub() const { return m_hub; }
    void setHubRunning(bool running);

signals:
    void loginChanged();
    void awakeChanged();

private:
    void reconcileAwake();

    std::unique_ptr<LoginItem> m_login;
    QString m_loginError;
    QString m_loginOther;

    std::unique_ptr<SleepInhibitor> m_inhibitor;
    QPointer<HubController> m_hub;
    bool m_hubRunning = false;  // sharing is on and the hub runs
    bool m_keepAwake = false;
    bool m_wanted = false;      // a hold is asked for or in place
    bool m_held = false;        // and in place
    int m_generation = 0;       // answers to an older acquire are dropped
    QString m_awakeError;
};

#endif // SYSTEMINTEGRATION_H
