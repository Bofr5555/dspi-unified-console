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
// Only in the uconsole-for-dspi-smoke target (CMake DSPI_SMOKE).
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
//
// -- WP QP19 -- A launch of its own (run.sh starts each) runs only the sections
// of its mode, chosen by the environment (behaviour.md §23):
//
//     static const SmokeDriver::Section tray("qp19-tray", 10, &traySection, SmokeDriver::Mode::Tray);
//
// A mode with no section in this build exits 2.
class SmokeDriver : public QObject {
public:
    enum class Mode {
        Main,        // --demo, the sections of every work package
        Hardware,    // DSPI_SMOKE_HW=1 (a real DSPi) or dry (the same pass on the demo)
        Replug,      // DSPI_SMOKE_HW=replug, the owner unplugs and replugs
        DemoEntry,   // DSPI_SMOKE_DEMO_ENTRY, started without --demo
        Tray,        // DSPI_SMOKE_TRAY=hidden|hidden-quit|app-quit|no-tray
        Pseudo,      // DSPI_SMOKE_PSEUDO=en|pseudo
        SelfTest,    // DSPI_SMOKE_SELFTEST=thread|noquit: the driver's own failure paths fire
    };
    struct App {
        QQmlApplicationEngine *engine;
        Core *core;
        DeviceModel *device;
        Shell *shell;
    };
    using SectionFn = void (*)(SmokeDriver &driver);
    struct Section {
        Section(const char *name, int order, SectionFn run, Mode mode = Mode::Main);
    };
    // This launch's mode, and the value of its variable ("dry", "hidden", ...)
    static Mode mode();
    static QString modeValue();

    explicit SmokeDriver(const App &app) : m_(app) {}
    void start();

    // For the sections
    const App &app() const { return m_; }
    void check(const QString &what, bool ok);
    // -- WP QP19 -- An item the hardware pass leaves out, with why; the summary
    // counts them (", <n> not run")
    void notRun(const QString &what, const QString &why);
    // A launch that ends the app its own way (the tray's): prints the summary
    // now, keeps the failures for the exit, and exits 3 when the app has not
    // quit `withinMs` later
    void expectQuit(int withinMs);
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
    // -- WP ATQTSIM -- This section may run up to `seconds` (a whole measurement
    // across units); the run's limit still applies when it is longer. Ends with
    // the section.
    void allowSectionTime(int seconds);

private:
    void run();
    void section(const char *name);
    // Ends the last section and prints the run's section times
    void sectionTimes();
    void summary();
    static int qmlErrors();

    App m_;
    int m_failures = 0;
    int m_checks = 0;
    int m_notRun = 0;
    bool m_quitExpected = false;
    QString m_out;
};

#endif // SMOKEDRIVER_H
