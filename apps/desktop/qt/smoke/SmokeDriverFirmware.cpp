// WP QD11: the firmware updater on the real app, the real guard and the real write,
// onto a fake bootloader drive: a directory holding an INFO_UF2.TXT, under the smoke
// output. Nothing here can reach a board: the seam replaces the bus scan, and the
// demo device is the only device. The rules are apps/terminal/src/flash.rs's.
#include "SmokeDriver.h"
#include "DemoDevice.h"

#include "DSPiBridge.h"
#include "FirmwareUpdater.h"

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMouseEvent>
#include <QMutex>
#include <QMutexLocker>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <QUrl>
#include <QVariant>
#include <atomic>
#include <memory>

extern "C" {
#include "dspi_core.h"
}

namespace {
const char kPhrase[] = "I ACCEPT THE CORE VOLTAGE RISK";
const char kWord[] = "OVERVOLT";
const char *const kDemo = DemoDevice::kSerial;

constexpr uint32_t kRp2040Family = 0xE48BFF56;
constexpr uint32_t kRp2350Family = 0xE48BFF59;

void put(QByteArray &f, int at, uint32_t v) {
    for (int i = 0; i < 4; i++) f[at + i] = char((v >> (8 * i)) & 0xFF);
}

// A UF2 with the SDK's binary-info layout: what the core's own tests build.
QByteArray makeUf2(uint32_t family, const QByteArray &version, const QByteArray &stamp) {
    constexpr uint32_t xip = 0x10000000;
    struct Entry { uint16_t tag; uint32_t id; QByteArray text; };
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
    put(flash, 0x180, 0);       // no RAM mapping
    for (int i = 0; i < entries.size(); i++) {
        const int at = 0x400 + 16 * i;
        put(flash, 0x200 + 4 * i, xip + uint32_t(at));
        flash[at] = 6;          // ID_AND_STRING
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

// The bus, as the seam reports it. The worker reads it, the driver changes it.
struct FakeBus {
    QMutex lock;
    QList<FirmwareUpdater::Drive> drives;
    int extraBoards = 0;
    std::atomic<int> otherThreadScans { 0 };
    std::atomic<int> uiThreadScans { 0 };
    std::atomic<int> scanDelayMs { 0 };
    QThread *ui = QThread::currentThread();

    FirmwareUpdater::Scan scan() {
        (QThread::currentThread() == ui ? uiThreadScans : otherThreadScans)++;
        if (scanDelayMs > 0) QThread::msleep(ulong(int(scanDelayMs)));
        QMutexLocker hold(&lock);
        FirmwareUpdater::Scan s;
        s.drives = drives;
        s.boards = int(drives.size()) + extraBoards;
        return s;
    }
    void mount(const FirmwareUpdater::Drive &d) { QMutexLocker hold(&lock); drives.append(d); }
    void clear() { QMutexLocker hold(&lock); drives.clear(); extraBoards = 0; }
};

void clickItem(QQuickItem *item) {
    QQuickWindow *w = item->window();
    const QPointF at = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
    const QPointF global = w->mapToGlobal(at.toPoint());
    QMouseEvent move(QEvent::MouseMove, at, at, global, Qt::NoButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &move);
    QMouseEvent press(QEvent::MouseButtonPress, at, at, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &press);
    QMouseEvent release(QEvent::MouseButtonRelease, at, at, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &release);
}

QQuickItem *findNamed(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *child : from->childItems())
        if (QQuickItem *hit = findNamed(child, name)) return hit;
    return nullptr;
}

QQuickItem *findWithProperty(QQuickItem *from, const char *property) {
    if (from->metaObject()->indexOfProperty(property) >= 0) return from;
    for (QQuickItem *child : from->childItems())
        if (QQuickItem *hit = findWithProperty(child, property)) return hit;
    return nullptr;
}

QByteArray versionOf(const DspState *s) {
    QByteArray v = QByteArray::number(s->fw_major) + '.' + QByteArray::number(s->fw_minor) + '.' + QByteArray::number(s->fw_patch);
    if (s->fw_beta > 0 && s->fw_beta != FW_BETA_EARLY) v += "-beta" + QByteArray::number(s->fw_beta);
    return v;
}
}

void SmokeDriver::testFirmware() {
    section("QD11: firmware updater on a fake bootloader drive");
    auto *fw = m_.firmware;
    auto *b = m_.bridge;
    FfiCore *core = b->core();
    const QString demo = QString::fromLatin1(kDemo);
    const QString base = m_out + QStringLiteral("/qd11");
    const auto expect = [this](const QString &what, bool ok) { check(qPrintable(QStringLiteral("QD11: ") + what), ok); };
    b->setHubSession(false);
    b->setDemoWire(0);

    // ── The files ──
    QDir().mkpath(base);
    const auto writeFile = [&](const QString &name, const QByteArray &bytes) {
        const QString path = base + QLatin1Char('/') + name;
        QFile f(path);
        const bool ok = f.open(QIODevice::WriteOnly) && f.write(bytes) == bytes.size();
        f.close();
        expect(QStringLiteral("test file ") + name, ok);
        return QUrl::fromLocalFile(path);
    };
    const auto inspect = [](const QByteArray &bytes) {
        QByteArray reply(4096, '\0');
        const uint32_t needed = dspi_uf2_inspect(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()), reply.data(), uint32_t(reply.size()));
        return QJsonDocument::fromJson(reply.left(int(needed) - 1)).object();
    };

    // ── OD4: stock v1.1.6, one for each chip, and what this Console says it expects ──
    for (int platform : { 0, 1 }) {
        QFile f(fw->bundledImageResource(platform));
        expect(QStringLiteral("a bundled image for chip %1").arg(platform), f.open(QIODevice::ReadOnly));
        const QByteArray bytes = f.readAll();
        const QJsonObject info = inspect(bytes);
        expect(QStringLiteral("it is valid, for that chip"), info.value("valid").toBool() && info.value("platform").toInt(-1) == platform);
        expect(QStringLiteral("it is version 1.1.6"), info.value("firmwareVersion").toString().startsWith(QLatin1String("1.1.6")) && !info.value("firmwareVersion").toString().contains(QLatin1String("beta")));
        expect(QStringLiteral("this Console expects that"), FwVersion::parse(info.value("firmwareVersion").toString()).text() == fw->expectedVersion());
    }

    // ── What the core's reply about drives looks like ──
    const auto parsed = FirmwareUpdater::parseDrives(R"({"ok":true,"value":[{"path":"/run/media/u/RP2350","platform":1,"info":"x"}]})");
    expect(QStringLiteral("a drive in the core's reply"), parsed.size() == 1 && parsed.first().path.endsWith(QLatin1String("RP2350")) && parsed.first().platform == 1);
    QByteArray drivesReply(4096, '\0');
    const uint32_t drivesNeeded = dspi_bootloader_drives(drivesReply.data(), uint32_t(drivesReply.size()));
    expect(QStringLiteral("the core's reply parses"), QJsonDocument::fromJson(drivesReply.left(int(drivesNeeded) - 1)).object().value("value").isArray());

    b->selectDevice(demo);
    spin(100);
    expect(QStringLiteral("the demo is an RP2350"), b->connected() && dspi_get_state(core)->platform_id == 1);
    const QByteArray running = versionOf(dspi_get_state(core));

    // The demo board needs 1.20 V, like a K340: an image stamped for that is clear,
    // and an unstamped one, which is what stock images are, takes the phrase.
    const QByteArray goodBytes = makeUf2(kRp2350Family, running, QByteArrayLiteral("1200"));
    const QUrl good = writeFile("good.uf2", goodBytes);
    const QUrl unstamped = writeFile("unstamped.uf2", makeUf2(kRp2350Family, running, QByteArray()));
    {
        QByteArray reply(1024, '\0');
        const uint32_t needed = dspi_firmware_guard_board(core, reply.data(), uint32_t(reply.size()));
        expect(QStringLiteral("the demo board reads as 1.20 V"), QJsonDocument::fromJson(reply.left(int(needed) - 1)).object().value("requiredMv").toInt() == 1200);
    }
    const QUrl wrongChip = writeFile("rp2040.uf2", makeUf2(kRp2040Family, running, QByteArrayLiteral("1200")));
    const QUrl junk = writeFile("junk.uf2", QByteArray(600, 'x'));
    const QUrl other = writeFile("other.uf2", makeUf2(kRp2350Family, QByteArrayLiteral("9.9.9"), QByteArrayLiteral("1200")));
    const QByteArray nameless = makeUf2(kRp2350Family, QByteArray(), QByteArrayLiteral("1200"));
    const QUrl noVersion = writeFile("noversion.uf2", nameless);
    const QByteArray stamped1250 = makeUf2(kRp2350Family, running, QByteArrayLiteral("1250"));
    const QUrl s1250 = writeFile("s1250.uf2", stamped1250);

    auto bus = std::make_shared<FakeBus>();
    int driveNo = 0;
    const auto newDrive = [&](int platform = 1) {
        const QString dir = base + QStringLiteral("/drive%1").arg(++driveNo);
        QDir().mkpath(dir);
        QFile info(dir + QStringLiteral("/INFO_UF2.TXT"));
        const bool ok = info.open(QIODevice::WriteOnly);
        info.write(platform == 1 ? "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n"
                                 : "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n");
        info.close();
        expect(QStringLiteral("a fake drive"), ok);
        FirmwareUpdater::Drive d;
        d.path = dir;
        d.platform = platform;
        return d;
    };
    const auto onDrive = [](const FirmwareUpdater::Drive &d) {
        QFile f(d.path + QStringLiteral("/firmware.uf2"));
        return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
    };
    const auto empty = [](const FirmwareUpdater::Drive &d) { return !QFile::exists(d.path + QStringLiteral("/firmware.uf2")); };

    FirmwareUpdater::TestSeam seam;
    seam.scan = [bus]() { return bus->scan(); };
    seam.demoIsBoard = true;
    seam.driveWaitMs = 1000;
    seam.verifyWaitMs = 2500;

    // Each case starts on a fresh demo device, with nothing mounted
    const auto start = [&](const QUrl &image = QUrl()) {
        fw->reset();
        fw->stopWatching();
        fw->useBundledImage();
        bus->clear();
        fw->setTestSeam(seam);
        b->selectDevice(demo);
        spin(50);
        if (!image.isEmpty()) {
            fw->chooseImage(image);
            waitFor(2000, [fw]() { return !fw->imageLoading(); });
        }
    };
    // The reboot: the device drops off the bus, and the app's hotplug opens it again
    const auto board = [&]() { b->disconnectDevice(); };
    const auto waitState = [&](const char *state, int ms = 3000) {
        const QString wanted = QString::fromLatin1(state);
        return waitFor(ms, [fw, wanted]() { return fw->state() == wanted; });
    };
    FirmwareUpdater::TestSeam notBoard = seam;
    notBoard.demoIsBoard = false;
    int enters = 0;
    int reads = 0;
    const auto mark = [&]() { enters = fw->bootloaderRequests(); reads = fw->boardReads(); };
    const auto noReboot = [&]() { return fw->bootloaderRequests() == enters; };
    const auto setVoltage = [core](uint8_t code) { return dspi_core_vreg_op(core, 0, code, kWord) == 0; };

    // ── The version banner ──
    fw->clearTestSeam();
    b->selectDevice(demo);
    spin(50);
    expect(QStringLiteral("the demo device is not a board: no banner"), fw->match() == 1);
    expect(QStringLiteral("and it can't be rebooted"), !fw->canReboot() && !fw->rebootBlocked().isEmpty());
    fw->setTestSeam(seam);
    expect(QStringLiteral("another version shows the banner"), fw->match() == 2);
    expect(QStringLiteral("the core compares, build stamp included"), dspi_firmware_version_matches(core, (running + "-20-g04122d3").constData()) == 1);

    // ── Wrong chip: refused before any question, read or reboot ──
    start(wrongChip);
    mark();
    expect(QStringLiteral("a wrong-chip file says so when chosen"), fw->imageProblem().contains(QLatin1String("RP2040")) && fw->imageProblem().contains(QLatin1String("RP2350")));
    fw->installWhenReady();
    expect(QStringLiteral("a wrong-chip file fails"), waitState("failed") && fw->failure().contains(QLatin1String("RP2040")));
    expect(QStringLiteral("before any question or board read"), noReboot() && fw->boardReads() == reads && fw->questionPhrase().isEmpty());

    start(wrongChip);
    FirmwareUpdater::Drive drive = newDrive();
    bus->mount(drive);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a wrong-chip file on a mounted drive fails"), waitState("failed") && fw->questionPhrase().isEmpty());
    expect(QStringLiteral("and the drive is untouched"), empty(drive) && noReboot());

    start(good);
    drive = newDrive(0);
    bus->mount(drive);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a file for the other chip than the drive's fails"), waitState("failed") && empty(drive) && noReboot());

    start(junk);
    mark();
    expect(QStringLiteral("a file that is not firmware says so when chosen"), fw->imageProblem().contains(QLatin1String("not a firmware file")));
    fw->installWhenReady();
    expect(QStringLiteral("and is refused"), waitState("failed") && noReboot() && fw->boardReads() == reads);

    start(QUrl::fromLocalFile(base + QStringLiteral("/no-such-file.uf2")));
    expect(QStringLiteral("a file that can't be read says so"), fw->imageProblem().contains(QLatin1String("Could not read")));

    // ── No risk: read just before the reboot, written, and back with its version ──
    start(good);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("no risk: read and rebooted"), waitState("waitingForDrive"));
    expect(QStringLiteral("one read, one reboot"), fw->boardReads() == reads + 1 && fw->bootloaderRequests() == enters + 1);
    expect(QStringLiteral("no question"), fw->questionPhrase().isEmpty());
    expect(QStringLiteral("a second click doesn't reboot it again"), [&]() { fw->enterBootloader(); fw->installWhenReady(); spin(50); return fw->bootloaderRequests() == enters + 1; }());
    board();
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("the drive takes the bytes"), waitFor(3000, [&]() { return onDrive(drive) == goodBytes; }));
    expect(QStringLiteral("the board back with the image's version is success"), waitState("verified", 4000) && !fw->verifiedVersion().isEmpty());

    // ── Writing is not success ──
    start(other);
    fw->installWhenReady();
    waitState("waitingForDrive");
    board();
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("another version coming back is a failure"), waitState("failed", 5000) && fw->failure().contains(QLatin1String("did not take")));

    start(noVersion);
    fw->installWhenReady();
    waitState("waitingForDrive");
    board();
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("an image with no version can't be confirmed"), waitState("unconfirmed", 5000) && onDrive(drive) == nameless);

    // A device that never leaves says nothing about what was written
    start(good);
    fw->installWhenReady();
    waitState("waitingForDrive");
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("a device that never left is not the board back"), waitState("failed", 5000) && fw->failure().contains(QLatin1String("not confirmed")));

    // ── The guard's phrase, in the real window's real field ──
    start();
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("firmware")));
    spin(150);
    QQuickWindow *win = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->metaObject()->indexOfSignal("exportRequested()") >= 0) win = w;
    expect(QStringLiteral("the window"), win && win->isVisible());
    if (!win) return;
    // Opening the window starts over, on this Console's own image
    fw->chooseImage(unstamped);
    waitFor(2000, [fw]() { return !fw->imageLoading(); });
    QQuickItem *content = win->contentItem();
    QQuickItem *install = findNamed(content, QStringLiteral("firmwareInstall"));
    QQuickItem *exportButton = findNamed(content, QStringLiteral("firmwareExport"));
    expect(QStringLiteral("the buttons"), install && exportButton);
    if (!install || !exportButton) return;
    expect(QStringLiteral("export is offered first"), exportButton->isVisible());
    mark();
    clickItem(install);
    expect(QStringLiteral("an at-risk board gets the question"), waitState("question"));
    expect(QStringLiteral("it names the phrase"), fw->questionPhrase() == QLatin1String(kPhrase) && !fw->questionMessage().isEmpty());
    expect(QStringLiteral("nothing rebooted yet"), noReboot());
    expect(QStringLiteral("export has gone"), !exportButton->isVisible());
    spin(100);
    win->grabWindow().save(m_out + QStringLiteral("/window-firmware-question.png"));
    QQuickItem *field = findNamed(content, QStringLiteral("firmwareAnswer"));
    QQuickItem *go = findNamed(content, QStringLiteral("firmwareAnswerGo"));
    expect(QStringLiteral("a real text field"), field && field->inherits("QQuickTextInput") && field->isVisible());
    expect(QStringLiteral("and a button"), go && go->isVisible());
    if (!field || !go) return;
    expect(QStringLiteral("the button alone can't answer"), !go->isEnabled());
    clickItem(go);
    spin(100);
    expect(QStringLiteral("clicking it does nothing"), fw->state() == QLatin1String("question") && noReboot());
    field->setProperty("text", QStringLiteral("i accept the core voltage risk"));
    clickItem(go);
    expect(QStringLiteral("lower case is not the phrase"), waitFor(2000, [fw]() { return fw->answerWrong(); }) && fw->state() == QLatin1String("question") && noReboot());
    field->setProperty("text", QString::fromLatin1(kWord));
    clickItem(go);
    spin(200);
    expect(QStringLiteral("the word does not stand in for it"), fw->state() == QLatin1String("question") && noReboot());
    const int readsBefore = fw->boardReads();
    field->setProperty("text", QString::fromLatin1(kPhrase));
    clickItem(go);
    expect(QStringLiteral("the phrase lets it go"), waitState("waitingForDrive"));
    expect(QStringLiteral("after reading the board again"), fw->boardReads() == readsBefore + 1 && fw->bootloaderRequests() == enters + 1);
    board();
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("and the guarded write takes that answer"), waitFor(3000, [&]() { return !onDrive(drive).isEmpty(); }));
    waitState("verified", 4000);
    spin(100);
    win->grabWindow().save(m_out + QStringLiteral("/window-firmware-verified.png"));
    win->setVisible(false);
    spin(50);

    // Closing on a question: nothing is rebooted or written
    start(unstamped);
    mark();
    fw->installWhenReady();
    waitState("question");
    fw->beginWatching();
    fw->stopWatching();
    expect(QStringLiteral("closing on a question ends it"), fw->state() == QLatin1String("idle") && noReboot());
    fw->answer(QString::fromLatin1(kPhrase));
    spin(100);
    expect(QStringLiteral("and a late answer does nothing"), fw->state() == QLatin1String("idle") && noReboot());

    // ── Each risk takes its own answer, against the board as it is now ──
    start(s1250);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a raising image takes the word"), waitState("question") && fw->questionPhrase() == QLatin1String(kWord));
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("the phrase does not stand in for it"), waitFor(2000, [fw]() { return fw->answerWrong(); }) && noReboot());
    fw->answer(QString::fromLatin1(kWord));
    expect(QStringLiteral("the word lets it go"), waitState("waitingForDrive"));

    start(s1250);
    expect(QStringLiteral("a board at 1.30 V"), setVoltage(0x0F));
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a lower image takes the phrase"), waitState("question") && fw->questionPhrase() == QLatin1String(kPhrase));
    // While the question waits, the board comes down: now the same image raises it
    expect(QStringLiteral("the board comes down to 1.20 V"), setVoltage(0x0D));
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("the re-read asks the other risk"), waitFor(2000, [fw]() { return fw->questionPhrase() == QLatin1String(kWord); }) && !fw->answerWrong());
    expect(QStringLiteral("the first answer doesn't carry over"), noReboot());
    fw->answer(QString::fromLatin1(kWord));
    expect(QStringLiteral("its own answer does"), waitState("waitingForDrive"));
    board();
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("and the write goes through on the record it was asked for"), waitFor(3000, [&]() { return onDrive(drive) == stamped1250; }));
    waitState("verified", 4000);

    // ── A drive already mounted is a board nobody can ask ──
    start(good);
    drive = newDrive();
    bus->mount(drive);
    mark();
    FirmwareUpdater::TestSeam after = seam;
    fw->setTestSeam(after);
    fw->installWhenReady();
    expect(QStringLiteral("a mounted drive gets the question, though the connected DSPi would be clear"), waitState("question") && fw->questionPhrase() == QLatin1String(kPhrase));
    expect(QStringLiteral("without reading that DSPi or rebooting it"), fw->boardReads() == reads && noReboot());
    fw->answer(QStringLiteral("no"));
    expect(QStringLiteral("a wrong answer writes nothing"), waitFor(2000, [fw]() { return fw->answerWrong(); }) && empty(drive));
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("the phrase writes it"), waitFor(3000, [&]() { return onDrive(drive) == goodBytes; }));
    expect(QStringLiteral("still without a reboot"), noReboot());
    // The demo was on the bus already, so it isn't the board that was written
    expect(QStringLiteral("the DSPi that was there is not the board back"), waitState("failed", 5000) && fw->failure().contains(QLatin1String("not confirmed")));

    // The one that wasn't there before is, as soon as it appears
    start(good);
    auto present = std::make_shared<std::atomic<bool>>(false);
    after.serials = [present, demo]() { return *present ? QStringList { demo } : QStringList(); };
    fw->setTestSeam(after);
    drive = newDrive();
    bus->mount(drive);
    fw->installWhenReady();
    waitState("question");
    fw->answer(QString::fromLatin1(kPhrase));
    waitState("waitingForDevice");
    spin(400);
    expect(QStringLiteral("nothing judged until a new device"), fw->state() == QLatin1String("waitingForDevice"));
    *present = true;
    expect(QStringLiteral("the new device is the board"), waitState("verified", 4000));
    after.serials = nullptr;

    QQuickWindow *mainWindow = nullptr;
    QQuickItem *wizard = nullptr;
    // ── This Console's own images, each for its own chip ──
    const auto bundled = [fw](int platform) {
        QFile f(fw->bundledImageResource(platform));
        return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
    };
    start();
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a stock image on a board that needs 1.20 V takes the phrase"), waitState("question") && fw->questionPhrase() == QLatin1String(kPhrase) && noReboot());
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("then the reboot"), waitState("waitingForDrive") && fw->bootloaderRequests() == enters + 1);
    board();
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("the RP2350 image goes on the drive"), waitFor(3000, [&]() { return onDrive(drive) == bundled(1); }));
    expect(QStringLiteral("1.1.6 written and an older version back did not take"), waitState("failed", 5000) && fw->failure().contains(QLatin1String("did not take")));

    start();
    drive = newDrive(0);
    bus->mount(drive);
    fw->installWhenReady();
    expect(QStringLiteral("a board in its bootloader takes the phrase"), waitState("question"));
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("an RP2040 drive gets the RP2040 image"), waitFor(3000, [&]() { return onDrive(drive) == bundled(0); }));
    waitState("failed", 5000);

    // ── Getting Started goes the same way: its reboot waits for the guard ──
    start();
    root()->setProperty("setupRequested", true);
    spin(200);
    mainWindow = qobject_cast<QQuickWindow *>(root());
    wizard = mainWindow ? findWithProperty(mainWindow->contentItem(), "connectedMatch") : nullptr;
    expect(QStringLiteral("the wizard"), wizard != nullptr);
    if (wizard) {
        wizard->setProperty("stage", 1);
        spin(100);
        mark();
        QMetaObject::invokeMethod(wizard, "beginConnectedInstall");
        expect(QStringLiteral("the wizard's install gets the question"), waitState("question"));
        expect(QStringLiteral("before any reboot"), noReboot());
        spin(100);
        mainWindow->grabWindow().save(m_out + QStringLiteral("/window-wizard-firmware-question.png"));
        QQuickItem *answerField = findNamed(wizard, QStringLiteral("firmwareAnswer"));
        QQuickItem *answerGo = findNamed(wizard, QStringLiteral("firmwareAnswerGo"));
        expect(QStringLiteral("with a real field"), answerField && answerField->isVisible() && answerGo && answerGo->isVisible());
        if (answerField && answerGo) {
            answerField->setProperty("text", QString::fromLatin1(kPhrase));
            clickItem(answerGo);
            expect(QStringLiteral("the phrase reboots it, once"), waitState("waitingForDrive") && fw->bootloaderRequests() == enters + 1);
        }
        wizard->setProperty("stage", 0);
        spin(50);
    }
    root()->setProperty("setupRequested", false);
    spin(100);

    // ── Two boards ──
    start(good);
    FirmwareUpdater::Drive first = newDrive(), second = newDrive();
    bus->mount(first);
    bus->mount(second);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("two drives are refused"), waitState("failed") && fw->failure().contains(QLatin1String("2 boards")));
    expect(QStringLiteral("and nothing is asked, written or rebooted"), fw->questionPhrase().isEmpty() && empty(first) && empty(second) && noReboot());

    start(good);
    first = newDrive();
    bus->mount(first);
    {
        QMutexLocker hold(&bus->lock);
        bus->extraBoards = 1;
    }
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("one drive and a second board on the bus are refused too"), waitState("failed") && empty(first) && noReboot());

    start(good);
    fw->installWhenReady();
    waitState("waitingForDrive");
    board();
    first = newDrive();
    second = newDrive();
    {
        QMutexLocker hold(&bus->lock);
        bus->drives = { first, second };
    }
    expect(QStringLiteral("two drives appearing after the reboot: nothing written"), waitState("failed") && fw->failure().contains(QLatin1String("Nothing was written")) && empty(first) && empty(second));

    // A board in BOOTSEL with no drive yet is the one meant: no reboot past it
    start(good);
    {
        QMutexLocker hold(&bus->lock);
        bus->extraBoards = 1;
    }
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a board with no drive yet is waited for"), waitState("waitingForVolume") && noReboot());

    // ── No drive ──
    start(good);
    fw->installWhenReady();
    waitState("waitingForDrive");
    expect(QStringLiteral("no drive appearing: nothing written"), waitState("failed", 4000) && fw->failure().contains(QLatin1String("Nothing was written")) && fw->noBoardFailure());

    // ── The click is about the device there at the click ──
    start(good);
    mark();
    b->disconnectDevice();
    fw->installWhenReady();
    expect(QStringLiteral("armed with no device"), waitFor(2000, [fw]() { return fw->state() == QLatin1String("waitingForBoard"); }));
    waitFor(2000, [b]() { return b->connected(); });
    spin(1300);
    expect(QStringLiteral("a device that turns up later is not rebooted"), b->connected() && noReboot() && fw->state() == QLatin1String("waitingForBoard"));

    // ── Enter bootloader without installing ──
    start();
    mark();
    fw->enterBootloader();
    spin(50);
    expect(QStringLiteral("enter bootloader goes"), fw->bootloaderRequests() == enters + 1);
    b->selectDevice(demo);

    // ── Only USB, and the demo device is not a board ──
    start(unstamped);
    b->setHubSession(true);
    mark();
    expect(QStringLiteral("a hub's device can't be rebooted from here"), !fw->canReboot() && fw->rebootBlocked().contains(QLatin1String("hub")));
    fw->installWhenReady();
    expect(QStringLiteral("and its install is refused"), waitState("failed") && noReboot() && fw->boardReads() == reads);
    b->setHubSession(false);

    start(good);
    fw->setTestSeam(notBoard);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("the demo device has no bootloader to reach"), waitState("failed") && fw->failure().contains(QLatin1String("demo")) && noReboot());

    // ── No UF2 bootloader: no update offered ──
    FirmwareUpdater::TestSeam stm = seam;
    stm.platform = 2;
    start(good);
    fw->setTestSeam(stm);
    expect(QStringLiteral("an STM32H723 isn't supported, and has no banner"), !fw->supported() && fw->match() == 0 && !fw->canReboot());
    spin(50);
    win->setVisible(false);
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("firmware")));
    spin(100);
    expect(QStringLiteral("its update window doesn't open"), !win->isVisible());
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("and an install is refused"), waitState("failed") && noReboot());
    fw->setTestSeam(seam);

    // ── Off the UI thread ──
    start(good);
    bus->otherThreadScans = 0;
    bus->uiThreadScans = 0;
    bus->scanDelayMs = 300;
    QElapsedTimer clock;
    clock.start();
    fw->installWhenReady();
    expect(QStringLiteral("a slow bus doesn't hold the UI"), clock.elapsed() < 150);
    waitState("waitingForDrive");
    bus->scanDelayMs = 0;
    expect(QStringLiteral("scans are on the worker"), bus->otherThreadScans > 0 && bus->uiThreadScans == 0);

    // ── Read-only: another wire format can still update ──
    for (int wire : { 17, 33 }) {
        const QString tag = QStringLiteral("V%1: ").arg(wire);
        fw->reset();
        fw->stopWatching();
        bus->clear();
        b->setDemoWire(wire);
        b->selectDevice(demo);
        spin(100);
        expect(tag + QStringLiteral("read-only"), b->connected() && !b->writable());
        const uint32_t refusals = dspi_read_only_refusals(core);
        const QByteArray itsVersion = versionOf(dspi_get_state(core));
        const QByteArray bytes = makeUf2(kRp2350Family, itsVersion, QByteArrayLiteral("1250"));
        fw->chooseImage(writeFile(QStringLiteral("wire%1.uf2").arg(wire), bytes));
        waitFor(2000, [fw]() { return !fw->imageLoading(); });
        mark();
        expect(tag + QStringLiteral("it can be rebooted"), fw->canReboot());
        fw->installWhenReady();
        waitFor(3000, [fw]() { return fw->state() == QLatin1String("question") || fw->state() == QLatin1String("waitingForDrive") || fw->state() == QLatin1String("failed"); });
        if (fw->state() == QLatin1String("question")) {
            fw->answer(fw->questionPhrase());
            waitFor(3000, [fw]() { return fw->state() == QLatin1String("waitingForDrive") || fw->state() == QLatin1String("failed"); });
        }
        expect(tag + QStringLiteral("it reboots"), fw->state() == QLatin1String("waitingForDrive") && fw->bootloaderRequests() == enters + 1);
        board();
        drive = newDrive();
        bus->mount(drive);
        expect(tag + QStringLiteral("it writes"), waitFor(3000, [&]() { return onDrive(drive) == bytes; }));
        expect(tag + QStringLiteral("it comes back confirmed"), waitState("verified", 5000));
        expect(tag + QStringLiteral("no setter was refused on the way"), dspi_read_only_refusals(core) == refusals);
    }
    b->setDemoWire(0);

    // ── Demo open: nothing goes to any drive ──
    start();
    fw->setTestSeam(notBoard);
    drive = newDrive();
    bus->mount(drive);
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("demo open: this Console's image is refused on a mounted drive"), waitState("failed") && fw->failure().contains(QLatin1String("demo")) && fw->failure().contains(QLatin1String("Leave Demo")));
    expect(QStringLiteral("nothing asked, read, rebooted or written"), fw->questionPhrase().isEmpty() && fw->boardReads() == reads && noReboot() && empty(drive));

    start(good);
    fw->setTestSeam(notBoard);
    drive = newDrive();
    bus->mount(drive);
    fw->installWhenReady();
    expect(QStringLiteral("demo open: a custom file too"), waitState("failed") && fw->failure().contains(QLatin1String("demo")) && empty(drive));

    // The window has nothing to click, and says why
    start();
    fw->setTestSeam(notBoard);
    drive = newDrive();
    bus->mount(drive);
    win->setVisible(true);
    expect(QStringLiteral("demo open: the window sees the drive"), waitState("ready"));
    spin(100);
    expect(QStringLiteral("and its install button is off"), !install->isEnabled() && !fw->writeBlocked().isEmpty());
    clickItem(install);
    spin(200);
    expect(QStringLiteral("clicking it writes nothing"), fw->state() == QLatin1String("ready") && empty(drive));
    win->grabWindow().save(m_out + QStringLiteral("/window-firmware-demo.png"));
    win->setVisible(false);
    spin(50);

    // The wizard's install ends in the same refusal, in its own card
    start();
    fw->setTestSeam(notBoard);
    drive = newDrive();
    bus->mount(drive);
    root()->setProperty("setupRequested", true);
    spin(200);
    wizard = mainWindow ? findWithProperty(mainWindow->contentItem(), "connectedMatch") : nullptr;
    expect(QStringLiteral("the wizard again"), wizard != nullptr);
    if (wizard) {
        wizard->setProperty("stage", 1);
        expect(QStringLiteral("demo open: the wizard sees the drive"), waitState("ready"));
        QMetaObject::invokeMethod(wizard, "beginInstall");
        expect(QStringLiteral("and its install is refused, saying why"), waitState("failed") && fw->failure().contains(QLatin1String("Leave Demo")) && empty(drive));
        wizard->setProperty("stage", 0);
        spin(50);
    }
    root()->setProperty("setupRequested", false);
    spin(100);

    // The demo opened while a question waited
    start(s1250);
    drive = newDrive();
    bus->mount(drive);
    fw->installWhenReady();
    waitState("question");
    fw->setTestSeam(notBoard);
    fw->answer(QString::fromLatin1(kWord));
    expect(QStringLiteral("demo opened during the question: nothing written"), waitState("failed") && fw->failure().contains(QLatin1String("demo")) && empty(drive));

    // ── Cancel while a gate job runs ──
    struct Hold { std::atomic<bool> in { false }, release { false }; };
    const auto held = [&](FirmwareUpdater::TestSeam with, const std::shared_ptr<Hold> &hold) {
        with.gateHook = [hold]() {
            hold->in = true;
            while (!hold->release) QThread::msleep(5);
        };
        return with;
    };
    // Rebooted before the cancel arrived: a Cancel never ends in a write
    const auto cancelAfterReboot = [&]() {
        start(good);
        auto h = std::make_shared<Hold>();
        fw->setTestSeam(held(seam, h));
        mark();
        fw->installWhenReady();
        expect(QStringLiteral("a reboot in flight"), waitFor(3000, [&]() { return bool(h->in); }));
        fw->stopWatching();
        fw->reset();
        expect(QStringLiteral("cancel waits for the job"), fw->state() == QLatin1String("checking"));
        h->release = true;
        expect(QStringLiteral("cancel after the reboot says so"), waitState("inBootloader") && fw->bootloaderRequests() == enters + 1
               && fw->failure().contains(QLatin1String("nothing was written")) && fw->failure().contains(QLatin1String("unplug it")));
        fw->setTestSeam(seam);
    };
    cancelAfterReboot();
    auto hold = std::make_shared<Hold>();
    spin(50);
    expect(QStringLiteral("the window comes back to say it"), win->isVisible());
    expect(QStringLiteral("with Install usable"), install->isVisible() && install->isEnabled());
    spin(100);
    win->grabWindow().save(m_out + QStringLiteral("/window-firmware-in-bootloader.png"));
    board();
    drive = newDrive();
    bus->mount(drive);
    spin(1400);
    expect(QStringLiteral("the drive arriving writes nothing"), fw->state() == QLatin1String("inBootloader") && empty(drive));
    expect(QStringLiteral("nor does reset or a close and reopen"), [&]() {
        fw->reset();
        win->setVisible(false);
        spin(50);
        win->setVisible(true);
        spin(100);
        return fw->state() == QLatin1String("inBootloader") && empty(drive);
    }());
    reads = fw->boardReads();
    enters = fw->bootloaderRequests();
    clickItem(install);
    expect(QStringLiteral("Install again asks, as of a board nobody can ask"), waitState("question") && fw->questionPhrase() == QLatin1String(kPhrase));
    expect(QStringLiteral("without reading a device or another reboot"), fw->boardReads() == reads && noReboot() && empty(drive));
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("only then is it written"), waitFor(3000, [&]() { return onDrive(drive) == goodBytes; }));
    waitState("failed", 5000);
    win->setVisible(false);
    spin(50);

    // Install again before the drive is there: no reboot of what is connected
    cancelAfterReboot();
    enters = fw->bootloaderRequests();
    fw->installWhenReady();
    spin(400);
    expect(QStringLiteral("armed again with no drive: no second reboot"), noReboot() && fw->state() == QLatin1String("inBootloader"));
    drive = newDrive();
    bus->mount(drive);
    expect(QStringLiteral("and the drive gets its question"), waitState("question", 3000) && empty(drive));
    win->setVisible(false);
    spin(50);
    fw->reset();
    bus->clear();

    // Unplugged, it goes back to looking
    cancelAfterReboot();
    drive = newDrive();
    bus->mount(drive);
    spin(1300);
    expect(QStringLiteral("still parked with the drive there"), fw->state() == QLatin1String("inBootloader"));
    bus->clear();
    expect(QStringLiteral("unplugging ends it"), waitState("waitingForBoard", 3000) && fw->failure().isEmpty());
    win->setVisible(false);
    spin(50);

    // A board that never arrives doesn't hold it for ever
    cancelAfterReboot();
    expect(QStringLiteral("nor does a drive that never comes"), waitState("waitingForBoard", 4000));
    win->setVisible(false);
    spin(50);

    // Not rebooted: the cancel stands
    start(unstamped);
    fw->setTestSeam(held(seam, hold));
    mark();
    fw->installWhenReady();
    expect(QStringLiteral("a check in flight"), waitFor(3000, [&]() { return bool(hold->in); }));
    fw->stopWatching();
    hold->release = true;
    expect(QStringLiteral("cancel before any reboot ends it"), waitState("idle") && noReboot() && fw->questionPhrase().isEmpty());
    fw->setTestSeam(seam);

    start(unstamped);
    drive = newDrive();
    bus->mount(drive);
    fw->installWhenReady();
    waitState("question");
    hold = std::make_shared<Hold>();
    fw->setTestSeam(held(seam, hold));
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("an answer in flight"), waitFor(3000, [&]() { return bool(hold->in); }));
    fw->stopWatching();
    hold->release = true;
    expect(QStringLiteral("cancel on a mounted drive writes nothing"), waitState("idle"));
    spin(200);
    expect(QStringLiteral("nothing at all"), empty(drive));
    fw->setTestSeam(seam);

    // ── Hub session and a local drive: written, can't confirm ──
    start(good);
    b->setHubSession(true);
    drive = newDrive();
    bus->mount(drive);
    fw->installWhenReady();
    waitState("question");
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("a hub session: written"), waitFor(3000, [&]() { return !onDrive(drive).isEmpty(); }));
    expect(QStringLiteral("and says it can't confirm, not that the board is gone"), waitState("unconfirmed", 5000) && fw->failure().contains(QLatin1String("hub session")) && !fw->failure().contains(QLatin1String("did not come back")));
    b->setHubSession(false);

    // ── Two boards, after the answer ──
    start(good);
    first = newDrive();
    bus->mount(first);
    fw->installWhenReady();
    waitState("question");
    second = newDrive();
    bus->mount(second);
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("a second drive during the question is refused at the answer"), waitState("failed") && fw->failure().contains(QLatin1String("2 boards")) && empty(first) && empty(second));

    start(good);
    first = newDrive();
    bus->mount(first);
    fw->installWhenReady();
    waitState("question");
    {
        QMutexLocker hold2(&bus->lock);
        bus->extraBoards = 1;
    }
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("so is a second board on the bus"), waitState("failed") && empty(first));

    start(good);
    first = newDrive();
    bus->mount(first);
    fw->installWhenReady();
    waitState("question");
    bus->clear();
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("a drive gone during the question is not written"), waitState("failed") && fw->failure().contains(QLatin1String("has gone")) && empty(first));

    start(unstamped);
    mark();
    fw->installWhenReady();
    waitState("question");
    drive = newDrive();
    bus->mount(drive);
    fw->answer(QString::fromLatin1(kPhrase));
    expect(QStringLiteral("a drive appearing while a running board's question waits stops its reboot"), waitState("failed") && noReboot() && empty(drive));

    // Back as it was
    fw->reset();
    fw->stopWatching();
    fw->useBundledImage();
    fw->clearTestSeam();
    b->selectDevice(demo);
    spin(100);
    expect(QStringLiteral("the demo is back on its own firmware"), b->writable() && fw->match() == 1);
}
