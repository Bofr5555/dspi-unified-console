// -- WP QP16 -- The firmware window and its installer on the real app, with the
// real guard and the real write, onto a fake bootloader drive: a directory
// holding an INFO_UF2.TXT under the smoke output. Nothing here can reach a
// board: the locator's scan is replaced by a fake bus, and the demo is the only
// device. Also the mismatch banner, and the device-access (udev) dialog with a
// fake installer: pkexec never runs.
#include "SmokeDriver.h"

#include "BootloaderLocator.h"
#include "Core.h"
#include "DeviceAccess.h"
#include "DeviceModel.h"
#include "FirmwareController.h"
#include "ReadOnlyController.h"
#include "Registrar.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMutex>
#include <QMutexLocker>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <memory>

namespace {
const char kPhrase[] = "I ACCEPT THE CORE VOLTAGE RISK";
constexpr uint32_t kRp2040Family = 0xE48BFF56;
constexpr uint32_t kRp2350Family = 0xE48BFF59;

void put(QByteArray &f, int at, uint32_t v) {
    for (int i = 0; i < 4; i++) f[at + i] = char((v >> (8 * i)) & 0xFF);
}

// A UF2 with the SDK's binary-info layout (program name, version, the core
// voltage stamp), as the core's own tests build them
QByteArray makeUf2(uint32_t family, const QByteArray &version, const QByteArray &stamp) {
    constexpr uint32_t xip = 0x10000000;
    struct Entry {
        uint16_t tag;
        uint32_t id;
        QByteArray text;
    };
    QList<Entry> entries;
    entries.append({ 0x5052, 0x02031C86, QByteArrayLiteral("DSPi") });
    if (!version.isEmpty()) entries.append({ 0x5052, 0x11A9BC3A, version });
    if (!stamp.isEmpty()) entries.append({ 0x5344, 0x0B7A4C15, stamp });
    QByteArray flash(0x2000, '\0');
    put(flash, 0x100, 0x7188EBF2);
    put(flash, 0x104, xip + 0x200);
    put(flash, 0x108, xip + 0x200 + uint32_t(4 * entries.size()));
    put(flash, 0x10C, xip + 0x180);
    put(flash, 0x110, 0xE71AA390);
    put(flash, 0x180, 0);
    for (int i = 0; i < entries.size(); i++) {
        const int at = 0x400 + 16 * i;
        put(flash, 0x200 + 4 * i, xip + uint32_t(at));
        flash[at] = 6;
        flash[at + 1] = 0;
        flash[at + 2] = char(entries[i].tag & 0xFF);
        flash[at + 3] = char(entries[i].tag >> 8);
        put(flash, at + 4, entries[i].id);
        const int text = 0x800 + 64 * i;
        put(flash, at + 8, xip + uint32_t(text));
        flash.replace(text, entries[i].text.size(), entries[i].text);
    }
    const int pages = flash.size() / 256;
    QByteArray out;
    for (int p = 0; p < pages; p++) {
        QByteArray block(512, '\0');
        put(block, 0, 0x0A324655);
        put(block, 4, 0x9E5D5157);
        put(block, 8, 0x2000);
        put(block, 12, xip + uint32_t(256 * p));
        put(block, 16, 256);
        put(block, 20, uint32_t(p));
        put(block, 24, uint32_t(pages));
        put(block, 28, family);
        block.replace(32, 256, flash.mid(256 * p, 256));
        put(block, 508, 0x0AB16F30);
        out.append(block);
    }
    return out;
}

// The bus as the fake scan reports it: the worker reads it, the section changes it
struct Qp16Bus {
    QMutex lock;
    BootloaderLocator::Scan scan;
    BootloaderLocator::Scan read() {
        QMutexLocker hold(&lock);
        return scan;
    }
    void set(int rp2040, int rp2350, const QList<BootloaderLocator::Drive> &drives = {}) {
        QMutexLocker hold(&lock);
        scan.rp2040 = rp2040;
        scan.rp2350 = rp2350;
        scan.drives = drives;
    }
};

struct Qp16Fw {
    SmokeDriver &d;
    FirmwareController *fw;
    QQuickWindow *w = nullptr;
    std::shared_ptr<Qp16Bus> bus = std::make_shared<Qp16Bus>();
    QString base;
    int drives = 0;

    QQuickItem *item(const QString &name) const { return w ? d.item(name, w->contentItem()) : nullptr; }
    bool shows(const QString &name) const {
        QQuickItem *i = item(name);
        return i && i->isVisible();
    }
    void click(QQuickItem *i) {
        if (!i || !i->window()) return;
        QTest::mouseClick(i->window(), Qt::LeftButton, Qt::NoModifier, i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint());
        d.spin(30);
    }
    bool state(const char *s, int ms = 4000) {
        const QString want = QString::fromLatin1(s);
        return d.waitFor(ms, [&]() { return fw->stage() == want; });
    }
    QString writeFile(const QString &name, const QByteArray &bytes) {
        const QString path = base + QLatin1Char('/') + name;
        QFile f(path);
        if (f.open(QIODevice::WriteOnly)) f.write(bytes);
        return path;
    }
    // A fake drive: a directory with the boot ROM's INFO_UF2.TXT
    BootloaderLocator::Drive newDrive(int chip = BootloaderLocator::RP2350) {
        const QString dir = base + QStringLiteral("/drive%1").arg(++drives);
        QDir().mkpath(dir);
        QFile info(dir + QStringLiteral("/INFO_UF2.TXT"));
        if (info.open(QIODevice::WriteOnly))
            info.write(chip == BootloaderLocator::RP2350 ? "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n"
                                                         : "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n");
        return { dir, chip };
    }
    static QByteArray written(const BootloaderLocator::Drive &drive) {
        QFile f(drive.path + QStringLiteral("/firmware.uf2"));
        return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
    }
    // The device as it leaves for its bootloader: gone from the app
    void leave() {
        d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
        d.app().device->refresh();
        d.waitFor(3000, [&]() { return !d.app().device->connected(); });
    }
    // The demo back (a new connection, as after a reboot)
    void back() {
        d.app().device->openDemo();
        d.waitFor(3000, [&]() { return d.app().device->connected(); });
    }
    // A fresh run on the demo, nothing on the bus
    void fresh() {
        bus->set(0, 0);
        fw->useBundledImage();
        if (!d.app().device->connected()) back();
        if (w) w->close();
        d.spin(50);
        d.app().shell->trigger(QStringLiteral("device.firmwareUpdate"));
        d.waitFor(2000, [&]() { return w && w->isVisible(); });
        d.waitIdle();
        state("waitingForBoard", 3000);
    }
};

void bundled(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    d.check(QStringLiteral("qp16: the app expects firmware 1.1.6, from its bundled images"),
            f.fw->expectedVersion() == QLatin1String("1.1.6"));
    d.check(QStringLiteral("qp16: a version from an image's name"),
            FirmwareController::versionFromName(QStringLiteral("DSPi-RP2350-v1.1.7.uf2")) == QLatin1String("1.1.7") &&
                FirmwareController::versionFromName(QStringLiteral("DSPi-RP2040-v1.1.6-beta2.uf2")) == QLatin1String("1.1.6-beta2") &&
                FirmwareController::versionFromName(QStringLiteral("firmware.uf2")).isEmpty());
    for (const char *chip : { "RP2040", "RP2350" }) {
        QFile img(QStringLiteral(":/firmware/DSPi-%1-v1.1.6.uf2").arg(QLatin1String(chip)));
        const bool open = img.open(QIODevice::ReadOnly);
        const QByteArray bytes = img.readAll();
        const QJsonObject info = QJsonDocument::fromJson(readSized([&](char *b, uint32_t n) {
                                     return dspi_uf2_inspect(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()), b, n);
                                 })).object();
        d.check(QStringLiteral("qp16: the %1 image is bundled, valid, version 1.1.6").arg(QLatin1String(chip)),
                open && info.value(QStringLiteral("valid")).toBool() &&
                    info.value(QStringLiteral("firmwareVersion")).toString().startsWith(QLatin1String("1.1.6")));
    }
    d.check(QStringLiteral("qp16: the bundled image is shown as checked"), d.waitFor(3000, [&]() { return f.fw->imageChecked(); }));
    d.check(QStringLiteral("qp16: the BootloaderLocator pairs drives to boards, the first board gets the drive"), [] {
        BootloaderLocator::Scan s;
        s.rp2350 = 2;
        s.drives = { { QStringLiteral("/a"), BootloaderLocator::RP2350 } };
        const QList<BootloaderLocator::Board> b = BootloaderLocator::pair(s);
        BootloaderLocator::Scan onlyDrive;
        onlyDrive.drives = { { QStringLiteral("/b"), BootloaderLocator::RP2040 } };
        const QList<BootloaderLocator::Board> c = BootloaderLocator::pair(onlyDrive);
        return b.size() == 2 && b.at(0).drive == QLatin1String("/a") && b.at(1).drive.isEmpty() && c.size() == 1 &&
               c.at(0).chip == BootloaderLocator::RP2040;
    }());
}

void mismatch(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    FirmwareController *fw = f.fw;
    QObject *banner = d.object(QStringLiteral("FirmwareBanner"));
    d.check(QStringLiteral("qp16: the demo runs the bundled version: no mismatch banner"),
            fw->match() == QLatin1String("match") && !fw->mismatchShown() && banner && !banner->property("visible").toBool());
    fw->setExpectedVersionForTest(QStringLiteral("1.1.7"));
    d.spin(50);
    d.check(QStringLiteral("qp16: a device behind the expected version shows the banner"),
            fw->match() == QLatin1String("older") && banner && banner->property("visible").toBool());
    d.check(QStringLiteral("qp16: it names both versions and the app"),
            fw->mismatchText() == QStringLiteral("This device runs firmware 1.1.6; %1 expects 1.1.7.").arg(QLatin1String("uConsole for DSPi")));
    QObject *update = d.object(QStringLiteral("FirmwareBanner.update"));
    d.check(QStringLiteral("qp16: its button says Update..."), update && update->property("text").toString() == QStringLiteral("Update…"));
    d.check(QStringLiteral("qp16: screenshot of the mismatch banner"), d.grab(d.mainWindow(), QStringLiteral("window-firmware-banner.png")));
    fw->setExpectedVersionForTest(QStringLiteral("1.1.5"));
    d.spin(50);
    d.check(QStringLiteral("qp16: a newer device: Details..., and says some features may not be shown"),
            fw->match() == QLatin1String("newer") && update && update->property("text").toString() == QStringLiteral("Details…") &&
                fw->mismatchText().contains(QLatin1String("newer than uConsole for DSPi 1.1.5")));
    fw->hideMismatch();
    d.spin(50);
    d.check(QStringLiteral("qp16: Hide hides it for this launch"), !fw->mismatchShown() && !banner->property("visible").toBool());
    fw->setExpectedVersionForTest(QString());
}

void window(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    Shell *shell = d.app().shell;
    shell->trigger(QStringLiteral("device.firmwareUpdate"));
    f.w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("device.firmwareUpdate")));
    d.check(QStringLiteral("qp16: Device > Firmware Update... opens the window"), f.w && d.waitFor(3000, [&]() { return f.w->isVisible(); }));
    if (!f.w) return;
    d.check(QStringLiteral("qp16: its title and size are the Mac's"),
            f.w->title() == QLatin1String("Firmware Update") && f.w->width() >= 460 && f.w->height() >= 470);
    d.check(QStringLiteral("qp16: the locator runs while it shows"), f.fw->locator()->running());
    f.fw->locator()->setScanForTest([bus = f.bus]() { return bus->read(); });
    d.waitIdle();
    d.check(QStringLiteral("qp16: nothing on the bus: ready when you are"),
            f.state("waitingForBoard") && f.fw->statusTitle() == QLatin1String("Ready when you are"));
    d.check(QStringLiteral("qp16: it says nothing is written without the click"),
            f.fw->statusMessage().startsWith(QLatin1String("Click Update Firmware to begin.")));
    d.check(QStringLiteral("qp16: the versions: this Console 1.1.6, the device 1.1.6"),
            f.item(QStringLiteral("FirmwareWindow.expected")) &&
                f.item(QStringLiteral("FirmwareWindow.expected"))->property("value").toString() == QLatin1String("1.1.6") &&
                f.item(QStringLiteral("FirmwareWindow.device"))->property("value").toString() == QLatin1String("1.1.6"));
    d.check(QStringLiteral("qp16: the bundled RP2350 image is named, and checked"),
            f.fw->imageName() == QLatin1String("DSPi-RP2350-v1.1.6.uf2") && f.shows(QStringLiteral("FirmwareWindow.imageChecked")));
    d.check(QStringLiteral("qp16: Export Configuration... and the manual bootloader link are offered"),
            f.shows(QStringLiteral("FirmwareWindow.export")) && f.shows(QStringLiteral("FirmwareWindow.manualHint")) &&
                !f.shows(QStringLiteral("FirmwareWindow.bootselHint")));
    d.check(QStringLiteral("qp16: the primary button says Update Firmware"),
            f.item(QStringLiteral("FirmwareWindow.primary")) &&
                f.item(QStringLiteral("FirmwareWindow.primary"))->property("text").toString() == QLatin1String("Update Firmware"));
    d.check(QStringLiteral("qp16: screenshot of the firmware window"), d.grab(f.w, QStringLiteral("window-firmware.png")));
}

void customImages(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    FirmwareController *fw = f.fw;
    fw->useCustomImage(f.writeFile(QStringLiteral("rp2040.uf2"), makeUf2(kRp2040Family, "1.1.6", "1200")));
    d.check(QStringLiteral("qp16: an image for the other chip says so when chosen"),
            d.waitFor(3000, [&]() { return fw->custom() && !fw->imageProblem().isEmpty(); }) &&
                fw->imageProblem().contains(QLatin1String("RP2040")) && fw->imageProblem().contains(QLatin1String("RP2350")));
    d.check(QStringLiteral("qp16: and Update Firmware is off"),
            f.item(QStringLiteral("FirmwareWindow.primary")) && !f.item(QStringLiteral("FirmwareWindow.primary"))->isEnabled());
    fw->useCustomImage(f.writeFile(QStringLiteral("junk.uf2"), QByteArray(600, 'x')));
    d.check(QStringLiteral("qp16: a file that is not firmware: the core's sentence"),
            d.waitFor(3000, [&]() { return fw->imageProblem().startsWith(QLatin1String("This is not a firmware file")); }));
    d.check(QStringLiteral("qp16: screenshot of a refused custom image"), d.grab(f.w, QStringLiteral("window-firmware-custom.png")));
    QQuickItem *bundledButton = f.item(QStringLiteral("FirmwareWindow.useBundled"));
    d.check(QStringLiteral("qp16: \"Use this Console's firmware instead\" shows"), bundledButton && bundledButton->isVisible());
    f.click(bundledButton);
    d.check(QStringLiteral("qp16: and goes back to the bundled image"), !fw->custom() && fw->imageProblem().isEmpty());
}

void guardedInstall(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    FirmwareController *fw = f.fw;
    DeviceModel *device = d.app().device;
    f.fresh();
    int bootloaderRequests = 0;
    QMetaObject::Connection c = QObject::connect(device, &DeviceModel::writeFinished, device,
                                                 [&](const QString &what, bool ok, const QString &, const QString &) {
                                                     if (what == QLatin1String("enter bootloader") && ok) ++bootloaderRequests;
                                                 });
    // The demo board needs 1.20 V; the bundled stock image has no stamp: the guard asks
    f.click(f.item(QStringLiteral("FirmwareWindow.primary")));
    d.check(QStringLiteral("qp16: an at-risk board gets the guard's question before anything else"),
            d.waitFor(3000, [&]() { return fw->guardAsking(); }) && fw->guardPhrase() == QLatin1String(kPhrase) &&
                !fw->guardMessage().isEmpty());
    d.waitIdle();
    d.check(QStringLiteral("qp16: nothing restarted yet, nothing committed"), bootloaderRequests == 0 && !fw->confirmed());
    d.check(QStringLiteral("qp16: the installer contract says question"), fw->property("state").toString() == QLatin1String("question"));
    QObject *guard = f.w->findChild<QObject *>(QStringLiteral("FirmwareWindow.guard"));
    QQuickItem *typed = guard ? guard->findChild<QQuickItem *>(QStringLiteral("FirmwareWindow.guardTyped")) : nullptr;
    QQuickItem *go = guard ? guard->findChild<QQuickItem *>(QStringLiteral("FirmwareWindow.guardGo")) : nullptr;
    d.check(QStringLiteral("qp16: the question has a text field and Go Ahead"), typed && go && guard->property("opened").toBool());
    if (!typed || !go) return;
    d.check(QStringLiteral("qp16: Go Ahead is off until the words are typed"), !go->isEnabled());
    typed->setProperty("text", QStringLiteral("i accept the core voltage risk"));
    d.check(QStringLiteral("qp16: lower case is not the phrase"), !go->isEnabled());
    fw->answerGuard(QStringLiteral("i accept the core voltage risk"));
    d.check(QStringLiteral("qp16: and answering with it does nothing"), fw->guardAsking() && !fw->confirmed());
    typed->setProperty("text", QString::fromLatin1(kPhrase));
    d.check(QStringLiteral("qp16: the exact phrase turns Go Ahead on"), go->isEnabled());
    d.check(QStringLiteral("qp16: screenshot of the guard's question"), d.grab(f.w, QStringLiteral("window-firmware-question.png")));
    f.click(go);
    d.check(QStringLiteral("qp16: Go Ahead commits: looking for the board"),
            d.waitFor(2000, [&]() { return fw->confirmed() && !fw->guardAsking(); }) &&
                fw->statusTitle() == QLatin1String("Looking for the board"));
    d.check(QStringLiteral("qp16: the device is asked to restart into its bootloader once"),
            d.waitFor(3000, [&]() { return bootloaderRequests == 1; }));
    d.check(QStringLiteral("qp16: the BOOTSEL hint shows while it looks"), f.shows(QStringLiteral("FirmwareWindow.bootselHint")));
    f.leave();
    // The board appears; its drive a moment later
    f.bus->set(0, 1);
    d.check(QStringLiteral("qp16: a board without its drive: waiting for the drive"),
            f.state("waitingForVolume") && fw->statusTitle() == QLatin1String("RP2350 (Pico 2) found"));
    const BootloaderLocator::Drive drive = f.newDrive();
    f.bus->set(0, 1, { drive });
    QFile bundledImage(QStringLiteral(":/firmware/DSPi-RP2350-v1.1.6.uf2"));
    bundledImage.open(QIODevice::ReadOnly);
    const QByteArray bundledBytes = bundledImage.readAll();
    d.check(QStringLiteral("qp16: the drive takes the bundled image, byte for byte"),
            d.waitFor(5000, [&]() { return Qp16Fw::written(drive) == bundledBytes; }));
    d.check(QStringLiteral("qp16: then it waits for the device"), f.state("waitingForDevice") && fw->step() == 2);
    f.back();
    d.check(QStringLiteral("qp16: the device back on the image's version is success"),
            f.state("verified", 5000) && fw->statusMessage().contains(QLatin1String("1.1.6")) && fw->step() == 3);
    d.check(QStringLiteral("qp16: Done and Update Another Board"),
            f.shows(QStringLiteral("FirmwareWindow.done")) && f.shows(QStringLiteral("FirmwareWindow.another")));
    d.check(QStringLiteral("qp16: screenshot of a verified update"), d.grab(f.w, QStringLiteral("window-firmware-verified.png")));
    QObject::disconnect(c);
}

// The installer contract QP17's wizard drives (OnboardingController.h): with
// the window closed, a guard question opens the window to ask it
void wizardContract(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    FirmwareController *fw = f.fw;
    f.fresh();
    f.w->close();
    d.spin(50);
    d.check(QStringLiteral("qp16: nothing watching: the locator stops"), !fw->locator()->running());
    fw->beginWatching();
    d.check(QStringLiteral("qp16: the wizard watching starts it"), fw->locator()->running());
    d.waitIdle();
    d.check(QStringLiteral("qp16: the contract's properties"),
            fw->property("state").toString() == QLatin1String("waitingForBoard") && fw->property("version").toString() == QLatin1String("1.1.6") &&
                !fw->property("mundane").toBool() && fw->property("fraction").toDouble() == 0.0);
    fw->installConnected();
    d.check(QStringLiteral("qp16: a question from the wizard's run opens the window, asking it"),
            d.waitFor(3000, [&]() { return fw->guardAsking() && f.w->isVisible(); }) &&
                d.waitFor(1000, [&]() {
                    QObject *g = f.w->findChild<QObject *>(QStringLiteral("FirmwareWindow.guard"));
                    return g && g->property("opened").toBool();
                }));
    fw->cancelGuard();
    d.check(QStringLiteral("qp16: Don't Install leaves nothing committed"), !fw->confirmed() && !fw->guardAsking());
    // The wizard's own pair, installWhenReady() then installConnected(), as
    // the Mac's view calls it: the second joins the run the first started
    int bootloaderRequests = 0;
    QMetaObject::Connection c = QObject::connect(d.app().device, &DeviceModel::writeFinished, d.app().device,
                                                 [&](const QString &what, bool ok, const QString &, const QString &) {
                                                     if (what == QLatin1String("enter bootloader") && ok) ++bootloaderRequests;
                                                 });
    fw->installWhenReady();
    fw->installConnected();
    d.check(QStringLiteral("qpi3: the wizard's pair asks the guard's question once"),
            d.waitFor(3000, [&]() { return fw->guardAsking(); }) && bootloaderRequests == 0);
    fw->answerGuard(QString::fromLatin1(kPhrase));
    d.check(QStringLiteral("qpi3: and then restarts the connected device into its bootloader once"),
            d.waitFor(3000, [&]() { return bootloaderRequests == 1; }) && fw->confirmed());
    fw->installConnected();
    d.spin(200);
    d.check(QStringLiteral("qpi3: a further installConnected() asks no second restart"), bootloaderRequests == 1);
    QObject::disconnect(c);
    f.leave();
    fw->reset();
    f.back();
    d.waitIdle();
    f.w->close();
    d.spin(50);
    d.check(QStringLiteral("qp16: the wizard still watching keeps the locator"), fw->locator()->running());
    fw->stopWatching();
    d.check(QStringLiteral("qp16: and stopping stops it"), !fw->locator()->running());
}

void failures(Qp16Fw &f) {
    SmokeDriver &d = f.d;
    FirmwareController *fw = f.fw;
    fw->setTimeoutsForTest(400, 1500);
    // An image stamped for what the demo board needs: no question
    const QByteArray good = makeUf2(kRp2350Family, "1.1.6", "1200");
    const QString goodPath = f.writeFile(QStringLiteral("good.uf2"), good);

    f.fresh();
    f.bus->set(0, 2);
    d.check(QStringLiteral("qp16: two boards: not quite ready, disconnect all but one"),
            f.state("failed") && fw->statusTitle() == QLatin1String("Not quite ready") &&
                fw->statusMessage().startsWith(QLatin1String("2 boards are in bootloader mode")));
    f.bus->set(0, 1);
    d.check(QStringLiteral("qp16: a drive that never appears: not quite ready"),
            f.state("waitingForVolume") && f.state("failed", 3000) && fw->statusMessage().contains(QLatin1String("RP2350 drive has not appeared")));
    d.check(QStringLiteral("qp16: Try Again"), f.item(QStringLiteral("FirmwareWindow.primary")) &&
                                                   f.item(QStringLiteral("FirmwareWindow.primary"))->property("text").toString() == QLatin1String("Try Again"));
    d.check(QStringLiteral("qp16: screenshot of a failure"), d.grab(f.w, QStringLiteral("window-firmware-failed.png")));

    // A device that never comes back
    f.fresh();
    fw->useCustomImage(goodPath);
    d.waitFor(3000, [&]() { return fw->custom(); });
    fw->start();
    d.check(QStringLiteral("qp16: a stamped image the board can take: no question"),
            d.waitFor(3000, [&]() { return fw->confirmed(); }) && !fw->guardAsking());
    f.leave();
    BootloaderLocator::Drive drive = f.newDrive();
    f.bus->set(0, 1, { drive });
    d.check(QStringLiteral("qp16: the custom image is written"), d.waitFor(5000, [&]() { return Qp16Fw::written(drive) == good; }));
    d.check(QStringLiteral("qp16: a device that does not come back is a failure"),
            f.state("failed", 4000) && fw->statusMessage().contains(QLatin1String("did not reappear")));
    f.back();
    d.waitIdle();
    d.check(QStringLiteral("qp16: and stays one when it comes back afterwards"), fw->failed());

    // Another version coming back
    f.fresh();
    fw->useCustomImage(f.writeFile(QStringLiteral("other.uf2"), makeUf2(kRp2350Family, "9.9.9", "1200")));
    d.waitFor(3000, [&]() { return fw->custom(); });
    fw->start();
    d.waitFor(3000, [&]() { return fw->confirmed(); });
    f.leave();
    drive = f.newDrive();
    f.bus->set(0, 1, { drive });
    f.state("waitingForDevice", 5000);
    f.back();
    d.check(QStringLiteral("qp16: written is not success: the device on another version fails"),
            f.state("failed", 5000) && fw->statusMessage() == QLatin1String("The device came back running firmware 1.1.6 instead of 9.9.9."));

    // The wrong chip's drive: the core refuses the write, the drive stays empty
    f.fresh();
    fw->useCustomImage(goodPath);
    d.waitFor(3000, [&]() { return fw->custom(); });
    fw->start();
    d.waitFor(3000, [&]() { return fw->confirmed(); });
    f.leave();
    drive = f.newDrive(BootloaderLocator::RP2040);
    f.bus->set(1, 0, { drive });
    d.check(QStringLiteral("qp16: an RP2350 image on an RP2040's drive is refused, in the core's words"),
            f.state("failed", 5000) && fw->statusMessage().contains(QLatin1String("RP2040")) && Qp16Fw::written(drive).isEmpty());
    f.back();
    fw->setTimeoutsForTest(8000, 30000);
}

void firmwareSection(SmokeDriver &d) {
    auto *fw = Registrar::instance<FirmwareController>();
    d.check(QStringLiteral("qp16: the firmware controller is registered"), fw != nullptr);
    if (!fw) return;
    d.waitIdle();
    Qp16Fw f{ d, fw };
    f.base = d.outDir() + QStringLiteral("/qp16-drives");
    QDir(f.base).removeRecursively();
    QDir().mkpath(f.base);
    bundled(f);
    mismatch(f);
    window(f);
    if (!f.w) return;
    customImages(f);
    guardedInstall(f);
    wizardContract(f);
    failures(f);
    f.w->close();
    d.spin(50);
    d.check(QStringLiteral("qp16: closing the window stops the locator"), !fw->locator()->running());
    fw->locator()->setScanForTest(nullptr);
    fw->useBundledImage();
    if (!d.app().device->connected()) f.back();
    d.waitIdle();
}

// ---- The device-access dialog ----------------------------------------------

void accessSection(SmokeDriver &d) {
    auto *access = Registrar::instance<DeviceAccess>();
    DeviceModel *device = d.app().device;
    d.check(QStringLiteral("qp16: the device-access controller is registered"), access != nullptr);
    if (!access) return;
    d.check(QStringLiteral("qp16: the rule gives the seat's person vendor 2e8b"),
            DeviceAccess::ruleText().contains(QLatin1String("ATTR{idVendor}==\"2e8b\"")) &&
                DeviceAccess::ruleText().contains(QLatin1String("TAG+=\"uaccess\"")) &&
                DeviceAccess::rulePath() == QLatin1String("/etc/udev/rules.d/70-dspi.rules"));
    QList<QString> installs;
    int nextExit = 126;
    access->setInstallerForTest([&](const QString &path, std::function<void(int, const QString &)> done) {
        installs.append(path);
        done(nextExit, QString());
    });
    access->setCountForTest(0);
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.waitFor(3000, [&]() { return !device->connected(); });
    access->check();
    d.waitIdle();
    d.check(QStringLiteral("qp16: every unit opens: no dialog"), !access->offered());
    access->setCountForTest(1);
    access->check();
    QObject *dialog = d.object(QStringLiteral("UdevDialog"));
    d.check(QStringLiteral("qp16: a unit that cannot be opened offers the dialog"),
            d.waitFor(3000, [&]() { return access->offered() && dialog && dialog->property("opened").toBool(); }));
    d.check(QStringLiteral("qp16: on Linux it offers the rule"),
            access->system() == QLatin1String("linux") && dialog && dialog->property("title").toString() == QLatin1String("DSPi Needs Permission"));
    d.check(QStringLiteral("qp16: screenshot of the dialog"), d.grab(d.mainWindow(), QStringLiteral("window-udev.png")));
    access->installRule();
    d.check(QStringLiteral("qp16: a dismissed authentication says so, and nothing is installed"),
            installs.size() == 1 && !access->installed() && access->error().contains(QLatin1String("cancelled")));
    QFile rule(installs.value(0));
    d.check(QStringLiteral("qp16: the rule file handed over is the rule"),
            rule.open(QIODevice::ReadOnly) && QString::fromUtf8(rule.readAll()) == DeviceAccess::ruleText() &&
                !installs.value(0).startsWith(QLatin1String("/tmp")));
    d.check(QStringLiteral("qp16: screenshot of the failure"), d.grab(d.mainWindow(), QStringLiteral("window-udev-error.png")));
    nextExit = 0;
    access->installRule();
    d.check(QStringLiteral("qp16: an installed rule says so"), access->installed() && access->error().isEmpty());
    access->setCountForTest(0);
    access->rescan();
    d.check(QStringLiteral("qp16: once the unit opens, the dialog goes"),
            d.waitFor(3000, [&]() { return !access->offered() && !dialog->property("opened").toBool(); }));
    access->setCountForTest(2);
    access->check();
    d.waitFor(2000, [&]() { return access->offered(); });
    access->dismiss();
    access->check();
    d.waitIdle();
    d.check(QStringLiteral("qp16: dismissed, it does not come back while the count stays"), !access->offered());
    // -- WP QPI3 -- the status area offers it again
    QQuickItem *offer = d.item(QStringLiteral("DeviceStatus.access"));
    d.check(QStringLiteral("qpi3: the status area offers the help while a unit cannot be opened"), offer && offer->isVisible());
    if (offer) QMetaObject::invokeMethod(offer, "clicked");
    d.check(QStringLiteral("qpi3: and its button shows the dialog again"),
            d.waitFor(2000, [&]() { return access->offered() && dialog->property("opened").toBool(); }));
    access->dismiss();
    d.waitIdle();
    access->setCountForTest(0);
    access->check();
    d.waitIdle();
    access->setCountForTest(-1);
    access->setInstallerForTest(nullptr);
    device->openDemo();
    d.waitFor(3000, [&]() { return device->connected(); });
    d.waitIdle();
}
} // namespace

static const SmokeDriver::Section qp16Firmware("qp16-firmware", 160, &firmwareSection);
static const SmokeDriver::Section qp16Access("qp16-access", 161, &accessSection);
