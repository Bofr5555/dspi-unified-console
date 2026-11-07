// WP QD6: firmware of another wire format on the real app: settings shown, every
// control off, nothing written, reads going on, the way out still there. And a
// write queued for one device never reaches the next.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "ExtController.h"
#include "FirmwareUpdater.h"
#include "NamesController.h"
#include "OutputLatencyController.h"

#include <QCoreApplication>
#include <QMouseEvent>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <QVariant>
#include <QWheelEvent>
#include <atomic>
#include <cstdio>
#include <cstring>
#include <functional>

namespace {
const char *const kDemoSerial = "DEMO0000DSPI0001";
const char *const kToolWindows[] = { "matrixWindow", "loudnessWindow", "crossfeedWindow", "levellerWindow", "psybassWindow",
                                     "subharmWindow", "tubeWindow", "upmixWindow", "siggenWindow", "autoeqWindow" };

QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

bool isClass(const QObject *o, const char *prefix) {
    return QString::fromLatin1(o->metaObject()->className()).startsWith(QLatin1String(prefix));
}

// What a user works: not a scroll bar, not plain text
bool isControl(const QObject *o) {
    if (o->inherits("QQuickScrollBar")) return false;
    return o->inherits("QQuickMouseArea") || o->inherits("QQuickControl") || o->inherits("QQuickTextInput") || o->inherits("QQuickTextEdit");
}

// A control that takes clicks: an area with no button, like the wheel zones, doesn't
bool takesClicks(const QQuickItem *item) {
    return !item->inherits("QQuickMouseArea") || item->property("acceptedButtons").toInt() != Qt::NoButton;
}

void collect(QQuickItem *item, QVector<QPointer<QQuickItem>> *out) {
    // The window's own: leaves or resizes it, and is no setting of the device
    if (isClass(item, "WindowTitleBar") || isClass(item, "WindowResizeEdges")) return;
    if (!item->isVisible()) return;
    if (isControl(item) && takesClicks(item)) out->append(item);
    for (QQuickItem *c : item->childItems()) collect(c, out);
}

QString describe(const QQuickItem *item) {
    QStringList path;
    for (const QQuickItem *i = item; i && path.size() < 5; i = i->parentItem())
        path << QString::fromLatin1(i->metaObject()->className()).section(QLatin1String("_QML"), 0, 0);
    return path.join(QLatin1String(" < "));
}

void sendMouse(QQuickWindow *w, QEvent::Type type, const QPointF &at, Qt::MouseButton button, Qt::MouseButtons held) {
    QMouseEvent e(type, at, at, QPointF(w->mapToGlobal(at.toPoint())), button, held, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &e);
}

void click(QQuickWindow *w, const QPointF &at, Qt::MouseButton button = Qt::LeftButton) {
    sendMouse(w, QEvent::MouseMove, at, Qt::NoButton, Qt::NoButton);
    sendMouse(w, QEvent::MouseButtonPress, at, button, button);
    sendMouse(w, QEvent::MouseButtonRelease, at, button, Qt::NoButton);
}

void wheel(QQuickWindow *w, const QPointF &at, int delta) {
    QWheelEvent e(at, QPointF(w->mapToGlobal(at.toPoint())), QPoint(), QPoint(0, delta), Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(w, &e);
}

// What a write would change, read back from the device
QByteArray deviceSettings(FfiCore *core) {
    if (!dspi_fetch_all(core)) return QByteArray();
    const DspState *s = dspi_get_state(core);
    // Field by field: the records have padding
    QByteArray out;
    const auto add = [&out](const void *p, size_t n) { out.append(static_cast<const char *>(p), int(n)); };
    for (int w = 0; w < s->num_channels; w++) {
        for (int band = 0; band < BANDS_PER_CHANNEL; band++) {
            const FilterParams &f = s->filters[w][band];
            add(&f.filter_type, sizeof(f.filter_type));
            add(&f.freq, sizeof(f.freq));
            add(&f.gain, sizeof(f.gain));
            add(&f.q, sizeof(f.q));
            add(&f.bypass, sizeof(f.bypass));
        }
    }
    add(s->channel_names, sizeof(s->channel_names));
    add(s->matrix_gain, sizeof(s->matrix_gain));
    add(&s->master_volume_db, sizeof(s->master_volume_db));
    add(&s->user_volume_db, sizeof(s->user_volume_db));
    add(&s->bypass, sizeof(s->bypass));
    add(&s->input_source, sizeof(s->input_source));
    return out;
}

template <class T>
T *controller(QQmlApplicationEngine *engine, const char *name) {
    return qobject_cast<T *>(engine->rootContext()->contextProperty(QString::fromLatin1(name)).value<QObject *>());
}
}

// Click and scroll on whatever still takes it, left and right: none of it may reach
// the core as a write.
void SmokeDriver::readOnlyClickAround(QQuickWindow *window, const char *where, QQuickItem *scope) {
    FfiCore *core = m_.bridge->core();
    QVector<QPointer<QQuickItem>> items;
    collect(scope ? scope : window->contentItem(), &items);
    int clicks = 0;
    for (const QPointer<QQuickItem> &item : items) {
        if (!item || !item->isVisible() || !item->isEnabled() || !item->window()) continue;
        const QPointF at = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
        if (at.x() < 0 || at.y() < 0 || at.x() >= window->width() || at.y() >= window->height()) continue;
        const QString what = describe(item);
        const uint32_t before = dspi_read_only_refusals(core);
        click(window, at);
        click(window, at, Qt::RightButton);
        wheel(window, at, 120);
        QCoreApplication::processEvents();
        clicks++;
        if (dspi_read_only_refusals(core) != before)
            check(qPrintable(QStringLiteral("QD6: %1: this control tried to write: %2").arg(QLatin1String(where), what)), false);
    }
    printf("smoke: QD6 %s: %d clicked of %d\n", where, clicks, int(items.size()));
    fflush(stdout);
}

void SmokeDriver::testReadOnlyFixture(int wire) {
    auto *b = m_.bridge;
    FfiCore *core = b->core();
    const QString tag = QStringLiteral("QD6 V%1: ").arg(wire);
    const auto expect = [&](const QString &what, bool ok) { check(qPrintable(tag + what), ok); };

    b->setDemoWire(wire);
    b->selectDevice(QString::fromLatin1(kDemoSerial));
    spin(300);
    const int want = wire > 32 ? COMPAT_FIRMWARE_TOO_NEW : COMPAT_FIRMWARE_TOO_OLD;
    expect(QStringLiteral("connected, as firmware of that format"), b->connected() && b->compat() == want && dspi_get_state(core)->format_version == wire);
    expect(QStringLiteral("not writable"), !b->writable());

    auto *main = qobject_cast<QQuickWindow *>(root());
    main->show();
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(300);

    // ── Shown ──
    expect(QStringLiteral("the settings are shown"), qAbs(b->masterVolumeDB() + 20.0f) < 0.01f && b->numOutputChannels() == 9 && !b->channelName(0).isEmpty());
    QQuickItem *banner = findItem(main->contentItem(), QStringLiteral("readOnlyBanner"));
    QQuickItem *message = banner ? findItem(banner, QStringLiteral("readOnlyMessage")) : nullptr;
    QQuickItem *action = banner ? findItem(banner, QStringLiteral("readOnlyAction")) : nullptr;
    expect(QStringLiteral("the banner shows"), banner && banner->isVisible() && banner->height() > 0 && message && action);
    if (!banner || !message || !action) return;
    const QString text = message->property("text").toString();
    printf("smoke: %s%s\n", qPrintable(tag), qPrintable(text));
    expect(QStringLiteral("it says read-only"), text.contains(QLatin1String("read-only")) && text.contains(b->firmwareVersion()));
    expect(QStringLiteral("it offers the way out"), action->property("text").toString() == (wire > 32 ? QStringLiteral("Details…") : QStringLiteral("Update…")));
    main->grabWindow().save(m_out + QStringLiteral("/readonly-v%1.png").arg(wire));

    // ── What the firmware lacks is hidden ──
    const QVariantList groups = [&]() {
        for (QObject *o : root()->findChildren<QObject *>())
            if (o->metaObject()->indexOfMethod("activateItem(QVariant)") >= 0) return o->property("groups").toList();
        return QVariantList();
    }();
    QStringList windows;
    for (const QVariant &g : groups)
        for (const QVariant &r : g.toMap().value("items").toList()) windows << r.toMap().value("window").toString();
    expect(QStringLiteral("the menu has the app's windows"), windows.contains(QLatin1String("loudness")) && windows.contains(QLatin1String("firmware")));
    expect(QStringLiteral("the menu hides what it lacks"), windows.contains(QLatin1String("tube")) == (wire >= 31) && windows.contains(QLatin1String("subharm")) == (wire >= 29)
                                                               && windows.contains(QLatin1String("upmix")) == (wire >= 25) && windows.contains(QLatin1String("psybass")) == (wire >= 23));
    expect(QStringLiteral("so does the bridge"), b->firmwareHas(QStringLiteral("limiter")) == (wire >= 32) && b->upmixSupported() == (wire >= 25));

    // ── Reads go on ──
    expect(QStringLiteral("the status polls"), waitFor(1500, [b]() { return b->cpu0() > 0 || b->cpu1() > 0 || b->activeInputChannels() > 0 || b->peakLevel(0) > 0; }));

    const QByteArray settingsBefore = deviceSettings(core);
    expect(QStringLiteral("the settings read back"), !settingsBefore.isEmpty());
    const uint32_t refusals = dspi_read_only_refusals(core);

    // ── Every control is off ──
    const auto allOff = [&](QQuickItem *scope, const QString &where) {
        QVector<QPointer<QQuickItem>> items;
        collect(scope, &items);
        int on = 0;
        for (const QPointer<QQuickItem> &item : items)
            if (item->isEnabled()) {
                on++;
                printf("FAIL %s%s: a control is on: %s\n", qPrintable(tag), qPrintable(where), qPrintable(describe(item)));
            }
        m_failures += on;
        // A page of what this firmware model doesn't have shows only that
        const bool empty = where == QLatin1String("page control") && !b->hardware().value("ctrlSupported").toBool();
        expect(where + QStringLiteral(" has controls"), empty || !items.isEmpty());
    };

    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    expect(QStringLiteral("the settings window"), settings != nullptr);
    if (!settings) return;
    settings->setVisible(true);
    spin(200);
    const std::function<QQuickItem *(QQuickItem *)> findPage = [&](QQuickItem *from) -> QQuickItem * {
        if (from->isVisible() && from->inherits("QQuickFlickable") && from->property("writesToDevice").isValid()) return from;
        for (QQuickItem *c : from->childItems())
            if (QQuickItem *found = findPage(c)) return found;
        return nullptr;
    };
    for (const char *id : { "advanced", "inputs", "outputs", "i2s", "global", "surfaces", "control", "groups", "macros", "aux" }) {
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QString::fromLatin1(id)));
        spin(150);
        QQuickItem *page = findPage(settings->contentItem());
        const QString where = QStringLiteral("page %1").arg(QString::fromLatin1(id));
        expect(where + QStringLiteral(" opens"), page != nullptr);
        if (!page) continue;
        expect(where + QStringLiteral(" writes to the device"), page->property("writesToDevice").toBool());
        allOff(page, where);
        // It still scrolls
        const int height = settings->height();
        settings->setHeight(settings->minimumHeight());
        spin(50);
        if (page->property("contentHeight").toReal() > page->height()) {
            page->setProperty("contentY", 0);
            wheel(settings, page->mapToScene(QPointF(page->width() / 2, page->height() / 2)), -120);
            spin(50);
            expect(where + QStringLiteral(" scrolls"), page->property("contentY").toReal() > 0);
        }
        settings->setHeight(height);
    }
    settings->grabWindow().save(m_out + QStringLiteral("/readonly-v%1-settings.png").arg(wire));
    // The save never goes out, whatever is on it
    if (QObject *ctx = settings->findChild<QObject *>(QStringLiteral("settingsContext"))) {
        ctx->setProperty("edited", true);
        ctx->setProperty("masterVolumeMode", b->masterVolumeMode() ? 0 : 1);
        QMetaObject::invokeMethod(ctx, "save");
        ctx->setProperty("masterVolumeMode", b->masterVolumeMode());
        ctx->setProperty("edited", false);
    } else {
        expect(QStringLiteral("the settings context"), false);
    }
    // Pages of this app's own settings and readings stay live, and don't write
    for (const char *id : { "graphing", "spectrum", "overview" }) {
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QString::fromLatin1(id)));
        spin(150);
        readOnlyClickAround(settings, id);
    }
    settings->setVisible(false);

    for (const char *name : kToolWindows) {
        auto *w = root()->findChild<QQuickWindow *>(QString::fromLatin1(name));
        expect(QStringLiteral("window %1").arg(QLatin1String(name)), w != nullptr);
        if (!w) continue;
        w->setVisible(true);
        spin(150);
        QQuickItem *content = findItem(w->contentItem(), QStringLiteral("appWindowContent"));
        expect(QStringLiteral("window %1 has content").arg(QLatin1String(name)), content != nullptr);
        if (content) allOff(content, QStringLiteral("window %1").arg(QLatin1String(name)));
        readOnlyClickAround(w, name);
        // A window opened short still scrolls to everything
        if (w->objectName() == QLatin1String("tubeWindow") && content) {
            QQuickItem *flick = nullptr;
            const std::function<void(QQuickItem *)> find = [&](QQuickItem *i) {
                if (!flick && i->inherits("QQuickFlickable") && i->isVisible()) flick = i;
                for (QQuickItem *c : i->childItems()) find(c);
            };
            find(content);
            const int height = w->height();
            w->setHeight(w->minimumHeight());
            spin(100);
            expect(QStringLiteral("the tube window shows Advanced"), w->property("showAdvanced").toBool());
            if (flick && flick->property("contentHeight").toReal() > flick->height()) {
                flick->setProperty("contentY", 0);
                wheel(w, flick->mapToScene(QPointF(flick->width() / 2, flick->height() / 2)), -120);
                spin(50);
                expect(QStringLiteral("a disabled tool window scrolls"), flick->property("contentY").toReal() > 0);
            }
            w->setHeight(height);
        }
        w->setVisible(false);
    }
    for (const char *tool : { "stats", "spectrum", "monitor" }) {
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QString::fromLatin1(tool)));
        spin(150);
        for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
            if (w != settings && w->isVisible() && w != main) {
                readOnlyClickAround(w, tool);
                w->setVisible(false);
            }
    }

    // ── The main window: the overview, an input, an output and its crossover tab ──
    // A click on the sidebar leaves the page, so each page first on its own
    QQuickItem *page = findItem(main->contentItem(), QStringLiteral("contentLoader"));
    expect(QStringLiteral("the content loader"), page != nullptr);
    QMetaObject::invokeMethod(root(), "selectChannel", Q_ARG(QVariant, 0));
    spin(300);
    if (page) readOnlyClickAround(main, "input", page);
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
    spin(300);
    main->grabWindow().save(m_out + QStringLiteral("/readonly-v%1-output.png").arg(wire));
    if (page) readOnlyClickAround(main, "output", page);
    int editors = 0;
    const std::function<void(QQuickItem *)> findEditors = [&](QQuickItem *i) {
        if (i->inherits("PeqEditorItem")) {
            editors++;
            expect(QStringLiteral("the graph editor is off"), !i->isEnabled());
        }
        for (QQuickItem *c : i->childItems()) findEditors(c);
    };
    findEditors(main->contentItem());
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>()) findEditors(w->contentItem());   // the pop-out graph's
    expect(QStringLiteral("the graph editor is there"), editors > 0);
    QQuickItem *source = findItem(main->contentItem(), QStringLiteral("sourcePicker"));
    expect(QStringLiteral("the source picker is off"), source && !source->isEnabled());
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(200);
    readOnlyClickAround(main, "main window");
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(200);

    // The keyboard's way to a save or an import
    for (QObject *o : root()->findChildren<QObject *>())
        if (o->inherits("QQuickShortcut")) {
            const QString seq = o->property("sequence").toString();
            if (seq == QLatin1String("Ctrl+S") || seq == QLatin1String("Ctrl+I")) expect(seq + QStringLiteral(" is off"), !o->property("enabled").toBool());
        }

    // ── The controllers themselves refuse ──
    auto *names = controller<NamesController>(m_.engine, "deviceNames");
    auto *latency = controller<OutputLatencyController>(m_.engine, "outputLatency");
    auto *ext = controller<ExtController>(m_.engine, "ext");
    expect(QStringLiteral("the controllers"), names && latency && ext);
    if (names && latency && ext) {
        names->editDevice(QStringLiteral("QD6"));
        names->editSource(1, QStringLiteral("QD6"));
        expect(QStringLiteral("a name takes no draft"), !names->dirty());
        names->save();
        latency->edit(3);
        expect(QStringLiteral("the latency takes no draft"), !latency->dirty());
        latency->save();
        expect(QStringLiteral("the pair map refuses"), !ext->applyPreset(QStringLiteral("k340")) && !ext->applyPairs({}));
        waitFor(500, [&]() { return !names->saving() && !latency->applying(); });
    }

    expect(QStringLiteral("no setter was called"), dspi_read_only_refusals(core) == refusals);
    expect(QStringLiteral("the device's settings did not change"), deviceSettings(core) == settingsBefore);

    // ── What stays: clearing clips, and the bootloader, on their own requests ──
    expect(QStringLiteral("clips clear"), dspi_clear_clips(core));
    b->clearClips();
    expect(QStringLiteral("no clip marker stays"), b->clipFlags() == 0);
    expect(QStringLiteral("the bootloader request goes"), dspi_enter_bootloader(core));
    m_.firmware->enterBootloader();
    expect(QStringLiteral("neither is refused"), dspi_read_only_refusals(core) == refusals);
    expect(QStringLiteral("and the update window opens"), [&]() {
        click(main, action->mapToScene(QPointF(action->width() / 2, action->height() / 2)));
        spin(150);
        bool open = false;
        for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
            if (w != main && w->isVisible() && w->metaObject()->indexOfSignal("exportRequested()") >= 0) { open = true; w->setVisible(false); }
        return open;
    }());

    // ── Called directly, every setter is stopped: the core's the last line ──
    testSweep();
    expect(QStringLiteral("after every setter, the device is as it was"), deviceSettings(core) == settingsBefore);
    emit b->stateChanged();
}

// A write the worker holds when the device changes: the controller stops it before
// the next device opens, not when stateChanged comes at the end.
void SmokeDriver::testDeviceChanging() {
    section("QD6: a queued write stops before the next device opens");
    auto *b = m_.bridge;
    FfiCore *core = b->core();
    auto *names = controller<NamesController>(m_.engine, "deviceNames");
    auto *latency = controller<OutputLatencyController>(m_.engine, "outputLatency");
    auto *ext = controller<ExtController>(m_.engine, "ext");
    check("QD6: the controllers", names && latency && ext);
    if (!names || !latency || !ext) return;
    const QString serial = QString::fromLatin1(kDemoSerial);
    b->selectDevice(serial);
    spin(300);
    check("QD6: writable on its own firmware", b->writable());

    std::atomic<bool> release { false };
    std::atomic<int> held { 0 };
    const auto hook = [&]() {
        held++;
        while (!release) QThread::msleep(2);
        return true;
    };
    // Runs after the controllers' own handlers, and gives the worker time to act
    // while the device it had is still open
    bool stillOpen = false;
    bool firedEarly = false;
    const auto during = [&](const std::function<void()> &read) {
        QMetaObject::Connection c = connect(b, &DSPiBridge::deviceChanging, this, [&, read]() {
            stillOpen = b->connected();
            firedEarly = true;
            release = true;
            QThread::msleep(80);
            read();
        });
        b->selectDevice(serial);
        disconnect(c);
        release = false;
        held = 0;
    };

    // Names
    char name[32] = {};
    names->setWriteHook(hook);
    names->editDevice(QStringLiteral("QD6 late"));
    names->save();
    check("QD6: names: the worker holds the write", waitFor(1000, [&]() { return held > 0; }));
    during([&]() { dspi_cached_device_name(core, name, sizeof(name)); });
    check("QD6: the signal comes with the old device still open", firedEarly && stillOpen);
    check("QD6: names: the write stopped", QString::fromUtf8(name) != QLatin1String("QD6 late"));
    check("QD6: names: it finishes", waitFor(1000, [&]() { return !names->saving(); }));
    names->setWriteHook(nullptr);
    spin(200);
    check("QD6: names: nothing arrived later", dspi_cached_device_name(core, name, sizeof(name)) && QString::fromUtf8(name) != QLatin1String("QD6 late"));
    names->revert();

    // Output latency
    uint32_t sets = 0;
    const uint32_t setsBefore = dspi_demo_latency_sets(core);
    latency->setWriteHook(hook);
    check("QD6: latency: it has the setting", waitFor(1000, [&]() { return latency->supported(); }));
    latency->edit(3);
    latency->save();
    check("QD6: latency: the worker holds the write", waitFor(1000, [&]() { return held > 0; }));
    during([&]() { sets = dspi_demo_latency_sets(core); });
    check("QD6: latency: the write stopped", sets == setsBefore);
    check("QD6: latency: it finishes", waitFor(2000, [&]() { return !latency->applying(); }));
    latency->setWriteHook(nullptr);
    latency->revert();

    // Pair map
    uint8_t before[8] = {}, after[8] = {};
    check("QD6: pair map: it has one", dspi_fetch_crossfeed_pair_map(core, before, sizeof(before)));
    ext->setWriteHook(hook);
    check("QD6: pair map: the preset is taken", ext->applyPreset(QStringLiteral("k340")));
    check("QD6: pair map: the worker holds the write", waitFor(1000, [&]() { return held > 0; }));
    during([&]() { dspi_fetch_crossfeed_pair_map(core, after, sizeof(after)); });
    check("QD6: pair map: the write stopped", memcmp(before, after, sizeof(before)) == 0);
    spin(200);
    ext->setWriteHook(nullptr);
}

void SmokeDriver::testReadOnly() {
    testDeviceChanging();
    section("QD6: read-only firmware");
    auto *b = m_.bridge;
    for (int wire : { 17, 26, 31, 33 }) testReadOnlyFixture(wire);
    // Back on its own firmware, everything is on again
    b->setDemoWire(0);
    b->selectDevice(QString::fromLatin1(kDemoSerial));
    spin(300);
    check("QD6: back on V32, writable", b->writable() && b->compat() == COMPAT_OK);
    auto *main = qobject_cast<QQuickWindow *>(root());
    QQuickItem *banner = main ? findItem(main->contentItem(), QStringLiteral("readOnlyBanner")) : nullptr;
    check("QD6: back on V32, no banner", banner && !banner->isVisible());
    QQuickWindow *tube = root()->findChild<QQuickWindow *>(QStringLiteral("tubeWindow"));
    QQuickItem *content = tube ? findItem(tube->contentItem(), QStringLiteral("appWindowContent")) : nullptr;
    check("QD6: back on V32, the windows are live", content && content->isEnabled());
    const uint32_t refusals = dspi_read_only_refusals(b->core());
    b->setBypass(true);
    b->setBypass(false);
    check("QD6: back on V32, a setter goes through", dspi_read_only_refusals(b->core()) == refusals);
    b->disconnectDevice();
    check("QD6: no device: nothing to write to", !b->writable());
    b->selectDevice(QString::fromLatin1(kDemoSerial));
    spin(200);
}
