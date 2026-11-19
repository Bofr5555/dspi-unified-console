#ifndef DEVICEACCESS_H
#define DEVICEACCESS_H

#include <QObject>
#include <QPointer>
#include <QString>
#include <QTimer>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;
class QQuickWindow;

// -- WP QP16 -- Help for a DSPi the app sees but cannot open (behaviour.md §13,
// dspi_inaccessible_devices).
//
// While no device is open, every hot-plug event and the start-up read ask the
// core how many units its scan could not open. When that goes above zero the
// dialog (qml/platform/UdevDialog.qml) offers itself, once until the count
// drops to zero again:
// - Linux: it installs the device rule (70-dspi.rules: vendor 2e8b and the
//   older firmware's 2e8a:feaa, the uaccess tag) through pkexec, one authentication, then scans again. A
//   declined or failed pkexec says what failed and leaves nothing installed.
//   The packages install the same rule (-- WP QTCLI -- the same file: the
//   text is packaging/linux/70-uconsole-for-dspi.rules); the dialog matters for the AppImage
//   and the tarball.
// - Windows: advice (other programs holding the device; Zadig's WinUSB driver
//   for very old firmware), and a new scan.
// - Elsewhere: that the device cannot be opened.
class DeviceAccess : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(int count READ count NOTIFY changed)
    Q_PROPERTY(bool offered READ offered NOTIFY changed)
    // "linux", "windows" or "other"
    Q_PROPERTY(QString system READ system CONSTANT)
    Q_PROPERTY(bool installing READ installing NOTIFY changed)
    Q_PROPERTY(bool installed READ installed NOTIFY changed)
    Q_PROPERTY(QString error READ error NOTIFY changed)
public:
    // Runs the install of the rule file at `path`; `done(exitCode, detail)`
    // with pkexec's exit code (-1: it did not start)
    using Installer = std::function<void(const QString &path, std::function<void(int, const QString &)> done)>;

    DeviceAccess(const AppContext &ctx, QObject *parent = nullptr);
    static DeviceAccess *create(QQmlEngine *, QJSEngine *);

    int count() const { return m_count; }
    bool offered() const { return m_offered; }
    static QString system();
    bool installing() const { return m_installing; }
    bool installed() const { return m_installed; }
    QString error() const { return m_error; }

    // The rule as it is installed
    static QString ruleText();
    static QString rulePath() { return QStringLiteral("/etc/udev/rules.d/70-dspi.rules"); }

    Q_INVOKABLE void installRule();
    // Scans again (and opens the first DSPi found when none is open)
    Q_INVOKABLE void rescan();
    Q_INVOKABLE void dismiss();
    // -- WP QPI3 -- the status area's offer: the dialog again, while a unit
    // still cannot be opened
    Q_INVOKABLE void offer();

    // The smoke driver's: a count instead of the core's (-1: the core's), and an
    // installer instead of pkexec
    void setCountForTest(int count);
    void setInstallerForTest(Installer installer);
    // Asks the core now (otherwise on hot-plug events and the start-up read)
    void check();

signals:
    void changed();

private:
    void wire(QQuickWindow *window);
    void applyCount(int count);
    void finished(int exitCode, const QString &detail);

    AppContext m_ctx;
    int m_count = 0;
    bool m_offered = false;
    bool m_installing = false;
    bool m_installed = false;
    QString m_error;
    bool m_checking = false;
    QTimer m_settle;   // hot-plug events come in bursts: one check after them
    int m_countForTest = -1;
    Installer m_installer;
    QPointer<QObject> m_dialog;
};

#endif // DEVICEACCESS_H
