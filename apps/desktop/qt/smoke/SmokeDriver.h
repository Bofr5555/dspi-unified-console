#ifndef SMOKEDRIVER_H
#define SMOKEDRIVER_H

#include <QObject>
#include <QStringList>
#include <functional>

class QQmlApplicationEngine;
class QQuickItem;
class QQuickWindow;
class QImage;
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
    void testImportAppliesOnce();
    void testSeam();
    void testAutoEq();
    void testWindows();
    void testSettingsPages();
    void testSweep();
    void testReconnect();
    void testIdleAndCost();
    void testBypassRepaint();
    // -- WP QD30 --
    void testPopoverAnchor();
    // -- WP QD4 --
    void testNames();
    // -- WP QD5 --
    void testOutputLatency();
    void testClockDiagAndCpuBars();
    // -- WP QD1 --
    void testCsRefusals();
    void testCsCallerRefusals();
    void testSaveBarRefusal();
    // -- WP CG16 --
    void testCsPages();
    // -- WP QD2 --
    void testPairMap();
    // -- WP QD6 --
    void testReadOnly();
    void testDeviceChanging();
    void testReadOnlyFixture(int wire);
    void readOnlyClickAround(QQuickWindow *window, const char *where, QQuickItem *scope = nullptr);
    // -- WP QD8 --
    void testLink();
    // -- WP QD12 --
    void testTray();
    void runTrayLaunch(const QString &mode);

    // Whether a bypass toggle repaints the open channel's graph editor. Leaves
    // bypass as it found it. NotRun fills `why`.
    enum class Outcome { Pass, Fail, NotRun };
    Outcome bypassRepaints(QString *why);
    QImage grabItem(QQuickItem *item);
    static int qmlErrors();

    // The hardware passes (DSPI_SMOKE_HW; SmokeDriverHw.cpp, SmokeDriverReplug.cpp)
    struct Baseline {
        QByteArray image;       // the bulk image, re-read from the device
        QByteArray perItem;     // what is outside it: preset directory, Core 1 mode, solo
    };
    void runHardware();
    void runReplug();
    bool replugCycle(int cycle, const QString &serial, Baseline *want);
    bool hwPreflight();
    bool readBaseline(Baseline *out);
    bool compareBaseline(const Baseline &want, const char *when);
    void hwRta();
    void hwRtaTap(int tap, const char *name);
    void hwControlSurfaces();
    void hwStats();
    void hwIoStatus();
    void hwMeters();
    void hwSiggenCaps();
    void hwBypass();
    void hwReselect(const Baseline &want);
    void pass(const QString &item, const QString &detail = QString());
    void fail(const QString &item, const QString &detail = QString());
    void notRun(const QString &item, const QString &why);
    void outcome(const QString &item, bool ok, const QString &detail = QString());
    void reading(const QString &item, bool ok, const QString &detail = QString());
    void finish();
    int m_passed = 0;
    int m_notRun = 0;

    App m_;
    int m_failures = 0;
    int m_checks = 0;
    QString m_out;
};

#endif // SMOKEDRIVER_H
