#include "FirmwareUpdater.h"
#include "DSPiBridge.h"
#include "DemoDevice.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QRegularExpression>
#include <QRunnable>
#include <algorithm>

extern "C" {
#include "dspi_core.h"
}

namespace {
// How often the bus and the drives are looked at
constexpr int kScanMs = 1000;
constexpr int kScanWaitingMs = 250;     // while a rebooting board's drive is awaited
// How long a board may sit on the bus with no drive before that's a failure
constexpr int kVolumeWaitMs = 8000;
// How long a board sent to its bootloader has to mount its drive
constexpr int kDriveWaitMs = 30000;
// How long to wait for the device to come back after a write (reboot, USB
// enumeration, our reconnect and platform read)
constexpr int kVerifyTimeoutMs = 30000;
// Between attempts to open the board that came back
constexpr int kSelectRetryMs = 2000;
// A board can keep changing while its question waits, but not for ever
constexpr int kMaxQuestions = 3;
// A UF2 of a whole RP2350 flash window is 64 MiB; uf2_inspect refuses more with a reason
constexpr qint64 kMaxFile = 65 * 1024 * 1024;

constexpr int kRp2040 = 0;
constexpr int kRp2350 = 1;

QString chipOf(int platform) {
    return platform == kRp2350 ? QStringLiteral("RP2350") : platform == kRp2040 ? QStringLiteral("RP2040") : QCoreApplication::translate("FirmwareUpdater", "unknown chip");
}

// Text from a file or a path, for a Text item: no control characters, and nothing
// that would read as rich text.
QString plain(const QString &text) {
    QString out;
    out.reserve(text.size());
    for (const QChar c : text)
        if (c.isPrint() && c != QLatin1Char('<') && c != QLatin1Char('>')) out.append(c);
    return out;
}

QByteArray replyOf(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(16 * 1024, '\0');
    uint32_t needed = call(buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = call(buf.data(), uint32_t(buf.size()));
    }
    return buf.left(int(qMax<uint32_t>(1, needed) - 1));
}

// The bundled image for a chip, e.g. :/firmware/DSPi-RP2350-v1.1.6.uf2
QString imagePath(int platform) {
    const QString token = platform == kRp2350 ? "-RP2350-" : "-RP2040-";
    for (const QString &name : QDir(":/firmware").entryList({ "*.uf2" }))
        if (name.contains(token)) return ":/firmware/" + name;
    return QString();
}

// The version a bundled image's file name gives, "1.1.6"
QString versionInName(const QString &path) {
    const int at = path.lastIndexOf("-v");
    if (at < 0) return QString();
    QString version = path.mid(at + 2);
    version.chop(4);    // .uf2
    return version;
}
}

// ── Versions ──

FwVersion FwVersion::parse(const QString &text) {
    FwVersion v;
    QRegularExpressionMatch m = QRegularExpression("^(\\d+)(?:\\.(\\d+))?(?:\\.(\\d+))?(.*)$").match(text.trimmed());
    if (!m.hasMatch()) return v;
    v.major = m.captured(1).toInt();
    v.minor = m.captured(2).toInt();
    v.patch = m.captured(3).toInt();
    QRegularExpressionMatch b = QRegularExpression("^[-. ]*beta\\s*(\\d+)", QRegularExpression::CaseInsensitiveOption)
                                    .match(m.captured(4));
    v.beta = b.hasMatch() ? b.captured(1).toInt() : 0;
    v.valid = true;
    return v;
}

QString FwVersion::text() const {
    if (!valid) return QString();
    QString t = QString("%1.%2.%3").arg(major).arg(minor).arg(patch);
    if (beta == -1) return t + " early beta";
    if (beta > 0) return t + QString(" beta %1").arg(beta);
    return t;
}

bool FwVersion::operator<(const FwVersion &o) const {
    if (major != o.major) return major < o.major;
    if (minor != o.minor) return minor < o.minor;
    if (patch != o.patch) return patch < o.patch;
    return betaRank() < o.betaRank();
}

// ── Updater ──

FirmwareUpdater::FirmwareUpdater(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge)
{
    m_worker.setMaxThreadCount(1);
    // The version this Console targets is the one it carries
    m_expectedRaw = versionInName(imagePath(kRp2350));
    m_expected = FwVersion::parse(m_expectedRaw);
    m_scanTimer.setInterval(kScanMs);
    connect(&m_scanTimer, &QTimer::timeout, this, &FirmwareUpdater::requestScan);
    m_verifyTimer.setInterval(250);
    connect(&m_verifyTimer, &QTimer::timeout, this, &FirmwareUpdater::verifyTick);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &FirmwareUpdater::deviceChanged);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &FirmwareUpdater::imageChanged);
    // statusChanged comes with every meter poll; pass on only a connection change
    connect(m_bridge, &DSPiBridge::statusChanged, this, [this]() {
        if (m_bridge->connected() != m_wasConnected) { m_wasConnected = m_bridge->connected(); emit deviceChanged(); }
        if (!m_bridge->connected()) m_sawGone = true;
    });
    connect(m_bridge, &DSPiBridge::deviceChanging, this, [this]() { m_sawGone = true; });
}

FirmwareUpdater::~FirmwareUpdater() {
    m_worker.waitForDone();
}

#ifdef DSPI_SMOKE
void FirmwareUpdater::setTestSeam(const TestSeam &seam) {
    m_seam = seam;
    emit deviceChanged();
}
void FirmwareUpdater::clearTestSeam() {
    m_worker.waitForDone();
    m_seam = TestSeam();
    emit deviceChanged();
}
QString FirmwareUpdater::bundledImageResource(int platform) const { return imagePath(platform); }
#endif

bool FirmwareUpdater::connectedNow() const { return m_bridge->connected(); }

bool FirmwareUpdater::isDemoDevice() const {
#ifdef DSPI_SMOKE
    if (m_seam.demoIsBoard) return false;
#endif
    return m_bridge->selectedSerial() == QLatin1String(DemoDevice::kSerial);
}

static FwVersion deviceFw(DSPiBridge *bridge) {
    FwVersion v;
    if (!bridge->connected()) return v;
    const DspState *s = dspi_get_state(bridge->core());
    if (s->fw_major == 0 && s->fw_minor == 0 && s->fw_patch == 0) return v;
    v.major = s->fw_major; v.minor = s->fw_minor; v.patch = s->fw_patch;
    v.beta = s->fw_beta == FW_BETA_EARLY ? -1 : s->fw_beta;
    v.valid = true;
    return v;
}

QString FirmwareUpdater::deviceVersion() const { return deviceFw(m_bridge).text(); }

bool FirmwareUpdater::supported() const {
    if (!m_bridge->connected()) return true;
    int platform = dspi_get_state(m_bridge->core())->platform_id;
#ifdef DSPI_SMOKE
    if (m_seam.platform >= 0) platform = m_seam.platform;
#endif
    return platform == kRp2040 || platform == kRp2350;
}

int FirmwareUpdater::match() const {
    if (!supported()) return 0;
    const FwVersion d = deviceFw(m_bridge);
    if (!d.valid || !m_expected.valid) return 0;
    // The demo device is not a board, so it has nothing to update
    if (isDemoDevice()) return 1;
    const QByteArray wanted = m_expectedRaw.toUtf8();
    const int same = dspi_firmware_version_matches(m_bridge->core(), wanted.constData());
    if (same == 1) return 1;
    if (same < 0) return 0;
    return d < m_expected ? 2 : 3;
}

QString FirmwareUpdater::rebootBlocked() const {
    if (!connectedNow()) return QString();
    if (!supported()) return tr("This board has no UF2 bootloader.");
    if (m_bridge->hubSession())
        return tr("This device is on a hub. Its bootloader drive would appear where the hub runs, so update it there.");
    if (isDemoDevice()) return writeBlocked();
    return QString();
}

QString FirmwareUpdater::writeBlocked() const {
    if (!isDemoDevice()) return QString();
    return tr("The demo is open, so nothing is written to a real board. Use Leave Demo in the banner, or start the app without --demo, and try again.");
}

QString FirmwareUpdater::chipName() const {
    return m_chip == kRp2350 ? "RP2350 (Pico 2)" : m_chip == kRp2040 ? "RP2040 (Pico)" : QString();
}
QString FirmwareUpdater::volumeName() const {
    return m_chip == kRp2350 ? "RP2350" : m_chip == kRp2040 ? "RPI-RP2" : QString();
}

QString FirmwareUpdater::imageName() const {
    return m_custom.loaded ? plain(m_custom.name) : tr("This Console's firmware");
}

QString FirmwareUpdater::imageVersion() const {
    if (!m_custom.loaded) return m_expected.text();
    return m_custom.version.isEmpty() ? tr("names no version") : plain(m_custom.version);
}

QString FirmwareUpdater::imageProblem() const {
    if (!m_custom.problem.isEmpty()) return plain(m_custom.problem);
    if (!m_custom.loaded) return QString();
    // The board this would go on, if one can be named yet
    int board = m_chip;
    if (board < 0 && connectedNow() && supported()) board = dspi_get_state(m_bridge->core())->platform_id;
    if (board < 0 || board == m_custom.platform) return QString();
    return tr("%1 is firmware for the %2, and this board is an %3.").arg(plain(m_custom.name), chipOf(m_custom.platform), chipOf(board));
}

void FirmwareUpdater::publish(const QString &state) {
    if (state == m_state) return;
    m_state = state;
    emit stateChanged();
}

void FirmwareUpdater::fail(const QString &message, bool mundane, bool noBoard) {
    m_failure = plain(message);
    m_failureMundane = mundane;
    m_noBoard = noBoard;
    m_state = "failed";
    m_armed = false;
    m_armedForDevice = false;
    emit stateChanged();
}

void FirmwareUpdater::beginWatching() {
    m_watching = true;
    m_scanTimer.setInterval(kScanMs);
    m_scanTimer.start();
    requestScan();
}

void FirmwareUpdater::stopWatching() {
    m_watching = false;
    // A write finishes, and its outcome holds until reset()
    if (m_state == QLatin1String("writing") || m_state == QLatin1String("waitingForDevice")) return;
    // The job may have rebooted the board already: gateDone decides
    if (m_state == QLatin1String("checking")) { m_cancelPending = true; return; }
    m_scanTimer.stop();
    // Still so when reopened, but a close takes back an Install pressed again
    if (m_state == QLatin1String("inBootloader")) { m_armed = false; return; }
    if (m_installing && (m_state == QLatin1String("failed") || m_state == QLatin1String("verified") || m_state == QLatin1String("unconfirmed"))) return;
    // Whatever waited on a question or a drive ends here. Nothing is written.
    m_generation++;
    m_installing = false;
    m_armed = false;
    m_armedForDevice = false;
    m_image = Image();
    m_typed.clear();
    m_questionMessage.clear();
    m_questionPhrase.clear();
    m_answerWrong = false;
    emit questionChanged();
    publish("idle");
}

void FirmwareUpdater::requestScan() {
    if (m_scanBusy) { m_rescan = true; return; }
    m_scanBusy = true;
    const unsigned generation = m_generation;
#ifdef DSPI_SMOKE
    const std::function<Scan()> seamScan = m_seam.scan;
#else
    const std::function<Scan()> seamScan;
#endif
    m_worker.start(QRunnable::create([this, generation, seamScan]() {
        const Scan scan = seamScan ? seamScan() : runScan();
        QMetaObject::invokeMethod(this, [this, generation, scan]() {
            m_scanBusy = false;
            if (generation == m_generation) scanDone(scan);
            if (m_rescan) { m_rescan = false; requestScan(); }
        }, Qt::QueuedConnection);
    }));
}

QList<FirmwareUpdater::Drive> FirmwareUpdater::parseDrives(const QByteArray &reply) {
    QList<Drive> drives;
    const QJsonArray list = QJsonDocument::fromJson(reply).object().value("value").toArray();
    for (const QJsonValue &v : list) {
        const QJsonObject o = v.toObject();
        Drive d;
        d.path = o.value("path").toString();
        d.platform = o.value("platform").toInt(-1);
        if (!d.path.isEmpty()) drives.append(d);
    }
    return drives;
}

// Worker thread: listing the bus can take a while, and a board that rebooted
// mid-write can leave a dead mount behind, where looking at a file can block.
FirmwareUpdater::Scan FirmwareUpdater::runScan() const {
    Scan scan;
    uint32_t rp2040 = 0, rp2350 = 0;
    dspi_bootloader_counts(&rp2040, &rp2350);
    scan.boards = int(rp2040 + rp2350);
    if (scan.boards == 1) scan.busPlatform = rp2350 ? kRp2350 : kRp2040;
    scan.drives = parseDrives(replyOf([](char *buf, uint32_t len) { return dspi_bootloader_drives(buf, len); }));
    if (scan.boards == 1 && scan.drives.isEmpty()) {
        const QString byLabel = QString("/dev/disk/by-label/") + (rp2350 ? "RP2350" : "RPI-RP2");
        if (QFileInfo::exists(byLabel)) scan.device = QFileInfo(byLabel).canonicalFilePath();
    }
    return scan;
}

// What is attached, as a state, and for an armed install what comes next
void FirmwareUpdater::scanDone(const Scan &scan) {
    const bool awaiting = m_state == QLatin1String("waitingForDrive");
    if (m_installing && !awaiting) return;
    if (!m_bridge->connected()) m_sawGone = true;

    const int boards = qMax(scan.boards, int(scan.drives.size()));
    const bool parked = m_state == QLatin1String("inBootloader");
    if (parked && boards > 0) m_parkedSeen = true;
    // Nothing happens to a parked board until Install is pressed again
    if (boards > 1 && !(parked && !m_armed)) {
        m_volumeWait.invalidate();
        fail(awaiting
             ? tr("%1 boards are in bootloader mode, and nothing says which is this one. Nothing was written. Disconnect all but the one you want to update.").arg(boards)
             : tr("%1 boards are in bootloader mode. Disconnect all but the one you want to update.").arg(boards), true);
        return;
    }

    const int chip = !scan.drives.isEmpty() ? scan.drives.first().platform : scan.busPlatform;
    if (chip != m_chip && !(awaiting && chip < 0)) {
        m_chip = chip;
        m_mountTried.clear();
        emit stateChanged();
        emit imageChanged();
    }

    if (awaiting) {
        if (!scan.drives.isEmpty()) {
            const Drive drive = scan.drives.first();
            if (drive.platform != m_image.platform) {
                fail(tr("The board that appeared is an %1, and %2 is firmware for the %3. Nothing was written.")
                         .arg(chipOf(drive.platform), m_image.name, chipOf(m_image.platform)), false);
                return;
            }
            startWrite(drive);
            return;
        }
        askUdisks(scan);
        int limit = kDriveWaitMs;
#ifdef DSPI_SMOKE
        if (m_seam.driveWaitMs) limit = m_seam.driveWaitMs;
#endif
        if (m_driveClock.elapsed() >= limit)
            fail(tr("No bootloader drive appeared. Nothing was written. Look for it in your file manager, or unplug the board and hold BOOTSEL while plugging it back in."), true, true);
        return;
    }

    if (parked) {
        if (boards > 0) {
            if (m_armed && !scan.drives.isEmpty()) {
                m_drive = scan.drives.first();
                beginMounted(m_drive);
            } else {
                askUdisks(scan);
            }
            return;
        }
        int limit = kDriveWaitMs;
#ifdef DSPI_SMOKE
        if (m_seam.driveWaitMs) limit = m_seam.driveWaitMs;
#endif
        // Unplugged, or it never arrived: back to looking
        if (!m_parkedSeen && m_driveClock.elapsed() < limit) return;
        m_failure.clear();
        emit stateChanged();
    }

    if (!scan.drives.isEmpty()) {
        m_drive = scan.drives.first();
        m_volumeWait.invalidate();
        publish("ready");
        if (m_armed) beginMounted(m_drive);
        return;
    }

    if (boards == 0) {
        m_volumeWait.invalidate();
        publish("waitingForBoard");
        if (m_armedForDevice) beginDevice();
        return;
    }

    // A board on the bus with no drive yet. That is the board the next click is
    // about: the connected device is not rebooted past it.
    m_armedForDevice = false;
    askUdisks(scan);
    if (!m_volumeWait.isValid()) m_volumeWait.start();
    if (m_volumeWait.elapsed() >= kVolumeWaitMs) {
        fail(tr("The board is connected but its %1 drive could not be mounted. Mount it from your file manager and try again.").arg(volumeName()), true);
        return;
    }
    publish("waitingForVolume");
}

// The drive appears a moment after the board; most Linux desktops don't mount it
// on their own, so ask udisks once it shows up
void FirmwareUpdater::askUdisks(const Scan &scan) {
    if (scan.device.isEmpty() || m_mountTried == scan.device || !writeBlocked().isEmpty()) return;
    m_mountTried = scan.device;
    auto *p = new QProcess(this);
    connect(p, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished), this, [this, p](int, QProcess::ExitStatus) {
        p->deleteLater();
        if (m_scanTimer.isActive()) requestScan();
    });
    connect(p, &QProcess::errorOccurred, p, [p](QProcess::ProcessError e) { if (e == QProcess::FailedToStart) p->deleteLater(); });
    p->start("udisksctl", { "mount", "--no-user-interaction", "-b", scan.device });
}

void FirmwareUpdater::installWhenReady() {
    if (m_installing) return;
    m_armed = true;
    // Only a device connected at this click is what the click was about. One that
    // turns up later is not rebooted on the strength of it.
    // Nor is a device while a board that was sent to its bootloader is on its way.
    m_armedForDevice = connectedNow() && m_state != QLatin1String("inBootloader");
    // A fresh look: a drive mounted a moment ago decides which board is being asked
    if (!m_scanTimer.isActive()) m_scanTimer.start();
    requestScan();
}

void FirmwareUpdater::reset() {
    if (m_state == QLatin1String("writing") || m_state == QLatin1String("waitingForDevice")) return;
    if (m_state == QLatin1String("checking")) { m_cancelPending = true; return; }
    if (m_state == QLatin1String("inBootloader")) { if (m_watching) requestScan(); return; }
    m_generation++;
    m_installing = false;
    m_armed = false;
    m_armedForDevice = false;
    m_path = Path::None;
    m_image = Image();
    m_typed.clear();
    m_questions = 0;
    m_questionMessage.clear();
    m_questionPhrase.clear();
    m_answerWrong = false;
    emit questionChanged();
    m_volumeWait.invalidate();
    m_mountTried.clear();
    m_failure.clear();
    m_noBoard = false;
    m_progress = 0;
    emit progressChanged();
    m_state = "idle";
    emit stateChanged();
    if (m_watching) { m_scanTimer.setInterval(kScanMs); requestScan(); }
}

void FirmwareUpdater::enterBootloader() {
    if (m_armed || m_installing || !canReboot()) return;
    FfiCore *core = m_bridge->core();
#ifdef DSPI_SMOKE
    m_bootloaderRequests++;
#endif
    // The device drops off the bus answering this
    m_worker.start(QRunnable::create([core]() { dspi_enter_bootloader(core); }));
}

int FirmwareUpdater::compareVersions(const QString &a, const QString &b) const {
    const FwVersion x = FwVersion::parse(a), y = FwVersion::parse(b);
    return x == y ? 0 : x < y ? -1 : 1;
}

// ── The file ──

// Worker thread. Bounded, so a wrong file can't take the memory.
FirmwareUpdater::Image FirmwareUpdater::readImage(const QString &path, const QString &name) {
    Image image;
    image.name = plain(name);
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        image.problem = tr("Could not read %1: %2.").arg(image.name, f.errorString());
        return image;
    }
    image.bytes = f.read(kMaxFile + 1);
    if (f.error() != QFileDevice::NoError) {
        image.problem = tr("Could not read %1: %2.").arg(image.name, f.errorString());
        image.bytes.clear();
        return image;
    }
    const QByteArray &bytes = image.bytes;
    const QByteArray reply = replyOf([&bytes](char *buf, uint32_t len) {
        return dspi_uf2_inspect(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()), buf, len);
    });
    const QJsonObject info = QJsonDocument::fromJson(reply).object();
    if (!info.value("valid").toBool()) {
        image.problem = tr("%1 is not a firmware file: %2").arg(image.name, info.value("problem").toString(tr("it could not be checked.")));
        image.bytes.clear();
        return image;
    }
    image.platform = info.value("platform").toInt(-1);
    image.version = info.value("firmwareVersion").toString();
    image.loaded = true;
    return image;
}

void FirmwareUpdater::chooseImage(const QUrl &file) {
    if (m_installing) return;
    const QString path = file.isLocalFile() ? file.toLocalFile() : file.toString();
    const QString name = QFileInfo(path).fileName();
    m_imageLoading = true;
    m_custom = Image();
    emit imageChanged();
    m_worker.start(QRunnable::create([this, path, name]() {
        const Image image = readImage(path, name);
        QMetaObject::invokeMethod(this, [this, image]() {
            m_imageLoading = false;
            m_custom = image;
            emit imageChanged();
        }, Qt::QueuedConnection);
    }));
}

void FirmwareUpdater::useBundledImage() {
    if (m_installing) return;
    m_custom = Image();
    emit imageChanged();
}

// ── The install ──

// A running board: read it, ask it, reboot it
void FirmwareUpdater::beginDevice() {
    m_armed = false;
    m_armedForDevice = false;
    m_installing = true;
    m_path = Path::Device;
    if (!connectedNow()) {
        fail(tr("The board is gone. Nothing was changed."), true, true);
        return;
    }
    const QString blocked = rebootBlocked();
    if (!blocked.isEmpty()) {
        fail(tr("%1 Nothing was changed.").arg(blocked), true);
        return;
    }
    m_boardPlatform = dspi_get_state(m_bridge->core())->platform_id;
    m_chip = m_boardPlatform;
    emit stateChanged();
    m_backSerial = m_bridge->selectedSerial();
    m_questions = 0;
    m_image = Image();
    runGate(QString());
}

// A drive nobody can ask about
void FirmwareUpdater::beginMounted(const Drive &drive) {
    m_armed = false;
    m_armedForDevice = false;
    m_installing = true;
    m_path = Path::Mounted;
    m_drive = drive;
    m_boardPlatform = drive.platform;
    m_questions = 0;
    m_image = Image();
    if (!writeBlocked().isEmpty()) {
        fail(tr("%1 Nothing was changed.").arg(writeBlocked()), true);
        return;
    }
    if (drive.platform != kRp2040 && drive.platform != kRp2350) {
        fail(tr("%1 is not the drive of an RP2040 or RP2350 bootloader. Nothing was changed.").arg(drive.path), false);
        return;
    }
    runGate(QString());
}

// The file, the chip, then the guard. On a running board the guard reads the board
// now, and if it allows the image the reboot follows in this same job.
void FirmwareUpdater::runGate(const QString &typed) {
    publish("checking");
    const unsigned generation = m_generation;
    const bool device = m_path == Path::Device;
    if (device) m_sawGone = false;
    const int platform = m_boardPlatform;
    const Image held = m_image;
    const Image custom = m_custom;
    const QString expected = m_expectedRaw;
    const QByteArray answerBytes = typed.toUtf8();
    const QString drivePath = m_drive.path;
    FfiCore *core = m_bridge->core();
#ifdef DSPI_SMOKE
    int *reads = &m_boardReads;
    int *enters = &m_bootloaderRequests;
    const std::function<Scan()> seamScan = m_seam.scan;
    const std::function<void()> gateHook = m_seam.gateHook;
#else
    const std::function<Scan()> seamScan;
    const std::function<void()> gateHook;
#endif
    m_worker.start(QRunnable::create([=]() {
        GateResult r;
        Image image = held;
        if (!image.loaded) {
            if (custom.loaded || !custom.problem.isEmpty()) {
                image = custom;
                r.failure = custom.problem;
            } else {
                const QString path = imagePath(platform);
                if (path.isEmpty()) {
                    r.failure = tr("This build of DSPi Console does not include firmware for the %1. Nothing was changed.").arg(chipOf(platform));
                } else {
                    image = readImage(path, QFileInfo(path).fileName());
                    if (!image.loaded) r.failure = image.problem;
                    else if (!(FwVersion::parse(image.version) == FwVersion::parse(expected)))
                        r.failure = tr("The bundled firmware is version %1 but this Console expects %2. The app was built incorrectly; do not install it.").arg(image.version, expected);
                }
            }
        }
        if (r.failure.isEmpty() && image.platform != platform) {
            r.failure = tr("%1 is firmware for the %2, and this board is an %3. Nothing was changed.")
                            .arg(image.name, chipOf(image.platform), chipOf(platform));
        }
        // What is on the bus now, not what was there when a question was first asked
        if (r.failure.isEmpty()) {
            const Scan scan = seamScan ? seamScan() : runScan();
            const int boards = qMax(scan.boards, int(scan.drives.size()));
            r.failureMundane = true;
            if (device && boards > 0) {
                r.failure = tr("A board has appeared in bootloader mode, and with this one there would be two. Nothing was changed. Disconnect it and try again.");
            } else if (!device && boards > 1) {
                r.failure = tr("%1 boards are in bootloader mode. Nothing was changed. Disconnect all but the one you want to update.").arg(boards);
            } else if (!device && std::none_of(scan.drives.cbegin(), scan.drives.cend(), [&drivePath](const Drive &d) { return d.path == drivePath; })) {
                r.failure = tr("The drive at %1 has gone. Nothing was changed.").arg(drivePath);
            } else {
                r.failureMundane = false;
            }
        }
        if (r.failure.isEmpty()) {
            r.image = image;
            // 0 is unknown, the record of a board nobody can ask
            // (firmware_guard_board_in_bootloader)
            if (device) {
#ifdef DSPI_SMOKE
                ++*reads;
#endif
                const QJsonObject board = QJsonDocument::fromJson(replyOf([core](char *buf, uint32_t len) {
                    return dspi_firmware_guard_board(core, buf, len);
                })).object();
                r.requiredMv = board.value("requiredMv").toInt(0);
                r.boardGone = board.value("source").toString() == QLatin1String("noDevice");
            }
            if (!r.boardGone) {
                const QByteArray &bytes = image.bytes;
                const int mv = r.requiredMv;
                const QJsonObject verdict = QJsonDocument::fromJson(replyOf([&bytes, mv, &answerBytes](char *buf, uint32_t len) {
                    return dspi_firmware_guard_check(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()),
                                                     uint16_t(mv), answerBytes.constData(), buf, len);
                })).object();
                // A failure of the call is no permission
                r.allowed = verdict.value("allowed").toBool(false);
                r.decision = verdict.value("decision").toString();
                r.message = verdict.value("message").toString();
                r.phrase = verdict.value("confirmationPhrase").toString();
                if (r.allowed && device) {
#ifdef DSPI_SMOKE
                    ++*enters;
#endif
                    // No answer to wait for, and a failure looks like success: only
                    // the drive arriving says, and without one nothing gets written
                    dspi_enter_bootloader(core);
                    r.entered = true;
                }
            }
        }
        if (gateHook) gateHook();
        QMetaObject::invokeMethod(this, [this, generation, r, typed]() {
            if (generation == m_generation) gateDone(r, typed);
        }, Qt::QueuedConnection);
    }));
}

void FirmwareUpdater::gateDone(const GateResult &r, const QString &typed) {
    const bool cancelled = m_cancelPending;
    m_cancelPending = false;
    if (cancelled) {
        // In time: as if nothing was asked
        if (!r.entered) {
            m_state = "idle";
            reset();
            if (!m_watching) m_scanTimer.stop();
            return;
        }
        // The reboot can't be taken back, and a Cancel never ends in a write
        m_installing = false;
        m_armed = false;
        m_armedForDevice = false;
        m_path = Path::None;
        m_image = Image();
        m_typed.clear();
        m_questionMessage.clear();
        m_questionPhrase.clear();
        m_answerWrong = false;
        emit questionChanged();
        m_failure = tr("The board is in its bootloader and nothing was written. Install again, or unplug it to go back to the old firmware.");
        m_failureMundane = true;
        m_parkedSeen = false;
        m_driveClock.start();
        m_scanTimer.setInterval(kScanMs);
        m_scanTimer.start();
        m_state = "inBootloader";
        emit stateChanged();
        return;
    }
    if (!r.failure.isEmpty()) {
        fail(r.failure, r.failureMundane);
        return;
    }
    if (r.boardGone) {
        fail(tr("The board is gone. Nothing was changed."), true, true);
        return;
    }
    m_image = r.image;
    m_requiredMv = r.requiredMv;
    if (!r.allowed) {
        if (r.decision != QLatin1String("needsConfirmation") || r.phrase.isEmpty()) {
            fail(tr("%1 Nothing was changed.").arg(r.message.isEmpty() ? tr("The firmware guard refused this image.") : r.message), false);
            return;
        }
        const QString message = plain(r.message);
        const bool sameQuestion = r.phrase == m_questionPhrase && message == m_questionMessage;
        // A new or changed risk takes its own answer
        if (!sameQuestion && ++m_questions > kMaxQuestions) {
            fail(tr("This board keeps changing. Nothing was changed."), false);
            return;
        }
        m_typed.clear();
        m_questionMessage = message;
        m_questionPhrase = r.phrase;
        m_answerWrong = sameQuestion;
        emit questionChanged();
        publish("question");
        return;
    }
    m_typed = typed.toUtf8();
    m_questionMessage.clear();
    m_questionPhrase.clear();
    m_answerWrong = false;
    emit questionChanged();
    if (m_path == Path::Mounted) {
        startWrite(m_drive);
        return;
    }
    // Sent. The drive decides from here.
    publish("waitingForDrive");
    m_driveClock.start();
    m_mountTried.clear();
    m_scanTimer.setInterval(kScanWaitingMs);
    if (!m_scanTimer.isActive()) m_scanTimer.start();
}

void FirmwareUpdater::answer(const QString &typed) {
    if (m_state != QLatin1String("question")) return;
    // Read the board again: the answer is judged against it as it is now
    runGate(typed);
}

// Through the guard, with the board record the answers were given against
void FirmwareUpdater::startWrite(const Drive &drive) {
    m_scanTimer.stop();
    m_scanTimer.setInterval(kScanMs);
    m_drive = drive;
    // The demo may have been opened since the last check
    if (!writeBlocked().isEmpty()) {
        fail(tr("%1 Nothing was written.").arg(writeBlocked()), true);
        return;
    }
    if (m_path == Path::Mounted) m_before = serialsNow();
    m_progress = -1;
    emit progressChanged();
    publish("writing");
    const unsigned generation = m_generation;
    const QByteArray bytes = m_image.bytes;
    const QByteArray path = drive.path.toUtf8();
    const int platform = drive.platform;
    const int mv = m_requiredMv;
    const QByteArray typed = m_typed;
    m_worker.start(QRunnable::create([this, generation, bytes, path, platform, mv, typed]() {
        const QByteArray reply = replyOf([&](char *buf, uint32_t len) {
            return dspi_flash_to_drive_guarded(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()),
                                               path.constData(), int8_t(platform), uint16_t(mv), typed.constData(), buf, len);
        });
        QMetaObject::invokeMethod(this, [this, generation, reply]() {
            if (generation == m_generation) writeDone(reply);
        }, Qt::QueuedConnection);
    }));
}

void FirmwareUpdater::writeDone(const QByteArray &reply) {
    const QJsonObject o = QJsonDocument::fromJson(reply).object();
    if (!o.value("ok").toBool(false)) {
        fail(tr("%1. The board stays in its bootloader at %2; Try Again asks about core voltage, as a board there cannot say what it needs.")
                 .arg(o.value("error").toString(tr("The firmware could not be written")), m_drive.path), false);
        return;
    }
    m_progress = 1;
    emit progressChanged();
    // The copy returning proves nothing: success is the device coming back and
    // reporting what was written
    publish("waitingForDevice");
    m_verifyClock.start();
    m_selectClock.invalidate();
    m_verifyTimer.start();
}

QStringList FirmwareUpdater::serialsNow() const {
#ifdef DSPI_SMOKE
    if (m_seam.serials) return m_seam.serials();
#endif
    return m_bridge->availableSerials();
}

// Open `serial` if the app isn't on it already. True when it is.
bool FirmwareUpdater::bringBack(const QString &serial) {
    if (m_bridge->connected() && m_bridge->selectedSerial() == serial) return true;
    if (m_bridge->hubSession() || !serialsNow().contains(serial)) return false;
    if (m_selectClock.isValid() && m_selectClock.elapsed() < kSelectRetryMs) return false;
    m_selectClock.start();
    m_bridge->selectDevice(serial);
    return m_bridge->connected() && m_bridge->selectedSerial() == serial;
}

void FirmwareUpdater::verifyTick() {
    if (!m_bridge->connected()) m_sawGone = true;

    // A hub session keeps the app off USB devices, so the board can't be opened
    if (m_bridge->hubSession()) {
        m_verifyTimer.stop();
        m_failure = tr("The firmware was written to the drive, but a hub session is open, so the board can't be opened here to confirm it. Leave the hub session and check the version it reports.");
        m_failureMundane = false;
        m_state = "unconfirmed";
        emit stateChanged();
        return;
    }

    QString error;
    bool judge = false;
    if (m_path == Path::Device) {
        // What it reported before it left says nothing about what was written
        judge = m_sawGone && bringBack(m_backSerial);
    } else {
        // Anything on the bus already is not the board that was written
        QStringList fresh = serialsNow();
        for (const QString &serial : m_before) fresh.removeAll(serial);
        if (fresh.size() > 1) error = tr("%1 new DSPis appeared, so which is this board cannot be told").arg(fresh.size());
        else if (fresh.size() == 1) judge = bringBack(fresh.first());
    }

    if (judge) {
        const FwVersion now = deviceFw(m_bridge);
        if (m_image.version.isEmpty()) {
            m_verifyTimer.stop();
            m_failure = tr("The board is back, running %1. This image names no version, so whether it took cannot be confirmed.").arg(now.text());
            m_failureMundane = false;
            m_state = "unconfirmed";
            emit stateChanged();
            return;
        }
        const QByteArray wanted = m_image.version.toUtf8();
        const int same = dspi_firmware_version_matches(m_bridge->core(), wanted.constData());
        if (same == 1) {
            m_verifyTimer.stop();
            m_verified = now;
            publish("verified");
            return;
        }
        if (same == 0) {
            m_verifyTimer.stop();
            fail(tr("The board came back running firmware %1, but the image was %2: it did not take.").arg(now.text(), m_image.version), false);
            return;
        }
        // -1: it went away again, so keep waiting
    }

    int limit = kVerifyTimeoutMs;
#ifdef DSPI_SMOKE
    if (m_seam.verifyWaitMs) limit = m_seam.verifyWaitMs;
#endif
    if (!error.isEmpty() || m_verifyClock.elapsed() > limit) {
        m_verifyTimer.stop();
        fail(tr("The write ended, but %1, so this is not confirmed. Unplug the board, plug it back in, and check whether it works. If the drive is still there the image did not take.")
                 .arg(error.isEmpty() ? tr("the board did not come back") : error), false);
    }
}
