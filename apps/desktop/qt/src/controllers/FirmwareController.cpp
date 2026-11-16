// -- WP QP16 -- The firmware window, its installer, the bundled images and the
// version-mismatch banner (FirmwareController.h)
#include "FirmwareController.h"
#include "Confirm.h"   // -- WP P1QT2 --

#include "AppIdentity.h"
#include "Core.h"
#include "DeviceModel.h"
#include "FilesController.h"
#include "Shell.h"
#include "Snapshot.h"
#if __has_include("OnboardingController.h")
#include "OnboardingController.h"
#define DSPI_QP16_ONBOARDING 1
#endif

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJSEngine>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <array>
#include <memory>

static const Registrar::Controller<FirmwareController> firmwareController;
static const Registrar::Window firmwareWindow("device.firmwareUpdate", "FirmwareWindow");
static const Registrar::Banner firmwareBanner("FirmwareBanner", 10);

namespace {
// The Mac's FirmwareInstaller.verifyTimeout: the reboot, the new enumeration
// and the app's own reconnect, generous on purpose
constexpr int kVerifyMs = 30000;
// The largest file taken as a firmware image: the core refuses anything past
// the RP2350's whole flash window anyway
constexpr qint64 kMaxImageBytes = 40 * 1024 * 1024;

// "1.1.6" or "1.1.6-beta3" at the start of a version string
QString leadingVersion(const QString &text) {
    static const QRegularExpression re(QStringLiteral("^(\\d+\\.\\d+\\.\\d+(?:-beta\\d+)?)"));
    const QRegularExpressionMatch m = re.match(text);
    return m.hasMatch() ? m.captured(1) : QString();
}

// The core's check of an image. Worker thread.
QJsonObject inspect(const QByteArray &bytes) {
    return QJsonDocument::fromJson(readSized([&bytes](char *b, uint32_t n) {
               return dspi_uf2_inspect(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()), b, n);
           })).object();
}

// The guard's verdict on `bytes` for a board needing `requiredMv`. Worker thread.
QJsonObject guardCheck(const QByteArray &bytes, int requiredMv, const QByteArray &typed) {
    return QJsonDocument::fromJson(readSized([&](char *b, uint32_t n) {
               return dspi_firmware_guard_check(reinterpret_cast<const uint8_t *>(bytes.constData()), uint32_t(bytes.size()),
                                                uint16_t(requiredMv), typed.isEmpty() ? nullptr : typed.constData(), b, n);
           })).object();
}

// -- WP SECFIX -- The core's manifest on `bytes` named `name` ("" for none):
// verified, notBundled or mismatch. Worker thread.
QString verifyBundled(const QByteArray &bytes, const QString &name) {
    const QByteArray n = name.toUtf8();
    return QJsonDocument::fromJson(readSized([&](char *b, uint32_t len) {
               return dspi_firmware_verify_bundled(reinterpret_cast<const uint8_t *>(bytes.constData()),
                                                   uint32_t(bytes.size()), n.constData(), b, len);
           }))
        .object()
        .value(QStringLiteral("value"))
        .toString();
}

// A sentence of the core's, as a line on its own
QString sentence(QString text) {
    if (!text.isEmpty()) text[0] = text.at(0).toUpper();
    return text;
}
} // namespace

FirmwareController::FirmwareController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_locator(new BootloaderLocator(ctx.core, this)) {
    // -- WP SECFIX -- the label's words: the core's row (a pure function of the build)
    {
        const QJsonObject row = QJsonDocument::fromJson(readSized([](char *b, uint32_t n) {
                                    return dspi_notice_for("firmware.not_verified", nullptr, b, n);
                                })).object();
        const bool ours = row.value(QStringLiteral("key")).toString() == QLatin1String("firmware.not_verified");
        m_unverifiedLabel = ours ? row.value(QStringLiteral("title")).toString() : tr("Not a verified DSPi Console image");
        m_unverifiedText = ours ? row.value(QStringLiteral("reason")).toString() + QLatin1Char(' ')
                                      + row.value(QStringLiteral("action")).toString()
                                : QString();
    }
    loadBundled();
    connect(m_locator, &BootloaderLocator::scanned, this, &FirmwareController::onScan);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        emit deviceChanged();
        emit stateChanged();   // the cards depend on whether a device is connected
        checkReturned();
    });
    // -- QP17's wizard installs through this controller (OnboardingController.h's
    // installer contract), handed over once every controller exists
#if DSPI_QP16_ONBOARDING
    QTimer::singleShot(0, this, [this] {
        if (auto *onboarding = Registrar::instance<OnboardingController>()) onboarding->setInstaller(this);
    });
#endif
    m_verifyDeadline.setSingleShot(true);
    m_verifyDeadline.setInterval(kVerifyMs);
    connect(&m_verifyDeadline, &QTimer::timeout, this, [this] {
        if (m_state == WaitingForDevice) fail(DeviceDidNotReturn);
    });
}

FirmwareController *FirmwareController::create(QQmlEngine *, QJSEngine *) {
    FirmwareController *self = Registrar::instance<FirmwareController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QString FirmwareController::versionFromName(const QString &fileName) {
    const int at = fileName.lastIndexOf(QLatin1String("-v"));
    if (at < 0) return QString();
    QString tail = fileName.mid(at + 2);
    if (tail.endsWith(QLatin1String(".uf2"), Qt::CaseInsensitive)) tail.chop(4);
    return leadingVersion(tail);
}

// ---- The bundled images ------------------------------------------------------

void FirmwareController::loadBundled() {
    const QDir dir(QStringLiteral(":/firmware"));
    for (const QString &name : dir.entryList({ QStringLiteral("*.uf2") }, QDir::Files, QDir::Name)) {
        QFile f(dir.filePath(name));
        if (!f.open(QIODevice::ReadOnly)) continue;
        Image img;
        img.name = name;
        img.bytes = f.readAll();
        img.chip = name.contains(QLatin1String("-RP2350-")) ? BootloaderLocator::RP2350
                   : name.contains(QLatin1String("-RP2040-")) ? BootloaderLocator::RP2040
                                                             : -1;
        if (img.chip < 0) continue;
        m_bundled.append(img);
        if (m_bundledVersion.isEmpty()) m_bundledVersion = versionFromName(name);
    }
    m_expected = m_bundledVersion;
    // The core's check of each, once
    QList<QByteArray> images;
    for (const Image &img : std::as_const(m_bundled)) images.append(img.bytes);
    m_ctx.core->worker()->call(
        this,
        [images](FfiCore *) {
            QList<QJsonObject> out;
            for (const QByteArray &b : images) out.append(inspect(b));
            return out;
        },
        [this](QList<QJsonObject> checked) {
            for (int i = 0; i < checked.size() && i < m_bundled.size(); ++i) {
                m_bundled[i].valid = checked.at(i).value(QStringLiteral("valid")).toBool();
                m_bundled[i].version = checked.at(i).value(QStringLiteral("firmwareVersion")).toString();
                m_bundled[i].problem = checked.at(i).value(QStringLiteral("problem")).toString();
            }
            emit imageChanged();
        });
}

FirmwareController::Image FirmwareController::imageFor(int chip) const {
    if (custom()) return m_customImage;
    for (const Image &img : m_bundled)
        if (img.chip == chip) return img;
    return Image();
}

int FirmwareController::deviceChip() const {
    const QString p = m_ctx.device->connected() ? m_ctx.device->platformName() : QString();
    if (p == QLatin1String("RP2350")) return BootloaderLocator::RP2350;
    if (p == QLatin1String("RP2040")) return BootloaderLocator::RP2040;
    return -1;
}

QString FirmwareController::imageName() const {
    if (custom()) return QFileInfo(m_customPath).fileName();
    const int chip = deviceChip() >= 0 ? deviceChip() : m_lastChip;
    QStringList names;
    for (const Image &img : m_bundled)
        if (chip < 0 || img.chip == chip) names.append(img.name);
    return names.join(QLatin1String(" · "));
}

QString FirmwareController::imageVersion() const {
    if (custom()) return m_customImage.version.isEmpty() ? tr("unknown") : leadingVersion(m_customImage.version);
    return m_expected.isEmpty() ? tr("unknown") : m_expected;
}

bool FirmwareController::imageChecked() const {
    if (custom()) return m_customImage.valid && m_customProblem.isEmpty();
    const int chip = deviceChip() >= 0 ? deviceChip() : m_lastChip;
    bool any = false;
    for (const Image &img : m_bundled) {
        if (chip >= 0 && img.chip != chip) continue;
        const QString v = leadingVersion(img.version);
        if (!img.valid || (!v.isEmpty() && v != m_expected)) return false;
        any = true;
    }
    return any;
}

void FirmwareController::chooseCustomImage() {
    FilesController::pickFile(false, tr("Choose Firmware"), tr("Firmware images (*.uf2)"), QString(), this,
                              [this](const QString &path) { useCustomImage(path); });
}

void FirmwareController::useCustomImage(const QString &path) {
    if (path.isEmpty() || m_confirmed) return;
    const int device = deviceChip();
    m_ctx.core->worker()->call(
        this,
        [path](FfiCore *) {
            QFile f(path);
            QByteArray bytes;
            if (f.open(QIODevice::ReadOnly) && f.size() <= kMaxImageBytes) bytes = f.readAll();
            QJsonObject info = inspect(bytes);
            // -- WP SECFIX -- by its file name: a copy of a bundled image is verified
            info.insert(QStringLiteral("bundled"), verifyBundled(bytes, QFileInfo(path).fileName()));
            // An image the core refuses: the guard's sentence says why
            const QString refusal = info.value(QStringLiteral("valid")).toBool()
                                        ? QString()
                                        : guardCheck(bytes, 0, QByteArray()).value(QStringLiteral("message")).toString();
            return std::make_tuple(bytes, info, refusal);
        },
        [this, path, device](std::tuple<QByteArray, QJsonObject, QString> r) {
            const QJsonObject &info = std::get<1>(r);
            m_customPath = path;
            m_customImage = Image();
            m_customImage.name = QFileInfo(path).fileName();
            m_customImage.bytes = std::get<0>(r);
            m_customImage.valid = info.value(QStringLiteral("valid")).toBool();
            m_customImage.version = info.value(QStringLiteral("firmwareVersion")).toString();
            m_customImage.chip = info.value(QStringLiteral("platform")).toInt(-1);
            m_customImage.bundled = info.value(QStringLiteral("bundled")).toString();   // -- WP SECFIX --
            m_customProblem = sentence(std::get<2>(r));
            if (m_customImage.valid && device >= 0 && m_customImage.chip != device)
                m_customProblem = tr("This image is for the %1; the connected device is an %2.")
                                      .arg(BootloaderLocator::displayName(m_customImage.chip),
                                           BootloaderLocator::displayName(device));
            emit imageChanged();
            emit stateChanged();
        });
}

void FirmwareController::useBundledImage() {
    if (m_confirmed || !custom()) return;
    m_customPath.clear();
    m_customImage = Image();
    m_customProblem.clear();
    emit imageChanged();
    emit stateChanged();
}

// ---- The device's version and the mismatch banner ---------------------------

QString FirmwareController::describe(int major, int minor, int patch, int beta) const {
    const QString base = QStringLiteral("%1.%2.%3").arg(major).arg(minor).arg(patch);
    if (beta == 0) return base;
    if (beta == FW_BETA_EARLY) return tr("%1 early beta").arg(base);
    return tr("%1 beta %2").arg(base).arg(beta);
}

QString FirmwareController::runningVersion() const {
    const DspState *s = m_ctx.device->connected() ? m_ctx.device->state() : nullptr;
    if (!s || (s->fw_major == 0 && s->fw_minor == 0 && s->fw_patch == 0)) return QString();
    return describe(s->fw_major, s->fw_minor, s->fw_patch, s->fw_beta);
}

// The Mac's FirmwareMatch: a final release outranks every beta of its patch,
// and an early beta (one that could not say which) sorts below beta 1
QString FirmwareController::match() const {
    const DspState *s = m_ctx.device->connected() ? m_ctx.device->state() : nullptr;
    const QStringList want = m_expected.section(QLatin1Char('-'), 0, 0).split(QLatin1Char('.'));
    if (!s || want.size() != 3 || (s->fw_major == 0 && s->fw_minor == 0 && s->fw_patch == 0)) return QString();
    const auto rank = [](int beta) { return beta == 0 ? 256 : beta == FW_BETA_EARLY ? -1 : beta; };
    const int wantBeta = m_expected.contains(QLatin1String("-beta")) ? m_expected.section(QLatin1String("-beta"), 1).toInt() : 0;
    const std::array<int, 4> have{ s->fw_major, s->fw_minor, s->fw_patch, rank(s->fw_beta) };
    const std::array<int, 4> expected{ want.at(0).toInt(), want.at(1).toInt(), want.at(2).toInt(), rank(wantBeta) };
    if (have == expected) return QStringLiteral("match");
    return have < expected ? QStringLiteral("older") : QStringLiteral("newer");
}

bool FirmwareController::supported() const {
    return m_ctx.device->platformName() != QLatin1String("STM32H723");
}

bool FirmwareController::mismatchShown() const {
    const QString m = match();
    return !m_mismatchHidden && supported() && m_ctx.device->connected() && !m_ctx.device->readOnly() &&
           (m == QLatin1String("older") || m == QLatin1String("newer"));
}

QString FirmwareController::mismatchText() const {
    const QString device = runningVersion().isEmpty() ? tr("unknown") : runningVersion();
    const QString expected = m_expected.isEmpty() ? tr("unknown") : m_expected;
    if (match() == QLatin1String("newer"))
        return tr("This device runs firmware %1, which is newer than %2 %3. Some of its features may not be shown.")
            .arg(device, AppIdentity::displayName(), expected);
    return tr("This device runs firmware %1; %2 expects %3.").arg(device, AppIdentity::displayName(), expected);
}

void FirmwareController::hideMismatch() {
    if (m_mismatchHidden) return;
    m_mismatchHidden = true;
    emit deviceChanged();
}

void FirmwareController::openWindow() {
    m_ctx.shell->trigger(QStringLiteral("device.firmwareUpdate"));
}

// ---- The installer -----------------------------------------------------------

QString FirmwareController::stage() const {
    switch (m_state) {
    case Idle: return QStringLiteral("idle");
    case WaitingForBoard: return QStringLiteral("waitingForBoard");
    case WaitingForVolume: return QStringLiteral("waitingForVolume");
    case Ready: return QStringLiteral("ready");
    case Writing: return QStringLiteral("writing");
    case WaitingForDevice: return QStringLiteral("waitingForDevice");
    case Verified: return QStringLiteral("verified");
    case Failed: return QStringLiteral("failed");
    }
    return QString();
}

int FirmwareController::step() const {
    switch (m_state) {
    case Writing: return 1;
    case WaitingForDevice: return 2;
    case Verified: return 3;
    default: return 0;
    }
}

void FirmwareController::setState(State s) {
    if (s != Failed) {
        m_error = NoError;
        m_errorDetail.clear();
    }
    m_state = s;
    emit stateChanged();
}

void FirmwareController::fail(Error e, const QString &detail) {
    m_error = e;
    m_errorDetail = detail;
    m_state = Failed;
    m_verifyDeadline.stop();
    emit stateChanged();
}

void FirmwareController::setTimeoutsForTest(int volumeWaitMs, int verifyMs) {
    m_driveWaitMs = volumeWaitMs;
    m_verifyDeadline.setInterval(verifyMs);
}

void FirmwareController::setExpectedVersionForTest(const QString &version) {
    m_expected = version.isEmpty() ? m_bundledVersion : version;
    emit deviceChanged();
    emit imageChanged();
}

void FirmwareController::windowShown(bool shown) {
    if (shown == m_windowShown) return;
    m_windowShown = shown;
    watch(shown);
}

// A run the person started and that has not ended: the read and the question
// before it, the hunt for the board, the write and its check. A verified or
// failed run has ended.
bool FirmwareController::underWay() const {
    if (m_guardAsking || m_reading || m_state == Writing || m_state == WaitingForDevice) return true;
    return m_confirmed && m_state != Verified && m_state != Failed;
}

// The window and the Getting Started wizard each watch while they show. The
// first to start watching starts a fresh run (unless one is under way, which
// it then shows); detection stops when the last one stops, unless a write is
// still to be checked.
void FirmwareController::watch(bool on) {
    m_watchers = qMax(0, m_watchers + (on ? 1 : -1));
    if (m_watchers == 0) {
        if (m_state != Writing && m_state != WaitingForDevice) m_locator->stop();
        return;
    }
    if (on && m_watchers == 1 && !underWay()) {
        m_afterGuard = nullptr;
        resetRun();
    }
    m_locator->start();
}

void FirmwareController::askGuard(const QJsonObject &verdict, std::function<void()> after) {
    m_guardMessage = verdict.value(QStringLiteral("message")).toString();
    m_guardPhrase = verdict.value(QStringLiteral("confirmationPhrase")).toString();
    m_afterGuard = std::move(after);
    m_guardAsking = true;
    emit guardChanged();
    emit stateChanged();
    // The typed confirmation is the window's: a run the wizard started shows it there
    if (!m_windowShown) openWindow();
}

void FirmwareController::resetRun() {
    if (m_state == Writing || m_state == WaitingForDevice) return;
    m_installing = false;
    m_armed = false;
    m_confirmed = false;
    m_rebootRequested = false;
    m_rebootWanted = false;
    m_boardRead = false;
    m_typed.clear();
    m_unconfirmed = false;
    m_driveWait.invalidate();
    m_verifyDeadline.stop();
    setState(Idle);
    onScan(m_locator->boards());
}

// What is attached, as a state; frozen once an install began (the board
// vanishing mid-write is the expected reboot, and an outcome stays until reset)
void FirmwareController::onScan(const QList<BootloaderLocator::Board> &boards) {
    if (m_installing) return;
    if (boards.isEmpty()) {
        m_driveWait.invalidate();
        setState(WaitingForBoard);
    } else if (boards.size() == 1) {
        m_board = boards.first();
        m_lastChip = m_board.chip;
        if (!m_board.drive.isEmpty()) {
            m_driveWait.invalidate();
            setState(Ready);
        } else {
            if (!m_driveWait.isValid()) m_driveWait.start();
            if (m_driveWait.elapsed() >= m_driveWaitMs) fail(VolumeNotMounted, BootloaderLocator::volumeName(m_board.chip));
            else setState(WaitingForVolume);
        }
    } else {
        m_driveWait.invalidate();
        fail(MultipleBoards, QString::number(boards.size()));
    }
    // Committed before the board arrived: write it now that it is here
    if (m_armed && m_state == Ready) install(m_board);
}

void FirmwareController::start() { begin(true); }

// The person's decision: install on the next ready board; with `reboot`, also
// restart the connected device into its bootloader
void FirmwareController::begin(bool reboot) {
    // An ended run is reset first (Try Again, Update Another Board)
    if (underWay() || m_installing || m_confirmed || m_watchers == 0) return;
    if (custom() && !m_customProblem.isEmpty()) return;
    const auto proceed = [this, reboot] {
        m_confirmed = true;
        m_armed = true;
        emit stateChanged();
        if (m_state == Ready) install(m_board);
        if (reboot || m_rebootWanted) enterBootloaderForInstall();
    };
    if (!m_ctx.device->connected()) {
        // A board already in its bootloader cannot be asked: the guard
        // judges it unknown, at the write
        proceed();
        return;
    }
    const Image image = imageFor(deviceChip());
    if (image.bytes.isEmpty()) {
        m_installing = true;   // -- WP QPI3 -- the outcome stays until reset
        fail(ImageMissing, m_ctx.device->platformName());
        return;
    }
    // The board's needs, read now: once it is in the bootloader it cannot be asked
    const QByteArray bytes = image.bytes;
    m_reading = true;
    emit stateChanged();
    m_ctx.core->worker()->call(
        this,
        [bytes](FfiCore *c) {
            const QJsonObject board = QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) {
                                                                  return dspi_firmware_guard_board(c, b, n);
                                                              })).object();
            const int required = board.value(QStringLiteral("requiredMv")).toInt();
            return std::make_pair(required, guardCheck(bytes, required, QByteArray()));
        },
        [this, proceed](std::pair<int, QJsonObject> r) {
            m_reading = false;
            m_requiredMv = r.first;
            m_boardRead = true;
            const QJsonObject &verdict = r.second;
            const QString decision = verdict.value(QStringLiteral("decision")).toString();
            if (verdict.value(QStringLiteral("allowed")).toBool()) {
                proceed();
            } else if (decision == QLatin1String("needsConfirmation")) {
                askGuard(verdict, proceed);
            } else {
                m_installing = true;
                fail(GuardRefused, verdict.value(QStringLiteral("message")).toString());
            }
        });
}

void FirmwareController::answerGuard(const QString &typed) {
    if (!m_guardAsking || typed != m_guardPhrase) return;
    m_typed = typed;
    m_guardAsking = false;
    emit guardChanged();
    emit stateChanged();
    std::function<void()> next = std::move(m_afterGuard);
    m_afterGuard = nullptr;
    if (next) next();
}

void FirmwareController::cancelGuard() {
    if (!m_guardAsking) return;
    m_guardAsking = false;
    m_afterGuard = nullptr;
    if (!m_installing) m_rebootWanted = false;
    emit guardChanged();
    emit stateChanged();
    // Asked at the write: nothing was written, back to detection
    if (m_installing && m_state == Ready) resetRun();
}

void FirmwareController::enterBootloaderForInstall() {
    if (!m_ctx.device->connected() || m_rebootRequested) return;
    m_rebootRequested = true;
    m_leftGeneration = m_ctx.device->snapshot().generation;
    m_ctx.device->writeEvenReadOnly(QStringLiteral("enter bootloader"), [](FfiCore *c) { return Confirm::enterBootloader(c); },   // -- WP P1QT2 -- token
                                    [this](bool sent) {
                                        if (sent || m_installing) return;
                                        m_armed = false;
                                        m_installing = true;
                                        fail(DeviceLocked);
                                    });
}

void FirmwareController::enterBootloaderOnly() {
    if (!m_ctx.device->connected()) return;
    m_ctx.device->writeEvenReadOnly(QStringLiteral("enter bootloader"), [](FfiCore *c) { return Confirm::enterBootloader(c); });   // -- WP P1QT2 -- token
}

void FirmwareController::exportConfiguration() {
    m_ctx.shell->trigger(QStringLiteral("file.exportConfiguration"));
}

void FirmwareController::install(const BootloaderLocator::Board &board) {
    if (m_installing) return;
    // The commitment is spent here, success or not: a later board is never
    // written on a decision made about an earlier one
    m_armed = false;
    m_installing = true;
    if (board.drive.isEmpty()) {
        fail(VolumeNotMounted, BootloaderLocator::volumeName(board.chip));
        return;
    }
    const Image image = imageFor(board.chip);
    if (image.bytes.isEmpty()) {
        fail(ImageMissing, BootloaderLocator::displayName(board.chip));
        return;
    }
    if (!custom()) {
        const QString v = leadingVersion(image.version);
        if (!v.isEmpty() && v != m_expected) {
            fail(ImageStale, v);
            return;
        }
    }
    const QByteArray bytes = image.bytes;
    const int required = m_boardRead ? m_requiredMv : 0;
    const QByteArray typed = m_typed.toUtf8();
    m_ctx.core->worker()->call(
        this, [bytes, required, typed](FfiCore *) { return guardCheck(bytes, required, typed); },
        [this, board, image](QJsonObject verdict) {
            const QString decision = verdict.value(QStringLiteral("decision")).toString();
            if (verdict.value(QStringLiteral("allowed")).toBool()) {
                write(board, image);
            } else if (decision == QLatin1String("needsConfirmation")) {
                askGuard(verdict, [this, board] {
                    m_installing = false;
                    install(board);
                });
            } else {
                fail(GuardRefused, verdict.value(QStringLiteral("message")).toString());
            }
        });
}

void FirmwareController::write(const BootloaderLocator::Board &board, const Image &image) {
    setState(Writing);
    const QByteArray bytes = image.bytes;
    const QByteArray drive = board.drive.toUtf8();
    const int chip = board.chip;
    const int required = m_boardRead ? m_requiredMv : 0;
    const QByteArray typed = m_typed.toUtf8();
    m_ctx.core->worker()->call(
        this,
        [bytes, drive, chip, required, typed](FfiCore *c) {
            // -- WP P1QT2 -- with a token minted now, the board waiting in its
            // bootloader (the person started the install; the guard's word, if
            // it asked for one, is `typed`)
            return QJsonDocument::fromJson(Confirm::run(c, Confirm::kFirmwareFlash, [&](const char *t, char *b, uint32_t n) {
                       return dspi_flash_confirmed(c, reinterpret_cast<const uint8_t *>(bytes.constData()),
                                                   uint32_t(bytes.size()), drive.constData(), int8_t(chip),
                                                   uint16_t(required), typed.isEmpty() ? nullptr : typed.constData(),
                                                   t, b, n);
                   }).json).object();
        },
        [this, image](QJsonObject reply) {
            if (!reply.value(QStringLiteral("ok")).toBool()) {
                fail(WriteFailed, reply.value(QStringLiteral("error")).toString());
                return;
            }
            // The copy returning proves nothing: the device must come back
            // running what was written
            m_writtenVersion = leadingVersion(image.version);
            setState(WaitingForDevice);
            m_verifyDeadline.start();
            checkReturned();
        });
}

void FirmwareController::checkReturned() {
    if (m_state != WaitingForDevice || m_checking || !m_ctx.device->connected()) return;
    // The connection the device left on is not the one it comes back on
    if (m_leftGeneration != 0 && m_ctx.device->snapshot().generation == m_leftGeneration) return;
    if (m_writtenVersion.isEmpty()) {
        // An image that names no version: back, but nothing to check it against
        m_unconfirmed = true;
        m_verifiedVersion = runningVersion();
        m_verifyDeadline.stop();
        setState(Verified);
        return;
    }
    m_checking = true;
    const QByteArray version = m_writtenVersion.toUtf8();
    m_ctx.core->worker()->call(
        this, [version](FfiCore *c) { return int(dspi_firmware_version_matches(c, version.constData())); },
        [this](int matches) {
            m_checking = false;
            if (m_state != WaitingForDevice) return;
            if (matches == 1) {
                m_verifiedVersion = runningVersion();
                m_verifyDeadline.stop();
                setState(Verified);
            } else if (matches == 0) {
                fail(VersionMismatch, runningVersion());
            }
        });
}

// ---- The installer as the Getting Started wizard drives it (QP17) ------------

QString FirmwareController::installerState() const {
    if (m_guardAsking) return QStringLiteral("question");
    if (m_reading) return QStringLiteral("checking");
    if (m_state == Verified && m_unconfirmed) return QStringLiteral("unconfirmed");
    return stage();
}

QString FirmwareController::chip() const { return boardName(); }

QString FirmwareController::volume() const {
    return m_lastChip >= 0 ? BootloaderLocator::volumeName(m_lastChip) : QString();
}

double FirmwareController::fraction() const {
    return m_state == WaitingForDevice || m_state == Verified ? 1.0 : 0.0;
}

bool FirmwareController::mundane() const {
    return m_state == Failed && (m_error == MultipleBoards || m_error == VolumeNotMounted);
}

void FirmwareController::beginWatching() { watch(true); }
void FirmwareController::stopWatching() { watch(false); }
void FirmwareController::installWhenReady() { begin(false); }
// The wizard calls installWhenReady() and then this, as the Mac's view does:
// a run already started for the open device is continued with its restart
void FirmwareController::installConnected() {
    if (!m_reading && !m_guardAsking && !(m_confirmed && underWay())) {
        begin(true);
        return;
    }
    m_rebootWanted = true;
    if (m_confirmed && !m_installing) enterBootloaderForInstall();
}
void FirmwareController::reset() { resetRun(); }

// ---- What the window says ----------------------------------------------------

QString FirmwareController::boardName() const {
    return m_lastChip >= 0 ? BootloaderLocator::displayName(m_lastChip) : QString();
}

QString FirmwareController::primaryText() const {
    switch (m_state) {
    case Failed: return tr("Try Again");
    case Writing:
    case WaitingForDevice:
    case Verified: return QString();
    default: return downgrade() && !custom() ? tr("Downgrade") : tr("Update Firmware");
    }
}

bool FirmwareController::showBootselHint() const {
    return (m_state == Idle || m_state == WaitingForBoard) && (m_confirmed || !m_ctx.device->connected());
}

bool FirmwareController::showManualHint() const {
    return m_ctx.device->connected() && !m_confirmed && (m_state == Idle || m_state == WaitingForBoard);
}

bool FirmwareController::showExport() const {
    return m_ctx.device->connected() && !m_confirmed && m_state != Verified;
}

bool FirmwareController::statusBusy() const {
    switch (m_state) {
    case Idle:
    case WaitingForBoard: return m_confirmed;
    case WaitingForVolume:
    case Writing:
    case WaitingForDevice: return true;
    case Ready: return m_confirmed;
    default: return false;
    }
}

QString FirmwareController::statusIcon() const {
    switch (m_state) {
    case Idle:
    case WaitingForBoard: return m_confirmed ? QStringLiteral("search")
                                 : m_ctx.device->connected() ? QStringLiteral("circle-check")
                                                             : QStringLiteral("cable");
    case WaitingForVolume:
    case Ready: return QStringLiteral("hard-drive");
    case Writing: return QStringLiteral("download");
    case WaitingForDevice: return QStringLiteral("refresh-cw");
    case Verified: return QStringLiteral("circle-check");
    case Failed: return m_error == MultipleBoards || m_error == VolumeNotMounted ? QStringLiteral("circle-alert")
                                                                                  : QStringLiteral("triangle-alert");
    }
    return QString();
}

QString FirmwareController::statusTint() const {
    switch (m_state) {
    case Idle:
    case WaitingForBoard: return m_confirmed || m_ctx.device->connected() ? QStringLiteral("accent") : QStringLiteral("muted");
    case Ready: return m_confirmed ? QStringLiteral("accent") : QStringLiteral("success");
    case Verified: return QStringLiteral("success");
    case Failed: return QStringLiteral("warning");
    default: return QStringLiteral("accent");
    }
}

QString FirmwareController::statusTitle() const {
    switch (m_state) {
    case Idle:
    case WaitingForBoard:
        if (m_confirmed) return tr("Looking for the board");
        return m_ctx.device->connected() ? tr("Ready when you are") : tr("Connect a board");
    case WaitingForVolume: return tr("%1 found").arg(boardName());
    case Ready: return m_confirmed ? tr("Preparing to write") : tr("%1 ready").arg(boardName());
    case Writing: return tr("Writing firmware");
    case WaitingForDevice: return tr("Firmware written");
    case Verified: return tr("Update complete");
    case Failed:
        return m_error == MultipleBoards || m_error == VolumeNotMounted ? tr("Not quite ready")
                                                                         : tr("The update did not complete");
    }
    return QString();
}

QString FirmwareController::statusMessage() const {
    const QString chipDrive = BootloaderLocator::volumeName(m_board.chip);
    switch (m_state) {
    case Idle:
    case WaitingForBoard:
        if (m_confirmed)
            return tr("Waiting for it to appear in bootloader mode. If nothing happens after a few seconds, unplug the "
                      "board, hold BOOTSEL, and plug it back in.");
        if (m_ctx.device->connected())
            return tr("Click %1 to begin. The device will restart into bootloader mode, and audio will stop until the "
                      "update finishes. Nothing is written without this click.")
                .arg(primaryText());
        return tr("No device is connected. Hold the BOOTSEL button while plugging a board in, and it will appear here.");
    case WaitingForVolume:
        return tr("The board is in bootloader mode. Waiting for its %1 drive to mount - this usually takes a second or two.")
            .arg(chipDrive);
    case Ready:
        if (m_confirmed) return tr("Opening the %1's %2 drive.").arg(boardName(), chipDrive);
        return tr("The board is in bootloader mode and ready to receive firmware %1. Click %2 to begin.")
            .arg(imageVersion(), primaryText());
    case Writing:
        return tr("Near the end the board restarts itself and its drive disappears. That is normal - do not unplug it.");
    case WaitingForDevice:
        return tr("The board is restarting with its new firmware. This can take up to half a minute; leave it plugged in.");
    case Verified:
        if (m_unconfirmed)
            return tr("The device is back, running firmware %1. The image does not name its version, so it could not be "
                      "checked.")
                .arg(m_verifiedVersion);
        return tr("The device is back and confirmed running firmware %1.").arg(m_verifiedVersion);
    case Failed: return errorText();
    }
    return QString();
}

QString FirmwareController::errorText() const {
    switch (m_error) {
    case NoError: return QString();
    case MultipleBoards:
        return tr("%1 boards are in bootloader mode. Disconnect all but the one you want to update.").arg(m_errorDetail);
    case VolumeNotMounted:
        return tr("The board is connected but its %1 drive has not appeared. Open it in your file manager so that it "
                  "mounts, then try again.")
            .arg(m_errorDetail);
    case ImageMissing:
        return tr("This build of %1 does not include firmware for the %2.").arg(AppIdentity::displayName(), m_errorDetail);
    case ImageStale:
        return tr("The bundled firmware is version %1 but this Console expects %2. The app was built incorrectly; do not "
                  "install it.")
            .arg(m_errorDetail, m_expected);
    case WriteFailed:
    case GuardRefused: return sentence(m_errorDetail);
    case DeviceDidNotReturn:
        return tr("The firmware was written but the device did not reappear. Unplug it, plug it back in, and check whether "
                  "it works.");
    case VersionMismatch:
        return tr("The device came back running firmware %1 instead of %2.").arg(m_errorDetail, m_writtenVersion);
    case DeviceLocked: return tr("Another console is applying changes. Try again when it has finished.");
    }
    return QString();
}
