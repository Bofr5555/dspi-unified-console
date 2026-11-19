// -- WP PGQT -- The app on each fake protocol generation (PLAN-PG wave PG-D):
// G1s (stock 1.1.6), G1b with and without PAGEGUARD, G2s (stock 1.1.7), G2b
// (ours, dual-stack 1.1.7), the DSPi-Zero and the alink sender, each opened
// through the generation probe (dspi_demo_open_generation). Per generation:
// Settings > About's device info, read once for the connection; the raw
// request by command id with the core's refusals in words; the update screen
// (no image for a board without BOOTSEL, the sender's page in place of the DSP
// screens); and the count of requests the generation must not get, 0. Then
// the flow PG-D part 1's review found a dead end: our firmware on a board at
// 1.15 V and the bundled stock image. The update check asks the lineage
// phrase before the device leaves for its bootloader, so the board is never
// left waiting in BOOTSEL with the write refused; answered, the write passes
// the core's gate on the core that read the board. The write goes to a fake
// drive (a directory under the smoke output); nothing can reach a board.
#include "SmokeDriver.h"

#include "BootloaderLocator.h"
#include "Core.h"
#include "DeviceInfoController.h"
#include "DeviceModel.h"
#include "FirmwareController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMutex>
#include <QMutexLocker>
#include <QQuickItem>
#include <QQuickWindow>
#include <atomic>
#include <memory>

namespace {

const char kLineagePhrase[] = "I ACCEPT LOSING THE EXTENSIONS";

struct Generation {
    int code;
    const char *name;
    const char *generation;
    const char *ext;   // the Extensions row's words
    int board;
    bool sender;
};
const Generation kGenerations[] = {
    { 0, "G1s", "legacy", "none (no extension channel)", 0, false },
    { 1, "G1b", "legacy", "REQ_EXTENSION 0x00 on page 0", 0, false },
    { 2, "G1b without PAGEGUARD", "legacy", "REQ_EXTENSION 0x00 on page 0", 0, false },
    { 3, "G2s", "paged", "none (no extension channel)", 0, false },
    { 4, "G2b", "paged", "tunnel 0xE010 on page 0xE0", 0, false },
    { 5, "Zero", "legacy", "none (no extension channel)", 1, false },
    { 6, "sender", "legacy", "REQ_EXTENSION 0x00 on page 0", 0, true },
};

QString rowValue(DeviceInfoController *info, const QString &key) {
    for (const QVariant &v : info->rows()) {
        const QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("key")).toString() == key) return m.value(QStringLiteral("value")).toString();
    }
    return QString();
}

// A value the next check judges, in the log beside it
void say(const QString &what, const QVariant &value) {
    qInfo("smoke: pgqt %s = %s", qPrintable(what), qPrintable(value.toString()));
}

struct Bus {
    QMutex lock;
    BootloaderLocator::Scan scan;
    BootloaderLocator::Scan read() {
        QMutexLocker hold(&lock);
        return scan;
    }
    void set(int rp2350, const QList<BootloaderLocator::Drive> &drives = {}) {
        QMutexLocker hold(&lock);
        scan.rp2040 = 0;
        scan.rp2350 = rp2350;
        scan.drives = drives;
    }
};

struct Run {
    SmokeDriver &d;
    DeviceInfoController *info;
    FirmwareController *fw;
    std::shared_ptr<Bus> bus = std::make_shared<Bus>();
    QQuickWindow *fwWindow = nullptr;
    int bootloaderRequests = 0;

    // Opens fake `code` as the open device and waits for its device info
    bool open(int code) {
        const int before = info->reads();
        auto ok = std::make_shared<std::atomic<bool>>(false);
        d.app().core->worker()->post([code, ok](FfiCore *c) { ok->store(dspi_demo_open_generation(c, uint8_t(code))); });
        d.app().device->refresh();
        const bool in = d.waitFor(6000, [&]() { return info->known() && info->reads() > before; });
        d.waitIdle();
        return ok->load() && in;
    }
    uint32_t violations() {
        auto n = std::make_shared<std::atomic<uint32_t>>(UINT32_MAX);
        d.app().core->worker()->post([n](FfiCore *c) { n->store(dspi_demo_generation_violations(c)); });
        d.waitIdle();
        return n->load();
    }
    bool raw(bool set, const char *id, const char *value, const char *last) {
        if (set) info->rawSet(QString::fromLatin1(id), QString::fromLatin1(value), QString::fromLatin1(last));
        else info->rawGet(QString::fromLatin1(id), QString::fromLatin1(value), QString::fromLatin1(last));
        return d.waitFor(4000, [&]() { return !info->rawBusy(); });
    }
    QQuickItem *fwItem(const QString &name) const { return fwWindow ? d.item(name, fwWindow->contentItem()) : nullptr; }
    void showFirmware() {
        d.app().shell->trigger(QStringLiteral("device.firmwareUpdate"));
        fwWindow = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("device.firmwareUpdate")));
        d.waitFor(3000, [&]() { return fwWindow && fwWindow->isVisible(); });
        fw->locator()->setScanForTest([bus = bus]() { return bus->read(); });
        bus->set(0);
        fw->resetRun();
        // An earlier section's custom image (one left from a run that ended
        // committed) is not this section's
        fw->useBundledImage();
        d.waitIdle();
    }
    void hideFirmware() {
        fw->resetRun();
        if (fwWindow) fwWindow->close();
        d.spin(50);
    }
};

// The visible item named `name` under `scope` (a page shown earlier may linger)
QQuickItem *visibleItem(QQuickItem *scope, const QString &name) {
    if (!scope || !scope->isVisible()) return nullptr;
    if (scope->objectName() == name) return scope;
    for (QQuickItem *child : scope->childItems())
        if (QQuickItem *found = visibleItem(child, name)) return found;
    return nullptr;
}

// The pure parts: the rows, the notes and the raw request's refusals before
// anything is sent
void pure(SmokeDriver &d) {
    const QJsonObject hubPaged{ { QStringLiteral("ok"), true },         { QStringLiteral("generation"), QStringLiteral("paged") },
                                { QStringLiteral("protocolMajor"), 1 }, { QStringLiteral("protocolMinor"), 0 },
                                { QStringLiteral("pages"), QJsonArray() }, { QStringLiteral("extEnvelope"), QStringLiteral("legacy") },
                                { QStringLiteral("boardId"), 0 },       { QStringLiteral("boardName"), QStringLiteral("DSPi") },
                                { QStringLiteral("deviceKind"), QStringLiteral("dsp") } };
    const QStringList notes = DeviceInfoController::notesOf(hubPaged);
    d.check(QStringLiteral("pgqt: a paged device with no page list (a hub's local or principal path) gets the note"),
            notes.size() == 1 && notes.first().contains(QLatin1String("carries no page list")) &&
                DeviceInfoController::rowsOf(hubPaged).value(3).toMap().value(QStringLiteral("value")).toString() ==
                    QLatin1String("none listed on this connection"));
    using R = DeviceInfoController::RawRequest;
    const R id = DeviceInfoController::parseRaw(false, QStringLiteral("0x10550"), QString(), QStringLiteral("4"));
    const R value = DeviceInfoController::parseRaw(false, QStringLiteral("0x0550"), QStringLiteral("0x10000"), QStringLiteral("4"));
    const R length = DeviceInfoController::parseRaw(false, QStringLiteral("0x0550"), QString(), QStringLiteral("65536"));
    const R byte = DeviceInfoController::parseRaw(true, QStringLiteral("0x0522"), QString(), QStringLiteral("0x1ff"));
    const R odd = DeviceInfoController::parseRaw(true, QStringLiteral("0x0522"), QString(), QStringLiteral("123"));
    const R big = DeviceInfoController::parseRaw(true, QStringLiteral("0x0522"), QString(), QString(2 * 65536, QLatin1Char('0')));
    const R junk = DeviceInfoController::parseRaw(true, QStringLiteral("0x0522"), QString(), QStringLiteral("zz"));
    d.check(QStringLiteral("pgqt: an id, wValue or length above 16 bits is refused before anything is sent"),
            id.error.contains(QLatin1String("16 bits")) && value.error.contains(QLatin1String("16 bits")) &&
                length.error.contains(QLatin1String("16 bits")));
    d.check(QStringLiteral("pgqt: a payload byte above 0xFF, a cut byte or a payload past 65535 bytes is refused"),
            byte.error.contains(QLatin1String("8 bits")) && odd.error.contains(QLatin1String("8 bits")) &&
                big.error.contains(QLatin1String("65535")) && !junk.error.isEmpty() && byte.data.isEmpty() && big.data.isEmpty());
    const R good = DeviceInfoController::parseRaw(true, QStringLiteral("0x0522"), QStringLiteral("3"), QStringLiteral("01 ff,0x02"));
    d.check(QStringLiteral("pgqt: a good SET parses as typed"),
            good.error.isEmpty() && good.id == 0x0522 && good.value == 3 && good.data == QByteArray("\x01\xff\x02", 3));
    d.check(QStringLiteral("pgqt: a firmware result code is said by its name"),
            DeviceInfoController::refusalWords(0x0550, QJsonObject{ { QStringLiteral("code"), QStringLiteral("refused") },
                                                                    { QStringLiteral("error"), QStringLiteral("device refused the request (status 18)") } })
                .contains(QLatin1String("VC_ERR_LENGTH (18)")));
}

void perGeneration(Run &r, const Generation &g) {
    SmokeDriver &d = r.d;
    DeviceInfoController *info = r.info;
    const QString name = QString::fromLatin1(g.name);
    d.check(QStringLiteral("pgqt: %1 opens through the probe").arg(name), r.open(g.code));
    // Once per connection: another snapshot of it reads nothing again
    const int reads = info->reads();
    d.app().device->refresh();
    d.waitIdle();
    d.spin(100);
    d.check(QStringLiteral("pgqt: %1: the device info is read once for the connection").arg(name), info->reads() == reads);
    d.check(QStringLiteral("pgqt: %1: generation %2").arg(name, QLatin1String(g.generation)),
            info->generation() == QLatin1String(g.generation) &&
                rowValue(info, QStringLiteral("generation"))
                    .startsWith(QLatin1String(g.generation)));
    const bool paged = info->paged();
    d.check(QStringLiteral("pgqt: %1: the protocol and page rows").arg(name),
            paged ? rowValue(info, QStringLiteral("protocol")).startsWith(QLatin1String("paged 1.")) &&
                        rowValue(info, QStringLiteral("pages")).contains(QLatin1String("0x01 SYS r"))
                  : rowValue(info, QStringLiteral("protocol")) == QLatin1String("1.1.6 opcodes") &&
                        rowValue(info, QStringLiteral("pages")) == QLatin1String("none: page 0 only"));
    d.check(QStringLiteral("pgqt: %1: the extension envelope").arg(name),
            rowValue(info, QStringLiteral("extensions")) == QLatin1String(g.ext) &&
                (g.code != 4 || rowValue(info, QStringLiteral("pages")).contains(QLatin1String("0xE0 EXT"))));
    d.check(QStringLiteral("pgqt: %1: the board").arg(name),
            info->boardId() == g.board && rowValue(info, QStringLiteral("board")).contains(info->boardName()) &&
                (g.board != 1 || info->boardName() == QLatin1String("DSPi Zero")) && info->bootsel() == (g.board == 0));
    d.check(QStringLiteral("pgqt: %1: the notes").arg(name),
            (g.board == 1) == info->notes().join(QLatin1Char(' ')).contains(QLatin1String("BOOTSEL")) &&
                g.sender == info->notes().join(QLatin1Char(' ')).contains(QLatin1String("audio link sender")) &&
                info->sender() == g.sender);

    // ---- The raw request by id ----
    if (paged) {
        r.raw(false, "0x0110", "", "16");
        say(QStringLiteral("raw GET 0x0110"), info->rawResult());
        d.check(QStringLiteral("pgqt: %1: raw GET SYS.PROTOCOL_INFO by id answers 16 bytes, \"DSPi\"").arg(name),
                !info->rawFailed() && info->rawResult().contains(QLatin1String("16 bytes from 0x0110")) &&
                    info->rawResult().contains(QLatin1String("DSPi")));
        r.raw(true, "0x0522", "0", "00");
        say(QStringLiteral("raw SET 0x0522"), info->rawResult());
        d.check(QStringLiteral("pgqt: %1: a plain value SET by id goes").arg(name),
                !info->rawFailed() && info->rawResult() == QLatin1String("Sent 1 byte to 0x0522."));
        r.raw(true, "0x0130", "", "");
        d.check(QStringLiteral("pgqt: %1: SYS.ENTER_BOOTLOADER by id is the core's refusal, in words").arg(name),
                info->rawFailed() && info->rawResult().startsWith(QLatin1String("0x0130 was not sent: ")) && d.app().device->connected());
        r.raw(true, "0x0552", "", "");
        d.check(QStringLiteral("pgqt: %1: a command that writes flash by id is refused").arg(name),
                info->rawFailed() && info->rawResult().startsWith(QLatin1String("0x0552 was not sent: ")));
    } else {
        r.raw(false, "0x0110", "", "16");
        d.check(QStringLiteral("pgqt: %1: a raw request by id is refused: the 1.1.6 protocol has no pages").arg(name),
                info->rawFailed() && info->rawResult().contains(QLatin1String("speaks the 1.1.6 protocol")));
    }
    r.raw(false, "0x10110", "", "16");
    d.check(QStringLiteral("pgqt: %1: an id that would go out cut is refused before the core").arg(name),
            info->rawFailed() && info->rawResult().contains(QLatin1String("16 bits")));

    // ---- What the update screen offers ----
    r.showFirmware();
    QQuickItem *primary = r.fwItem(QStringLiteral("FirmwareWindow.primary"));
    say(name + QStringLiteral(" firmware"), QStringLiteral("noUpdate %1, note '%2', image '%3', primary %4 enabled %5, window %6")
                                                .arg(r.fw->noUpdate()).arg(r.fw->imageNote(), r.fw->imageName())
                                                .arg(primary != nullptr).arg(primary && primary->isEnabled()).arg(r.fwWindow != nullptr));
    if (g.board != 0) {
        d.check(QStringLiteral("pgqt: %1: no image is offered to a board without BOOTSEL").arg(name),
                r.fw->noUpdate() && r.fw->imageNote().contains(QLatin1String("BOOTSEL")) && r.fw->imageName().isEmpty() &&
                    primary && !primary->isEnabled());
        d.check(QStringLiteral("pgqt: %1: nor the bootloader entry without installing").arg(name),
                !(r.fwItem(QStringLiteral("FirmwareWindow.manualHint")) && r.fwItem(QStringLiteral("FirmwareWindow.manualHint"))->isVisible()));
        const int asked = r.bootloaderRequests;
        r.fw->start();
        d.check(QStringLiteral("pgqt: %1: Update Firmware ends before anything is read or sent").arg(name),
                d.waitFor(2000, [&]() { return r.fw->failed(); }) && r.fw->statusMessage().contains(QLatin1String("BOOTSEL")) &&
                    r.bootloaderRequests == asked && d.app().device->connected());
        d.grab(r.fwWindow, QStringLiteral("pgqt-firmware-zero.png"));
    } else if (g.sender) {
        d.check(QStringLiteral("pgqt: sender: the DSP's bundled image is not offered; its own image is asked for"),
                !r.fw->noUpdate() && r.fw->imageNote().contains(QLatin1String("sender")) && r.fw->imageName().isEmpty() &&
                    primary && !primary->isEnabled());
    } else if (paged) {
        // The bundled 1.1.6 image on a 1.1.7 device: the check says the change
        r.fw->start();
        d.check(QStringLiteral("pgqt: %1: the update check says the generation change").arg(name),
                d.waitFor(3000, [&]() { return !r.fw->generationChange().isEmpty(); }) &&
                    r.fw->generationChange().contains(QLatin1String("1.1.6")));
        d.grab(r.fwWindow, QStringLiteral("pgqt-firmware-%1.png").arg(g.code));
        if (r.fw->guardAsking()) r.fw->cancelGuard();
        d.waitIdle();
        if (!d.app().device->connected()) r.open(g.code);   // it was let through: the fake rebooted
    }
    r.hideFirmware();

    // ---- The sender's page in place of the DSP screens ----
    QQuickItem *senderPage = d.item(QStringLiteral("SenderPage"));
    QQuickItem *rightPane = d.item(QStringLiteral("RightPane"));
    if (g.sender) {
        d.check(QStringLiteral("pgqt: sender: its page shows and the DSP screens do not"),
                d.waitFor(2000, [&]() { return d.item(QStringLiteral("SenderPage")) && d.item(QStringLiteral("SenderPage"))->isVisible(); }) &&
                    rightPane && !rightPane->isVisible());
        d.check(QStringLiteral("pgqt: sender: the audio link status is read"),
                d.waitFor(3000, [&]() { return !info->alinkBusy() && (!info->alinkRows().isEmpty() || !info->alinkProblem().isEmpty()); }) &&
                    !info->alinkRows().isEmpty() && info->alinkProblem().isEmpty());
        d.check(QStringLiteral("pgqt: screenshot of the sender's page"), d.grab(d.mainWindow(), QStringLiteral("pgqt-sender.png")));
    } else {
        d.check(QStringLiteral("pgqt: %1: the DSP screens show, no sender page").arg(name),
                rightPane && rightPane->isVisible() && !(senderPage && senderPage->isVisible()));
    }
    d.check(QStringLiteral("pgqt: %1: no request the generation must not get").arg(name), r.violations() == 0);
}

// Settings > About's rows on a paged device, and the raw dialog
void aboutPage(Run &r) {
    SmokeDriver &d = r.d;
    auto *s = Registrar::instance<SettingsController>();
    if (!s) return;
    r.open(4);
    d.app().shell->trigger(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    if (!w) return;
    d.waitFor(3000, [&]() { return w->isVisible(); });
    s->open(QStringLiteral("about"));
    d.spin(300);
    d.waitFor(2000, [&]() { return visibleItem(w->contentItem(), QStringLiteral("About.raw")) != nullptr; });
    QQuickItem *gen = visibleItem(w->contentItem(), QStringLiteral("DeviceInfoRows.generation"));
    d.check(QStringLiteral("pgqt: Settings > About shows the device's generation row"),
            gen && gen->property("value").toString() == QLatin1String("paged: firmware 1.1.7 and later"));
    d.check(QStringLiteral("pgqt: screenshot of About on G2b"), d.grab(w, QStringLiteral("pgqt-about-g2b.png")));
    QQuickItem *button = visibleItem(w->contentItem(), QStringLiteral("About.raw"));
    if (button) QMetaObject::invokeMethod(button, "clicked");
    // The dialog of the page that holds this button (a page shown earlier may linger)
    QQuickItem *page = button;
    while (page && page->objectName() != QLatin1String("AboutPage")) page = page->parentItem();
    QObject *dialog = page ? page->findChild<QObject *>(QStringLiteral("RawRequestDialog")) : nullptr;
    say(QStringLiteral("raw dialog"), QStringLiteral("button %1, page %2, dialog %3").arg(button != nullptr).arg(page != nullptr).arg(dialog != nullptr));
    d.check(QStringLiteral("pgqt: Raw Request... opens the dialog"),
            dialog && d.waitFor(2000, [&]() { return dialog->property("opened").toBool(); }));
    d.check(QStringLiteral("pgqt: screenshot of the raw dialog"), d.grab(w, QStringLiteral("pgqt-raw-dialog.png")));
    if (dialog) QMetaObject::invokeMethod(dialog, "close");
    w->close();
    d.spin(50);
}

// The dead end PG-D part 1's review found, and that it is gone
void lineageBeforeBootloader(Run &r) {
    SmokeDriver &d = r.d;
    FirmwareController *fw = r.fw;
    d.check(QStringLiteral("pgqt: G1b at 1.15 V opens"), r.open(7));
    r.bootloaderRequests = 0;
    QFile bundled(QStringLiteral(":/firmware/DSPi-RP2350-v1.1.6.uf2"));
    const QByteArray image = bundled.open(QIODevice::ReadOnly) ? bundled.readAll() : QByteArray();
    // The voltage rule alone, as the screen asked before: clear, so it asked nothing
    auto old = std::make_shared<QJsonObject>();
    d.app().core->worker()->post([old, image](FfiCore *c) {
        const QJsonObject board = QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_firmware_guard_board(c, b, n); })).object();
        *old = QJsonDocument::fromJson(readSized([&](char *b, uint32_t n) {
                   return dspi_firmware_guard_check(reinterpret_cast<const uint8_t *>(image.constData()), uint32_t(image.size()),
                                                    uint16_t(board.value(QStringLiteral("requiredMv")).toInt()), nullptr, b, n);
               })).object();
    });
    d.waitIdle();
    d.check(QStringLiteral("pgqt: the voltage rule alone clears the bundled stock image on our board at 1.15 V"),
            !image.isEmpty() && old->value(QStringLiteral("allowed")).toBool());
    r.showFirmware();
    r.fw->setTimeoutsForTest(2000, 3000);
    fw->start();
    d.waitFor(3000, [&]() { return fw->guardAsking() || fw->failed() || fw->confirmed(); });
    say(QStringLiteral("lineage run"), QStringLiteral("stage %1, asking %2, phrase '%3', message '%4', confirmed %5, requests %6, target %7")
                                           .arg(fw->stage()).arg(fw->guardAsking()).arg(fw->guardPhrase(), fw->statusMessage())
                                           .arg(fw->confirmed()).arg(r.bootloaderRequests).arg(QString::fromUtf8(fw->targetForTest())));
    d.check(QStringLiteral("pgqt: the update check asks the lineage phrase, before the bootloader entry"),
            d.waitFor(3000, [&]() { return fw->guardAsking(); }) && fw->guardPhrase() == QLatin1String(kLineagePhrase) &&
                r.bootloaderRequests == 0 && !fw->confirmed());
    QObject *guard = r.fwWindow ? r.fwWindow->findChild<QObject *>(QStringLiteral("FirmwareWindow.guard")) : nullptr;
    QQuickItem *typed = guard ? guard->findChild<QQuickItem *>(QStringLiteral("FirmwareWindow.guardTyped")) : nullptr;
    d.check(QStringLiteral("pgqt: in the dialog, its field empty: the phrase is never typed for the person"),
            guard && d.waitFor(1000, [&]() { return guard->property("opened").toBool(); }) && typed &&
                typed->property("text").toString().isEmpty());
    d.check(QStringLiteral("pgqt: the generation change is said (none here: 1.1.6 onto 1.1.6)"), fw->generationChange().isEmpty());
    d.check(QStringLiteral("pgqt: screenshot of the lineage question"), d.grab(r.fwWindow, QStringLiteral("pgqt-lineage-question.png")));
    fw->answerGuard(QStringLiteral("I ACCEPT THE CORE VOLTAGE RISK"));
    d.check(QStringLiteral("pgqt: another phrase does not answer it"), fw->guardAsking() && r.bootloaderRequests == 0);
    fw->cancelGuard();
    d.waitIdle();
    d.check(QStringLiteral("pgqt: Don't Install: nothing sent, the device stays as it was"),
            !fw->confirmed() && r.bootloaderRequests == 0 && d.app().device->connected());
    fw->resetRun();
    fw->start();
    d.waitFor(3000, [&]() { return fw->guardAsking(); });
    fw->answerGuard(QString::fromLatin1(kLineagePhrase));
    d.check(QStringLiteral("pgqt: answered, the device is sent to its bootloader once"),
            d.waitFor(3000, [&]() { return r.bootloaderRequests == 1; }) && fw->confirmed());
    // The device leaves for BOOTSEL; the board and its drive appear
    d.waitFor(3000, [&]() { return !d.app().device->connected(); });
    if (d.app().device->connected()) {
        d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
        d.app().device->refresh();
        d.waitFor(3000, [&]() { return !d.app().device->connected(); });
    }
    const QString dir = d.outDir() + QStringLiteral("/pgqt-drive");
    QDir(dir).removeRecursively();
    QDir().mkpath(dir);
    QFile infoFile(dir + QStringLiteral("/INFO_UF2.TXT"));
    if (infoFile.open(QIODevice::WriteOnly)) infoFile.write("UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n");
    infoFile.close();
    r.bus->set(1, { { dir, BootloaderLocator::RP2350 } });
    QFile written(dir + QStringLiteral("/firmware.uf2"));
    d.check(QStringLiteral("pgqt: the write passes the core's gate on the core that read the board: not left in BOOTSEL"),
            d.waitFor(6000, [&]() {
                QFile f(dir + QStringLiteral("/firmware.uf2"));
                return f.open(QIODevice::ReadOnly) && f.readAll() == image;
            }) && fw->stage() != QLatin1String("failed") && fw->writesWithTargetForTest());
    d.check(QStringLiteral("pgqt: and the run waits for the device"), d.waitFor(3000, [&]() { return fw->stage() == QLatin1String("waitingForDevice"); }));
    r.bus->set(0);
    r.open(7);
    d.waitFor(5000, [&]() { return fw->stage() == QLatin1String("verified") || fw->failed(); });
    d.check(QStringLiteral("pgqt: the device back on the image's version"), fw->stage() == QLatin1String("verified"));
    r.fw->setTimeoutsForTest(8000, 30000);
    r.hideFirmware();
}

void generationsSection(SmokeDriver &d) {
    auto *info = Registrar::instance<DeviceInfoController>();
    auto *fw = Registrar::instance<FirmwareController>();
    d.check(QStringLiteral("pgqt: the device info controller is registered"), info && fw);
    if (!info || !fw) return;
    d.waitIdle();
    // The demo opens without the probe (a fake handed to the core): its generation is not said
    d.check(QStringLiteral("pgqt: the demo's device info is read"), d.waitFor(3000, [&]() { return info->known(); }) &&
                                                                     !info->sender() && info->bootsel());
    say(QStringLiteral("demo generation"), info->generation());
    pure(d);
    Run r{ d, info, fw };
    QMetaObject::Connection c = QObject::connect(d.app().device, &DeviceModel::writeFinished, d.app().device,
                                                 [&r](const QString &what, bool ok, const QString &, const QString &) {
                                                     if (what == QLatin1String("enter bootloader") && ok) ++r.bootloaderRequests;
                                                 });
    for (const Generation &g : kGenerations) perGeneration(r, g);
    aboutPage(r);
    lineageBeforeBootloader(r);
    QObject::disconnect(c);
    fw->locator()->setScanForTest(nullptr);
    d.app().device->openDemo();
    d.waitFor(3000, [&]() { return d.app().device->connected() && info->known() && !info->sender(); });
    d.waitIdle();
}

} // namespace

static const SmokeDriver::Section pgqtGenerations("pgqt-generations", 215, &generationsSection);
