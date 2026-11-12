// -- WP QP16 -- Read-only mode (behaviour.md §14) on the real app, for each of
// the recorded firmware fixtures V17, V26, V31 (older) and V33 (newer): the
// banner and its way out, what the firmware lacks hidden, reads going on, and
// every control on every window clicked, right-clicked and scrolled with no
// write reaching the device. What stays: clearing the clip marks and entering
// the bootloader. A write queued for one device never reaches the next (QD6).
// The delay-refusal banner (behaviour.md §15) on the writable demo.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "FirmwareController.h"
#include "HardwareController.h"
#include "Poller.h"
#include "ReadOnlyController.h"
#include "Registrar.h"
#include "RtaController.h"
#include "SettingsController.h"
#include "Shell.h"
#include "UnsavedGuard.h"

#include "CoreApi.h"

#include <QApplication>
#include <QCoreApplication>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <QThread>
#include <QWheelEvent>
#include <QWidget>
#include <atomic>
#include <cstdio>
#include <functional>

namespace {

// What a person works: not a scroll bar, not plain text
bool isControl(const QObject *o) {
    if (o->inherits("QQuickScrollBar")) return false;
    return o->inherits("QQuickMouseArea") || o->inherits("QQuickControl") || o->inherits("QQuickTextInput") ||
           o->inherits("QQuickTextEdit");
}

// An area with no button (the wheel zones) takes no click
bool takesClicks(const QQuickItem *item) {
    return !item->inherits("QQuickMouseArea") || item->property("acceptedButtons").toInt() != Qt::NoButton;
}

// The controls under `item`, leaving out the subtrees named in `skip`
void collect(QQuickItem *item, const QStringList &skip, QVector<QPointer<QQuickItem>> *out) {
    if (!item->isVisible() || skip.contains(item->objectName())) return;
    if (isControl(item) && takesClicks(item)) out->append(item);
    for (QQuickItem *c : item->childItems()) collect(c, skip, out);
}

QString describe(const QQuickItem *item) {
    QStringList path;
    for (const QQuickItem *i = item; i && path.size() < 5; i = i->parentItem()) {
        const QString name = i->objectName();
        path << (name.isEmpty() ? QString::fromLatin1(i->metaObject()->className()).section(QLatin1String("_QML"), 0, 0) : name);
    }
    return path.join(QLatin1String(" < "));
}

void wheel(QQuickWindow *w, const QPointF &at, int delta) {
    QWheelEvent e(at, QPointF(w->mapToGlobal(at.toPoint())), QPoint(), QPoint(0, delta), Qt::NoButton, Qt::NoModifier,
                  Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(w, &e);
}

// What a write would change, read back from the device. Worker thread.
QByteArray deviceSettings(FfiCore *core) {
    if (!dspi_fetch_all(core)) return QByteArray();
    const DspState *s = dspi_get_state(core);
    QByteArray out;
    const auto add = [&out](const void *p, size_t n) { out.append(static_cast<const char *>(p), int(n)); };
    for (int w = 0; w < s->num_channels; w++) {
        for (int b = 0; b < BANDS_PER_CHANNEL; ++b) {
            const FilterParams &f = s->filters[w][b];
            add(&f.filter_type, sizeof(f.filter_type));
            add(&f.freq, sizeof(f.freq));
            add(&f.gain, sizeof(f.gain));
            add(&f.q, sizeof(f.q));
            add(&f.bypass, sizeof(f.bypass));
        }
    }
    add(s->channel_names, sizeof(s->channel_names));
    add(s->matrix_gain, sizeof(s->matrix_gain));
    add(s->output_delay_ms, sizeof(s->output_delay_ms));
    add(&s->master_volume_db, sizeof(s->master_volume_db));
    add(&s->user_volume_db, sizeof(s->user_volume_db));
    add(&s->bypass, sizeof(s->bypass));
    add(&s->input_source, sizeof(s->input_source));
    return out;
}

struct Qp16Ro {
    SmokeDriver &d;
    DeviceModel *device;
    QString tag;
    // Writes a control asked for: refused by DeviceModel (writeFailed) or sent
    // (writeFinished ok); the two kinds a read-only device takes are left out
    QStringList attempts;
    int clicked = 0;
    int total = 0;

    uint32_t coreRefusals() {
        std::atomic<uint32_t> n{ 0 };
        d.app().core->worker()->post([&n](FfiCore *c) { n = dspi_read_only_refusals(c); });
        d.waitIdle();
        return n.load();
    }
    QByteArray settings() {
        QByteArray out;
        std::atomic<bool> done{ false };
        d.app().core->worker()->post([&](FfiCore *c) {
            out = deviceSettings(c);
            done = true;
        });
        d.waitFor(5000, [&]() { return done.load(); });
        return out;
    }

    // Whatever a click opened: popups closed, other windows hidden, file dialogs shut
    void tidy(QQuickWindow *keep) {
        const QList<QWindow *> windows = QGuiApplication::topLevelWindows();
        for (QWindow *w : windows) {
            auto *qw = qobject_cast<QQuickWindow *>(w);
            if (!qw) continue;
            for (QObject *o : qw->findChildren<QObject *>())
                if (o->inherits("QQuickPopup") && o->property("opened").toBool()) QMetaObject::invokeMethod(o, "close");
            if (qw != keep && qw != d.mainWindow() && qw->isVisible()) qw->close();
        }
        for (QWidget *w : QApplication::topLevelWidgets())
            if (w->isVisible()) w->close();
    }

    // Clicks, right-clicks and scrolls on every enabled control under `scope`
    void clickAround(QQuickWindow *window, QQuickItem *scope, const QString &where, const QStringList &skip = {}) {
        if (!window || !scope) {
            d.check(tag + where + QStringLiteral(": shown"), false);
            return;
        }
        QVector<QPointer<QQuickItem>> items;
        collect(scope, skip, &items);
        const int before = int(attempts.size());
        int n = 0;
        for (const QPointer<QQuickItem> &item : items) {
            if (!item || !item->isVisible() || !item->isEnabled() || item->window() != window) continue;
            const QPointF at = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
            if (at.x() < 0 || at.y() < 0 || at.x() >= window->width() || at.y() >= window->height()) continue;
            const int had = int(attempts.size());
            QTest::mouseClick(window, Qt::LeftButton, Qt::NoModifier, at.toPoint());
            QTest::mouseClick(window, Qt::RightButton, Qt::NoModifier, at.toPoint());
            wheel(window, at, 120);
            QCoreApplication::processEvents();
            d.waitIdle(2000);
            if (attempts.size() != had && item)
                printf("FAIL %s%s: this control tried to write (%s): %s\n", qPrintable(tag), qPrintable(where),
                       qPrintable(attempts.mid(had).join(QLatin1String(", "))), qPrintable(describe(item)));
            tidy(window);
            ++n;
        }
        clicked += n;
        total += int(items.size());
        printf("smoke: %s%s: %d clicked of %d\n", qPrintable(tag), qPrintable(where), n, int(items.size()));
        fflush(stdout);
        d.check(tag + where + QStringLiteral(": no control writes"), int(attempts.size()) == before);
    }
};

void openFixture(SmokeDriver &d, int wire) {
    d.app().core->worker()->post([wire](FfiCore *c) { dspi_demo_open_wire(c, uint8_t(wire)); });
    d.app().device->refresh();
}

void fixture(Qp16Ro &r, int wire) {
    SmokeDriver &d = r.d;
    DeviceModel *device = r.device;
    QQuickWindow *main = d.mainWindow();
    r.tag = QStringLiteral("qp16 V%1: ").arg(wire);
    const QString &tag = r.tag;
    openFixture(d, wire);
    const int want = wire > 32 ? COMPAT_FIRMWARE_TOO_NEW : COMPAT_FIRMWARE_TOO_OLD;
    d.check(tag + QStringLiteral("opens read-only, as firmware of that format"),
            d.waitFor(5000, [&]() { return device->readOnly() && device->compat() == want && device->wireFormat() == wire; }));
    d.waitIdle();
    main->setProperty("selectedChannel", -1);
    d.spin(150);

    // ---- The banner ----
    auto *ro = Registrar::instance<ReadOnlyController>();
    auto *fw = Registrar::instance<FirmwareController>();
    QObject *banner = d.object(QStringLiteral("ReadOnlyBanner"));
    QObject *action = d.object(QStringLiteral("ReadOnlyBanner.action"));
    QObject *mismatch = d.object(QStringLiteral("FirmwareBanner"));
    d.check(tag + QStringLiteral("the read-only banner shows, the mismatch banner does not"),
            banner && banner->property("visible").toBool() && mismatch && !mismatch->property("visible").toBool());
    const QString text = ro ? ro->text() : QString();
    printf("smoke: %s%s\n", qPrintable(tag), qPrintable(text));
    d.check(tag + QStringLiteral("it says read-only, with the device's firmware"),
            text.contains(QLatin1String("read-only")) && fw && text.contains(fw->runningVersion()) && !fw->runningVersion().isEmpty());
    d.check(tag + (wire > 32 ? QStringLiteral("newer: it offers Details...") : QStringLiteral("older: it offers Update...")),
            action && action->property("visible").toBool() &&
                action->property("text").toString() == (wire > 32 ? QLatin1String("Details...") : QLatin1String("Update...")));
    d.check(tag + QStringLiteral("screenshot"), d.grab(main, QStringLiteral("readonly-v%1.png").arg(wire)));

    // ---- What the firmware lacks is hidden ----
    const uint32_t features = device->snapshot().wireFeatures;
    QObject *psybass = d.object(QStringLiteral("QuickStrip.psybass"));
    d.check(tag + QStringLiteral("the quick strip's psychoacoustic bass follows the firmware"),
            psybass && psybass->property("visible").toBool() == ((features & WIRE_FEAT_PSYBASS) != 0));
    if (wire == 17) d.check(tag + QStringLiteral("V17 has no psychoacoustic bass"), psybass && !psybass->property("visible").toBool());
    if (wire == 33) d.check(tag + QStringLiteral("newer firmware hides nothing it knows"), psybass && psybass->property("visible").toBool());

    // ---- Reads go on ----
    Poller *poller = d.app().device ? Poller::instance() : nullptr;
    const int meterReads = poller ? poller->counts(poller->meterPoll()).completed : 0;
    d.check(tag + QStringLiteral("the meters keep polling"),
            poller && d.waitFor(2000, [&]() { return poller->counts(poller->meterPoll()).completed > meterReads + 2; }));

    const QByteArray settingsBefore = r.settings();
    d.check(tag + QStringLiteral("the settings read back"), !settingsBefore.isEmpty());
    uint32_t refusals = r.coreRefusals();
    r.attempts.clear();
    r.clicked = r.total = 0;

    // ---- Every control on every window ----
    // The menu bar is not in the content item; the device button reconnects
    // (to the plain demo) and the banners are checked above
    const QStringList mainSkip{ QStringLiteral("DeviceStatus.button"), QStringLiteral("BannerHost") };
    r.clickAround(main, main->contentItem(), QStringLiteral("main window"), mainSkip);
    const ChannelListModel *rows = device->channels();
    int input = -1, output = -1;
    for (int i = 0; i < rows->rowCount(); ++i) {
        const int ch = rows->get(i, QStringLiteral("channel")).toInt();
        if (rows->get(i, QStringLiteral("kind")).toString() == QLatin1String("input") && input < 0) input = ch;
        if (rows->get(i, QStringLiteral("kind")).toString() == QLatin1String("output") && output < 0) output = ch;
    }
    for (const auto &[channel, page] : { std::make_pair(input, QStringLiteral("input page")), std::make_pair(output, QStringLiteral("output page")) }) {
        main->setProperty("selectedChannel", channel);
        d.spin(200);
        if (page == QLatin1String("output page")) {
            // Any limiter cell showing on the output page
            bool limiterShown = false;
            const std::function<void(QQuickItem *)> find = [&](QQuickItem *i) {
                if (!i->isVisible()) return;
                if (i->objectName() == QLatin1String("OutputCard.limiter")) limiterShown = true;
                for (QQuickItem *c : i->childItems()) find(c);
            };
            if (QQuickItem *pane = d.item(QStringLiteral("RightPane"))) find(pane);
            d.check(tag + QStringLiteral("the output card's limiter follows the firmware"),
                    limiterShown == ((features & WIRE_FEAT_LIMITER) != 0));
            if (wire == 17) d.check(tag + QStringLiteral("V17 has no limiter cell"), !limiterShown);
            d.check(tag + QStringLiteral("screenshot of an output"), d.grab(main, QStringLiteral("readonly-v%1-output.png").arg(wire)));
        }
        r.clickAround(main, d.item(QStringLiteral("RightPane")), page, mainSkip);
    }
    main->setProperty("selectedChannel", -1);
    d.spin(100);

    // The tool windows registered (the firmware window is its own: it may restart the device)
    Shell *shell = d.app().shell;
    for (const Registrar::WindowEntry &entry : Registrar::windows()) {
        const QString id = QString::fromLatin1(entry.action);
        if (id == QLatin1String("device.firmwareUpdate") || id == QLatin1String("edit.settings") || id.startsWith(QLatin1String("smoke.")))
            continue;
        auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(id));
        if (!w || !d.waitFor(2000, [w]() { return w->isVisible(); })) {
            d.check(tag + QStringLiteral("%1 opens").arg(id), false);
            continue;
        }
        d.spin(150);
        if (id == QLatin1String("tools.spectrum")) {
            // -- WP QPI3 -- Opening the analyser is the person asking for it:
            // the core's one refusal, and the window says why
            auto *rta = Registrar::instance<RtaController>();
            QObject *notice = d.item(QStringLiteral("SpectrumWindow.unavailable"), w->contentItem());
            const bool said = d.waitFor(2000, [&]() { return rta && rta->readOnlyRefused(); });
            const uint32_t after = r.coreRefusals();
            d.check(tag + QStringLiteral("qpi3: opening the analyser asks the core once; it refuses, and the window says read-only"),
                    said && after == refusals + 1 && notice && notice->property("visible").toBool() &&
                        notice->property("caption").toString().contains(QLatin1String("read-only")));
            refusals = after;
        }
        r.clickAround(w, w->contentItem(), id);
        w->close();
        d.spin(30);
    }

    // Settings: the device's pages, and Advanced (Reset Channel Names)
    auto *settings = Registrar::instance<SettingsController>();
    auto *sw = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check(tag + QStringLiteral("the settings window"), settings && sw && d.waitFor(2000, [sw]() { return sw->isVisible(); }));
    if (settings && sw) {
        for (const SettingsRegistry::PageEntry &page : SettingsRegistry::pages()) {
            const QString id = QString::fromLatin1(page.id);
            if (page.group != SettingsRegistry::System && page.group != SettingsRegistry::Control &&
                page.group != SettingsRegistry::Hidden && id != QLatin1String("advanced"))
                continue;
            settings->open(id);
            d.spin(150);
            if (settings->currentPage() != id) continue;   // a page this firmware lacks
            r.clickAround(sw, d.item(QStringLiteral("Settings.page"), sw->contentItem()), QStringLiteral("settings %1").arg(id));
        }
        settings->open(QStringLiteral("global"));
        d.spin(150);
        d.check(tag + QStringLiteral("screenshot of the settings"), d.grab(sw, QStringLiteral("readonly-v%1-settings.png").arg(wire)));
        sw->close();
    }
    printf("smoke: %sclicked %d controls of %d\n", qPrintable(tag), r.clicked, r.total);

    // ---- The controllers themselves send nothing ----
    if (auto *cc = Registrar::instance<ChannelController>()) {
        d.check(tag + QStringLiteral("the channel controller is not writable"), !cc->writable());
        cc->setOutputGain(0, -7);
        cc->setOutputDelay(0, 3);
        cc->setOutputMute(0, true);
        d.waitIdle();
    }
    device->setChannelName(0, QStringLiteral("qp16"));
    d.waitIdle();
    d.check(tag + QStringLiteral("nothing reached the core's setters"), r.coreRefusals() == refusals);
    d.check(tag + QStringLiteral("the device's settings did not change"), r.settings() == settingsBefore);

    // ---- What stays: clearing the clips, and the bootloader ----
    bool cleared = false;
    device->writeEvenReadOnly(QStringLiteral("clear clips"), [](FfiCore *c) { return dspi_clear_clips(c); }, [&](bool ok) { cleared = ok; });
    d.check(tag + QStringLiteral("the clip marks clear on the device"), d.waitFor(2000, [&]() { return cleared; }));
    if (wire == 17 && fw) {
        int requests = 0;
        QMetaObject::Connection c = QObject::connect(device, &DeviceModel::writeFinished, device,
                                                     [&](const QString &what, bool ok, const QString &, const QString &) {
                                                         if (what == QLatin1String("enter bootloader") && ok) ++requests;
                                                     });
        fw->enterBootloaderOnly();
        d.check(tag + QStringLiteral("entering the bootloader is not refused"), d.waitFor(2000, [&]() { return requests == 1; }));
        QObject::disconnect(c);
        openFixture(d, wire);   // as if back from the bootloader
        d.waitFor(3000, [&]() { return device->readOnly(); });
        d.waitIdle();
    }
    d.check(tag + QStringLiteral("neither counts as a refused write"), r.coreRefusals() == refusals);
    if (action) {
        QQuickItem *button = qobject_cast<QQuickItem *>(action);
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, button->mapToScene(QPointF(button->width() / 2, button->height() / 2)).toPoint());
        QWindow *fwWindow = shell->openWindow(QStringLiteral("device.firmwareUpdate"));
        d.check(tag + QStringLiteral("the banner's button opens the firmware window"), fwWindow && d.waitFor(2000, [&]() { return fwWindow->isVisible(); }));
        if (fwWindow) fwWindow->close();
    }
}

// A write asked for while another device is already opening never reaches it (QD6)
void queuedWrite(Qp16Ro &r) {
    SmokeDriver &d = r.d;
    DeviceModel *device = r.device;
    device->openDemo();
    d.waitFor(3000, [&]() { return device->connected() && !device->readOnly(); });
    d.waitIdle();
    std::atomic<bool> release{ false };
    // Hold the worker, so the next device opens only after the write is asked for
    d.app().core->worker()->post([&release](FfiCore *) {
        while (!release) QThread::msleep(2);
    });
    device->openDemo();   // queued behind the hold
    const int setters = device->settersRun();
    QString dropped;
    QMetaObject::Connection c = QObject::connect(device, &DeviceModel::writeFinished, device,
                                                 [&](const QString &what, bool, const QString &code, const QString &) {
                                                     if (what == QLatin1String("channel name 0")) dropped = code;
                                                 });
    device->setChannelName(0, QStringLiteral("QP16 late"));   // still for the old connection
    release = true;
    d.check(QStringLiteral("qp16: a write queued for the device that left is dropped"),
            d.waitFor(3000, [&]() { return dropped == QLatin1String("dropped"); }) && device->settersRun() == setters);
    d.waitIdle();
    d.check(QStringLiteral("qp16: and the next device does not get it"),
            device->channels()->rowCount() > 0 && device->channels()->get(0, QStringLiteral("name")).toString() != QLatin1String("QP16 late"));
    QObject::disconnect(c);
}

// The delay-refusal banner on the writable demo (behaviour.md §15)
void delayRefusal(Qp16Ro &r) {
    SmokeDriver &d = r.d;
    DeviceModel *device = r.device;
    auto *ro = Registrar::instance<ReadOnlyController>();
    auto *cc = Registrar::instance<ChannelController>();
    QObject *banner = d.object(QStringLiteral("DelayRefusalBanner"));
    d.check(QStringLiteral("qp16: the delay-refusal banner is there, hidden"), ro && cc && banner && !banner->property("visible").toBool());
    if (!ro || !cc || !banner) return;
    std::atomic<float> limit{ 0 };
    d.app().core->worker()->post([&limit](FfiCore *c) { limit = dspi_max_delay_ms(c); });
    d.waitIdle();
    const double past = double(limit.load()) + 5;
    cc->setOutputDelay(0, past);
    d.check(QStringLiteral("qp16: a delay past the line shows the core's sentence"),
            d.waitFor(3000, [&]() { return banner->property("visible").toBool(); }) &&
                ro->delayRefusal().contains(QLatin1String("out of range")) &&
                ro->delayRefusal().contains(QStringLiteral("0 to %1 ms").arg(double(limit.load()))));
    printf("smoke: qp16: delay refusal: %s\n", qPrintable(ro->delayRefusal()));
    d.check(QStringLiteral("qp16: and the delay stays as the device has it"),
            device->state() && device->state()->output_delay_ms[0] <= limit.load());
    d.check(QStringLiteral("qp16: screenshot of the delay refusal"), d.grab(d.mainWindow(), QStringLiteral("window-delay-refusal.png")));
    const float kept = device->state() ? device->state()->output_delay_ms[0] : 0;
    cc->setOutputDelay(0, 1.5);
    d.check(QStringLiteral("qp16: the next accepted delay clears it"),
            d.waitFor(3000, [&]() { return !banner->property("visible").toBool(); }));
    cc->setOutputDelay(0, past);
    d.waitFor(3000, [&]() { return banner->property("visible").toBool(); });
    QQuickItem *dismiss = nullptr;
    if (auto *item = qobject_cast<QQuickItem *>(banner)) {
        const std::function<void(QQuickItem *)> find = [&](QQuickItem *i) {
            if (!dismiss && i->inherits("QQuickControl") && i->isVisible() && i->property("iconName").toString() == QLatin1String("x"))
                dismiss = i;
            for (QQuickItem *c : i->childItems()) find(c);
        };
        find(item);
    }
    d.check(QStringLiteral("qp16: it has a dismiss button"), dismiss != nullptr);
    if (dismiss)
        QTest::mouseClick(dismiss->window(), Qt::LeftButton, Qt::NoModifier,
                          dismiss->mapToScene(QPointF(dismiss->width() / 2, dismiss->height() / 2)).toPoint());
    d.check(QStringLiteral("qp16: dismissing hides it"), d.waitFor(2000, [&]() { return !banner->property("visible").toBool(); }));
    cc->setOutputDelay(0, past);
    d.waitFor(3000, [&]() { return banner->property("visible").toBool(); });
    device->openDemo();
    d.check(QStringLiteral("qp16: another device clears it"), d.waitFor(3000, [&]() { return !banner->property("visible").toBool(); }));
    d.waitIdle();
    cc->setOutputDelay(0, double(kept));
    d.waitIdle();
}

// F8: a device switch while Settings holds changes for this device asks first
// (the Mac's "Unsaved Settings Changes"), and Cancel keeps the device
void deviceSwitch(Qp16Ro &r) {
    SmokeDriver &d = r.d;
    auto *hw = Registrar::instance<HardwareController>();
    auto *guard = Registrar::instance<UnsavedGuard>();
    d.check(QStringLiteral("qp16: the hardware controller and the guard"), hw && guard);
    if (!hw || !guard) return;
    const int startMode = hw->draft().value(QStringLiteral("startupMode")).toInt();
    hw->setDraft(QStringLiteral("startupMode"), startMode == 0 ? 1 : 0);
    d.check(QStringLiteral("qp16: a held startup change"), hw->pending());
    QString title;
    QMetaObject::Connection asked = QObject::connect(guard, &UnsavedGuard::settingsQuestion, guard,
                                                     [&](const QString &t, const QString &) { title = t; });
    bool switched = false;
    QObject ctx;
    UnsavedGuard::before(QStringLiteral("device"), &ctx, [&] { switched = true; });
    QObject *dialog = d.object(QStringLiteral("FilesHost.settingsQuestion"));
    d.check(QStringLiteral("qp16: switching devices asks about the unsaved settings first"),
            title == QLatin1String("Unsaved Settings Changes") && dialog && d.waitFor(1000, [&]() { return dialog->property("opened").toBool(); }) &&
                dialog->property("acceptText").toString() == QLatin1String("Discard and Switch"));
    d.check(QStringLiteral("qp16: screenshot of the question"), d.grab(d.mainWindow(), QStringLiteral("window-switch-settings.png")));
    if (dialog) QMetaObject::invokeMethod(dialog, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitIdle();
    d.check(QStringLiteral("qp16: Cancel keeps the device and the held change"), !switched && !guard->asking() && hw->pending());
    title.clear();
    UnsavedGuard::before(QStringLiteral("device"), &ctx, [&] { switched = true; });
    d.check(QStringLiteral("qp16: asked again"), title == QLatin1String("Unsaved Settings Changes"));
    // -- WP QP19 -- Answered on the dialogs, as a user does: an answer given to
    // the guard alone left both open for the rest of the run
    QObject *presetQuestion = d.object(QStringLiteral("FilesHost.unsavedQuestion"));
    const auto answer = [&](QObject *dlg, const char *what, const std::function<void()> &orGuard) {
        if (dlg && d.waitFor(1000, [&]() { return dlg->property("opened").toBool(); }))
            QMetaObject::invokeMethod(dlg, "finish", Q_ARG(QVariant, QLatin1String(what)));
        else
            orGuard();
    };
    answer(dialog, "accept", [&]() { guard->answerSettings(QStringLiteral("accept")); });
    // Then the preset's own question, if the preset has unsaved changes
    d.check(QStringLiteral("qp16: Discard and Switch goes on to the switch"), d.waitFor(3000, [&]() {
        if (guard->asking() && !switched) answer(presetQuestion, "alternative", [&]() { guard->answer(QStringLiteral("alternative")); });
        return switched;
    }));
    QObject::disconnect(asked);
    hw->revert();
    d.check(QStringLiteral("qp16: the held change is dropped again"), !hw->pending());
    title.clear();
    switched = false;
    UnsavedGuard::before(QStringLiteral("device"), &ctx, [&] { switched = true; });
    d.waitFor(3000, [&]() {
        if (guard->asking() && !switched) answer(presetQuestion, "alternative", [&]() { guard->answer(QStringLiteral("alternative")); });
        return switched;
    });
    d.check(QStringLiteral("qp16: nothing held: no settings question"), title.isEmpty() && switched);
}

void readOnlySection(SmokeDriver &d) {
    Qp16Ro r{ d, d.app().device, QString() };
    QMetaObject::Connection refused = QObject::connect(d.app().device, &DeviceModel::writeFailed, d.app().device,
                                                       [&r](const QString &what) { r.attempts.append(what); });
    QMetaObject::Connection sent = QObject::connect(
        d.app().device, &DeviceModel::writeFinished, d.app().device, [&r](const QString &what, bool ok, const QString &, const QString &) {
            if (ok && what != QLatin1String("clear clips") && what != QLatin1String("enter bootloader")) r.attempts.append(what);
        });
    for (int wire : { 17, 26, 31, 33 }) fixture(r, wire);
    QObject::disconnect(refused);
    QObject::disconnect(sent);

    // Back on its own firmware, everything is on again
    DeviceModel *device = d.app().device;
    device->openDemo();
    d.check(QStringLiteral("qp16: back on the demo's own firmware: writable, no banner"),
            d.waitFor(3000, [&]() { return device->connected() && !device->readOnly(); }) &&
                !d.object(QStringLiteral("ReadOnlyBanner"))->property("visible").toBool());
    d.waitIdle();
    const int setters = device->settersRun();
    device->setChannelName(0, device->channels()->get(0, QStringLiteral("name")).toString());
    d.waitIdle();
    d.check(QStringLiteral("qp16: and a write goes through"), device->settersRun() == setters + 1);

    queuedWrite(r);
    delayRefusal(r);
    deviceSwitch(r);
    device->openDemo();
    d.waitFor(3000, [&]() { return device->connected(); });
    d.waitIdle();
}
} // namespace

static const SmokeDriver::Section qp16ReadOnly("qp16-readonly", 165, &readOnlySection);
