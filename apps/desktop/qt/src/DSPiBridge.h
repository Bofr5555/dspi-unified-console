#ifndef DSPIBRIDGE_H
#define DSPIBRIDGE_H

#include <QElapsedTimer>
#include <QObject>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QVector>
#include <QString>
#include <QStringList>
#include <QThreadPool>   // -- WP QD34 --
#include <functional>

extern "C" {
#include "dspi_core.h"
}

class BodePlotItem; // forward

// The QML speaks stable app channel ids; the core speaks firmware wire
// channel indices. App ids (same scheme as the Windows Console):
//   0, 1    inputs 1-2 (USB L/R)
//   2..10   outputs 1-9
//   11..16  inputs 3-8 (RP2350 multichannel inputs)
// Output-indexed calls (matrix, output gain/mute/delay) take an output
// index 0..numOutputChannels-1, and input-indexed calls an input index.
static constexpr int kAppChannelCount = 17;
static constexpr int kAppExtraInputFirst = 11;

class DSPiBridge : public QObject
{
    Q_OBJECT

    // Connection / device
    Q_PROPERTY(bool connected READ connected NOTIFY statusChanged)
    // -- BUG1, review c86 N1 -- Read in a binding (or a function a binding calls) that shows
    // output labels, so it follows them as they change with the routing and the filters
    Q_PROPERTY(int labelsRevision READ labelsRevision NOTIFY outputLabelsChanged)
    // The live settings differ from the active preset (the `*` after its name)
    Q_PROPERTY(bool presetDirty READ presetDirty NOTIFY presetDirtyChanged)
    // Why the last connection attempt failed ("" if it didn't)
    Q_PROPERTY(QString connectionError READ connectionError NOTIFY statusChanged)
    Q_PROPERTY(QString selectedSerial READ selectedSerial NOTIFY stateChanged)
    Q_PROPERTY(QStringList availableSerials READ availableSerials NOTIFY devicesChanged)
    // DSPis on the bus this user can't open (the udev rule is missing)
    Q_PROPERTY(int inaccessibleDevices READ inaccessibleDevices NOTIFY inaccessibleDevicesChanged)
    Q_PROPERTY(QString platformName READ platformName NOTIFY stateChanged)
    // Highest preamp the chip applies correctly: +18 dB on RP2040, +24 on RP2350
    Q_PROPERTY(float preampMaxDB READ preampMaxDB NOTIFY stateChanged)
    Q_PROPERTY(int numChannels READ numChannels NOTIFY stateChanged)
    Q_PROPERTY(int numInputChannels READ numInputChannels NOTIFY stateChanged)
    Q_PROPERTY(int numOutputChannels READ numOutputChannels NOTIFY stateChanged)
    Q_PROPERTY(QString firmwareVersion READ firmwareVersion NOTIFY stateChanged)
    Q_PROPERTY(int compat READ compat NOTIFY stateChanged)
    Q_PROPERTY(QString compatMessage READ compatMessage NOTIFY stateChanged)
    // Connected to firmware this Console may write to. Every control that changes
    // the device is off otherwise; reads, clearing clips and the bootloader go on.
    Q_PROPERTY(bool writable READ writable NOTIFY writableChanged)
    // WIRE_FEAT_* of the connected firmware's wire format; all of them with no device
    Q_PROPERTY(int wireFeatures READ wireFeatures NOTIFY stateChanged)
    Q_PROPERTY(float maxDelayMs READ maxDelayMs NOTIFY stateChanged)
    // -- WP DL3 -- The core's words for the last delay it refused, empty after an
    // accepted one or dismissDelayRefusal().
    Q_PROPERTY(QString delayRefusal READ delayRefusal NOTIFY delayRefusalChanged)

    // Global
    Q_PROPERTY(float preampDB READ preampDB NOTIFY stateChanged)
    Q_PROPERTY(bool bypass READ bypass NOTIFY stateChanged)

    // Volume
    Q_PROPERTY(float masterVolumeDB READ masterVolumeDB NOTIFY stateChanged)
    Q_PROPERTY(int masterVolumeMode READ masterVolumeMode NOTIFY stateChanged)
    Q_PROPERTY(float userVolumeDB READ userVolumeDB NOTIFY stateChanged)
    Q_PROPERTY(bool userMute READ userMute NOTIFY stateChanged)

    // Loudness
    Q_PROPERTY(bool loudnessEnabled READ loudnessEnabled NOTIFY stateChanged)
    Q_PROPERTY(float loudnessRefSPL READ loudnessRefSPL NOTIFY stateChanged)
    Q_PROPERTY(float loudnessIntensity READ loudnessIntensity NOTIFY stateChanged)
    Q_PROPERTY(int loudnessOutputMask READ loudnessOutputMask NOTIFY stateChanged)

    // Crossfeed
    Q_PROPERTY(bool crossfeedEnabled READ crossfeedEnabled NOTIFY stateChanged)
    Q_PROPERTY(int crossfeedPreset READ crossfeedPreset NOTIFY stateChanged)
    Q_PROPERTY(float crossfeedFreq READ crossfeedFreq NOTIFY stateChanged)
    Q_PROPERTY(float crossfeedFeed READ crossfeedFeed NOTIFY stateChanged)
    Q_PROPERTY(bool crossfeedITD READ crossfeedITD NOTIFY stateChanged)
    Q_PROPERTY(int crossfeedOutputPairMask READ crossfeedOutputPairMask NOTIFY stateChanged)

    // CPU / Status
    Q_PROPERTY(int cpu0 READ cpu0 NOTIFY statusChanged)
    Q_PROPERTY(int cpu1 READ cpu1 NOTIFY statusChanged)
    Q_PROPERTY(int clipFlags READ clipFlags NOTIFY statusChanged)
    Q_PROPERTY(int activeInputChannels READ activeInputChannels NOTIFY statusChanged)

    // Presets
    Q_PROPERTY(int activePresetSlot READ activePresetSlot NOTIFY stateChanged)
    Q_PROPERTY(int presetOccupied READ presetOccupied NOTIFY stateChanged)
    Q_PROPERTY(int presetStartupMode READ presetStartupMode NOTIFY stateChanged)
    Q_PROPERTY(int presetDefaultSlot READ presetDefaultSlot NOTIFY stateChanged)
    // 0 = output (pin/IO) config stored independently, 1 = saved with presets.
    Q_PROPERTY(int outputConfigMode READ outputConfigMode NOTIFY stateChanged)

    // Input source / quick-access feature toggles
    Q_PROPERTY(int inputSource READ inputSource NOTIFY stateChanged)
    Q_PROPERTY(QVariantList inputSources READ inputSources NOTIFY stateChanged)
    Q_PROPERTY(bool levellerEnabled READ levellerEnabled NOTIFY stateChanged)
    Q_PROPERTY(bool psybassEnabled READ psybassEnabled NOTIFY stateChanged)
    // Upmixer / subharmonic synth / tube modeller parameters, indexed by the
    // core's UPMIX_PARAM_* / SUBHARM_PARAM_* / TUBE_PARAM_* ids
    Q_PROPERTY(bool upmixSupported READ upmixSupported NOTIFY stateChanged)
    Q_PROPERTY(QVariantList upmixParams READ upmixParams NOTIFY stateChanged)
    Q_PROPERTY(QVariantList subharmParams READ subharmParams NOTIFY stateChanged)
    Q_PROPERTY(bool subharmSolo READ subharmSolo NOTIFY stateChanged)
    Q_PROPERTY(QVariantList tubeParams READ tubeParams NOTIFY stateChanged)
    // Hardware IO (Settings): every pin, type, clock, input and control
    // interface setting in one map, and the GPIOs a user may assign
    Q_PROPERTY(QVariantMap hardware READ hardware NOTIFY stateChanged)
    Q_PROPERTY(QVariantList validPins READ validPins CONSTANT)
    // Hardware edits made in independent mode that Save Output Config would keep
    Q_PROPERTY(bool hardwareUnsaved READ hardwareUnsaved NOTIFY stateChanged)

    // Volume leveller parameters
    Q_PROPERTY(float levellerAmount READ levellerAmount NOTIFY stateChanged)
    Q_PROPERTY(int levellerSpeed READ levellerSpeed NOTIFY stateChanged)
    Q_PROPERTY(float levellerMaxGain READ levellerMaxGain NOTIFY stateChanged)
    Q_PROPERTY(float levellerGate READ levellerGate NOTIFY stateChanged)
    Q_PROPERTY(bool levellerLookahead READ levellerLookahead NOTIFY stateChanged)
    Q_PROPERTY(int levellerDetectorMask READ levellerDetectorMask NOTIFY stateChanged)
    Q_PROPERTY(int levellerApplyMask READ levellerApplyMask NOTIFY stateChanged)

    // Psychoacoustic bass parameters
    Q_PROPERTY(float psybassCutoff READ psybassCutoff NOTIFY stateChanged)
    Q_PROPERTY(float psybassHarmonics READ psybassHarmonics NOTIFY stateChanged)
    Q_PROPERTY(float psybassDrive READ psybassDrive NOTIFY stateChanged)
    Q_PROPERTY(float psybassCharacter READ psybassCharacter NOTIFY stateChanged)
    Q_PROPERTY(float psybassOriginal READ psybassOriginal NOTIFY stateChanged)
    Q_PROPERTY(int psybassOutputMask READ psybassOutputMask NOTIFY stateChanged)

    // Core1
    Q_PROPERTY(int core1Mode READ core1Mode NOTIFY stateChanged)

    // -- WP QD17 -- How many input strips the app shows (main.qml binds hostAudio's
    // inputCount here), so pairs, the analyser and filter files count the same inputs
    // as the sidebar. 0 until set: then the device's report, as before.
    Q_PROPERTY(int shownInputCount READ shownInputCount WRITE setShownInputCount NOTIFY statusChanged)
    // -- WP QD37 -- A hub session is up (setHubSession): rescan and reconnect do nothing
    Q_PROPERTY(bool hubSession READ hubSession NOTIFY hubSessionChanged)

public:
    // Demo also offers the built-in demo device, and opens it first (--demo)
    enum class Mode { Hardware, Demo };
    explicit DSPiBridge(Mode mode = Mode::Hardware, QObject *parent = nullptr);
    ~DSPiBridge();

    // Property getters
    bool connected() const;
    QString selectedSerial() const;
    QStringList availableSerials() const;
    int inaccessibleDevices() const { return m_inaccessible; }
    QString platformName() const;
    int numChannels() const;
    int numInputChannels() const;
    int numOutputChannels() const;
    QString firmwareVersion() const;
    int compat() const;
    QString compatMessage() const;
    bool writable() const { return m_writable; }
    int wireFeatures() const;
    // A feature of this firmware's wire format: psybass, upmix, subharm, tube or
    // limiter. Features firmware lacks are hidden.
    Q_INVOKABLE bool firmwareHas(const QString &feature) const;
    float maxDelayMs() const;
    // -- WP DL3 --
    QString delayRefusal() const { return m_delayRefusal; }
    Q_INVOKABLE void dismissDelayRefusal();
    // A pasted delay: the clip's, held to the connected device's line (0 to
    // `line` ms), as the sliders hold theirs. `line` 0 (no device): unchanged.
    static float pastedDelay(float clip, float line);

    float preampDB() const;
    bool bypass() const;

    float masterVolumeDB() const;
    int masterVolumeMode() const;
    float userVolumeDB() const;
    bool userMute() const;

    bool loudnessEnabled() const;
    float loudnessRefSPL() const;
    float loudnessIntensity() const;
    int loudnessOutputMask() const;

    bool crossfeedEnabled() const;
    int crossfeedPreset() const;
    float crossfeedFreq() const;
    float crossfeedFeed() const;
    bool crossfeedITD() const;
    int crossfeedOutputPairMask() const;

    int cpu0() const;
    int cpu1() const;
    int clipFlags() const;
    int activeInputChannels() const;

    int activePresetSlot() const;
    int presetOccupied() const;
    int presetStartupMode() const;
    int presetDefaultSlot() const;
    int outputConfigMode() const;

    int core1Mode() const;
    int inputSource() const;
    QVariantList inputSources() const;   // [{id, name}] available and enabled
    bool levellerEnabled() const;
    bool psybassEnabled() const;
    float levellerAmount() const { return state()->leveller_amount; }
    int levellerSpeed() const { return state()->leveller_speed; }
    float levellerMaxGain() const { return state()->leveller_max_gain_db; }
    float levellerGate() const { return state()->leveller_gate_db; }
    bool levellerLookahead() const { return state()->leveller_lookahead; }
    int levellerDetectorMask() const { return state()->leveller_detector_mask; }
    int levellerApplyMask() const { return state()->leveller_apply_mask; }
    bool upmixSupported() const { return connected() && state()->platform_id == 1 && firmwareHas(QStringLiteral("upmix")); }
    QVariantList upmixParams() const { return floatList(state()->upmix, UPMIX_PARAM_COUNT); }
    QVariantList subharmParams() const { return floatList(state()->subharm, SUBHARM_PARAM_COUNT); }
    bool subharmSolo() const { return state()->subharm_solo; }
    QVariantList tubeParams() const { return floatList(state()->tube, TUBE_PARAM_COUNT); }
    float psybassCutoff() const { return state()->psybass_cutoff_hz; }
    float psybassHarmonics() const { return state()->psybass_harmonics_db; }
    float psybassDrive() const { return state()->psybass_drive_db; }
    float psybassCharacter() const { return state()->psybass_character_pct; }
    float psybassOriginal() const { return state()->psybass_original_db; }
    int psybassOutputMask() const { return state()->psybass_output_mask; }

    // Channel identity (app ids)
    Q_INVOKABLE bool channelExists(int ch) const;
    Q_INVOKABLE bool isInputChannel(int ch) const;
    Q_INVOKABLE int inputAppId(int input) const;   // input index -> app id
    Q_INVOKABLE float peakLevel(int ch) const;
    Q_INVOKABLE bool isClipping(int ch) const;
    Q_INVOKABLE QString channelName(int ch) const;
    Q_INVOKABLE QString channelColor(int ch) const;
    Q_INVOKABLE QString channelDescriptor(int ch) const;
    // -- BUG1 -- What to show for a channel: an input's name; an output's user-set name, else the
    // core's label from the routing and the filters ("Out 1 · R · Low"). channelName stays the
    // stored name, what a rename starts from.
    Q_INVOKABLE QString channelLabel(int ch) const;
    int labelsRevision() const { return m_labelsRevision; }
    // The firmware's name for an output's slot ("I2S 1 L", "PDM"), for a tooltip; "" if absent
    Q_INVOKABLE QString outputSlotName(int output) const;
    // The dashboard's output cards, [{left, right (-1: one output), title, leftName, rightName}]:
    // paired by the crossfeed pair map when the device has one, else by slot
    Q_INVOKABLE QVariantList outputCards() const;

    // PEQ bands (app id, band 0..9)
    Q_INVOKABLE int filterType(int ch, int band) const;
    Q_INVOKABLE float filterFreq(int ch, int band) const;
    Q_INVOKABLE float filterGain(int ch, int band) const;
    Q_INVOKABLE float filterQ(int ch, int band) const;
    Q_INVOKABLE float filterQp(int ch, int band) const;
    Q_INVOKABLE bool filterBypass(int ch, int band) const;

    // Crossover bands (output app id, band 0..3)
    Q_INVOKABLE int crossoverType(int ch, int band) const;
    Q_INVOKABLE float crossoverFreq(int ch, int band) const;
    Q_INVOKABLE bool crossoverBypass(int ch, int band) const;

    // Inputs (input index)
    Q_INVOKABLE float inputPreampDB(int input) const;
    float preampMaxDB() const { return dspi_preamp_max_db(m_core); }
    Q_INVOKABLE float channelDelayMS(int ch) const;

    // Outputs (output index)
    Q_INVOKABLE bool outputEnabled(int idx) const;
    Q_INVOKABLE bool outputMuted(int idx) const;
    Q_INVOKABLE float outputGainDB(int idx) const;
    Q_INVOKABLE float outputDelayMS(int idx) const;
    Q_INVOKABLE bool isPdmOutput(int idx) const;

    // Matrix (input index, output index)
    Q_INVOKABLE bool matrixRouting(int input, int output) const;
    Q_INVOKABLE float matrixGain(int input, int output) const;
    Q_INVOKABLE bool matrixInvert(int input, int output) const;
    Q_INVOKABLE int outputPin(int physOut) const;
    Q_INVOKABLE int numPinOutputs() const;

    Q_INVOKABLE QString presetName(int slot) const;
    Q_INVOKABLE bool isPresetOccupied(int slot) const;

    Q_INVOKABLE QVariantList magnitudeCurve(int ch);

    // Setters
    Q_INVOKABLE void setPreamp(float db);
    Q_INVOKABLE void sendPreampToDevice(float db);
    Q_INVOKABLE void setInputPreamp(int input, float db, bool sendOnly = false);
    Q_INVOKABLE void setBypass(bool en);

    Q_INVOKABLE void setMasterVolume(float db, bool sendOnly = false);
    Q_INVOKABLE bool setMasterVolumeMode(int mode);   // -- WP QD31 -- false if not written
    Q_INVOKABLE int saveMasterVolume();
    Q_INVOKABLE void setUserVolume(float db, bool sendOnly = false);
    Q_INVOKABLE void setUserMute(bool muted);

    Q_INVOKABLE void setFilter(int ch, int band, int type, float freq, float gain, float q);
    Q_INVOKABLE void setLinkwitzTransform(int ch, int band, float f0, float q0, float fp, float qp);
    Q_INVOKABLE void setBandBypass(int ch, int band, bool bypass);
    Q_INVOKABLE void setCrossover(int ch, int band, int type, float freq);
    Q_INVOKABLE void setCrossoverBypass(int ch, int band, bool bypass);
    Q_INVOKABLE void setChannelDelay(int ch, float ms);

    Q_INVOKABLE void setLoudness(bool en);
    Q_INVOKABLE void setLoudnessRef(float spl, bool sendOnly = false);
    Q_INVOKABLE void setLoudnessIntensity(float pct, bool sendOnly = false);
    Q_INVOKABLE void setLoudnessOutputMask(int mask);

    Q_INVOKABLE void setCrossfeed(bool en);
    Q_INVOKABLE void setCrossfeedPreset(int p);
    Q_INVOKABLE void setCrossfeedFreq(float freq, bool sendOnly = false);
    Q_INVOKABLE void setCrossfeedFeed(float feed, bool sendOnly = false);
    Q_INVOKABLE void setCrossfeedITD(bool en);
    Q_INVOKABLE void setCrossfeedOutputPairMask(int mask);

    Q_INVOKABLE void setMatrixRoute(int input, int output, bool enabled, float gain, bool invert);
    Q_INVOKABLE bool setOutputEnable(int output, bool enabled);
    Q_INVOKABLE void setOutputGain(int output, float db);
    Q_INVOKABLE void sendOutputGainToDevice(int output, float db);
    Q_INVOKABLE void setOutputMute(int output, bool muted);
    Q_INVOKABLE void setOutputDelay(int output, float ms);
    Q_INVOKABLE void sendOutputDelayToDevice(int output, float ms);
    Q_INVOKABLE int setOutputPin(int output, int pin);
    Q_INVOKABLE bool setOutputConfigMode(int mode);   // -- WP QD31 -- false if not written
    Q_INVOKABLE int saveOutputConfig();

    Q_INVOKABLE void setChannelName(int ch, const QString &name);
    Q_INVOKABLE void resetChannelNames();   // every channel back to its default name

    Q_INVOKABLE int savePreset(int slot);
    bool presetDirty() const { return m_presetDirty; }
    QString connectionError() const { return m_connectionError; }
    // Drop the connection, rescan USB and reconnect (the same device if it's
    // still there). The open demo stays open, and is read again with its state
    // (WP QD36). Does nothing while a hub session is up (WP QD36), as reconnect().
    Q_INVOKABLE void rescan();
    // Install the udev rule through pkexec (asks for a password); answers
    // with udevRuleInstalled. The hot-plug poll then finds the device.
    Q_INVOKABLE void installUdevRule();
    // What differs from the active preset, one sentence each
    Q_INVOKABLE QStringList presetChanges() const;
    Q_INVOKABLE int loadPreset(int slot);
    Q_INVOKABLE int deletePreset(int slot);
    Q_INVOKABLE int copyPreset(int fromSlot, int toSlot);
    Q_INVOKABLE void setPresetName(int slot, const QString &name);
    Q_INVOKABLE bool setPresetStartup(int mode, int slot);   // -- WP QD31 -- false if not written

    Q_INVOKABLE int saveParams();
    Q_INVOKABLE int loadParams();
    Q_INVOKABLE int factoryReset();
    Q_INVOKABLE void enterBootloader();

    Q_INVOKABLE void clearClips();
    Q_INVOKABLE int checkCore1Conflict(int output);

    Q_INVOKABLE void scanDevices();
    Q_INVOKABLE void selectDevice(const QString &serial);
    Q_INVOKABLE void disconnectDevice();
    Q_INVOKABLE void reconnect();

    Q_INVOKABLE void setInputSource(int source);
    Q_INVOKABLE void setLevellerEnabled(bool en);
    Q_INVOKABLE void setPsybassEnabled(bool en);

    // Leveller (sendOnly = live drag: no stateChanged, avoids feedback)
    Q_INVOKABLE void setLevellerAmount(float pct, bool sendOnly = false);
    Q_INVOKABLE void setLevellerSpeed(int speed);
    Q_INVOKABLE void setLevellerMaxGain(float db, bool sendOnly = false);
    Q_INVOKABLE void setLevellerGate(float db, bool sendOnly = false);
    Q_INVOKABLE void setLevellerLookahead(bool en);
    Q_INVOKABLE void setLevellerMasks(int detector, int apply);

    // Psybass: param 0 cutoff, 1 harmonics, 2 drive, 3 character, 4 original
    Q_INVOKABLE void setPsybassParam(int param, float value, bool sendOnly = false);
    Q_INVOKABLE void setPsybassOutputMask(int mask);
    // ── Hardware IO. Setters return the PIN_CONFIG_* status (255 = no reply)
    QVariantMap hardware() const;
    QVariantList validPins() const;
    bool hardwareUnsaved() const { return m_hardwareUnsaved; }
    // [{pin, owner, role}] for every claimed GPIO; role: output, clock,
    // input, control or other
    Q_INVOKABLE QVariantList pinOwners() const;
    Q_INVOKABLE int setOutputType(int slot, int kind);
    Q_INVOKABLE int setI2sBckPin(int role, int pin);
    Q_INVOKABLE int setI2sClockPinMode(int mode);
    Q_INVOKABLE int setMckEnabled(bool enabled);
    Q_INVOKABLE int setMckPin(int pin);
    Q_INVOKABLE int setMckMultiplier(int mult);
    Q_INVOKABLE int setAdatOutEnabled(bool enabled);
    Q_INVOKABLE int setAdatOutPin(int pin);
    Q_INVOKABLE int setSpdifRxPin(int index, int pin);
    Q_INVOKABLE int setSpdifInputEnabled(int index, bool enabled);
    Q_INVOKABLE int setI2sRxPin(int pair, int pin);
    Q_INVOKABLE int setI2sInputChannels(int channels);
    Q_INVOKABLE int setAdatInputEnabled(bool enabled);
    Q_INVOKABLE int setAdatInputPin(int pin);
    Q_INVOKABLE int setAdatInputClockMode(int mode);
    Q_INVOKABLE void setInputRate(int index);
    Q_INVOKABLE void setI2sClockMode(int mode);
    Q_INVOKABLE void setLgSoundSync(bool enabled);
    // DAC mute: applied by the device later; dacMuteApplied reports the outcome.
    // -- WP QD31 -- false if not sent, and dacMuteApplied(false) has fired
    Q_INVOKABLE bool setDacMute(bool enabled, bool activeLow, int pin, int holdMs, int releaseMs);
    Q_INVOKABLE int testDacMute();
    // Control interfaces: ctrlIfaceApplied(which 0 = UART / 1 = I2C, status)
    Q_INVOKABLE void setUart(bool enabled, int txPin, int rxPin, bool notify, int baud);
    Q_INVOKABLE void setI2c(bool enabled, int sdaPin, int sclPin, int address);
    Q_INVOKABLE void refreshCtrlIfaces();
    // Live readings for the pages that show them
    Q_INVOKABLE QVariantMap fetchInputLock(int which);   // 0 I2S slave, 1 ADAT in
    Q_INVOKABLE QVariantMap fetchAdatOutStatus();
    Q_INVOKABLE QVariantMap fetchLgStatus();
    Q_INVOKABLE int fetchInputRate();

    Q_INVOKABLE void setUpmixParam(int id, float value, bool sendOnly = false);
    Q_INVOKABLE void setSubharmParam(int id, float value, bool sendOnly = false);
    Q_INVOKABLE void setSubharmSolo(bool solo);
    Q_INVOKABLE void setTubeParam(int idx, float value, bool sendOnly = false);
    // Live readings, polled by the tool windows while they are open
    Q_INVOKABLE QVariantMap fetchUpmixStatus();
    Q_INVOKABLE QVariant fetchSubharmHeadroom();          // dB, undefined on failure
    Q_INVOKABLE QVariantList fetchSubharmMeter();         // 0..1 per output

    // Matrix helpers
    Q_INVOKABLE void directRouting();          // input k -> output k at 0 dB, PDM off
    Q_INVOKABLE void clearRouting();           // disconnect all, keep gains/polarity
    // Outputs that would be switched off to enable `output` (PDM vs Core 1 EQ worker)
    Q_INVOKABLE QVariantList core1ConflictOutputs(int output) const;
    Q_INVOKABLE void enableOutputResolvingConflict(int output);

    // Input stereo-pair links (IN1/2, IN3/4, ...). A linked pair mirrors
    // preamp and filter edits. Every pair starts unlinked; links are
    // remembered per device and pause while fewer inputs are live.
    Q_INVOKABLE int liveInputCount() const;
    int shownInputCount() const { return m_shownInputs; }   // -- WP QD17 --
    void setShownInputCount(int n);
    Q_INVOKABLE int pairPartner(int ch) const;     // the other input of ch's pair, or -1
    Q_INVOKABLE bool pairAvailable(int ch) const;  // both inputs of the pair are live
    Q_INVOKABLE bool isInputLinked(int ch) const;  // link set and pair live
    Q_INVOKABLE int linkedPartner(int ch) const;   // partner while linked, else -1
    Q_INVOKABLE bool inputPairMatches(int ch) const;
    // keepCh: app id whose settings are copied onto the other input, or -1
    Q_INVOKABLE void setInputLinked(int ch, bool linked, int keepCh);

    // Whole-list band operations (crossover = the XO tab of an output)
    Q_INVOKABLE void clearPeq(int ch);
    Q_INVOKABLE void clearAllBands(int ch, bool crossover);
    Q_INVOKABLE void setAllBandsBypass(int ch, bool bypass, bool crossover);
    Q_INVOKABLE bool canSetAllBypass(int ch, bool bypass, bool crossover) const;

    // Output limiter (output index)
    Q_INVOKABLE bool limiterEnabled(int out) const;
    Q_INVOKABLE float limiterThreshold(int out) const;
    Q_INVOKABLE float limiterRelease(int out) const;
    Q_INVOKABLE int limiterLinkGroup(int out) const;
    Q_INVOKABLE float limiterReduction(int out) const;   // dB, > 0 while limiting
    Q_INVOKABLE void setLimiterEnabled(int out, bool en);
    Q_INVOKABLE void setLimiterThreshold(int out, float db, bool sendOnly = false);
    Q_INVOKABLE void setLimiterRelease(int out, float ms, bool sendOnly = false);
    Q_INVOKABLE void setLimiterLinkGroup(int out, int group);
    Q_INVOKABLE void copyLimiterToAll(int out);
    Q_INVOKABLE void linkAllLimiterPairs();
    Q_INVOKABLE void unlinkAllLimiters();
    Q_INVOKABLE void disableAllLimiters();

    // Channel clipboard (Copy / Paste Parameters)
    Q_INVOKABLE void copyChannel(int ch);
    Q_INVOKABLE bool canPaste() const;
    Q_INVOKABLE void pasteChannel(int ch);

    // Open the demo device as firmware of this wire format from now on (0 is its
    // own, V32). For the read-only tests: selectDevice uses it.
    void setDemoWire(int wire) { m_demoWire = wire; }

    // The core, for the spectrum analyser controller
    FfiCore *core() const { return m_core; }

    // -- WP QD34 -- For the smoke driver: runs on the status worker before each
    // status read, to show the read is off the GUI thread and may be slow.
    void setStatusReadHook(std::function<void()> hook) { m_statusReadHook = std::move(hook); }

    // For configuration and filter file imports
    uint8_t inputLinks() const { return m_links; }
    void restoreInputLinks(uint8_t links);
    void markHardwareUnsaved();
    // Everything changed outside the setters: redraw from the core's state. Also
    // for after a Link reconnect, which reads the device again by itself.
    void refreshAll();

    // A device somebody else opened and read on this core (a Link device): show it,
    // as selectDevice shows what it opens. No traffic.
    void adoptDevice(const QString &serial);
    // A hub session is up, bound or still choosing: scans keep to themselves, so a
    // USB device doesn't take over from it. An explicit selectDevice still works.
    // On the way down, a device the session left closed goes from the UI.
    void setHubSession(bool active);
    // -- WP QD11 -- The firmware updater: a hub's device can't reach this machine's USB
    bool hubSession() const { return m_hubSession; }

    // Magnitude curve access for C++ BodePlotItem (app ids)
    void getMagnitudeCurve(int ch, double *out);
    // Phase (degrees) of a channel's PEQ and crossover bands, 201 points
    void getPhaseCurve(int ch, bool unwrap, double *out);

    // ── On-graph editor support (C++ only) ──
    // The channel's PEQ bands (BANDS_PER_CHANNEL) and, for an output, its
    // crossover bands; false if the channel doesn't exist
    bool channelBands(int ch, FilterParams *bands, FilterParams *xover, int *xoverCount) const;
    // dB the graph adds to the channel's curve (output gain, input preamp)
    float channelGainOffset(int ch) const;
    // Mid-drag: device (and linked partner) only, no signals
    void sendBandLive(int ch, int band, const FilterParams &p);
    // Release: write the bands once, then refresh everything
    void commitBands(int ch, const QVector<int> &bands, const QVector<FilterParams> &params);
    bool isMagnitudeDirty(int ch) const;
    void clearMagnitudeDirty(int ch);

    // Channel visibility for graph (app ids)
    Q_INVOKABLE bool channelVisible(int ch) const;
    Q_INVOKABLE void setChannelVisible(int ch, bool visible);

signals:
    // About to open, reopen or drop a device, before any of it happens. A
    // controller with writes queued stops them here: the next device gets none.
    void deviceChanging();
    void writableChanged();
    void stateChanged();
    void statusChanged();
    void devicesChanged();
    void inaccessibleDevicesChanged();
    void hubSessionChanged();   // -- WP QD37 --
    void delayRefusalChanged();  // -- WP DL3 --
    void udevRuleInstalled(bool ok, const QString &message);
    // The signal generator started or stopped (on the device or elsewhere)
    void siggenNotified();
    void presetDirtyChanged();
    // An aux output changed, or IR learning finished
    void csNotified(bool aux, bool learn);
    // A batch of device notifications arrived (the Interrupt Monitor reads them)
    void notificationsArrived();
    void deviceArrived(const QString &serial);
    void deviceDeparted(const QString &serial);
    void magnitudesChanged();
    // -- BUG1 -- an output's label or the dashboard's cards changed: after stateChanged, which
    // QML handlers receive before the bridge has read the new labels
    void outputLabelsChanged();
    void dacMuteApplied(bool accepted);
    void ctrlIfaceApplied(int which, int status);
    // A value shown on the graph changed mid-drag (output gain): redraw
    // straight away, without the full stateChanged refresh
    void previewChanged();

private slots:
    void pollStatus();
    void pollHotplug();
    void processNotifications();

private:
    FfiCore *m_core = nullptr;
    SystemStatus m_status = {};
    QString m_selectedSerial;
    QStringList m_availableSerials;
    int m_inaccessible = 0;
    bool m_demo = false;
    int m_demoWire = 0;
    int m_shownInputs = 0;   // -- WP QD17 --
    bool m_writable = false;
    void updateWritable();
    bool m_hubSession = false;
    void resetForDevice(const QString &serial);
    // -- WP QD36 -- selectDevice, or with keepDemo a read of the demo already open
    void openDevice(const QString &serial, bool keepDemo);
    void showOpenedDevice();
    QStringList listSerials();
    void updateInaccessible();
    QTimer *m_statusTimer = nullptr;
    QTimer *m_hotplugTimer = nullptr;
    // Device notifications are applied at most every 30 ms (a turning
    // hardware knob sends many), each batch with one stateChanged
    QTimer *m_notifyTimer = nullptr;
    bool m_hardwareUnsaved = false;
    // -- BUG1 -- the core's output labels and cards, read again after a stateChanged (never per frame)
    void refreshOutputLabels() const;
    void readOutputLabels();
    mutable bool m_labelsStale = true;
    int m_labelsRevision = 0;
    mutable QStringList m_outputLabels;
    mutable QStringList m_outputSlots;
    mutable QVariantList m_outputCards;
    bool m_presetDirty = false;
    QString m_connectionError;
    void updatePresetDirty();
    int hardwareEdited(int status);   // marks unsaved edits, emits, returns status
    QElapsedTimer m_lastNotify;

    // Magnitude caching (app ids)
    bool m_magnitudeDirty[kAppChannelCount] = {};
    double m_magnitudeCache[kAppChannelCount][MAGNITUDE_POINTS] = {};
    bool m_magnitudeValid[kAppChannelCount] = {};

    // Channel visibility (app ids)
    bool m_channelVisible[kAppChannelCount];

    // The core's state as of this call. The record stays where it is but is a
    // snapshot: re-read it after a write. GUI thread only (each call rewrites it).
    const DspState *state() const;
    static QVariantList floatList(const float *v, int n);
    bool usable() const;              // connected to compatible firmware
    int wire(int appCh) const;        // app id -> wire channel, -1 if absent
    int appId(int wireCh) const;      // wire channel -> app id, -1 if absent
    int outputOf(int appCh) const;    // output index of an output app id, else -1
    int inputOf(int appCh) const;     // input index of an input app id, else -1
    void markAllDirty();
    void markDirty(int appCh);
    void computeCurve(int appCh);

    void applyFilter(int ch, int band, const FilterParams &p);
    void copyInputSettings(int from, int to);
    int pairIndex(int ch) const;                  // 0..3 for an input, else -1
    void loadLinks();
    void saveLinks();
    uint8_t m_links = 0x00;                       // bit p: input pair p linked (none by default)
    float m_limiterGR[MAX_OUTPUTS] = {};
    qint64 m_lastClipMs[MAX_CHANNELS] = {};      // per wire channel, 0 = never
    int m_pollTick = 0;
    // -- WP QD34 -- The status reads run on m_pollWorker, one at a time. A reply from
    // before a device change, or clip flags read before clearClips, are dropped.
    struct StatusRead;
    void applyStatus(const StatusRead &r, bool anyLimiter, bool readLimiter);
    QThreadPool m_pollWorker;
    bool m_statusBusy = false;
    unsigned m_statusGeneration = 0, m_clipEpoch = 0;
    std::function<void()> m_statusReadHook;

    struct ChannelClip {
        bool valid = false;
        bool output = false;
        FilterParams filters[BANDS_PER_CHANNEL];
        FilterParams xover[MAX_XOVER_BANDS];
        float gain = 0, delay = 0;
        bool mute = false;
    } m_clip;

    // -- WP DL3 -- Every delay write: `path` ch.delay or out.delay through the
    // core's write_param, so a refusal comes back in the core's words
    // (delayRefusal). False when not written.
    bool writeDelay(const char *path, int index, float ms);
    void setDelayRefusal(const QString &words);
    QString m_delayRefusal;

    static void hotplugCallback(uint8_t event, const char *serial, void *userData);
    static void notifyCallback(void *userData);
    void scheduleNotifications();
};

#endif // DSPIBRIDGE_H
