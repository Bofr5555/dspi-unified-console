#ifndef FIRMWARECONTROLLER_H
#define FIRMWARECONTROLLER_H

#include <QByteArray>
#include <QJsonObject>
#include <QElapsedTimer>
#include <QObject>
#include <QString>
#include <QTimer>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "BootloaderLocator.h"
#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP16 -- Firmware: the update window (the Mac's FirmwareUpdateView and
// FirmwareInstaller), the version-mismatch banner, and the bundled images.
//
// The app carries the firmware of its own release (`:/firmware/DSPi-<chip>-v<version>.uf2`);
// that version is the one it expects a device to run. The installer never
// writes on its own: start() is the person's one decision, then it restarts a
// connected device into its bootloader, waits for exactly one board with a
// drive (BootloaderLocator), writes the image behind the core's firmware guard
// (dspi_flash_confirmed, with its token), and calls it done only when the device comes
// back running the image's version. The guard's typed confirmation (core
// voltage risk) is asked before the device leaves, while the board can still
// be read (dspi_firmware_guard_board).
//
// A custom .uf2 can stand in for the bundled image; it is checked by the core
// (dspi_uf2_inspect) when chosen and again by the guard and the write.
//
// Hidden on STM32H723: the app bundles no firmware for it and its menu row is
// hidden there.
class FirmwareController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // ---- The mismatch banner ----
    Q_PROPERTY(QString expectedVersion READ expectedVersion NOTIFY deviceChanged)
    // "match", "older" (the device is behind), "newer", or "" (unknown: no device, no version)
    Q_PROPERTY(QString match READ match NOTIFY deviceChanged)
    Q_PROPERTY(bool mismatchShown READ mismatchShown NOTIFY deviceChanged)
    Q_PROPERTY(QString mismatchText READ mismatchText NOTIFY deviceChanged)
    Q_PROPERTY(bool supported READ supported NOTIFY deviceChanged)
    // ---- The window ----
    Q_PROPERTY(QString runningVersion READ runningVersion NOTIFY deviceChanged)
    Q_PROPERTY(bool downgrade READ downgrade NOTIFY deviceChanged)
    Q_PROPERTY(QString stage READ stage NOTIFY stateChanged)
    Q_PROPERTY(int step READ step NOTIFY stateChanged)
    Q_PROPERTY(bool failed READ failed NOTIFY stateChanged)
    Q_PROPERTY(bool confirmed READ confirmed NOTIFY stateChanged)
    Q_PROPERTY(QString statusTitle READ statusTitle NOTIFY stateChanged)
    Q_PROPERTY(QString statusMessage READ statusMessage NOTIFY stateChanged)
    Q_PROPERTY(QString statusIcon READ statusIcon NOTIFY stateChanged)
    // "accent", "success", "muted" or "warning"
    Q_PROPERTY(QString statusTint READ statusTint NOTIFY stateChanged)
    Q_PROPERTY(bool statusBusy READ statusBusy NOTIFY stateChanged)
    Q_PROPERTY(QString boardName READ boardName NOTIFY stateChanged)
    Q_PROPERTY(QString primaryText READ primaryText NOTIFY stateChanged)
    Q_PROPERTY(bool showBootselHint READ showBootselHint NOTIFY stateChanged)
    Q_PROPERTY(bool showManualHint READ showManualHint NOTIFY stateChanged)
    Q_PROPERTY(bool showExport READ showExport NOTIFY stateChanged)
    // ---- The image ----
    Q_PROPERTY(QString imageName READ imageName NOTIFY imageChanged)
    Q_PROPERTY(QString imageVersion READ imageVersion NOTIFY imageChanged)
    // The bundled image passed the core's check and carries the expected version
    Q_PROPERTY(bool imageChecked READ imageChecked NOTIFY imageChanged)
    Q_PROPERTY(bool custom READ custom NOTIFY imageChanged)
    // Why the custom image cannot be used ("" when it can)
    Q_PROPERTY(QString imageProblem READ imageProblem NOTIFY imageChanged)
    // -- WP SECFIX -- A custom image that is not a bundled one byte for byte
    // (the core's manifest, dspi_firmware_verify_bundled; SECFWV): the label
    // "Not a verified DSPi Console image" and what it means (the core's notice
    // row firmware.not_verified). Information, not a refusal; "" otherwise.
    Q_PROPERTY(QString unverifiedLabel READ unverifiedLabel NOTIFY imageChanged)
    Q_PROPERTY(QString unverifiedText READ unverifiedText NOTIFY imageChanged)
    // ---- The guard's typed confirmation ----
    Q_PROPERTY(bool guardAsking READ guardAsking NOTIFY guardChanged)
    Q_PROPERTY(QString guardMessage READ guardMessage NOTIFY guardChanged)
    Q_PROPERTY(QString guardPhrase READ guardPhrase NOTIFY guardChanged)
    // -- WP PGQT -- The update check (dspi_firmware_update_check on the target
    // the core read, dspi_firmware_target) judges before the device leaves for
    // its bootloader: the protocol generation the image brings, in the core's
    // words ("" when none or not known yet)
    Q_PROPERTY(QString generationChange READ generationChange NOTIFY guardChanged)
    // The connected board is offered no image (no BOOTSEL: a DSPi-Zero or an
    // unknown board), and why; Update Firmware is off
    Q_PROPERTY(bool noUpdate READ noUpdate NOTIFY deviceChanged)
    // A sentence about the image for this board ("" for none): no BOOTSEL, or
    // the alink sender, for which this build bundles no image
    Q_PROPERTY(QString imageNote READ imageNote NOTIFY deviceChanged)
    // ---- The installer contract of QP17's Getting Started wizard
    // (OnboardingController.h): the Mac's states, and on the desktop
    // "checking" (the board is read before it restarts), "question" (the
    // guard's typed confirmation, asked in the window) and "unconfirmed" (back,
    // but the image names no version)
    Q_PROPERTY(QString state READ installerState NOTIFY stateChanged)
    Q_PROPERTY(QString chip READ chip NOTIFY stateChanged)
    Q_PROPERTY(QString volume READ volume NOTIFY stateChanged)
    Q_PROPERTY(QString version READ imageVersion NOTIFY imageChanged)
    Q_PROPERTY(QString failure READ errorText NOTIFY stateChanged)
    Q_PROPERTY(double fraction READ fraction NOTIFY stateChanged)
    Q_PROPERTY(bool mundane READ mundane NOTIFY stateChanged)
public:
    enum State { Idle, WaitingForBoard, WaitingForVolume, Ready, Writing, WaitingForDevice, Verified, Failed };
    Q_ENUM(State)

    FirmwareController(const AppContext &ctx, QObject *parent = nullptr);
    static FirmwareController *create(QQmlEngine *, QJSEngine *);

    QString expectedVersion() const { return m_expected; }
    QString match() const;
    bool mismatchShown() const;
    QString mismatchText() const;
    bool supported() const;
    QString runningVersion() const;
    bool downgrade() const { return match() == QLatin1String("newer"); }

    QString stage() const;
    State stateValue() const { return m_state; }
    int step() const;
    bool failed() const { return m_state == Failed; }
    bool confirmed() const { return m_confirmed; }
    QString statusTitle() const;
    QString statusMessage() const;
    QString statusIcon() const;
    QString statusTint() const;
    bool statusBusy() const;
    QString boardName() const;
    QString primaryText() const;
    bool showBootselHint() const;
    bool showManualHint() const;
    bool showExport() const;

    QString imageName() const;
    QString imageVersion() const;
    bool imageChecked() const;
    bool custom() const { return !m_customPath.isEmpty(); }
    QString imageProblem() const { return m_customProblem; }
    // -- WP SECFIX --
    bool imageUnverified() const { return custom() && m_customImage.bundled != QLatin1String("verified"); }
    QString unverifiedLabel() const { return imageUnverified() ? m_unverifiedLabel : QString(); }
    QString unverifiedText() const { return imageUnverified() ? m_unverifiedText : QString(); }

    bool guardAsking() const { return m_guardAsking; }
    QString guardMessage() const { return m_guardMessage; }
    QString guardPhrase() const { return m_guardPhrase; }
    // -- WP PGQT --
    QString generationChange() const { return m_generationChange; }
    bool noUpdate() const;
    QString imageNote() const;
    // The board read for this run (dspi_firmware_target's JSON, "" none) and
    // whether the write passes the core its voltage record (the smoke driver)
    QByteArray targetForTest() const { return m_target; }
    bool writesWithTargetForTest() const { return m_useTarget; }

    // The window showed (detection starts; a fresh run unless one is under way)
    // or closed (detection stops when nothing else watches)
    Q_INVOKABLE void windowShown(bool shown);
    // The installer contract (QP17)
    QString installerState() const;
    QString chip() const;
    QString volume() const;
    double fraction() const;
    bool mundane() const;
    Q_INVOKABLE void beginWatching();
    Q_INVOKABLE void stopWatching();
    Q_INVOKABLE void installWhenReady();
    Q_INVOKABLE void installConnected();
    Q_INVOKABLE void reset();
    // The person commits to the update (Update Firmware / Downgrade)
    Q_INVOKABLE void start();
    // Try Again, Update Another Board: back to detection, nothing armed
    Q_INVOKABLE void resetRun();
    // Restarts the connected device into its bootloader without installing
    // (allowed on read-only firmware too)
    Q_INVOKABLE void enterBootloaderOnly();
    Q_INVOKABLE void exportConfiguration();
    Q_INVOKABLE void chooseCustomImage();
    Q_INVOKABLE void useCustomImage(const QString &path);
    Q_INVOKABLE void useBundledImage();
    // The guard's question: `typed` must be the phrase exactly; cancel() stops the run
    Q_INVOKABLE void answerGuard(const QString &typed);
    Q_INVOKABLE void cancelGuard();
    // Hides the mismatch banner until the app is next started
    Q_INVOKABLE void hideMismatch();
    // Opens the update window (the banners' buttons)
    Q_INVOKABLE void openWindow();

    BootloaderLocator *locator() const { return m_locator; }
    // The smoke driver's knobs
    void setTimeoutsForTest(int volumeWaitMs, int verifyMs);
    // Another expected version ("": the bundled images' again)
    void setExpectedVersionForTest(const QString &version);
    // Parses "1.1.6" out of "DSPi-RP2350-v1.1.6.uf2"; "" when there is none
    static QString versionFromName(const QString &fileName);

signals:
    void deviceChanged();
    void stateChanged();
    void imageChanged();
    void guardChanged();

private:
    struct Image {
        QString name;
        QByteArray bytes;
        QString version;     // from the core's check, "" when it has none
        int chip = -1;
        bool valid = false;
        QString problem;
        QString bundled;     // -- WP SECFIX -- the manifest's answer: verified, notBundled, mismatch
    };
    enum Error {
        NoError, MultipleBoards, VolumeNotMounted, ImageMissing, ImageStale, WriteFailed, DeviceDidNotReturn,
        VersionMismatch, DeviceLocked, GuardRefused,
        NoBootsel   // -- WP PGQT --
    };

    void setState(State s);
    void fail(Error e, const QString &detail = QString());
    void onScan(const QList<BootloaderLocator::Board> &boards);
    void install(const BootloaderLocator::Board &board);
    void write(const BootloaderLocator::Board &board, const Image &image);
    void enterBootloaderForInstall();
    void checkReturned();
    void loadBundled();
    // The image for `chip`: the custom one, else the bundled one (empty bytes: none)
    Image imageFor(int chip) const;
    int deviceChip() const;
    QString describe(int major, int minor, int patch, int beta) const;
    void begin(bool reboot);
    void watch(bool on);
    bool underWay() const;
    void askGuard(const QJsonObject &verdict, std::function<void()> after);
public:
    QString errorText() const;
private:

    AppContext m_ctx;
    BootloaderLocator *m_locator;
    QString m_expected;
    QString m_bundledVersion;
    QList<Image> m_bundled;
    QString m_customPath;
    Image m_customImage;
    QString m_customProblem;
    QString m_unverifiedLabel, m_unverifiedText;   // -- WP SECFIX -- the core's row, read once

    State m_state = Idle;
    Error m_error = NoError;
    QString m_errorDetail;
    int m_watchers = 0;          // the window and the wizard, while they show
    bool m_windowShown = false;
    bool m_reading = false;      // the board's needs are being read
    bool m_unconfirmed = false;  // back, but the image names no version
    bool m_confirmed = false;
    bool m_armed = false;
    bool m_installing = false;
    bool m_rebootRequested = false;
    bool m_rebootWanted = false;   // installConnected() joined a run already started
    int m_lastChip = -1;
    BootloaderLocator::Board m_board;   // the one board detection last saw
    bool m_checking = false;            // a version check after the write is out
    QElapsedTimer m_driveWait;
    QString m_writtenVersion;
    QString m_verifiedVersion;
    uint64_t m_leftGeneration = 0;
    QTimer m_verifyDeadline;
    int m_driveWaitMs = 8000;

    bool m_mismatchHidden = false;

    bool m_guardAsking = false;
    QString m_guardMessage;
    QString m_guardPhrase;
    QString m_typed;
    int m_requiredMv = 0;
    bool m_boardRead = false;
    std::function<void()> m_afterGuard;
    // -- WP PGQT -- this run's board, as the core read it before the
    // bootloader entry, and the connection it was read on
    QByteArray m_target;
    uint64_t m_targetConnection = 0;
    QString m_generationChange;
    bool m_useTarget = false;   // the write passes the target's voltage record
    bool senderConnected() const;
    // The check before the write: the update check on this run's target, or
    // the voltage rule alone with the voltage unknown (a board not read by
    // this run). Worker thread.
    static QJsonObject checkFor(const QByteArray &bytes, const QByteArray &target, int requiredMv, const QByteArray &typed);
};

#endif // FIRMWARECONTROLLER_H
