#include "SmokeDriver.h"

#include "AutoEqLibrary.h"
#include "ConfigFiles.h"
#include "CsController.h"
#include "DSPiBridge.h"
#include "FirmwareUpdater.h"
#include "MonitorModel.h"
#include "PeqEditorItem.h"
#include "RtaController.h"
#include "SiggenController.h"
#include "StatsController.h"

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFileInfo>
#include <QImage>
#include <QMetaMethod>
#include <QQmlApplicationEngine>
// -- WP QD1 --
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItemGrabResult>
#include <QSharedPointer>
#include <QQuickWindow>
#include <QTimer>
#include <QUrl>
#include <QVariant>
// -- WP QD30 --
#include <QCursor>
#include <QMouseEvent>
#include <QQmlComponent>
#include <QQuickItem>
#include <cstdio>

// QML runtime errors fail the run (TypeError, ReferenceError, a broken binding)
static int g_qmlErrors = 0;
static QtMessageHandler g_previous = nullptr;

static void messageHandler(QtMsgType type, const QMessageLogContext &ctx, const QString &msg) {
    if (type != QtDebugMsg && msg.contains(QLatin1String("qrc:/")) && !msg.contains(QLatin1String("deprecated"))) {
        g_qmlErrors++;
        fprintf(stderr, "FAIL qml: %s\n", qPrintable(msg));
    }
    if (g_previous) g_previous(type, ctx, msg);
}

int SmokeDriver::qmlErrors() { return g_qmlErrors; }

void SmokeDriver::start() {
    m_out = qEnvironmentVariable("DSPI_SMOKE_OUT", QDir::currentPath());
    g_previous = qInstallMessageHandler(messageHandler);
    const bool hardware = !qEnvironmentVariableIsEmpty("DSPI_SMOKE_HW");
    const bool replug = qEnvironmentVariable("DSPI_SMOKE_HW") == QLatin1String("replug");
    QTimer::singleShot(200, this, [this, hardware, replug]() {
        if (replug) runReplug();
        else if (hardware) runHardware();
        else run();
    });
}

void SmokeDriver::run() {
    testState();
    testSetters();
    testRouting();
    testCrossfeedRows();
    testPresets();
    testNotificationsAndMonitor();
    testRta();
    testControlSurfaces();
    testSiggenAndStats();
    testFiles();
    testAutoEq();
    testWindows();
    // -- WP QD30 --
    testPopoverAnchor();
    testSettingsPages();
    testBypassRepaint();
    testSweep();
    testReconnect();
    // -- WP QD1 --
    testCsRefusals();
    testIdleAndCost();
    m_failures += g_qmlErrors;
    printf("smoke: %d checks, %d failures (%d QML errors)\n", m_checks, m_failures, g_qmlErrors);
    fflush(stdout);
    QCoreApplication::exit(m_failures ? 1 : 0);
}

void SmokeDriver::section(const char *name) {
    printf("smoke: == %s\n", name);
    fflush(stdout);
}

void SmokeDriver::check(const char *what, bool ok) {
    m_checks++;
    if (!ok) {
        m_failures++;
        printf("FAIL %s\n", what);
        fflush(stdout);
    }
}

void SmokeDriver::spin(int ms) {
    QEventLoop loop;
    QTimer::singleShot(ms, &loop, &QEventLoop::quit);
    loop.exec();
}

bool SmokeDriver::waitFor(int ms, const std::function<bool()> &done) {
    QElapsedTimer t;
    t.start();
    while (!done()) {
        if (t.elapsed() > ms) return false;
        spin(20);
    }
    return true;
}

QObject *SmokeDriver::root() const { return m_.engine->rootObjects().first(); }

static bool near(double a, double b, double tol = 0.01) { return qAbs(a - b) <= tol; }

void SmokeDriver::testState() {
    section("state");
    auto *b = m_.bridge;
    check("connected to the demo device", b->connected());
    check("demo serial selected", b->selectedSerial() == QLatin1String("DEMO0000DSPI0001"));
    check("demo serial listed", b->availableSerials().contains(b->selectedSerial()));
    check("compatible", b->compat() == COMPAT_OK);
    check("RP2350", b->platformName() == QLatin1String("RP2350"));
    check("8 inputs", b->numInputChannels() == 8);
    check("9 outputs", b->numOutputChannels() == 9);
    check("firmware version", !b->firmwareVersion().isEmpty());
    check("capture's master volume", near(b->masterVolumeDB(), -20.0, 0.5));
    check("no connection error", b->connectionError().isEmpty());
    check("status polled", waitFor(1000, [b]() { return b->cpu0() >= 0; }));
    check("channel name", !b->channelName(0).isEmpty());
    check("magnitude curve", !b->magnitudeCurve(0).isEmpty());
    check("firmware: device version", !m_.firmware->deviceVersion().isEmpty());
    check("the demo's version is the app's: no banner", m_.firmware->match() == 1);
}

void SmokeDriver::testSetters() {
    section("setters");
    auto *b = m_.bridge;
    b->setMasterVolume(-31.0f);
    check("master volume", near(b->masterVolumeDB(), -31.0));
    b->setMasterVolume(-20.0f);
    const bool bypass = b->bypass();
    b->setBypass(!bypass);
    check("bypass", b->bypass() == !bypass);
    b->setBypass(bypass);
    b->setUserVolume(-12.0f);
    check("user volume", near(b->userVolumeDB(), -12.0));
    b->setUserMute(true);
    check("user mute", b->userMute());
    b->setUserMute(false);
    b->setInputPreamp(1, -3.0f);
    check("input preamp", near(b->inputPreampDB(1), -3.0));
    b->setFilter(0, 2, 1, 1234.0f, 2.5f, 1.2f);
    check("filter type", b->filterType(0, 2) == 1);
    check("filter freq", near(b->filterFreq(0, 2), 1234.0));
    b->setBandBypass(0, 2, true);
    check("band bypass", b->filterBypass(0, 2));
    b->setBandBypass(0, 2, false);
    b->setChannelDelay(2, 1.5f);
    check("channel delay", near(b->channelDelayMS(2), 1.5, 0.05));
    b->setOutputGain(1, -6.0f);
    check("output gain", near(b->outputGainDB(1), -6.0));
    b->setOutputMute(1, true);
    check("output mute", b->outputMuted(1));
    b->setOutputMute(1, false);
    b->setOutputDelay(1, 2.0f);
    check("output delay", near(b->outputDelayMS(1), 2.0, 0.05));
    b->setLoudness(true);
    check("loudness", b->loudnessEnabled());
    b->setLoudness(false);
    b->setLevellerEnabled(true);
    check("leveller", b->levellerEnabled());
    b->setLevellerEnabled(false);
    b->setLimiterEnabled(0, true);
    check("limiter", b->limiterEnabled(0));
    b->setLimiterThreshold(0, -4.0f);
    check("limiter threshold", near(b->limiterThreshold(0), -4.0));
    spin(400);   // the meter poll runs with a limiter on
    b->copyLimiterToAll(0);
    check("copied limiter", b->limiterEnabled(1) && near(b->limiterThreshold(1), -4.0));
    b->disableAllLimiters();
    check("limiters off", !b->limiterEnabled(0) && !b->limiterEnabled(1));
    b->setChannelName(0, QStringLiteral("Smoke"));
    check("channel name", b->channelName(0) == QLatin1String("Smoke"));
    b->resetChannelNames();
    check("names reset", b->channelName(0) == QLatin1String("USB 1"));
    b->setMatrixRoute(2, 3, true, -2.0f, true);
    check("matrix route", b->matrixRouting(2, 3) && near(b->matrixGain(2, 3), -2.0) && b->matrixInvert(2, 3));
    b->copyChannel(0);
    check("clipboard", b->canPaste());
    b->pasteChannel(1);
    check("pasted band", near(b->filterFreq(1, 2), 1234.0));
}

void SmokeDriver::testRouting() {
    section("routing helpers");
    auto *b = m_.bridge;
    const int nIn = b->numInputChannels(), nOut = b->numOutputChannels();
    // The helpers below write in loops: a stale snapshot would show here.
    b->setMatrixRoute(0, 1, false, -7.5f, true);
    b->setOutputEnable(nOut - 1, true);   // PDM on: direct routing turns it off
    b->directRouting();
    bool diag = true, offDiag = true;
    for (int i = 0; i < nIn; i++)
        for (int o = 0; o < nOut; o++) {
            const bool want = i == o && !b->isPdmOutput(o);
            if (want && !(b->matrixRouting(i, o) && near(b->matrixGain(i, o), 0.0) && !b->matrixInvert(i, o))) diag = false;
            if (!want && b->matrixRouting(i, o)) offDiag = false;
        }
    check("direct: diagonal at 0 dB", diag);
    check("direct: nothing else routed", offDiag);
    check("direct: off-diagonal keeps its gain and polarity", near(b->matrixGain(0, 1), -7.5) && b->matrixInvert(0, 1));
    check("direct: PDM off", !b->outputEnabled(nOut - 1));
    bool on = true;
    for (int o = 0; o < qMin(nIn, nOut - 1); o++) on = on && b->outputEnabled(o);
    check("direct: outputs on", on);
    b->setMatrixRoute(1, 1, true, -3.0f, true);
    b->clearRouting();
    bool none = true;
    for (int i = 0; i < nIn; i++)
        for (int o = 0; o < nOut; o++) none = none && !b->matrixRouting(i, o);
    check("clear: nothing routed", none);
    check("clear: keeps gain and polarity", near(b->matrixGain(1, 1), -3.0) && b->matrixInvert(1, 1));
    b->directRouting();
}

void SmokeDriver::testCrossfeedRows() {
    section("crossfeed rows");
    auto *b = m_.bridge;
    QQuickWindow *w = nullptr;
    for (QQuickWindow *c : root()->findChildren<QQuickWindow *>())
        if (c->property("title").toString() == QLatin1String("Crossfeed")) w = c;
    check("window found", w != nullptr);
    if (!w) return;
    const auto shown = [w]() { return QPair<double, double>(w->property("shownFreq").toDouble(), w->property("shownFeed").toDouble()); };
    b->setCrossfeedFreq(1500.0f);
    b->setCrossfeedFeed(12.0f);
    b->setCrossfeedPreset(0);
    spin(50);
    check("Default shows its own values", near(shown().first, 700) && near(shown().second, 4.5));
    b->setCrossfeedPreset(1);
    spin(50);
    check("Chu Moy shows its own values", near(shown().first, 700) && near(shown().second, 6.0));
    b->setCrossfeedPreset(2);
    spin(50);
    check("Meier shows its own values", near(shown().first, 650) && near(shown().second, 9.5));
    check("the device's custom values stay", near(b->crossfeedFreq(), 1500) && near(b->crossfeedFeed(), 12));
    b->setCrossfeedPreset(3);
    spin(50);
    check("Custom shows the device's values", near(shown().first, 1500) && near(shown().second, 12));
    b->setCrossfeedPreset(0);
}

void SmokeDriver::testPresets() {
    section("presets");
    auto *b = m_.bridge;
    const int slot = 5;
    b->setOutputGain(0, -2.0f);
    check("save", b->savePreset(slot) == 0);
    check("occupied", waitFor(500, [&]() { return b->isPresetOccupied(slot); }));
    b->setPresetName(slot, QStringLiteral("Smoke"));
    check("name", b->presetName(slot) == QLatin1String("Smoke"));
    b->setOutputGain(0, -5.0f);
    check("changes listed", !b->presetChanges().isEmpty());
    check("load", b->loadPreset(slot) == 0);
    // The re-read after the device's BULK_INVALIDATED (source PRESET)
    check("load restored the gain", waitFor(1000, [&]() { return near(b->outputGainDB(0), -2.0); }));
    check("active slot", b->activePresetSlot() == slot);
    check("copy", b->copyPreset(slot, slot + 1) == 0);
    check("copied", waitFor(500, [&]() { return b->isPresetOccupied(slot + 1); }));
    check("delete", b->deletePreset(slot + 1) == 0);
    check("deleted", waitFor(500, [&]() { return !b->isPresetOccupied(slot + 1); }));
    b->setPresetStartup(1, slot);
    check("save params", b->saveParams() == 0);
    b->loadParams();
}

void SmokeDriver::testNotificationsAndMonitor() {
    section("notifications, monitor");
    auto *b = m_.bridge;
    auto *mon = m_.monitor;
    mon->setWatching(true);
    int arrived = 0, changes = 0;
    QMetaObject::Connection c1 = QObject::connect(b, &DSPiBridge::notificationsArrived, this, [&arrived]() { arrived++; });
    QMetaObject::Connection c2 = QObject::connect(b, &DSPiBridge::stateChanged, this, [&changes]() { changes++; });
    b->setMasterVolume(-24.0f);                // an own echo
    check("echo arrives", waitFor(1500, [&]() { return arrived > 0; }));
    check("monitor shows it", waitFor(1500, [&]() { return mon->count() > 0; }));
    const int before = changes;
    b->loadPreset(5);                          // the device re-announces its state
    check("preset load notifies", waitFor(1500, [&]() { return changes > before; }));
    check("monitor text", !mon->allText().isEmpty());
    printf("smoke: monitor sample: %s\n", qPrintable(mon->allText().left(240).replace('\n', " | ")));
    mon->setPaused(true);
    mon->setPaused(false);
    mon->clear();
    check("monitor cleared", mon->count() == 0);
    mon->setWatching(false);
    QObject::disconnect(c1);
    QObject::disconnect(c2);
}

void SmokeDriver::testRta() {
    section("spectrum analyser");
    auto *rta = m_.rta;
    if (!rta->supported()) {
        printf("smoke: NOT EXERCISED: the demo device has no spectrum analyser (fake.rs): frames, telemetry\n");
        const int id = rta->subscribe(0, 0x3, true);   // the views still subscribe
        spin(300);
        check("not running", !rta->running());
        rta->release(id);
        return;
    }
    check("input tap channels", rta->inputChannels() > 0);
    const int id = rta->subscribe(0, 0x3, true);
    const bool running = waitFor(3000, [rta]() { return rta->running(); });
    check("runs while subscribed", running);
    int frames = 0;
    QMetaObject::Connection c = QObject::connect(rta, &RtaController::frameChanged, this, [&frames]() { frames++; });
    const bool got = waitFor(3000, [&]() { return frames >= 3; });
    printf("smoke: rta frames: %d, fft order %d\n", frames, rta->appliedOrder());
    check("frames arrive", got);
    rta->setFftOrder(rta->orderMin());
    rta->setAvgMs(100);
    rta->setPeakDecay(6);
    spin(200);
    QObject::disconnect(c);
    rta->release(id);
    check("stops when released", waitFor(3000, [rta]() { return !rta->running(); }));
}

void SmokeDriver::testControlSurfaces() {
    section("control surfaces");
    auto *cs = m_.cs;
    cs->refresh();
    // A refused operation comes back with its reason, which the pages show.
    const QVariantMap bad = cs->apply({ { "op", "no-such-op" } });
    check("bad op refused", !bad.value("ok").toBool() && !bad.value("error").toString().isEmpty());
    printf("smoke: cs error: %s\n", qPrintable(bad.value("error").toString()));
    check("error reaches the status message", cs->statusMessage(0xFF) == bad.value("error").toString());
    const QVariantMap next = cs->apply({ { "op", "refreshStatus" } });
    check("the old error is gone", cs->statusMessage(0xFF) != bad.value("error").toString());
    if (!cs->supported()) {
        printf("smoke: NOT EXERCISED: the demo device has no control surfaces (fake.rs): model, bindings, save, learn\n");
        check("refused with its reason", next.value("error").toString().contains(QLatin1String("not supported")));
        check("save refused", cs->save() != 0);
        return;
    }
    check("a good op", next.value("ok").toBool());
    cs->setWatchDisplay(true);
    spin(1200);
    cs->setWatchDisplay(false);
    check("save", cs->save() == 0);
    check("revert", cs->revert() == 0);
}

void SmokeDriver::testSiggenAndStats() {
    section("signal generator, stats, firmware");
    auto *sg = m_.siggen;
    sg->setWatching(true);
    sg->selectType(1);
    sg->cycleOutput(0);
    if (sg->supported()) {
        check("siggen starts", sg->start());
        spin(300);
        sg->stopNow();
        check("siggen stops", waitFor(1500, [sg]() { return !sg->running(); }));
    } else {
        printf("smoke: NOT EXERCISED: the demo device has no signal generator (fake.rs)\n");
        check("siggen refuses", !sg->start());
    }
    sg->clearOutputs();
    sg->setWatching(false);

    auto *st = m_.stats;
    st->setWatching(true);
    check("stats info", waitFor(2000, [st]() { return !st->info().isEmpty(); }));
    if (!waitFor(1500, [st]() { return !st->buffers().isEmpty(); }))
        printf("smoke: NOT EXERCISED: the demo device has no buffer statistics (fake.rs)\n");
    st->resetWatermarks();
    st->setWatching(false);

    auto *fw = m_.firmware;
    fw->beginWatching();
    spin(300);
    fw->stopWatching();
    check("release notes", !fw->releaseNotes().isEmpty());
}

void SmokeDriver::testFiles() {
    section("config and filter files");
    auto *cf = m_.config;
    auto *b = m_.bridge;
    const QUrl config = QUrl::fromLocalFile(m_out + QStringLiteral("/smoke.dspiconfig"));
    b->setMasterVolume(-26.0f);
    QVariantMap r = cf->exportConfiguration(config);
    check("config export", r.value("ok").toBool());
    r = cf->inspectConfiguration(config);
    check("config inspect", r.value("ok").toBool());
    b->setMasterVolume(-40.0f);
    r = cf->importConfiguration(config, true, false);
    printf("smoke: config import: ok %d %s\n", r.value("ok").toBool(), qPrintable(r.value("error").toString()));
    check("config import", r.value("ok").toBool());
    check("config import restored the volume", near(b->masterVolumeDB(), -26.0));

    const QUrl filters = QUrl::fromLocalFile(m_out + QStringLiteral("/smoke.txt"));
    b->setFilter(0, 0, 1, 777.0f, 1.0f, 0.9f);
    r = cf->exportFilters(filters);
    check("filter export", r.value("ok").toBool());
    r = cf->inspectFilters(filters);
    check("filter inspect", r.value("ok").toBool());
    b->setFilter(0, 0, 1, 333.0f, 1.0f, 0.9f);
    QVariantList wires;
    for (const QVariant &c : r.value("channels").toList()) wires.append(c.toMap().value("wire"));
    r = cf->importFilters(wires);
    check("filter import", r.value("ok").toBool());
    check("filter import restored the band", near(b->filterFreq(0, 0), 777.0, 0.5));
}

void SmokeDriver::testAutoEq() {
    section("AutoEQ");
    auto *a = m_.autoeq;
    a->load();
    printf("smoke: autoeq entries: %d %s\n", a->entryCount(), qPrintable(a->loadError()));
    check("database", a->entryCount() > 0);
    a->setQuery(QStringLiteral("HD 6"));
    spin(300);
}

void SmokeDriver::openAndGrab(const char *name) {
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QString::fromLatin1(name)));
    spin(700);
    QQuickWindow *found = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->isVisible()) found = w;
    check(name, found != nullptr);
    if (!found) return;
    const QImage img = found->grabWindow();
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    check("grab", !img.isNull());   // Qt 5's software backend grabs nothing offscreen
#endif
    img.save(m_out + QStringLiteral("/window-%1.png").arg(QString::fromLatin1(name)));
    found->setVisible(false);
    spin(50);
}

void SmokeDriver::testWindows() {
    section("tool windows");
    if (auto *main = qobject_cast<QQuickWindow *>(root())) {
        main->show();
        spin(600);
        main->grabWindow().save(m_out + QStringLiteral("/window-main.png"));
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("gettingStarted")));
        spin(300);
        main->grabWindow().save(m_out + QStringLiteral("/window-wizard.png"));
        root()->setProperty("setupRequested", false);
        spin(100);
    }
    for (const char *n : { "matrix", "loudness", "crossfeed", "leveller", "psybass", "subharm", "tube", "upmix", "stats",
                           "spectrum", "graph", "siggen", "monitor", "firmware", "whatsNew", "autoeq", "autoeqRebuild" })
        openAndGrab(n);
}

// -- WP QD1 --
// A refused cs_apply shows its `error`. The demo device has no control surfaces,
// so a page can't open here: this runs the pages' own expression (applyGroup,
// applyMacro) from QML. A device status code and the polls need real surfaces, so
// they aren't exercised here.
void SmokeDriver::testCsRefusals() {
    section("control surfaces: a refusal shows its error (QD1)");
    auto *cs = m_.cs;
    auto *b = m_.bridge;
    const auto shown = [this](const QString &op) {
        QQmlExpression e(m_.engine->rootContext(), nullptr, QStringLiteral(
            "(function() { var r = controlSurfaces.apply({ op: \"%1\" });"
            " var st = r && r.status !== undefined ? r.status : 0xFF;"
            " return st !== 0 ? controlSurfaces.statusMessage(st) : undefined })()").arg(op));
        const QString text = e.evaluate().toString();
        check("QD1: the expression evaluates", !e.hasError());
        return text;
    };
    const QString generic = cs->statusMessage(0x7F);

    // The core's refusal, with no device status
    const QString unknown = shown(QStringLiteral("no-such-op"));
    printf("smoke: QD1 unknown op shows: %s\n", qPrintable(unknown));
    check("QD1: an unknown op shows the core's reason", unknown.contains(QLatin1String("no-such-op")));
    check("QD1: and not the generic text", unknown != generic);
    const QString refused = shown(QStringLiteral("save"));
    printf("smoke: QD1 save shows: %s\n", qPrintable(refused));
    check("QD1: a refused save shows the core's reason", !refused.isEmpty() && refused != generic);
    check("QD1: the reply carries that reason", cs->apply({ { "op", "save" } }).value("error").toString() == refused);

    // No device: refused before the core, still with a reason
    b->disconnectDevice();
    check("QD1: disconnected", !b->connected());
    const QVariantMap none = cs->apply({ { "op", "save" } });
    const QString gone = cs->statusMessage(0xFF);
    printf("smoke: QD1 no device shows: %s\n", qPrintable(gone));
    check("QD1: no device is refused", !none.value("ok").toBool() && none.value("error").toString() == gone);
    check("QD1: no device gives its reason", !gone.isEmpty() && gone != generic);
    check("QD1: the pages' expression shows it too", shown(QStringLiteral("save")) == gone);

    // Another device: the old reason doesn't follow it
    b->rescan();
    check("QD1: back on the demo", b->connected());
    check("QD1: a new device drops the old reason", cs->statusMessage(0xFF) == generic);
}

// -- WP QD30 --
// A PopoverWindow's anchor keeps its hand cursor while the popover is open, and a
// press anywhere on it closes the popover once. The popover's shadow margin
// overlaps the anchor's lower part, and the catcher covers the rest. Offscreen
// takes the non-Wayland path; PopoverWindow's own area is the same on Wayland.
static void sendMouse(QQuickWindow *w, QEvent::Type type, const QPointF &local, Qt::MouseButton button = Qt::NoButton) {
    const Qt::MouseButtons held = type == QEvent::MouseButtonPress ? Qt::MouseButtons(Qt::LeftButton) : Qt::MouseButtons(Qt::NoButton);
    QMouseEvent e(type, local, local, QPointF(w->mapToGlobal(local.toPoint())), button, held, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &e);
}

static void clickAt(QQuickWindow *w, const QPointF &local) {
    sendMouse(w, QEvent::MouseMove, local);
    sendMouse(w, QEvent::MouseButtonPress, local, Qt::LeftButton);
    sendMouse(w, QEvent::MouseButtonRelease, local, Qt::LeftButton);
}

void SmokeDriver::testPopoverAnchor() {
    section("QD30 popover anchor");
    auto *main = qobject_cast<QQuickWindow *>(root());
    check("QD30 main window", main != nullptr);
    if (!main) return;
    main->show();
    spin(200);

    // Every popover the main window has built, the graph gear's among them
    QList<QQuickWindow *> popovers;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->transientParent() == main && w->metaObject()->indexOfMethod("toggleBelow(QVariant)") >= 0) popovers << w;
    check("QD30 the main window has a popover", !popovers.isEmpty());

    QQmlComponent component(m_.engine);
    component.setData("import QtQuick 2.15\n"
                      "MouseArea {\n"
                      "    property QtObject popover\n"
                      "    property int clicks: 0\n"
                      "    width: 24; height: 22; z: 1000\n"
                      "    hoverEnabled: true\n"
                      "    cursorShape: Qt.PointingHandCursor\n"
                      "    onClicked: { clicks++; popover.toggleBelow(this) }\n"
                      "}\n", QUrl());
    for (QQuickWindow *pop : popovers) {
        const QByteArray name = pop->metaObject()->className();
        auto *gear = qobject_cast<QQuickItem *>(component.create());
        check(("QD30 test anchor " + name).constData(), gear != nullptr);
        if (!gear) continue;
        gear->setParentItem(main->contentItem());
        gear->setPosition(QPointF(main->width() / 2, 50));
        gear->setProperty("popover", QVariant::fromValue<QObject *>(pop));

        int opens = 0, closes = 0;
        auto counter = connect(pop, &QWindow::visibleChanged, this, [&](bool on) { (on ? opens : closes)++; });
        const QPointF upper(gear->x() + 12, gear->y() + 2);
        const QPointF lower(gear->x() + 12, gear->y() + 18);
        const QPoint lowerGlobal = main->mapToGlobal(lower.toPoint());
        const auto what = [&name](const char *s) { return ("QD30 " + name + ": " + s); };

        clickAt(main, upper);
        check(what("the anchor opens it").constData(), pop->isVisible() && opens == 1 && gear->property("clicks").toInt() == 1);
        // Without a move, as when the window has just appeared under the pointer
        check(what("the hand stays on opening").constData(), main->cursor().shape() == Qt::PointingHandCursor);

        check(what("its window overlaps the anchor (the reported case)").constData(), pop->geometry().contains(lowerGlobal));
        sendMouse(main, QEvent::MouseMove, QPointF(5, 5));
        check(what("the catcher shows no hand off the anchor").constData(), main->cursor().shape() != Qt::PointingHandCursor);
        sendMouse(main, QEvent::MouseMove, upper);
        check(what("back on the anchor, the hand, first move").constData(), main->cursor().shape() == Qt::PointingHandCursor);

        const QPointF inPopup = pop->mapFromGlobal(lowerGlobal);
        sendMouse(pop, QEvent::MouseMove, inPopup);
        check(what("the hand on the anchor under the popover").constData(), pop->cursor().shape() == Qt::PointingHandCursor);
        // The shadow beside the card: the card's own controls can show a hand
        sendMouse(pop, QEvent::MouseMove, QPointF(2, pop->height() / 2));
        check(what("no hand off the anchor in the popover").constData(), pop->cursor().shape() != Qt::PointingHandCursor);

        // The part of the anchor under the popover
        clickAt(pop, inPopup);
        spin(400);   // past the anchor's reopen guard: nothing may reopen it late
        check(what("a press under the popover closes it once").constData(), !pop->isVisible() && closes == 1 && opens == 1);
        check(what("that press does not reach the anchor").constData(), gear->property("clicks").toInt() == 1);

        // The part in the parent window
        clickAt(main, upper);
        check(what("reopens").constData(), pop->isVisible() && opens == 2);
        clickAt(main, upper);
        spin(400);
        check(what("a press in the parent window closes it once").constData(), !pop->isVisible() && closes == 2 && opens == 2);

        // The anchor's own click, the path on Wayland, where the catcher is off
        clickAt(main, upper);
        QMetaObject::invokeMethod(pop, "toggleBelow", Q_ARG(QVariant, QVariant::fromValue<QObject *>(gear)));
        spin(400);
        check(what("toggleBelow on an open popover closes it once").constData(), !pop->isVisible() && closes == 3 && opens == 3);

        disconnect(counter);
        sendMouse(main, QEvent::MouseMove, QPointF(5, 5));
        delete gear;
    }
    check("QD30 no anchor component error", !component.isError());
}

void SmokeDriver::testSettingsPages() {
    section("settings pages");
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    check("settings window", settings != nullptr);
    if (!settings) return;
    settings->setVisible(true);
    spin(300);
    for (const char *id : { "about", "advanced", "graphing", "spectrum", "overview", "inputs", "outputs", "i2s", "global",
                            "surfaces", "control", "groups", "macros", "aux" }) {
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QString::fromLatin1(id)));
        spin(400);
        settings->grabWindow().save(m_out + QStringLiteral("/settings-%1.png").arg(QString::fromLatin1(id)));
    }
    settings->setVisible(false);
}

QImage SmokeDriver::grabItem(QQuickItem *item) {
    QSharedPointer<QQuickItemGrabResult> grab = item->grabToImage();
    if (!grab) return {};
    bool done = false;
    QObject::connect(grab.data(), &QQuickItemGrabResult::ready, this, [&done]() { done = true; });
    waitFor(2000, [&done]() { return done; });
    return done ? grab->image() : QImage();
}

SmokeDriver::Outcome SmokeDriver::bypassRepaints(QString *why) {
    auto *b = m_.bridge;
    int channel = -1;
    for (int ch = 0; ch < 2 && channel < 0; ch++)
        for (int band = 0; band < BANDS_PER_CHANNEL; band++)
            if (b->filterType(ch, band) != FILTER_FLAT && !b->filterBypass(ch, band)) channel = ch;
    if (channel < 0) { *why = "no live PEQ band on IN1 or IN2 to dim, and this pass adds none"; return Outcome::NotRun; }
    b->setChannelVisible(channel, true);
    QMetaObject::invokeMethod(root(), "selectOverview");
    QMetaObject::invokeMethod(root(), "selectChannel", Q_ARG(QVariant, channel));
    spin(800);
    PeqEditorItem *editor = nullptr;
    for (PeqEditorItem *e : root()->findChildren<PeqEditorItem *>())
        if (e->active()) editor = e;
    Outcome result = Outcome::NotRun;
    if (!editor) {
        *why = "the channel page's graph editor did not become active";
    } else {
        const bool original = b->bypass();
        const QImage before = grabItem(editor);
        b->setBypass(!original);
        spin(600);
        const QImage toggled = grabItem(editor);
        b->setBypass(original);
        spin(600);
        const QImage after = grabItem(editor);
        if (before.isNull() || toggled.isNull() || after.isNull()) {
            *why = "this Qt's offscreen backend grabs no item image";
        } else {
            const bool changed = before != toggled, back = before == after;
            result = changed && back ? Outcome::Pass : Outcome::Fail;
            *why = QStringLiteral("toggling changed the editor's pixels: %1; restoring brought them back: %2")
                       .arg(changed ? "yes" : "NO", back ? "yes" : "NO");
        }
    }
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(200);
    return result;
}

void SmokeDriver::testBypassRepaint() {
    section("bypass repaint");
    QString why;
    const Outcome o = bypassRepaints(&why);
    printf("smoke: bypass repaint: %s\n", qPrintable(why));
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    check("bypass repaints the graph editor", o == Outcome::Pass);
#else
    check("bypass repaints the graph editor", o != Outcome::Fail);
#endif
}

// Every other invokable of the bridge, with in-range and out-of-range values: a
// bad index from QML must do nothing worse than fail.
void SmokeDriver::testSweep() {
    section("bridge sweep");
    static const QStringList skip = {
        "installUdevRule", "enterBootloader",   // leave the app or the device
        "factoryReset", "selectDevice", "disconnectDevice", "reconnect", "rescan", "scanDevices",
        "deleteLater", "destroyed", "objectNameChanged",
    };
    auto *b = m_.bridge;
    const QMetaObject *mo = b->metaObject();
    int calls = 0;
    const int firstVals[] = { 0, 1, -1, 99 };
    for (int v : firstVals) {
        for (int m = QObject::staticMetaObject.methodCount(); m < mo->methodCount(); m++) {
            const QMetaMethod method = mo->method(m);
            if (method.methodType() != QMetaMethod::Method && method.methodType() != QMetaMethod::Slot) continue;
            if (skip.contains(QString::fromLatin1(method.name()))) continue;
            if (method.parameterCount() > 6) continue;
            int ints[6];
            float floats[6];
            bool bools[6];
            QString strs[6];
            QGenericArgument args[6];
            bool ok = true;
            for (int p = 0; p < method.parameterCount(); p++) {
                const QByteArray t = method.parameterTypes().at(p);
                if (t == "int") { ints[p] = v; args[p] = QGenericArgument("int", &ints[p]); }
                else if (t == "float") { floats[p] = float(v); args[p] = QGenericArgument("float", &floats[p]); }
                else if (t == "bool") { bools[p] = v == 1; args[p] = QGenericArgument("bool", &bools[p]); }
                else if (t == "QString") { strs[p] = QStringLiteral("x"); args[p] = QGenericArgument("QString", &strs[p]); }
                else ok = false;
            }
            if (!ok) continue;
            // Results are not looked at, so leave out the return slot.
            method.invoke(b, Qt::DirectConnection, args[0], args[1], args[2], args[3], args[4], args[5]);
            calls++;
            QCoreApplication::processEvents();
        }
        spin(100);
    }
    printf("smoke: sweep: %d calls\n", calls);
    check("still connected", b->connected());
}

void SmokeDriver::testReconnect() {
    section("disconnect, reconnect");
    auto *b = m_.bridge;
    b->reconnect();
    check("reconnect", b->connected() && b->compat() == COMPAT_OK);
    b->disconnectDevice();
    check("disconnected", !b->connected());
    spin(300);
    // Every window with no device
    if (auto *main = qobject_cast<QQuickWindow *>(root())) main->grabWindow().save(m_out + QStringLiteral("/window-disconnected.png"));
    spin(700);   // a hotplug poll with no device selected: the demo comes back by itself
    check("the scan reselects the demo", b->connected());
    b->disconnectDevice();
    b->rescan();
    check("rescan finds the demo", b->connected() && b->selectedSerial() == QLatin1String("DEMO0000DSPI0001"));
    check("factory reset", b->factoryReset() == 0);
    spin(200);
    check("after the reset", b->connected());
}

void SmokeDriver::testIdleAndCost() {
    section("idle, cost of a state read");
    auto *b = m_.bridge;
    spin(5000);   // the 60 ms status timer and 500 ms hotplug timer: see run.sh's warning count
    QElapsedTimer t;
    t.start();
    int sink = 0;
    for (int i = 0; i < 2000; i++) sink += b->numChannels();
    const double us = double(t.nsecsElapsed()) / 2000.0 / 1000.0;
    printf("smoke: state read: %.1f us (%d)\n", us, sink / 2000);
    t.restart();
    for (int i = 0; i < 20; i++) {
        emit b->stateChanged();
        QCoreApplication::processEvents();
    }
    printf("smoke: stateChanged with the main window's bindings: %.1f ms\n", double(t.nsecsElapsed()) / 20.0 / 1e6);
    fflush(stdout);
}
