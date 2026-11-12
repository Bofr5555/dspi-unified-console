#ifndef SMOKEDRIVER_H
#define SMOKEDRIVER_H

#include <QObject>
#include <QString>
#include <functional>

class QQmlApplicationEngine;
class QQuickItem;
class QQuickWindow;
class Core;
class DeviceModel;
class Shell;

// Drives the real app (real QML, real controllers, the merged core) on the demo
// device, from inside its event loop, and exits with the number of failures.
// Only in the DSPiConsoleSmoke target (CMake DSPI_SMOKE).
//
// A work package adds its screens' checks as a section of its own, in its own
// file under smoke/, without editing this driver:
//
//     static void sidebarSection(SmokeDriver &d) {
//         d.check("qp1: the rows show the names", ...);
//     }
//     static const SmokeDriver::Section sidebar("qp1-sidebar", 100, &sidebarSection);
//
// Sections run in `order`, then by name; DSPI_SMOKE_SECTION=<name>[,<name>...]
// runs only those (run.sh passes it through). Object names follow the rule in
// NAMES.md.
class SmokeDriver : public QObject {
public:
    struct App {
        QQmlApplicationEngine *engine;
        Core *core;
        DeviceModel *device;
        Shell *shell;
    };
    using SectionFn = void (*)(SmokeDriver &driver);
    struct Section {
        Section(const char *name, int order, SectionFn run);
    };

    explicit SmokeDriver(const App &app) : m_(app) {}
    void start();

    // For the sections
    const App &app() const { return m_; }
    void check(const QString &what, bool ok);
    void spin(int ms);
    bool waitFor(int ms, const std::function<bool()> &done);
    // Until no core call is queued or running (DeviceModel.busy is false)
    bool waitIdle(int ms = 5000);
    QQuickWindow *mainWindow() const;
    // The first object named `name`: the main window's QObject children, then
    // its item tree (delegates of views are found there)
    QObject *object(const QString &name) const;
    QQuickItem *item(const QString &name, QQuickItem *scope = nullptr) const;
    // Saves a screenshot as <DSPI_SMOKE_OUT>/<file>; false if it came out empty
    bool grab(QQuickWindow *window, const QString &file);
    QString outDir() const { return m_out; }

private:
    void run();
    void section(const char *name);
    // Ends the last section and prints the run's section times
    void sectionTimes();
    static int qmlErrors();

    App m_;
    int m_failures = 0;
    int m_checks = 0;
    QString m_out;
};

#endif // SMOKEDRIVER_H
