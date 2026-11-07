#ifndef FIRMWAREUPDATER_H
#define FIRMWAREUPDATER_H

#include <QByteArray>
#include <QElapsedTimer>
#include <QList>
#include <QObject>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QUrl>
#include <QVariantList>
#include <functional>

class DSPiBridge;

// A firmware version, for telling older from newer and for showing it. Whether a
// device runs a given version is the core's call (dspi_firmware_version_matches):
// it knows about build stamps and early betas.
struct FwVersion {
    int major = 0, minor = 0, patch = 0, beta = 0;
    bool valid = false;
    static FwVersion parse(const QString &text);      // "1.1.6-beta4", "1.1.7"
    QString text() const;                              // "1.1.6 beta 4"
    // Final outranks every beta of its patch; an early beta sorts below beta 1
    int betaRank() const { return beta == 0 ? 256 : beta; }
    bool operator==(const FwVersion &o) const {
        return major == o.major && minor == o.minor && patch == o.patch && beta == o.beta;
    }
    bool operator<(const FwVersion &o) const;
};

// Puts firmware on a board, behind the core's firmware guard (WP QD11). The order
// is the safety, the same as `dspi flash` (apps/terminal/src/flash.rs):
//
// - Everything that can refuse does so while the board still plays: the file and
//   the chip first, then the guard, then any typed answer, and only then the reboot.
// - The board is read last thing before the reboot, in the same worker job as the
//   reboot itself, after every answer. An answer counts only for the risk it was
//   asked about: if the re-read shows another, that takes its own.
// - A drive that was already mounted belongs to a board nobody can ask, even with a
//   DSPi connected, as that DSPi need not be the board behind the drive.
// - Two boards in their bootloaders are refused; nothing says which is which. That
//   is looked at again on every pass of the guard, so after each answer too.
// - The bytes go out only through dspi_flash_to_drive_guarded. Writing is not
//   success: only the board coming back with the image's version is.
// - The reboot needs the board on this machine's USB, so a hub's device can't be
//   sent there. It works on a read-only device, on purpose: other firmware is when
//   a board most needs this. The demo device is not a board, and while it is the
//   open device nothing goes to any drive: the app says nothing reaches a real one.
//
// Bus, drive and device work runs on one worker thread, never on the UI thread.
class FirmwareUpdater : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString expectedVersion READ expectedVersion CONSTANT)
    Q_PROPERTY(QString deviceVersion READ deviceVersion NOTIFY deviceChanged)
    // 0 = no device / unknown, 1 = matches, 2 = device older, 3 = device newer
    Q_PROPERTY(int match READ match NOTIFY deviceChanged)
    // False on a board with no UF2 bootloader (the STM32H723): no update is offered
    Q_PROPERTY(bool supported READ supported NOTIFY deviceChanged)
    // The connected device can be sent to its bootloader from here
    Q_PROPERTY(bool canReboot READ canReboot NOTIFY deviceChanged)
    // Why it can't, for a device that is connected
    Q_PROPERTY(QString rebootBlocked READ rebootBlocked NOTIFY deviceChanged)
    // Why nothing may go to any drive now: the demo is the open device
    Q_PROPERTY(QString writeBlocked READ writeBlocked NOTIFY deviceChanged)

    // idle, waitingForBoard, waitingForVolume, ready: detection
    // checking, question: file, chip and guard; the typed answer
    // waitingForDrive, writing, waitingForDevice: under way
    // verified, unconfirmed, failed: outcomes. `unconfirmed` is written and back, by
    // an image that names no version, so that it took can't be said.
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    // 0..1; below 0 while the amount done isn't known (the core writes in one call)
    Q_PROPERTY(double progress READ progress NOTIFY progressChanged)
    Q_PROPERTY(QString chipName READ chipName NOTIFY stateChanged)       // "RP2350 (Pico 2)"
    Q_PROPERTY(QString volumeName READ volumeName NOTIFY stateChanged)   // "RP2350"
    Q_PROPERTY(QString failure READ failure NOTIFY stateChanged)
    // A detection-side stumble (no board, no drive, two boards) rather than a
    // failed write
    Q_PROPERTY(bool failureMundane READ failureMundane NOTIFY stateChanged)
    Q_PROPERTY(bool noBoardFailure READ noBoardFailure NOTIFY stateChanged)
    Q_PROPERTY(QString verifiedVersion READ verifiedVersion NOTIFY stateChanged)

    // What will be installed: this build's image, or a file of the user's
    Q_PROPERTY(bool imageCustom READ imageCustom NOTIFY imageChanged)
    Q_PROPERTY(QString imageName READ imageName NOTIFY imageChanged)
    Q_PROPERTY(QString imageVersion READ imageVersion NOTIFY imageChanged)
    Q_PROPERTY(bool imageLoading READ imageLoading NOTIFY imageChanged)
    // Why this file can't go on this board, before anything is asked
    Q_PROPERTY(QString imageProblem READ imageProblem NOTIFY imageChanged)

    // The guard's question, in state "question"
    Q_PROPERTY(QString questionMessage READ questionMessage NOTIFY questionChanged)
    Q_PROPERTY(QString questionPhrase READ questionPhrase NOTIFY questionChanged)
    Q_PROPERTY(bool answerWrong READ answerWrong NOTIFY questionChanged)

public:
    explicit FirmwareUpdater(DSPiBridge *bridge, QObject *parent = nullptr);
    ~FirmwareUpdater() override;

    QString expectedVersion() const { return m_expected.text(); }
    QString deviceVersion() const;
    int match() const;
    bool supported() const;
    bool canReboot() const { return connectedNow() && rebootBlocked().isEmpty(); }
    QString rebootBlocked() const;
    QString writeBlocked() const;
    QString state() const { return m_state; }
    double progress() const { return m_progress; }
    QString chipName() const;
    QString volumeName() const;
    QString failure() const { return m_failure; }
    bool failureMundane() const { return m_failureMundane; }
    bool noBoardFailure() const { return m_noBoard; }
    QString verifiedVersion() const { return m_verified.text(); }
    bool imageCustom() const { return m_custom.loaded; }
    QString imageName() const;
    QString imageVersion() const;
    bool imageLoading() const { return m_imageLoading; }
    QString imageProblem() const;
    QString questionMessage() const { return m_questionMessage; }
    QString questionPhrase() const { return m_questionPhrase; }
    bool answerWrong() const { return m_answerWrong; }

    // Detection runs while an update UI is open
    Q_INVOKABLE void beginWatching();
    // Closing lets go of everything but a write already under way
    Q_INVOKABLE void stopWatching();
    // The user's decision, for one board: the running device, if there is one and no
    // bootloader drive is about, else the next board to reach its bootloader
    Q_INVOKABLE void installWhenReady();
    // What the user typed for the question. Never compared here: the guard says.
    Q_INVOKABLE void answer(const QString &typed);
    // Back to detection after an outcome or a question (Try Again, Cancel)
    Q_INVOKABLE void reset();
    // Restart the connected device into its bootloader, installing nothing. Ignored
    // once an install has been asked for: that reboot comes after its guard.
    Q_INVOKABLE void enterBootloader();
    // A .uf2 of the user's, read once: what was checked is what gets written
    Q_INVOKABLE void chooseImage(const QUrl &file);
    Q_INVOKABLE void useBundledImage();
    // -1, 0 or 1 as version a sorts before, with or after b ("1.1.6-beta4")
    Q_INVOKABLE int compareVersions(const QString &a, const QString &b) const;

    struct Drive {
        QString path;
        int platform = -1;      // 0 RP2040, 1 RP2350
    };
    // What is on the bus and mounted, at one moment
    struct Scan {
        QList<Drive> drives;
        int boards = 0;         // boards in BOOTSEL on the bus, mounted or not
        int busPlatform = -1;   // the chip, when there is one board
        QString device;         // its block device, to ask udisks to mount
    };
    // dspi_bootloader_drives' reply
    static QList<Drive> parseDrives(const QByteArray &reply);

#ifdef DSPI_SMOKE
    // For the smoke driver: a fake bootloader drive. `scan` runs on the worker.
    struct TestSeam {
        std::function<Scan()> scan;
        std::function<QStringList()> serials;   // default: the bridge's
        bool demoIsBoard = false;
        int driveWaitMs = 0;                    // 0: as in the app
        int verifyWaitMs = 0;
        int platform = -1;                      // the board's, as if it reported this
        std::function<void()> gateHook;         // on the worker, at the end of a gate job
    };
    void setTestSeam(const TestSeam &seam);
    void clearTestSeam();
    int bootloaderRequests() const { return m_bootloaderRequests; }
    int boardReads() const { return m_boardReads; }
    QString bundledImageResource(int platform) const;
#endif

signals:
    void deviceChanged();
    void stateChanged();
    void progressChanged();
    void imageChanged();
    void questionChanged();

private:
    struct Image {
        bool loaded = false;
        QByteArray bytes;
        QString name;
        QString version;        // the image's own firmwareVersion; "" if it names none
        int platform = -1;
        QString problem;        // not firmware, or can't be read
    };
    struct GateResult {
        QString failure;
        bool failureMundane = false;
        Image image;
        bool boardGone = false;
        int requiredMv = 0;
        bool allowed = false;
        QString decision;
        QString message;
        QString phrase;
        bool entered = false;   // the reboot went out: there is no taking it back
    };
    enum class Path { None, Device, Mounted };

    static Image readImage(const QString &path, const QString &name);
    void publish(const QString &state);
    void fail(const QString &message, bool mundane, bool noBoard = false);
    bool connectedNow() const;
    bool isDemoDevice() const;
    void requestScan();
    Scan runScan() const;
    void scanDone(const Scan &scan);
    void askUdisks(const Scan &scan);
    void beginDevice();
    void beginMounted(const Drive &drive);
    void runGate(const QString &typed);
    void gateDone(const GateResult &result, const QString &typed);
    void startWrite(const Drive &drive);
    void writeDone(const QByteArray &reply);
    void verifyTick();
    bool bringBack(const QString &serial);
    QStringList serialsNow() const;

    DSPiBridge *m_bridge;
    FwVersion m_expected;
    QString m_expectedRaw;       // as the image file names it, "1.1.6"
    QTimer m_scanTimer;
    QTimer m_verifyTimer;
    QElapsedTimer m_verifyClock;
    QElapsedTimer m_volumeWait;
    QElapsedTimer m_driveClock;
    QElapsedTimer m_selectClock;

    QString m_state = "idle";
    double m_progress = 0;
    int m_chip = -1;
    Drive m_drive;
    QString m_mountTried;        // the device a mount was last asked for
    QString m_failure;
    bool m_failureMundane = false;
    bool m_noBoard = false;
    FwVersion m_verified;

    Image m_custom;              // the user's file
    Image m_image;               // what this install writes, once checked
    bool m_imageLoading = false;
    QString m_questionMessage;
    QString m_questionPhrase;
    bool m_answerWrong = false;
    int m_questions = 0;

    Path m_path = Path::None;
    int m_boardPlatform = -1;
    int m_requiredMv = 0;        // the record the answers were given against
    QByteArray m_typed;
    QString m_backSerial;        // Device path: the board coming back
    QStringList m_before;        // Mounted path: the DSPis that are not it
    bool m_sawGone = false;      // the device left: what it reports now is new

    bool m_installing = false;   // from the first check until reset()
    bool m_armed = false;
    bool m_armedForDevice = false; // the click was about the device connected then
    bool m_watching = false;
    bool m_wasConnected = false;
    bool m_scanBusy = false;
    bool m_rescan = false;
    unsigned m_generation = 0;   // a result from before a reset or close is dropped
    // Cancelled while a gate job runs, which may already have rebooted the board:
    // its result decides
    bool m_cancelPending = false;

#ifdef DSPI_SMOKE
    TestSeam m_seam;
    int m_bootloaderRequests = 0;
    int m_boardReads = 0;
#endif
    QThreadPool m_worker;        // one thread; last member, so it joins first: a
                                 // write is never cut short
};

#endif // FIRMWAREUPDATER_H
