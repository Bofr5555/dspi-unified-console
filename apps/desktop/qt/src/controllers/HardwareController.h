#ifndef HARDWARECONTROLLER_H
#define HARDWARECONTROLLER_H

#include <QList>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>
#include <memory>
#include <vector>

#include "CoreApi.h"
#include "PinMap.h"
#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// One part of what the Settings window's save bar saves and reverts (the
// Mac's SettingsSaveCoordinator categories). The hardware pages' own parts are
// built in; another package adds its own (the Control Surfaces live preview)
// with HardwareController::addSaveParticipant and calls
// HardwareController::participantChanged() when its pending state moves.
class SaveParticipant {
public:
    virtual ~SaveParticipant() = default;
    // Changes are held for the device
    virtual bool hasPending() const = 0;
    // The changes are already live on the device; Save only keeps them
    // across a restart (the bar's line then says so)
    virtual bool liveOnly() const { return false; }
    // An operation of its own is running: the bar's buttons wait
    virtual bool busy() const { return false; }
    // Writes what is held, then calls done(failures) on the GUI thread:
    // one short phrase per thing the device did not accept ("the latency")
    virtual void save(std::function<void(const QStringList &failures)> done) = 0;
    virtual void revert() = 0;
};

// -- WP QP11 -- Settings > Inputs, Outputs, I2S Configuration, Global
// Parameters and Control Interfaces (the Mac's HardwareSettingsTab,
// GlobalSettingsTab and ControlInterfacesSettingsTab), and the save bar's
// coordination (the Mac's SettingsSaveCoordinator).
//
// What applies when, as on the Mac:
// - Pins, output types, clocks and the input options apply to the device at
//   once (its RAM). The first such change since the last save records the
//   configuration as it was; while the output configuration is kept apart from
//   the presets, the bar then offers to save it to flash or put it back.
// - Global Parameters (startup preset, master volume and output configuration
//   modes, external mute, latency) and the names are held as drafts and only
//   written on Save.
// - The control interfaces apply per interface with their own Apply.
// A Save that the device partly refuses leaves the bar up with a line naming
// what was not accepted; nothing refused reads as saved.
//
// Which sections show follows the connect probe (dspi_feature_present) and
// the platform, never the firmware version. Every write goes through
// DeviceModel::write(), named by its target.
class HardwareController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(int revision READ revision NOTIFY changed FINAL)
    Q_PROPERTY(bool connected READ connected NOTIFY changed)
    Q_PROPERTY(bool writable READ writable NOTIFY changed)
    Q_PROPERTY(bool rp2040 READ rp2040 NOTIFY changed)
    Q_PROPERTY(bool stm32 READ stm32 NOTIFY changed)
    // What the open device has (the probe's answers)
    Q_PROPERTY(QVariantMap has READ has NOTIFY changed)
    // The device's hardware configuration, from the snapshot (see readState)
    Q_PROPERTY(QVariantMap hw READ hw NOTIFY changed)
    // The rate the pipeline runs at now, Hz (0 unknown)
    Q_PROPERTY(int liveRateHz READ liveRateHz NOTIFY liveChanged)
    // Lock state of the I2S slave input and the ADAT input, read while the
    // Inputs page shows: {state, text, detected, measured, locked}
    Q_PROPERTY(QVariantMap i2sLock READ i2sLock NOTIFY liveChanged)
    Q_PROPERTY(QVariantMap adatLock READ adatLock NOTIFY liveChanged)
    // The last pin change's outcome, shown on the page (the Mac's status row)
    Q_PROPERTY(QString statusText READ statusText NOTIFY statusChanged)
    Q_PROPERTY(bool statusError READ statusError NOTIFY statusChanged)

    // Global Parameters: the held draft
    Q_PROPERTY(QVariantMap draft READ draft NOTIFY draftChanged FINAL)
    Q_PROPERTY(QStringList presetSlotLabels READ presetSlotLabels NOTIFY changed)

    // Control Interfaces: the outcome of the last Apply per interface
    Q_PROPERTY(QVariantMap uartOutcome READ uartOutcome NOTIFY interfacesChanged)
    Q_PROPERTY(QVariantMap i2cOutcome READ i2cOutcome NOTIFY interfacesChanged)

    // The save bar
    Q_PROPERTY(bool pending READ pending NOTIFY pendingChanged)
    Q_PROPERTY(bool liveOnly READ liveOnly NOTIFY pendingChanged)
    Q_PROPERTY(bool saving READ saving NOTIFY pendingChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY pendingChanged)
    // What the last Save could not write ("" when all of it went)
    Q_PROPERTY(QString saveFailure READ saveFailure NOTIFY pendingChanged)
    // A saved setting the device holds but has not applied yet ("" when none)
    Q_PROPERTY(QString heldNote READ heldNote NOTIFY pendingChanged)
public:
    HardwareController(const AppContext &ctx, QObject *parent);
    ~HardwareController() override;
    static HardwareController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<HardwareController>(); }

    // Who holds a pin, for the pickers: the thing that changes it
    enum Consumer {
        OutputPin,      // index: the output slot (the PDM sub included)
        I2sBck,
        I2sBckSlave,
        Mck,
        SpdifRx,        // index: the S/PDIF input
        I2sRx,          // index: the pair
        AdatOut,
        AdatIn,
        DacMute,
        UartCtrl,
        I2cCtrl,
    };
    Q_ENUM(Consumer)

    int revision() const { return m_rev; }
    bool connected() const;
    bool writable() const;
    bool rp2040() const;
    bool stm32() const;
    QVariantMap has() const { return m_has; }
    QVariantMap hw() const { return m_hw; }
    int liveRateHz() const { return m_liveRate; }
    QVariantMap i2sLock() const { return m_i2sLock; }
    QVariantMap adatLock() const { return m_adatLock; }
    QString statusText() const { return m_statusText; }
    bool statusError() const { return m_statusError; }
    QVariantMap draft() const;
    QStringList presetSlotLabels() const;
    QVariantMap uartOutcome() const { return m_uartOutcome; }
    QVariantMap i2cOutcome() const { return m_i2cOutcome; }

    bool pending() const;
    bool liveOnly() const;
    bool saving() const { return m_saving; }
    bool busy() const;
    QString saveFailure() const { return m_saveFailure; }
    QString heldNote() const { return m_heldNote; }

    // The GPIOs a picker offers: every valid pin no one else holds, the
    // current one kept; [{pin, text}]. `pairWithNext` asks for pins whose
    // next pin is free too (BCK, whose LRCLK is BCK + 1).
    Q_INVOKABLE QVariantList pinChoices(int consumer, int index, int current, bool pairWithNext = false) const;
    // The MCK pins the chip can drive (GPOUT), free ones only
    Q_INVOKABLE QVariantList mckPinChoices() const;
    // The control interfaces' pins: the peripheral's mux rule (UART TX pin % 4
    // == 0, RX % 4 == 1; I2C SDA even, SCL odd) and free, `other` the
    // interface's other pin, `current` kept
    Q_INVOKABLE QVariantList interfacePinChoices(int consumer, bool second, int current, int other) const;
    // "Output 2" or "UART Control": who holds `pin` besides `consumer`; "" free
    Q_INVOKABLE QString pinOwner(int pin, int consumer, int index = 0) const;

    // Pages call these while shown: the live rate and the lock states are read
    // then (Poller), and the configuration is read again on open
    Q_INVOKABLE void watch(bool on);
    Q_INVOKABLE void clearStatus();

    // Outputs page (live)
    Q_INVOKABLE void setOutputType(int slot, int type);
    Q_INVOKABLE void setOutputPin(int output, int pin);
    Q_INVOKABLE void resetPins();
    Q_INVOKABLE void setAdatOutEnabled(bool on);
    Q_INVOKABLE void setAdatOutPin(int pin);
    // I2S Configuration (live)
    Q_INVOKABLE void setBckPin(int pin);
    Q_INVOKABLE void setClockPinMode(int mode);
    Q_INVOKABLE void setSlaveBckPin(int pin);
    Q_INVOKABLE void setMckEnabled(bool on);
    Q_INVOKABLE void setMckPin(int pin);
    Q_INVOKABLE void setMckMultiplier(int times);   // 128 or 256
    Q_INVOKABLE void setInputRate(int hz);
    // Inputs page (live)
    Q_INVOKABLE void setSpdifCount(int count);
    Q_INVOKABLE void setSpdifPin(int index, int pin);
    Q_INVOKABLE void setLgSoundSync(bool on);
    Q_INVOKABLE void setI2sClockMode(int clockMode);
    Q_INVOKABLE void setI2sChannels(int channels);
    Q_INVOKABLE void setI2sRxPin(int pair, int pin);
    Q_INVOKABLE void setAdatInEnabled(bool on);
    Q_INVOKABLE void setAdatInPin(int pin);
    Q_INVOKABLE void setAdatInClockMode(int mode);

    // Global Parameters: change the held draft (`key` one of startupMode,
    // defaultSlot, masterVolumeMode, outputConfigMode, muteEnabled,
    // muteActiveLow, mutePin, muteHoldMs, muteReleaseMs)
    Q_INVOKABLE void setDraft(const QString &key, const QVariant &value);
    // Pulse the external mute once (the device's saved settings)
    Q_INVOKABLE void testDacMute();

    // Control Interfaces: send one interface's configuration; the outcome
    // arrives in uartOutcome / i2cOutcome once the device has applied it.
    // {enabled, txPin, rxPin, baud, notify} / {enabled, sdaPin, sclPin, address}
    Q_INVOKABLE void applyUart(const QVariantMap &config);
    Q_INVOKABLE void applyI2c(const QVariantMap &config);
    // "115200" -> "115k" (the Mac's baud labels)
    Q_INVOKABLE QString baudLabel(int baud) const;

    // The save bar
    Q_INVOKABLE void save();
    Q_INVOKABLE void revert();

    // For other packages' parts of the bar (see SaveParticipant); `order`
    // places it among the built-in parts: the global settings 10, the latency
    // 20, the output configuration 30, the names 90.
    void addSaveParticipant(SaveParticipant *participant, int order);
    void removeSaveParticipant(SaveParticipant *participant);
    void participantChanged();
    // The output configuration is about to change outside these pages (the
    // output card, the limiter): record what to put back
    Q_INVOKABLE void beginOutputEdit();

    // "0.75 ms", "2 ms" (the Mac's formatLatencyMs)
    static QString latencyMs(int us);
    // Printable ASCII, at most 31 characters: what the device stores
    static QString sanitizeName(const QString &name);
    // A held latency the device has not applied (OutputLatencyController)
    void setHeldNote(const QString &note);

signals:
    void changed();
    void liveChanged();
    void statusChanged();
    void draftChanged();
    void interfacesChanged();
    void pendingChanged();

private:
    struct Draft {
        int startupMode = 0;
        int defaultSlot = 0;
        int masterVolumeMode = 0;
        int outputConfigMode = 0;
        bool muteEnabled = false;
        bool muteActiveLow = true;
        int mutePin = 0xFF;
        int muteHoldMs = 20;
        int muteReleaseMs = 20;
        bool operator==(const Draft &o) const {
            return startupMode == o.startupMode && defaultSlot == o.defaultSlot && masterVolumeMode == o.masterVolumeMode
                   && outputConfigMode == o.outputConfigMode && muteEnabled == o.muteEnabled
                   && muteActiveLow == o.muteActiveLow && mutePin == o.mutePin && muteHoldMs == o.muteHoldMs
                   && muteReleaseMs == o.muteReleaseMs;
        }
    };
    struct Part {
        SaveParticipant *participant;
        int order;
    };
    class GlobalPart;
    class OutputPart;

    void onSnapshot();
    void joinParts();
    void readFeatures();
    void readState();
    void readSurfaces();
    Draft deviceDraft() const;
    bool globalDirty() const;
    bool outputDirty() const;
    void setStatus(const QString &text, bool error);
    // One live hardware write: `setter` returns a PIN_CONFIG_* status (0xFF
    // when it could not be asked); `outcome` turns it into the status line
    void live(const QString &what, std::function<uint8_t(FfiCore *)> setter,
              std::function<void(uint8_t status)> outcome);
    QString ownerText(int pin, int consumer, int index) const;
    QString pinInUseText(int pin, int consumer, int index) const;
    void runSave(size_t step, std::shared_ptr<QStringList> failures, uint64_t generation);
    void finishSave(const QStringList &failures);
    void restoreOutputs();
    void pendingMaybeChanged();
    void readInterfacesLater(bool uart);

    AppContext m_ctx;
    int m_rev = 0;
    uint64_t m_generation = 0;
    QString m_serial;
    QVariantMap m_has;
    QList<PinMap::SurfacePin> m_surfaces;
    QVariantMap m_hw;
    int m_liveRate = 0;
    QVariantMap m_i2sLock, m_adatLock;
    QString m_statusText;
    bool m_statusError = false;
    int m_watchers = 0;
    int m_lockPoll = -1;
    // Read by the poll on the worker: which lock states are worth asking for
    std::shared_ptr<std::atomic<bool>> m_wantI2sLock = std::make_shared<std::atomic<bool>>(false);
    std::shared_ptr<std::atomic<bool>> m_wantAdatLock = std::make_shared<std::atomic<bool>>(false);

    Draft m_draft;
    bool m_globalEdited = false;
    bool m_outputEdited = false;
    std::shared_ptr<const DspState> m_outputBaseline;
    QVariantMap m_uartOutcome, m_i2cOutcome;

    GlobalPart *m_globalPart;
    OutputPart *m_outputPart;
    std::vector<Part> m_parts;
    bool m_saving = false;
    QString m_saveFailure;
    QString m_heldNote;
    bool m_partsJoined = false;
};

#endif // HARDWARECONTROLLER_H
