#ifndef SMOKEDRIVER_H
#define SMOKEDRIVER_H

#include <QObject>
#include <QStringList>
#include <functional>

class QQmlApplicationEngine;
class QQuickWindow;
class DSPiBridge;
class RtaController;
class SiggenController;
class CsController;
class FirmwareUpdater;
class AutoEqLibrary;
class StatsController;
class MonitorModel;
class ConfigFiles;

// Drives the real app (real QML, real controllers, the merged core) on the demo
// device, from inside its event loop, and exits with the number of failures.
// Only in the DSPiConsoleSmoke target (CMake DSPI_SMOKE).
class SmokeDriver : public QObject {
public:
    struct App {
        QQmlApplicationEngine *engine;
        DSPiBridge *bridge;
        RtaController *rta;
        SiggenController *siggen;
        CsController *cs;
        FirmwareUpdater *firmware;
        AutoEqLibrary *autoeq;
        StatsController *stats;
        MonitorModel *monitor;
        ConfigFiles *config;
    };
    explicit SmokeDriver(const App &app) : m_(app) {}
    void start();

private:
    void run();
    void check(const char *what, bool ok);
    void section(const char *name);
    void spin(int ms);
    bool waitFor(int ms, const std::function<bool()> &done);
    QObject *root() const;
    QQuickWindow *window(const char *id) const;
    void openAndGrab(const char *toolName);

    void testState();
    void testSetters();
    void testRouting();
    void testCrossfeedRows();
    void testPresets();
    void testNotificationsAndMonitor();
    void testRta();
    void testControlSurfaces();
    void testSiggenAndStats();
    void testFiles();
    void testAutoEq();
    void testWindows();
    void testSettingsPages();
    void testSweep();
    void testReconnect();
    void testIdleAndCost();

    App m_;
    int m_failures = 0;
    int m_checks = 0;
    QString m_out;
};

#endif // SMOKEDRIVER_H
