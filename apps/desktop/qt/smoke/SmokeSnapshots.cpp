// -- WP P1QT1 -- Snapshots, A/B, the import preview with its blocks, the safe
// apply's countdown bar, and offline files and templates (PLAN-CLIENTS 5.9),
// on the demo device. Each section leaves the demo as it found it; the new
// windows and dialogs are shot in the dark scheme and the light
// (<out>/p1qt1/<name>-<scheme>.png).
#include "Qp18Notices.h"
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "FilesController.h"
#include "HistoryController.h"
#include "ModeController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "Shell.h"
#include "SnapshotsController.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

struct Scheme {
    Qt::ColorScheme scheme;
    const char *name;
};
constexpr Scheme kSchemes[] = { { Qt::ColorScheme::Dark, "dark" }, { Qt::ColorScheme::Light, "light" } };

bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }

void press(QObject *button) {
    if (button) QMetaObject::invokeMethod(button, "clicked");
}

// An item inside a popup (a Dialog's content is not the host's QObject child)
QQuickItem *inPopup(SmokeDriver &d, QObject *popup, const QString &name) {
    QQuickItem *root = popup ? popup->property("contentItem").value<QQuickItem *>() : nullptr;
    return root ? d.item(name, root) : nullptr;
}

// A dialog's footer button: a QObject child of the files host
QQuickItem *footer(FilesController *files, const char *name) {
    return files && files->host() ? files->host()->findChild<QQuickItem *>(QLatin1String(name)) : nullptr;
}

// Shoots `window` in both schemes as p1qt1/<name>-<scheme>.png
void shootBoth(SmokeDriver &d, QQuickWindow *window, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme before = shell->colorSchemeOverride();
    QDir(d.outDir()).mkpath(QStringLiteral("p1qt1"));
    for (const Scheme &s : kSchemes) {
        shell->setColorSchemeOverride(s.scheme);
        d.spin(300);
        d.check(QStringLiteral("p1qt1: screenshot %1, %2").arg(name, QLatin1String(s.name)),
                d.grab(window, QStringLiteral("p1qt1/%1-%2.png").arg(name, QLatin1String(s.name))));
    }
    shell->setColorSchemeOverride(before);
    d.spin(100);
}

float gain0(DeviceModel *device) { return device->state() ? device->state()->output_gain_db[0] : NAN; }

void setGain0(SmokeDriver &d, float db) {
    d.app().device->write(QStringLiteral("p1qt1 gain"), [db](FfiCore *c) { return dspi_set_output_gain(c, 0, db); });
    d.waitIdle();
}

bool near(float a, float b) { return std::fabs(a - b) < 0.05f; }

// The FilesHost alert (a modal) is not open: a success never is one
bool noModal(FilesController *files) {
    QQuickItem *host = files ? files->host() : nullptr;
    QObject *notice = host ? host->findChild<QObject *>(QStringLiteral("FilesHost.notice")) : nullptr;
    return !opened(notice);
}

QString idNamed(SnapshotsController *s, const QString &name) {
    for (const QVariant &v : s->snapshots())
        if (v.toMap().value(QStringLiteral("name")).toString() == name) return v.toMap().value(QStringLiteral("id")).toString();
    return {};
}

bool waitStored(SmokeDriver &d, QSignalSpy &spy, const QString &op) {
    return d.waitFor(8000, [&] {
        for (const QList<QVariant> &args : spy)
            if (args.value(0).toString() == op) return true;
        return false;
    }) && d.waitIdle();
}

bool ensureDemo(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    return d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle();
}

// The unsaved-changes question, if one comes for leaving the demo: Discard
void discardIfAsked(SmokeDriver &d, FilesController *files) {
    QObject *q = files->host()->findChild<QObject *>(QStringLiteral("FilesHost.unsavedQuestion"));
    if (q && d.waitFor(1500, [&] { return opened(q); })) QMetaObject::invokeMethod(q, "finish", Q_ARG(QVariant, QStringLiteral("alternative")));
}

QByteArray readAll(const QString &path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
}

} // namespace

// ---- Snapshots, A/B and the guarded recall ----------------------------------------------

static void snapshotsSection(SmokeDriver &d) {
    qp18NoticeMark();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *files = Registrar::instance<FilesController>();
    auto *notices = Registrar::instance<NoticeCenter>();
    d.check("p1qt1: the snapshots controller is made", snaps && files && notices);
    if (!snaps || !files || !notices) return;
    d.check("p1qt1: on the demo", ensureDemo(d));
    d.check("p1qt1: the menu rows are handled",
            shell->handles(QStringLiteral("device.snapshots")) && shell->handles(QStringLiteral("device.abToggle")));
    auto *window = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.snapshots")));
    d.check("p1qt1: Device > Snapshots… opens the window", window && d.waitFor(3000, [&] { return window->isVisible(); }));
    if (!window) return;
    QSignalSpy stored(snaps, &SnapshotsController::stored);
    d.waitFor(3000, [&] { return !snaps->busy(); });
    const int first = int(snaps->snapshots().size());

    // ---- Capture two, recall the first ----
    const float start = gain0(device);
    const float other = start - 3.0f;
    snaps->capture(QStringLiteral("Bench A"), QStringLiteral("before the change"));
    d.check("p1qt1: Capture files a snapshot", waitStored(d, stored, QStringLiteral("capture"))
                                                 && d.waitFor(3000, [&] { return snaps->snapshots().size() == first + 1; }));
    d.check("p1qt1: newest first, with its name, note and time",
            !snaps->snapshots().isEmpty() && snaps->snapshots().first().toMap().value(QStringLiteral("name")) == QLatin1String("Bench A")
                && snaps->snapshots().first().toMap().value(QStringLiteral("note")) == QLatin1String("before the change")
                && !snaps->snapshots().first().toMap().value(QStringLiteral("saved")).toString().isEmpty());
    setGain0(d, other);
    stored.clear();
    snaps->capture(QStringLiteral("Bench B"), QString());
    d.check("p1qt1: a second capture", waitStored(d, stored, QStringLiteral("capture"))
                                         && d.waitFor(3000, [&] { return snaps->snapshots().size() == first + 2; }));
    const QString idA = idNamed(snaps, QStringLiteral("Bench A"));
    const QString idB = idNamed(snaps, QStringLiteral("Bench B"));
    d.check("p1qt1: both are listed", !idA.isEmpty() && !idB.isEmpty());
    d.check("p1qt1: capture is no modal", noModal(files));

    qp18NoticeMark();
    snaps->setRollback(false);
    snaps->recall(idA);
    d.check("p1qt1: Recall puts the settings back", d.waitFor(8000, [&] { return near(gain0(device), start); }) && d.waitIdle());
    d.check("p1qt1: and says so in a notice, not a modal",
            d.waitFor(3000, [] { return qp18NoticeNew(); }) && qp18TakeNotice() == QStringLiteral("Recalled “Bench A”.") && noModal(files));
    d.check("p1qt1: a recall ends the undo history before it", !Registrar::instance<HistoryController>()
                                                                     || !Registrar::instance<HistoryController>()->canUndo()
                                                                     || d.waitFor(3000, [] { return !Registrar::instance<HistoryController>()->canUndo(); }));

    // ---- Rename, note (any Unicode), export, import, delete ----
    const QString renamed = QStringLiteral("Bänk A — 試聴 ✓");
    stored.clear();
    snaps->rename(idA, renamed);
    snaps->setNote(idA, QStringLiteral("Noted ♪"));
    d.check("p1qt1: rename and note", waitStored(d, stored, QStringLiteral("note")) && d.waitFor(3000, [&] {
        return idNamed(snaps, renamed) == idA;
    }));
    const QString dir = d.outDir() + QStringLiteral("/p1qt1");
    QDir().mkpath(dir);
    const QString exported = dir + QStringLiteral("/bench-a.dspipreset");
    QFile::remove(exported);
    stored.clear();
    snaps->exportTo(idA, exported);
    const bool exportDone = waitStored(d, stored, QStringLiteral("export"));
    const QJsonObject doc = QJsonDocument::fromJson(readAll(exported)).object();
    d.check("p1qt1: Export writes the snapshot's document",
            exportDone
                && doc.value(QLatin1String("meta")).toObject().value(QLatin1String("kind")).toString() == QLatin1String("snapshot")
                && doc.value(QLatin1String("meta")).toObject().value(QLatin1String("name")).toString() == renamed);
    const int before = int(snaps->snapshots().size());
    stored.clear();
    snaps->importFrom(exported);
    d.check("p1qt1: Import files a document as a snapshot",
            waitStored(d, stored, QStringLiteral("import")) && d.waitFor(3000, [&] { return snaps->snapshots().size() == before + 1; }));
    const QString imported = snaps->snapshots().isEmpty() ? QString() : snaps->snapshots().first().toMap().value(QStringLiteral("id")).toString();
    stored.clear();
    snaps->remove(imported);
    d.check("p1qt1: Delete removes it", waitStored(d, stored, QStringLiteral("delete"))
                                          && d.waitFor(3000, [&] { return snaps->snapshots().size() == before; }));
    // The window's own Delete asks first
    if (QQuickItem *list = d.item(QStringLiteral("SnapshotsWindow.list"), window->contentItem())) list->setProperty("currentIndex", 0);
    d.spin(100);
    QObject *question = window->contentItem()->findChild<QObject *>(QStringLiteral("SnapshotsWindow.deleteQuestion"));
    if (!question) question = window->findChild<QObject *>(QStringLiteral("SnapshotsWindow.deleteQuestion"));
    press(d.item(QStringLiteral("SnapshotsWindow.delete"), window->contentItem()));
    d.check("p1qt1: Delete… asks first, as a destructive question",
            question && d.waitFor(3000, [&] { return opened(question); }) && question->property("destructive").toBool());
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitFor(2000, [&] { return !opened(question); });
    d.check("p1qt1: Cancel deletes nothing", snaps->snapshots().size() == before);

    // ---- A/B ----
    snaps->abStart(QString(), idB);
    d.check("p1qt1: Compare with Current starts the compare on A",
            d.waitFor(8000, [&] { return snaps->abActive() && snaps->abSide() == QLatin1String("a"); }) && d.waitIdle());
    d.check("p1qt1: A/B names its sides", snaps->abBName() == QLatin1String("Bench B") && !snaps->abAName().isEmpty());
    d.check("p1qt1: the diff from A to B names the output gain", d.waitFor(5000, [&] {
        for (const QVariant &r : snaps->abRows())
            if (r.toMap().value(QStringLiteral("block")).toString() == QLatin1String("outputs")) return true;
        return false;
    }));
    snaps->abToggle();
    d.check("p1qt1: Toggle recalls B", d.waitFor(8000, [&] { return snaps->abSide() == QLatin1String("b") && near(gain0(device), other); }));
    d.waitIdle();
    shell->trigger(QStringLiteral("device.abToggle"));
    d.check("p1qt1: the A/B key's row toggles back to A",
            d.waitFor(8000, [&] { return snaps->abSide() == QLatin1String("a") && near(gain0(device), start); }));
    d.waitIdle();

    // -- WP P1DCFIX -- the A/B key is a bare B: it toggles, but not while a
    // text field has the keyboard or a dialog asks something
    const auto keyB = [&] {
        window->requestActivate();
        d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == window; });
        QTest::keyClick(window, Qt::Key_B);
        d.spin(50);
    };
    window->contentItem()->forceActiveFocus();
    keyB();
    d.check("p1dcfix: a bare B toggles to B",
            d.waitFor(8000, [&] { return snaps->abSide() == QLatin1String("b") && near(gain0(device), other); }) && d.waitIdle());
    keyB();
    d.check("p1dcfix: and B again back to A",
            d.waitFor(8000, [&] { return snaps->abSide() == QLatin1String("a") && near(gain0(device), start); }) && d.waitIdle());
    if (QQuickItem *field = d.item(QStringLiteral("SnapshotsWindow.captureName"), window->contentItem())) {
        field->forceActiveFocus();
        d.spin(50);
        keyB();
        d.spin(300);
        d.check("p1dcfix: B in a text field is typed, not a toggle",
                snaps->abSide() == QLatin1String("a") && field->property("text").toString().contains(QLatin1Char('b'), Qt::CaseInsensitive));
        field->setProperty("text", QString());
        window->contentItem()->forceActiveFocus();
    } else {
        d.check("p1dcfix: the capture field", false);
    }
    QObject *askB = question;   // the delete question, asked again
    if (askB) QMetaObject::invokeMethod(askB, "open");
    d.check("p1dcfix: a question is open", d.waitFor(3000, [&] { return opened(askB); }));
    keyB();
    d.spin(300);
    d.check("p1dcfix: B while a dialog asks does nothing", snaps->abSide() == QLatin1String("a") && !snaps->busy());
    if (askB) QMetaObject::invokeMethod(askB, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitFor(3000, [&] { return !opened(askB); });
    {
        QObject *row = d.object(QStringLiteral("Menu.device.abToggle"));
        QObject *action = row ? row->property("action").value<QObject *>() : nullptr;
        d.check("p1dcfix: the menu row's shortcut is B", action && action->property("shortcut").toString() == QLatin1String("B"));
    }
    d.waitIdle();
    d.check("p1qt1: A/B is no modal", noModal(files));
    window->raise();
    shootBoth(d, window, QStringLiteral("snapshots-ab"));

    // ---- A guarded recall: the toggle is off, Revert ends the compare ----
    qp18NoticeMark();
    snaps->setRollback(true);
    // -- WP P1DCFIX -- the timeout: 30 s unless set, 5 to 300, used by the recall
    snaps->setGuardTimeoutS(30);
    d.check("p1dcfix: the timeout is 30 s by default", snaps->guardTimeoutS() == SnapshotsController::kGuardTimeoutDefaultS);
    snaps->setGuardTimeoutS(1);
    const int low = snaps->guardTimeoutS();
    snaps->setGuardTimeoutS(1000);
    d.check("p1dcfix: the timeout stays within 5 to 300 s", low == 5 && snaps->guardTimeoutS() == 300);
    snaps->setGuardTimeoutS(45);
    QQuickItem *timeoutBox = d.item(QStringLiteral("SnapshotsWindow.timeout"), window->contentItem());
    d.check("p1dcfix: Advanced shows the timeout", timeoutBox && timeoutBox->isVisible()
                                                       && timeoutBox->property("realValue").toDouble() == 45.0);
    snaps->recall(idB);
    d.check("p1qt1: a recall with rollback waits for Keep", d.waitFor(8000, [&] { return snaps->guardWaiting(); }) && d.waitIdle());
    d.check("p1dcfix: the recall waits the time set", snaps->guardTimeoutMs() == 45000);
    d.check("p1qt1: the device holds B meanwhile", near(gain0(device), other));
    QQuickItem *banner = d.item(QStringLiteral("SafeApplyBanner"));
    d.check("p1qt1: the countdown bar shows, with Keep and Revert",
            banner && banner->isVisible() && snaps->guardRemainingMs() > 0 && snaps->guardTimeoutMs() >= snaps->guardRemainingMs()
                && d.item(QStringLiteral("SafeApplyBanner.keep")) && d.item(QStringLiteral("SafeApplyBanner.revert")));
    d.check("p1qt1: the A/B toggle is off during the countdown", snaps->abActive() && !snaps->canToggle());
    // -- WP P1DCFIX -- the countdown in the core's words (notice guard.waiting)
    {
        const QString said = banner ? banner->property("text").toString() : QString();
        d.check("p1dcfix: the countdown is the core's sentence, with the seconds",
                snaps->guardWaitingText() == QLatin1String("Keep these settings? They go back in {s} s unless you press Keep.")
                    && said.contains(QLatin1String("Bench B")) && said.contains(QLatin1String("Keep these settings? They go back in "))
                    && !said.contains(QLatin1String("{s}")));
    }
    const QString side = snaps->abSide();
    shell->trigger(QStringLiteral("device.abToggle"));
    d.spin(300);
    d.check("p1qt1: and its key does nothing", snaps->abSide() == side && snaps->guardWaiting());
    shootBoth(d, d.mainWindow(), QStringLiteral("safe-apply-bar"));
    shootBoth(d, window, QStringLiteral("snapshots-guarded"));
    press(d.item(QStringLiteral("SafeApplyBanner.revert")));
    d.check("p1qt1: Revert puts the settings from before back",
            d.waitFor(8000, [&] { return snaps->guardPhase() == QLatin1String("reverted"); })
                && d.waitFor(5000, [&] { return near(gain0(device), start); }));
    d.check("p1qt1: a revert ends the compare", d.waitFor(5000, [&] { return !snaps->abActive(); }));
    d.check("p1qt1: the bar is gone and the revert is a notice",
            d.waitFor(3000, [&] { return banner && !banner->isVisible(); }) && d.waitFor(3000, [] { return qp18NoticeNew(); }) && noModal(files));
    d.waitIdle();

    // ---- And kept ----
    snaps->recall(idB);
    d.check("p1qt1: a second guarded recall", d.waitFor(8000, [&] { return snaps->guardWaiting(); }) && d.waitIdle());
    press(d.item(QStringLiteral("SafeApplyBanner.keep")));
    d.check("p1qt1: Keep keeps it", d.waitFor(8000, [&] { return snaps->guardPhase() == QLatin1String("kept"); })
                                       && near(gain0(device), other) && noModal(files));
    snaps->setRollback(false);
    snaps->setGuardTimeoutS(SnapshotsController::kGuardTimeoutDefaultS);   // -- WP P1DCFIX --
    d.waitIdle();

    // -- WP P1DCFIX -- Simple mode: recall and compare only
    if (auto *modes = Registrar::instance<ModeController>()) {
        const QString mode0 = modes->mode();
        const bool ownChoice = modes->rememberedForDevice();
        modes->choose(QStringLiteral("simple"), true);
        d.spin(100);
        const auto shown = [&](const char *name) {
            QQuickItem *i = d.item(QString::fromLatin1(name), window->contentItem());
            return i && i->isVisible();
        };
        d.check("p1dcfix: Simple shows the list, Recall and the compare",
                shown("SnapshotsWindow.list") && shown("SnapshotsWindow.recall") && shown("SnapshotsWindow.compare"));
        d.check("p1dcfix: Simple has no capture, rename, note, export, import or delete",
                !shown("SnapshotsWindow.captureRow") && !shown("SnapshotsWindow.editRow") && !shown("SnapshotsWindow.export")
                    && !shown("SnapshotsWindow.import") && !shown("SnapshotsWindow.delete"));
        d.check("p1dcfix: Simple has no rollback choice and no timeout",
                !shown("SnapshotsWindow.rollback") && !shown("SnapshotsWindow.timeout"));
        window->raise();
        shootBoth(d, window, QStringLiteral("snapshots-simple"));
        modes->choose(mode0, !ownChoice);
        d.spin(100);
        d.check("p1dcfix: back in Advanced, capture and the timeout are back",
                modes->mode() == mode0 && shown("SnapshotsWindow.captureRow") && shown("SnapshotsWindow.timeout"));
    } else {
        d.check("p1dcfix: the mode controller", false);
    }

    // Leave the demo as it was: the snapshots out of the list, the gain back
    for (const QString &id : { idA, idB }) {
        stored.clear();
        snaps->remove(id);
        waitStored(d, stored, QStringLiteral("delete"));
    }
    setGain0(d, start);
    notices->clear();
    window->close();
    d.check("p1qt1: the demo is as it was", near(gain0(device), start) && snaps->snapshots().size() == first);
}
static const SmokeDriver::Section snapshots("p1qt1-snapshots", 155, &snapshotsSection);

// ---- The import's preview and blocks, with rollback ------------------------------------------

static void importSection(SmokeDriver &d) {
    qp18NoticeMark();
    DeviceModel *device = d.app().device;
    auto *snaps = Registrar::instance<SnapshotsController>();
    auto *files = Registrar::instance<FilesController>();
    d.check("p1qt1-import: the controllers", snaps && files && files->host());
    if (!snaps || !files || !files->host()) return;
    d.check("p1qt1-import: on the demo", ensureDemo(d));
    QObject *options = files->host()->findChild<QObject *>(QStringLiteral("ImportConfig"));
    const QString dir = d.outDir() + QStringLiteral("/p1qt1");
    QDir().mkpath(dir);
    const QString file = dir + QStringLiteral("/import.dspipreset");
    QFile::remove(file);
    const float start = gain0(device);
    const FilterParams band0 = device->state()->filters[0][0];
    files->exportConfigurationTo(file);
    d.check("p1qt1-import: a configuration to import", d.waitFor(8000, [&] { return QFile::exists(file); }) && d.waitIdle());
    qp18TakeNotice();

    // The device moves away from the file: an output gain and a band
    setGain0(d, start - 4.0f);
    FilterParams changed = band0;
    changed.filter_type = FILTER_PEAKING;
    changed.freq = 1234.0f;
    changed.gain = 3.0f;
    changed.q = 1.5f;
    device->write(QStringLiteral("p1qt1 band"), [changed](FfiCore *c) { return dspi_set_filter(c, 0, 0, changed); });
    d.waitIdle();

    files->importConfigurationFrom(file);
    d.check("p1qt1-import: the options open with the preview", d.waitFor(5000, [&] { return opened(options); }));
    const QVariantMap preview = options ? options->property("preview").toMap() : QVariantMap();
    const QVariantMap blocks = preview.value(QStringLiteral("blocks")).toMap();
    d.check("p1qt1-import: the preview counts the changed blocks",
            blocks.value(QStringLiteral("outputs")).toInt() >= 1 && blocks.value(QStringLiteral("eq")).toInt() >= 1);
    QQuickItem *outputsBox = inPopup(d, options, QStringLiteral("ImportConfig.outputs"));
    QQuickItem *eqBox = inPopup(d, options, QStringLiteral("ImportConfig.eq"));
    QQuickItem *volumesBox = inPopup(d, options, QStringLiteral("ImportConfig.volumes"));
    QQuickItem *hardwareBox = inPopup(d, options, QStringLiteral("ImportConfig.hardware"));
    QQuickItem *rollback = inPopup(d, options, QStringLiteral("ImportConfig.rollback"));
    d.check("p1qt1-import: a box per block; volumes and hardware unticked, as before",
            outputsBox && eqBox && volumesBox && hardwareBox && rollback && outputsBox->property("checked").toBool()
                && eqBox->property("checked").toBool() && !volumesBox->property("checked").toBool()
                && !hardwareBox->property("checked").toBool());
    d.check("p1qt1-import: the box names its count", outputsBox && outputsBox->property("text").toString().contains(QLatin1Char('(')));
    d.check("p1qt1-import: on USB rollback is the person's choice", rollback && rollback->isEnabled() && !rollback->property("checked").toBool());
    const int all = options ? options->property("shownRows").toList().size() : 0;
    d.check("p1qt1-import: the changes are listed before anything is written", all >= 2 && near(gain0(device), start - 4.0f));
    if (outputsBox) {
        outputsBox->setProperty("checked", false);
        QMetaObject::invokeMethod(outputsBox, "toggled");
    }
    d.spin(100);
    const int withoutOutputs = options ? options->property("shownRows").toList().size() : 0;
    d.check("p1qt1-import: unticking a block takes its rows out", withoutOutputs < all && withoutOutputs >= 1);
    // Only the EQ: the band comes back, the gain stays
    for (const char *key : { "crossover", "routing", "limiters", "names", "effects" })
        if (QQuickItem *box = inPopup(d, options, QStringLiteral("ImportConfig.%1").arg(QLatin1String(key)))) {
            box->setProperty("checked", false);
            QMetaObject::invokeMethod(box, "toggled");
        }
    d.spin(200);
    shootBoth(d, d.mainWindow(), QStringLiteral("import-preview"));
    press(footer(files, "ImportConfig.import"));
    d.check("p1qt1-import: EQ alone brings the band back and leaves the gain",
            d.waitFor(8000, [&] { return !files->importing() && device->state()->filters[0][0].freq == band0.freq; })
                && d.waitIdle() && near(gain0(device), start - 4.0f));
    d.waitFor(3000, [] { return qp18NoticeNew(); });
    qp18TakeNotice();

    // Every block, with rollback: Keep
    files->importConfigurationFrom(file);
    d.check("p1qt1-import: asked again", d.waitFor(5000, [&] { return opened(options); }));
    rollback = inPopup(d, options, QStringLiteral("ImportConfig.rollback"));
    if (rollback) rollback->setProperty("checked", true);
    d.spin(100);
    QQuickItem *importButton = footer(files, "ImportConfig.import");
    d.check("p1qt1-import: the button says Apply with Rollback",
            importButton && importButton->property("text").toString() == QLatin1String("Apply with Rollback"));
    press(importButton);
    d.check("p1qt1-import: a guarded import waits for Keep", d.waitFor(10000, [&] { return snaps->guardWaiting(); }) && d.waitIdle());
    d.check("p1qt1-import: applied meanwhile", near(gain0(device), start));
    press(d.item(QStringLiteral("SafeApplyBanner.keep")));
    d.check("p1qt1-import: Keep keeps it", d.waitFor(8000, [&] { return snaps->guardPhase() == QLatin1String("kept"); })
                                              && near(gain0(device), start) && noModal(files));

    // With rollback: Revert puts it back
    setGain0(d, start - 2.0f);
    files->importConfigurationFrom(file);
    d.check("p1qt1-import: asked a third time", d.waitFor(5000, [&] { return opened(options); }));
    if ((rollback = inPopup(d, options, QStringLiteral("ImportConfig.rollback")))) rollback->setProperty("checked", true);
    press(footer(files, "ImportConfig.import"));
    d.check("p1qt1-import: waiting again", d.waitFor(10000, [&] { return snaps->guardWaiting(); }) && d.waitIdle());
    snaps->revert();
    d.check("p1qt1-import: Revert puts the device back as it was before the import",
            d.waitFor(8000, [&] { return snaps->guardPhase() == QLatin1String("reverted"); })
                && d.waitFor(5000, [&] { return near(gain0(device), start - 2.0f); }));
    d.waitIdle();
    setGain0(d, start);
    device->write(QStringLiteral("p1qt1 band"), [band0](FfiCore *c) { return dspi_set_filter(c, 0, 0, band0); });
    d.waitIdle();
    if (auto *n = Registrar::instance<NoticeCenter>()) n->clear();
    d.check("p1qt1-import: the demo is as it was", near(gain0(device), start) && device->state()->filters[0][0].freq == band0.freq);
}
static const SmokeDriver::Section importChecks("p1qt1-import", 156, &importSection);

// ---- Offline files and templates -----------------------------------------------------------

static void offlineSection(SmokeDriver &d) {
    qp18NoticeMark();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *files = Registrar::instance<FilesController>();
    auto *history = Registrar::instance<HistoryController>();
    d.check("p1qt1-offline: the controllers", files && history && files->host());
    if (!files || !history || !files->host()) return;
    d.check("p1qt1-offline: on the demo", ensureDemo(d));
    d.check("p1qt1-offline: the File rows are handled",
            shell->handles(QStringLiteral("file.openFile")) && shell->handles(QStringLiteral("file.saveFile"))
                && shell->handles(QStringLiteral("file.saveAsTemplate")) && shell->handles(QStringLiteral("file.newFromTemplate")));
    const QString dir = d.outDir() + QStringLiteral("/p1qt1");
    QDir().mkpath(dir);
    const QString original = dir + QStringLiteral("/Studio.dspipreset");
    const QString saved = dir + QStringLiteral("/Studio edited.dspipreset");
    QFile::remove(original);
    QFile::remove(saved);
    const float start = gain0(device);
    files->exportConfigurationTo(original);
    d.check("p1qt1-offline: a file to open", d.waitFor(8000, [&] { return QFile::exists(original); }) && d.waitIdle());
    qp18TakeNotice();

    files->openFileFrom(original);
    discardIfAsked(d, files);
    d.check("p1qt1-offline: Open File… opens it with no device",
            d.waitFor(8000, [&] { return files->offline() && device->connected() && device->document() && !device->demo(); }) && d.waitIdle());
    d.check("p1qt1-offline: the file's name is shown", files->offlineFile() == QLatin1String("Studio.dspipreset"));
    QQuickItem *status = d.item(QStringLiteral("DeviceStatus.button"));
    d.check("p1qt1-offline: the device button says File:",
            status && status->property("shownName").toString() == QLatin1String("File: Studio.dspipreset"));
    QQuickItem *banner = d.item(QStringLiteral("FileBanner"));
    d.check("p1qt1-offline: the file's banner shows", banner && banner->isVisible());
    QQuickItem *demoBanner = d.item(QStringLiteral("DemoBanner"));
    d.check("p1qt1-offline: not the demo's banner", !demoBanner || !demoBanner->isVisible());
    shootBoth(d, d.mainWindow(), QStringLiteral("offline-file"));

    // An edit, an undo, a redo-free edit again, then Save File
    setGain0(d, start - 5.0f);
    d.check("p1qt1-offline: a screen edits the file", near(gain0(device), start - 5.0f));
    d.check("p1qt1-offline: and undo works", d.waitFor(3000, [&] { return history->canUndo(); }));
    history->undo();
    d.check("p1qt1-offline: undone", d.waitFor(5000, [&] { return near(gain0(device), start); }) && d.waitIdle());
    setGain0(d, start - 5.0f);
    qp18NoticeMark();
    files->saveFileTo(saved);
    d.check("p1qt1-offline: Save File writes it, with a notice, no modal",
            d.waitFor(8000, [&] { return QFile::exists(saved); }) && d.waitFor(3000, [] { return qp18NoticeNew(); })
                && qp18TakeNotice() == QLatin1String("Saved Studio edited.dspipreset.") && noModal(files));
    const QByteArray a = readAll(original), b = readAll(saved);
    const QJsonObject diff = QJsonDocument::fromJson(readSized([&](char *buf, uint32_t n) {
                                 return dspi_config_diff(a.constData(), b.constData(), buf, n);
                             })).object();
    const QJsonArray rows = diff.value(QLatin1String("rows")).toArray();
    if (rows.size() != 1) std::printf("smoke: p1qt1-offline: diff %s\n", QJsonDocument(diff).toJson(QJsonDocument::Compact).constData());
    d.check("p1qt1-offline: the saved file differs from the original by exactly the edit",
            rows.size() == 1 && rows.first().toObject().value(QLatin1String("block")).toString() == QLatin1String("outputs"));

    // Templates: the K340 one shipped, Save as Template lists one of ours
    QSignalSpy picker(files, &FilesController::templatesPicker);
    shell->trigger(QStringLiteral("file.newFromTemplate"));
    d.check("p1qt1-offline: New from Template… lists the templates", d.waitFor(5000, [&] { return picker.count() > 0; }));
    QVariantMap k340;
    int k340Row = -1, rowAt = 0;
    for (const QVariant &t : picker.isEmpty() ? QVariantList() : picker.first().value(0).toList()) {
        if (t.toMap().value(QStringLiteral("builtin")).toBool() && t.toMap().value(QStringLiteral("id")).toString().contains(QLatin1String("k340"), Qt::CaseInsensitive)) {
            k340 = t.toMap();
            k340Row = rowAt;
        }
        ++rowAt;
    }
    d.check("p1qt1-offline: the K340 template is shipped", !k340.isEmpty());
    QObject *templates = files->host()->findChild<QObject *>(QStringLiteral("TemplatePicker"));
    d.check("p1qt1-offline: its picker opens", d.waitFor(3000, [&] { return opened(templates); }));
    // -- WP P1DCFIX -- F3: the picker shows the core's status line (the K340 template's notice;
    // -- WP P1EFIX -- approved on 2026-11-14 (31e50222), so no longer "Draft": any status line)
    {
        const QString status = k340.value(QStringLiteral("status")).toString();
        QQuickItem *line = nullptr;
        d.waitFor(3000, [&] { return (line = inPopup(d, templates, QStringLiteral("TemplatePicker.status.%1").arg(k340Row))) != nullptr; });
        d.check("p1dcfix: the K340 template's status line is shown",
                !status.isEmpty() && line && line->isVisible() && line->property("text").toString() == status);
    }
    shootBoth(d, d.mainWindow(), QStringLiteral("template-picker"));
    QMetaObject::invokeMethod(templates, "close");
    d.waitFor(2000, [&] { return !opened(templates); });
    files->openTemplate(k340.value(QStringLiteral("id")).toString(), k340.value(QStringLiteral("name")).toString());
    discardIfAsked(d, files);
    const auto playing = [&] {
        int on = 0;
        for (int o = 0; device->state() && o < device->numOutputs(); ++o) on += device->state()->output_enabled[o] ? 1 : 0;
        return on;
    };
    d.check("p1qt1-offline: the K340 template opens with its 4 outputs on",
            d.waitFor(8000, [&] { return files->offline() && files->offlineFile() == k340.value(QStringLiteral("name")).toString(); })
                && d.waitIdle() && playing() == 4 && device->state()->output_enabled[0] && device->state()->output_enabled[3]);
    const QString mine = FilesController::templatesDir() + QStringLiteral("/Bench template.dspipreset");
    QDir().mkpath(FilesController::templatesDir());
    QFile::remove(mine);
    files->saveTemplateTo(mine);
    d.check("p1qt1-offline: Save as Template… writes a template",
            d.waitFor(8000, [&] { return QFile::exists(mine); })
                && QJsonDocument::fromJson(readAll(mine)).object().value(QLatin1String("meta")).toObject().value(QLatin1String("kind")).toString()
                       == QLatin1String("template"));
    picker.clear();
    files->listTemplates();
    bool listed = false;
    if (d.waitFor(5000, [&] { return picker.count() > 0; }))
        for (const QVariant &t : picker.first().value(0).toList())
            listed = listed || (!t.toMap().value(QStringLiteral("builtin")).toBool()
                                && t.toMap().value(QStringLiteral("name")).toString() == QLatin1String("Bench template"));
    d.check("p1qt1-offline: and lists it after the shipped ones", listed);
    d.waitFor(3000, [&] { return opened(templates); });
    QMetaObject::invokeMethod(templates, "close");
    d.waitFor(2000, [&] { return !opened(templates); });
    QFile::remove(mine);

    // Back to the demo: the file is closed
    device->openDemo();
    d.check("p1qt1-offline: opening the demo leaves the file",
            d.waitFor(8000, [&] { return device->demo() && !device->document(); }) && d.waitFor(3000, [&] { return !files->offline(); })
                && d.waitIdle());
    d.check("p1qt1-offline: the banner is gone", banner && !banner->isVisible());
    if (auto *n = Registrar::instance<NoticeCenter>()) n->clear();
}
static const SmokeDriver::Section offline("p1qt1-offline", 157, &offlineSection);
