// -- WP QP11 -- The hardware settings pages and the save bar (HardwareController.h).
#include "HardwareController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "NamesController.h"
#include "OutputLatencyController.h"
#include "PinMap.h"
#include "Poller.h"
#include "SettingsController.h"

#include <QSet>
#include <QTimer>
#include <algorithm>
#include <array>
#include <cstring>

static const Registrar::Controller<HardwareController> hardwareController;

// The pages, in the Mac's System and Control groups; every row label is a
// search term, with the words people look for
static const SettingsRegistry::Page inputsPage{
    "inputs", SettingsRegistry::System, 20, QT_TRANSLATE_NOOP("SettingsPages", "Inputs"), "arrow-down-to-line",
    "InputsPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "S/PDIF Input"), QT_TRANSLATE_NOOP("SettingsPages", "Instances"),
      QT_TRANSLATE_NOOP("SettingsPages", "LG Sound Sync"), QT_TRANSLATE_NOOP("SettingsPages", "I2S Input"),
      QT_TRANSLATE_NOOP("SettingsPages", "Clock Mode"), QT_TRANSLATE_NOOP("SettingsPages", "Channels"),
      QT_TRANSLATE_NOOP("SettingsPages", "Serial Data"), QT_TRANSLATE_NOOP("SettingsPages", "ADAT Input"),
      QT_TRANSLATE_NOOP("SettingsPages", "Lock Status"), QT_TRANSLATE_NOOP("SettingsPages", "Source Names"),
      QT_TRANSLATE_NOOP("SettingsPages", "TOSLINK"), QT_TRANSLATE_NOOP("SettingsPages", "pin"),
      QT_TRANSLATE_NOOP("SettingsPages", "GPIO") } };
static const SettingsRegistry::Page outputsPage{
    "outputs", SettingsRegistry::System, 30, QT_TRANSLATE_NOOP("SettingsPages", "Outputs"), "cable", "OutputsPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Slots"), QT_TRANSLATE_NOOP("SettingsPages", "Subwoofer"),
      QT_TRANSLATE_NOOP("SettingsPages", "S/PDIF"), QT_TRANSLATE_NOOP("SettingsPages", "I2S"),
      QT_TRANSLATE_NOOP("SettingsPages", "PDM"), QT_TRANSLATE_NOOP("SettingsPages", "Bulk Output"),
      QT_TRANSLATE_NOOP("SettingsPages", "Enable ADAT"), QT_TRANSLATE_NOOP("SettingsPages", "Reset Pins"),
      QT_TRANSLATE_NOOP("SettingsPages", "pin"), QT_TRANSLATE_NOOP("SettingsPages", "GPIO") } };
static const SettingsRegistry::Page i2sPage{
    "i2s", SettingsRegistry::System, 40, QT_TRANSLATE_NOOP("SettingsPages", "I2S Configuration"), "waves-arrow-down",
    "I2sPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "BCK Pin"), QT_TRANSLATE_NOOP("SettingsPages", "Clock Pins"),
      QT_TRANSLATE_NOOP("SettingsPages", "Slave BCK Pin"), QT_TRANSLATE_NOOP("SettingsPages", "Master Clock (MCK)"),
      QT_TRANSLATE_NOOP("SettingsPages", "MCK Pin"), QT_TRANSLATE_NOOP("SettingsPages", "MCK Multiplier"),
      QT_TRANSLATE_NOOP("SettingsPages", "Input Sample Rate"), QT_TRANSLATE_NOOP("SettingsPages", "LRCK"),
      QT_TRANSLATE_NOOP("SettingsPages", "pin"), QT_TRANSLATE_NOOP("SettingsPages", "GPIO") } };
static const SettingsRegistry::Page globalPage{
    "global", SettingsRegistry::System, 50, QT_TRANSLATE_NOOP("SettingsPages", "Global Parameters"), "hard-drive",
    "GlobalPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Device Name"), QT_TRANSLATE_NOOP("SettingsPages", "Latency"),
      QT_TRANSLATE_NOOP("SettingsPages", "Output latency"), QT_TRANSLATE_NOOP("SettingsPages", "At power-on"),
      QT_TRANSLATE_NOOP("SettingsPages", "Startup Preset"), QT_TRANSLATE_NOOP("SettingsPages", "Default Preset"),
      QT_TRANSLATE_NOOP("SettingsPages", "Master Volume"), QT_TRANSLATE_NOOP("SettingsPages", "Hardware Configuration"),
      QT_TRANSLATE_NOOP("SettingsPages", "External Mute Control"), QT_TRANSLATE_NOOP("SettingsPages", "Mute Pin"),
      QT_TRANSLATE_NOOP("SettingsPages", "DAC mute"), QT_TRANSLATE_NOOP("SettingsPages", "boot") } };
static const SettingsRegistry::Page interfacesPage{
    "interfaces", SettingsRegistry::Control, 20, QT_TRANSLATE_NOOP("SettingsPages", "Control Interfaces"), "cpu",
    "InterfacesPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "UART"), QT_TRANSLATE_NOOP("SettingsPages", "I2C"),
      QT_TRANSLATE_NOOP("SettingsPages", "Baud Rate"), QT_TRANSLATE_NOOP("SettingsPages", "Target Address"),
      QT_TRANSLATE_NOOP("SettingsPages", "Push Notifications"), QT_TRANSLATE_NOOP("SettingsPages", "serial"),
      QT_TRANSLATE_NOOP("SettingsPages", "pin"), QT_TRANSLATE_NOOP("SettingsPages", "GPIO") } };
// The bar along the bottom of every page
static const SettingsRegistry::Footer saveBar{ 10, "SettingsSaveBar" };

namespace {

constexpr int kNoPin = 0xFF;
constexpr int kAdatOutDefault = 12;      // the Mac's ADAT_PIN_DEFAULT
constexpr int kMuteDelay = 150;          // ms before the external mute reads back
constexpr int kInterfaceDelay = 250;     // ms before a control interface reads back
constexpr std::array<int, 4> kDefaultDataPins{ 6, 7, 8, 9 };
constexpr int kDefaultSubPin = 10;
constexpr std::array<int, 3> kInputRates{ 44100, 48000, 96000 };

enum Source { SourceUsb = 0, SourceSpdif = 1, SourceI2s = 2, SourceAdat = 3 };

bool validPin(int pin) { return (pin >= 0 && pin <= 22) || (pin >= 26 && pin <= 28); }

QString gpio(int pin) { return HardwareController::tr("GPIO %1").arg(pin); }

QString khz(uint32_t hz) {
    if (hz == 0) return QStringLiteral("-");
    return HardwareController::tr("%1 kHz").arg(double(hz) / 1000.0, 0, 'f', 1);
}

// A lock status for the page: the Mac's I2sSlaveStatus / AdatInputStatus texts
QVariantMap lockMap(const InputLockStatus &s, bool adat) {
    QString text;
    switch (s.state) {
    case 0: text = HardwareController::tr("Inactive"); break;
    case 1: text = HardwareController::tr("Acquiring"); break;
    case 2: text = adat ? HardwareController::tr("Syncing") : HardwareController::tr("Relocking"); break;
    case 3: text = HardwareController::tr("Locked"); break;
    case 4: text = adat ? HardwareController::tr("Relocking") : HardwareController::tr("Unknown (%1)").arg(s.state); break;
    default: text = HardwareController::tr("Unknown (%1)").arg(s.state); break;
    }
    // 0 grey, 1 waiting, 2 relocking, 3 locked (the page picks the tokens)
    int tone = 0;
    if (s.state == 3) tone = 3;
    else if (s.state == 1 || (adat && s.state == 2)) tone = 1;
    else if ((!adat && s.state == 2) || (adat && s.state == 4)) tone = 2;
    const bool locked = s.state == 3;
    return { { QStringLiteral("state"), s.state },
             { QStringLiteral("text"), text },
             { QStringLiteral("tone"), tone },
             { QStringLiteral("locked"), locked },
             { QStringLiteral("detected"), (adat || locked) ? khz(s.detected_rate) : QStringLiteral("-") },
             { QStringLiteral("measured"), khz(s.measured_hz) } };
}

QVariantList boolList(uint8_t mask, int count) {
    QVariantList out;
    for (int i = 0; i < count; ++i) out.append((mask & (1u << i)) != 0);
    return out;
}

// The claims of everyone but `consumer`: its own pins go from a copy of the
// state, so PinMap's rules (QP10's, after the core's) decide the rest.
QSet<int> heldBy(const DspState &state, const QList<PinMap::SurfacePin> &surfaces, int consumer, int index,
                 QHash<int, QString> *labels = nullptr) {
    auto copy = std::make_unique<DspState>(state);
    DspState &s = *copy;
    switch (consumer) {
    case HardwareController::OutputPin:
        if (index >= 0 && index < MAX_PHYSICAL_OUTPUTS) s.output_pins[index] = kNoPin;
        break;
    case HardwareController::I2sBck: s.i2s_bck_pin = kNoPin; break;
    case HardwareController::I2sBckSlave: s.i2s_bck_pin_slave = 0; break;
    case HardwareController::Mck: s.mck_enabled = false; break;
    case HardwareController::SpdifRx:
        if (index >= 0 && index < 4) s.spdif_rx_pins[index] = kNoPin;
        break;
    case HardwareController::I2sRx:
        if (index >= 0 && index < 4) s.i2s_rx_pins[index] = kNoPin;
        break;
    case HardwareController::AdatOut: s.adat_out_enabled = false; break;
    case HardwareController::AdatIn:
        // The ADAT output's pin is allowed: a loopback self-test
        s.adat_input_enabled = false;
        s.adat_out_enabled = false;
        break;
    case HardwareController::DacMute: s.dac_mute_enabled = false; break;
    case HardwareController::UartCtrl: s.ctrl_status.uart_live = false; break;
    case HardwareController::I2cCtrl: s.ctrl_status.i2c_live = false; break;
    default: break;
    }
    QSet<int> held;
    for (const PinMap::Claim &c : PinMap::claims(s, surfaces)) {
        held.insert(c.pin);
        if (labels) labels->insert(c.pin, c.label);
    }
    return held;
}

// -- WP QPI2 -- an unset pin (255) reads "Not set", as the picker's own line
QVariantMap choice(int pin) {
    return { { QStringLiteral("pin"), pin },
             { QStringLiteral("text"), pin >= 0 && pin < 255 ? gpio(pin) : HardwareController::tr("Not set") } };
}

} // namespace

// ---- the bar's built-in parts --------------------------------------------------

// Global Parameters: the held draft, written on Save
class HardwareController::GlobalPart : public SaveParticipant {
public:
    explicit GlobalPart(HardwareController *h) : h(h) {}
    bool hasPending() const override { return h->globalDirty(); }
    void save(std::function<void(const QStringList &)> done) override;
    void revert() override {
        h->m_globalEdited = false;
        emit h->draftChanged();
    }
    HardwareController *h;
};

// Pins, types and clocks: live on the device, kept by a flash save
class HardwareController::OutputPart : public SaveParticipant {
public:
    explicit OutputPart(HardwareController *h) : h(h) {}
    bool hasPending() const override { return h->outputDirty(); }
    void save(std::function<void(const QStringList &)> done) override {
        auto status = std::make_shared<int>(-1);
        h->m_ctx.device->write(
            QStringLiteral("save output configuration"),
            [status](FfiCore *c) {
                *status = dspi_save_output_config(c);
                return *status == 0;
            },
            [this, status, done](bool ok) {
                if (ok) h->m_outputEdited = false;
                done(ok ? QStringList() : QStringList{ HardwareController::tr("the output configuration") });
            });
    }
    void revert() override { h->restoreOutputs(); }
    HardwareController *h;
};

void HardwareController::GlobalPart::save(std::function<void(const QStringList &)> done) {
    const Draft want = h->m_draft;
    const Draft have = h->deviceDraft();
    const bool muteSupported = h->m_has.value(QStringLiteral("dacMute")).toBool();
    const bool sendMute = muteSupported
                          && (want.muteEnabled != have.muteEnabled || want.muteActiveLow != have.muteActiveLow
                              || want.mutePin != have.mutePin || want.muteHoldMs != have.muteHoldMs
                              || want.muteReleaseMs != have.muteReleaseMs);
    // Which setters the device refused, in the order they ran
    auto refused = std::make_shared<QStringList>();
    h->m_ctx.device->write(
        QStringLiteral("save global parameters"),
        [want, have, sendMute, refused](FfiCore *c) {
            if (want.startupMode != have.startupMode || want.defaultSlot != have.defaultSlot) {
                if (!dspi_set_preset_startup(c, uint8_t(want.startupMode), uint8_t(want.defaultSlot)))
                    refused->append(HardwareController::tr("the startup preset"));
            }
            if (want.masterVolumeMode != have.masterVolumeMode
                && !dspi_set_master_volume_mode(c, uint8_t(want.masterVolumeMode)))
                refused->append(HardwareController::tr("the master volume mode"));
            if (want.outputConfigMode != have.outputConfigMode
                && !dspi_set_output_config_mode(c, uint8_t(want.outputConfigMode)))
                refused->append(HardwareController::tr("the hardware configuration mode"));
            if (sendMute
                && !dspi_set_dac_mute(c, want.muteEnabled, want.muteActiveLow, uint8_t(want.mutePin),
                                      uint16_t(want.muteHoldMs), uint16_t(want.muteReleaseMs)))
                refused->append(HardwareController::tr("the external mute settings"));
            return refused->isEmpty();
        },
        [this, want, sendMute, refused, done](bool) {
            if (!sendMute || refused->contains(HardwareController::tr("the external mute settings"))) {
                if (refused->isEmpty()) h->m_globalEdited = false;
                done(*refused);
                return;
            }
            // The firmware applies the mute line later and drops a bad one
            // silently: read it back before calling it saved
            QTimer::singleShot(kMuteDelay, h, [this, want, refused, done]() {
                h->m_ctx.device->write(
                    QStringLiteral("external mute read back"), [](FfiCore *c) { return dspi_fetch_dac_mute(c); },
                    [this, want, refused, done](bool) {
                        const Draft now = h->deviceDraft();
                        if (now.muteEnabled != want.muteEnabled || now.muteActiveLow != want.muteActiveLow
                            || (want.muteEnabled && now.mutePin != want.mutePin)
                            || now.muteHoldMs != want.muteHoldMs || now.muteReleaseMs != want.muteReleaseMs)
                            refused->append(HardwareController::tr("the external mute settings"));
                        if (refused->isEmpty()) h->m_globalEdited = false;
                        done(*refused);
                    });
            });
        });
}

// ---- the controller ------------------------------------------------------------------

HardwareController::HardwareController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_globalPart(new GlobalPart(this)), m_outputPart(new OutputPart(this)) {
    m_parts.push_back({ m_globalPart, 10 });
    m_parts.push_back({ m_outputPart, 30 });
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, &HardwareController::onSnapshot);

    // While a page shows: the live rate and the two lock states
    struct Live {
        uint32_t rate = 0;
        bool i2sOk = false, adatOk = false;
        InputLockStatus i2s{}, adat{};
    };
    // The lock states only while their input runs: firmware without them
    // refuses the read
    auto wantI2s = m_wantI2sLock, wantAdat = m_wantAdatLock;
    m_lockPoll = m_ctx.poller->add(
        QStringLiteral("hardware settings"), 1000, this,
        [wantI2s, wantAdat](FfiCore *c) {
            Live l;
            l.rate = dspi_fetch_input_rate(c);
            if (wantI2s->load()) l.i2sOk = dspi_fetch_input_lock(c, 0, &l.i2s);
            if (wantAdat->load()) l.adatOk = dspi_fetch_input_lock(c, 1, &l.adat);
            return l;
        },
        [this](const Live &l) {
            m_liveRate = int(l.rate);
            m_i2sLock = l.i2sOk ? lockMap(l.i2s, false) : QVariantMap();
            m_adatLock = l.adatOk ? lockMap(l.adat, true) : QVariantMap();
            emit liveChanged();
        });
    m_ctx.poller->setEnabled(m_lockPoll, false);
    // Not now: other controllers are still being made
    QTimer::singleShot(0, this, &HardwareController::onSnapshot);
}

// This package's other parts of the bar, once the controllers exist
void HardwareController::joinParts() {
    if (m_partsJoined) return;
    auto *latency = Registrar::instance<OutputLatencyController>();
    auto *names = Registrar::instance<NamesController>();
    if (!latency || !names) return;
    m_partsJoined = true;
    addSaveParticipant(latency, 20);
    addSaveParticipant(names, 90);
}

HardwareController::~HardwareController() {
    delete m_globalPart;
    delete m_outputPart;
}

bool HardwareController::connected() const { return m_ctx.device->connected() && m_ctx.device->state(); }
bool HardwareController::writable() const { return connected() && !m_ctx.device->readOnly(); }
bool HardwareController::rp2040() const { return m_ctx.device->platformName() == QLatin1String("RP2040"); }
bool HardwareController::stm32() const { return m_ctx.device->platformName() == QLatin1String("STM32H723"); }

void HardwareController::onSnapshot() {
    joinParts();
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    // A different device: what was held belongs to the old one (the Mac's
    // resetForNewDevice); a re-plug of the same unit keeps it
    if (snap.connected && !m_ctx.device->serial().isEmpty() && m_ctx.device->serial() != m_serial) {
        const bool first = m_serial.isEmpty();
        m_serial = m_ctx.device->serial();
        if (!first) {
            m_globalEdited = false;
            m_outputEdited = false;
            m_outputBaseline.reset();
            m_saveFailure.clear();
            m_uartOutcome.clear();
            m_i2cOutcome.clear();
            setStatus(QString(), false);
            emit draftChanged();
            emit interfacesChanged();
        }
    }
    if (snap.connected && snap.generation != m_generation) {
        m_generation = snap.generation;
        readFeatures();
    } else if (!snap.connected && m_generation != 0) {
        m_generation = 0;
        m_has.clear();
    }
    readState();
    readSurfaces();
    m_ctx.poller->setEnabled(m_lockPoll, m_watchers > 0 && connected());
    ++m_rev;
    emit changed();
    if (!m_globalEdited) emit draftChanged();
    pendingMaybeChanged();
}

void HardwareController::readFeatures() {
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            QVariantMap f;
            for (const char *name : { "input_source", "spdif_multi_input", "i2s_input_channels", "i2s_slave_clock",
                                      "i2s_clock_pin_mode", "adat_output", "adat_input", "lg_sound_sync",
                                      "dac_hardware_mute", "uart_control", "i2c_control" })
                f.insert(QString::fromLatin1(name), dspi_feature_present(c, name));
            return f;
        },
        [this](const QVariantMap &f) {
            const DspState *s = m_ctx.device->state();
            const bool rp40 = rp2040();
            auto on = [&f](const char *name) { return f.value(QString::fromLatin1(name)).toBool(); };
            m_has = {
                { QStringLiteral("inputSource"), on("input_source") },
                { QStringLiteral("multiSpdif"), on("spdif_multi_input") },
                { QStringLiteral("i2sInput"), on("i2s_input_channels") },
                { QStringLiteral("i2sClockMode"), on("i2s_slave_clock") },
                { QStringLiteral("i2sClockPinMode"), on("i2s_clock_pin_mode") },
                // Both ADAT engines are RP2350-only (RP2040 answers zeros)
                { QStringLiteral("adatOut"), on("adat_output") && !rp40 },
                { QStringLiteral("adatIn"), on("adat_input") && !rp40 },
                { QStringLiteral("lgSoundSync"), on("lg_sound_sync") },
                { QStringLiteral("dacMute"), on("dac_hardware_mute") || (s && s->dac_mute_supported) },
                { QStringLiteral("interfaces"),
                  on("uart_control") || on("i2c_control") || (s && s->ctrl_iface_supported) },
            };
            // Pages the device lacks leave the sidebar (the Mac's isAvailable)
            if (auto *settings = Registrar::instance<SettingsController>()) {
                settings->setPageAvailable(QStringLiteral("inputs"), on("input_source"));
                settings->setPageAvailable(QStringLiteral("i2s"), !stm32());
                settings->setPageAvailable(QStringLiteral("interfaces"),
                                           m_has.value(QStringLiteral("interfaces")).toBool());
            }
            // What the snapshot says depends on what the device has
            readState();
            ++m_rev;
            emit changed();
        });
}

void HardwareController::readState() {
    const DspState *s = m_ctx.device->state();
    if (!s) {
        m_hw.clear();
        return;
    }
    const bool rp40 = rp2040();
    const int slotCount = rp40 ? 2 : 4;
    const int pdm = rp40 ? 2 : 4;

    QVariantList outputs;
    bool anyI2s = false;
    for (int slot = 0; slot < slotCount; ++slot) {
        const int type = s->output_types[slot];
        anyI2s = anyI2s || type == 1;
        const int pin = s->output_pins[slot];
        outputs.append(QVariantMap{
            { QStringLiteral("id"), slot },
            { QStringLiteral("slot"), true },
            { QStringLiteral("title"), tr("OUT %1/%2").arg(slot * 2 + 1).arg(slot * 2 + 2) },
            { QStringLiteral("name"), tr("Output %1").arg(slot + 1) },
            { QStringLiteral("type"), type },
            { QStringLiteral("pin"), pin },
            { QStringLiteral("channel"), slot * 2 + 1 },
            { QStringLiteral("isDefault"), type == 0 && (stm32() || pin == kDefaultDataPins[size_t(slot)]) },
            // STM32H723: a slot's I2S needs its parent's (SAI clocks are shared)
            { QStringLiteral("i2sAllowed"),
              !stm32() || slot == 0 || (slot <= 2 ? s->output_types[0] == 1 : s->output_types[2] == 1) },
        });
    }
    outputs.append(QVariantMap{
        { QStringLiteral("id"), pdm },
        { QStringLiteral("slot"), false },
        { QStringLiteral("title"), tr("Sub") },
        { QStringLiteral("name"), tr("Subwoofer") },
        { QStringLiteral("type"), 0 },
        { QStringLiteral("pin"), int(s->output_pins[pdm]) },
        { QStringLiteral("channel"), -1 },
        { QStringLiteral("isDefault"), stm32() || s->output_pins[pdm] == kDefaultSubPin },
        { QStringLiteral("i2sAllowed"), false },
    });

    const bool multi = m_has.value(QStringLiteral("multiSpdif")).toBool();
    const int spdifCount = multi ? 4 : 1;
    int spdifEnabledCount = 1;
    for (int i = spdifCount - 1; i >= 1; --i) {
        if (s->spdif_inputs_enabled & (1u << i)) {
            spdifEnabledCount = i + 1;
            break;
        }
    }
    QVariantList spdifPins, i2sRxPins;
    for (int i = 0; i < 4; ++i) {
        spdifPins.append(int(s->spdif_rx_pins[i]));
        i2sRxPins.append(int(s->i2s_rx_pins[i]));
    }
    const int i2sChannels = std::max<int>(2, s->i2s_input_channels);
    const int maxPairs = rp40 ? 1 : 4;
    const int rateCode = std::min<int>(s->i2s_input_rate, int(kInputRates.size()) - 1);
    const int adatInPin = s->adat_input_pin == 0 ? kNoPin : s->adat_input_pin;

    m_wantI2sLock->store(m_has.value(QStringLiteral("i2sClockMode")).toBool() && s->i2s_clock_mode == 1
                         && s->input_source == SourceI2s);
    m_wantAdatLock->store(m_has.value(QStringLiteral("adatIn")).toBool() && s->input_source == SourceAdat);
    m_hw = {
        { QStringLiteral("outputs"), outputs },
        { QStringLiteral("numSlots"), slotCount },
        { QStringLiteral("anySlotIsI2s"), anyI2s },
        { QStringLiteral("i2sBckPin"), int(s->i2s_bck_pin) },
        { QStringLiteral("i2sBckPinSlave"), int(s->i2s_bck_pin_slave) },
        { QStringLiteral("clockPinMode"), int(s->i2s_clock_pin_mode) },
        { QStringLiteral("mckEnabled"), s->mck_enabled },
        { QStringLiteral("mckPin"), int(s->mck_pin) },
        { QStringLiteral("mckMultiplier"), s->mck_multiplier ? 256 : 128 },
        { QStringLiteral("inputRateHz"), kInputRates[size_t(std::max(rateCode, 0))] },
        { QStringLiteral("inputSource"), int(s->input_source) },
        { QStringLiteral("i2sClockMode"), int(s->i2s_clock_mode) },
        { QStringLiteral("i2sSlaveActive"), s->i2s_clock_mode == 1 && s->input_source == SourceI2s },
        { QStringLiteral("spdifCount"), spdifCount },
        { QStringLiteral("spdifEnabledCount"), spdifEnabledCount },
        { QStringLiteral("spdifEnabled"), boolList(s->spdif_inputs_enabled, 4) },
        { QStringLiteral("spdifPins"), spdifPins },
        { QStringLiteral("lgEnabled"), s->lg_sound_sync_enabled },
        { QStringLiteral("i2sChannels"), i2sChannels },
        { QStringLiteral("i2sMaxPairs"), maxPairs },
        { QStringLiteral("i2sActivePairs"), std::clamp(i2sChannels / 2, 1, maxPairs) },
        { QStringLiteral("i2sRxPins"), i2sRxPins },
        { QStringLiteral("adatOutEnabled"), s->adat_out_enabled },
        { QStringLiteral("adatOutPin"), s->adat_out_pin == 0 ? kAdatOutDefault : int(s->adat_out_pin) },
        { QStringLiteral("adatInEnabled"), s->adat_input_enabled },
        { QStringLiteral("adatInPin"), adatInPin },
        { QStringLiteral("adatInClockMode"), int(s->adat_input_clock_mode) },
        { QStringLiteral("adatInActive"), s->input_source == SourceAdat },
        { QStringLiteral("adatInSlaveActive"), s->adat_input_clock_mode == 1 && s->input_source == SourceAdat },
        { QStringLiteral("outputConfigMode"), int(s->output_config_mode) },
        { QStringLiteral("uart"),
          QVariantMap{ { QStringLiteral("enabled"), s->uart.enabled },
                       { QStringLiteral("txPin"), int(s->uart.tx_pin) },
                       { QStringLiteral("rxPin"), int(s->uart.rx_pin) },
                       { QStringLiteral("baud"), int(s->uart.baud) },
                       { QStringLiteral("notify"), s->uart.notify } } },
        { QStringLiteral("i2c"),
          QVariantMap{ { QStringLiteral("enabled"), s->i2c.enabled },
                       { QStringLiteral("sdaPin"), int(s->i2c.sda_pin) },
                       { QStringLiteral("sclPin"), int(s->i2c.scl_pin) },
                       { QStringLiteral("address"), int(s->i2c.address) } } },
        { QStringLiteral("uartLive"), s->ctrl_status.uart_live },
        { QStringLiteral("i2cLive"), s->ctrl_status.i2c_live },
        { QStringLiteral("protocolVersion"), int(s->ctrl_status.protocol_version) },
    };
}

// The control surfaces' live bindings hold pins too; they are the core's
// cache, not in the record
void HardwareController::readSurfaces() {
    // -- WP QPI3 -- a change is told, so an open page offers the pins anew
    const auto take = [this](const QList<PinMap::SurfacePin> &uses) {
        const bool same = uses.size() == m_surfaces.size()
            && std::equal(uses.cbegin(), uses.cend(), m_surfaces.cbegin(),
                          [](const PinMap::SurfacePin &a, const PinMap::SurfacePin &b) { return a.pin == b.pin && a.slot == b.slot; });
        if (same) return;
        m_surfaces = uses;
        ++m_rev;
        emit changed();
    };
    if (!connected()) {
        take({});
        return;
    }
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            QList<PinMap::SurfacePin> uses;
            std::array<uint8_t, 64> pins{}, owners{};
            const uint32_t n = std::min<uint32_t>(dspi_cs_pin_uses(c, pins.data(), owners.data(), uint32_t(pins.size())),
                                                  uint32_t(pins.size()));
            for (uint32_t i = 0; i < n; ++i) uses.append({ pins[i], owners[i] });
            return uses;
        },
        take);
}

// ---- pins --------------------------------------------------------------------------------


QVariantList HardwareController::pinChoices(int consumer, int index, int current, bool pairWithNext) const {
    QVariantList out;
    const DspState *s = m_ctx.device->state();
    if (!s) return out;
    const QSet<int> held = heldBy(*s, m_surfaces, consumer, index);
    for (int pin : PinMap::gpioPins()) {
        const bool free = !held.contains(pin) && (!pairWithNext || !held.contains(pin + 1));
        if (pin == current || free) out.append(choice(pin));
    }
    return out;
}

QVariantList HardwareController::mckPinChoices() const {
    QVariantList out;
    const DspState *s = m_ctx.device->state();
    if (!s) return out;
    // The chip's clock outputs (GPOUT): GPIO 21 on the RP2040; 13, 15 and 21
    // on the RP2350, 15 not while an I2S slot puts LRCLK there
    QList<int> pins{ 21 };
    if (!rp2040()) {
        pins = { 13, 15, 21 };
        if (m_hw.value(QStringLiteral("anySlotIsI2s")).toBool()) pins.removeAll(15);
    }
    const QSet<int> held = heldBy(*s, m_surfaces, Mck, 0);
    for (int pin : pins)
        if (!held.contains(pin) || pin == s->mck_pin) out.append(choice(pin));
    return out;
}

QVariantList HardwareController::interfacePinChoices(int consumer, bool second, int current, int other) const {
    QVariantList out;
    const DspState *s = m_ctx.device->state();
    if (!s) return out;
    const QSet<int> held = heldBy(*s, m_surfaces, consumer, 0);
    if (!validPin(current) || current == other) out.append(choice(current));
    for (int pin : PinMap::gpioPins()) {
        bool mux;
        if (consumer == UartCtrl) mux = second ? pin % 4 == 1 : pin % 4 == 0;
        else mux = second ? pin % 2 == 1 : pin % 2 == 0;
        if (!mux || pin == other) continue;
        if (pin == current || !held.contains(pin)) out.append(choice(pin));
    }
    return out;
}

QString HardwareController::pinOwner(int pin, int consumer, int index) const { return ownerText(pin, consumer, index); }

QString HardwareController::ownerText(int pin, int consumer, int index) const {
    const DspState *s = m_ctx.device->state();
    if (!s) return QString();
    QHash<int, QString> labels;
    heldBy(*s, m_surfaces, consumer, index, &labels);
    return labels.value(pin);
}

QString HardwareController::pinInUseText(int pin, int consumer, int index) const {
    const QString owner = ownerText(pin, consumer, index);
    return owner.isEmpty() ? tr("GPIO %1 is already in use").arg(pin)
                           : tr("GPIO %1 is already assigned to %2").arg(pin).arg(owner);
}

// ---- live writes ---------------------------------------------------------------------

void HardwareController::watch(bool on) {
    m_watchers = std::max(0, m_watchers + (on ? 1 : -1));
    if (on && m_watchers == 1 && connected()) {
        // Opening a page reads the device again (another console may have
        // changed it), the control interfaces and the mute line included
        const bool interfaces = m_has.value(QStringLiteral("interfaces")).toBool();
        const bool mute = m_has.value(QStringLiteral("dacMute")).toBool();
        m_ctx.core->worker()->post([interfaces, mute](FfiCore *c) {
            if (interfaces) dspi_fetch_ctrl_ifaces(c);
            if (mute) dspi_fetch_dac_mute(c);
        });
        m_ctx.device->refresh();
    }
    if (!on && m_watchers == 0) setStatus(QString(), false);
    m_ctx.poller->setEnabled(m_lockPoll, m_watchers > 0 && connected());
}

void HardwareController::clearStatus() { setStatus(QString(), false); }

void HardwareController::setStatus(const QString &text, bool error) {
    if (text == m_statusText && error == m_statusError) return;
    m_statusText = text;
    m_statusError = error;
    emit statusChanged();
}

void HardwareController::live(const QString &what, std::function<uint8_t(FfiCore *)> setter,
                              std::function<void(uint8_t)> outcome) {
    if (!writable()) return;
    beginOutputEdit();
    auto status = std::make_shared<int>(-1);
    m_ctx.device->write(
        what,
        [setter, status](FfiCore *c) {
            *status = setter(c);
            return *status == PIN_CONFIG_SUCCESS;
        },
        [this, status, outcome](bool) {
            // A write replaced by a newer one of the same name never ran
            if (*status >= 0) outcome(uint8_t(*status));
            pendingMaybeChanged();
        });
}

void HardwareController::setOutputType(int slot, int type) {
    const QString name = tr("Output %1").arg(slot + 1);
    live(QStringLiteral("output type %1").arg(slot),
         [slot, type](FfiCore *c) { return dspi_set_output_type(c, uint8_t(slot), uint8_t(type)); },
         [this, name, type](uint8_t st) {
             if (st == PIN_CONFIG_SUCCESS)
                 setStatus(tr("%1 set to %2").arg(name, type == 1 ? tr("I2S") : tr("S/PDIF")), false);
             else
                 setStatus(tr("Failed to change %1 type").arg(name), true);
         });
}

void HardwareController::setOutputPin(int output, int pin) {
    QString name = tr("Output %1").arg(output);
    for (const QVariant &v : m_hw.value(QStringLiteral("outputs")).toList())
        if (v.toMap().value(QStringLiteral("id")).toInt() == output) name = v.toMap().value(QStringLiteral("name")).toString();
    live(QStringLiteral("output pin %1").arg(output),
         [output, pin](FfiCore *c) { return dspi_set_output_pin(c, uint8_t(output), uint8_t(pin)); },
         [this, name, output, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("%1 reassigned to GPIO %2").arg(name).arg(pin), false); break;
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 is not available on this platform").arg(pin), true); break;
             case PIN_CONFIG_PIN_IN_USE: {
                 const QString owner = ownerText(pin, OutputPin, output);
                 setStatus(owner.isEmpty() ? tr("GPIO %1 is already in use by another output").arg(pin)
                                           : tr("GPIO %1 is already assigned to %2").arg(pin).arg(owner),
                           true);
                 break;
             }
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("Invalid output index"), true); break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("PDM output must be disabled before changing its pin"), true); break;
             default: setStatus(tr("USB communication error"), true); break;
             }
         });
}

void HardwareController::resetPins() {
    if (!writable()) return;
    QList<QPair<int, int>> targets;
    QStringList names;
    for (const QVariant &v : m_hw.value(QStringLiteral("outputs")).toList()) {
        const QVariantMap entry = v.toMap();
        const int id = entry.value(QStringLiteral("id")).toInt();
        const bool slot = entry.value(QStringLiteral("slot")).toBool();
        targets.append({ id, slot ? kDefaultDataPins[size_t(std::clamp(id, 0, 3))] : kDefaultSubPin });
        names.append(entry.value(QStringLiteral("name")).toString());
    }
    auto failed = std::make_shared<int>(-1);
    beginOutputEdit();
    m_ctx.device->write(
        QStringLiteral("reset output pins"),
        [targets, failed](FfiCore *c) {
            for (int i = 0; i < targets.size(); ++i) {
                if (dspi_set_output_pin(c, uint8_t(targets[i].first), uint8_t(targets[i].second)) != PIN_CONFIG_SUCCESS) {
                    *failed = i;
                    return false;
                }
            }
            return true;
        },
        [this, failed, names](bool ok) {
            if (ok) setStatus(tr("All pins reset to defaults"), false);
            else setStatus(tr("Failed to reset %1").arg(names.value(*failed)), true);
            pendingMaybeChanged();
        });
}

void HardwareController::setAdatOutEnabled(bool on) {
    const int pin = m_hw.value(QStringLiteral("adatOutPin")).toInt();
    live(QStringLiteral("adat output"), [on](FfiCore *c) { return dspi_set_adat_out_enabled(c, on); },
         [this, on, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(on ? tr("ADAT output enabled") : tr("ADAT output disabled"), false); break;
             case PIN_CONFIG_PIN_IN_USE: setStatus(pinInUseText(pin, AdatOut, 0), true); break;
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 isn't available on this device").arg(pin), true); break;
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("ADAT output isn't supported on this device"), true); break;
             default: setStatus(on ? tr("Failed to enable ADAT output") : tr("Failed to disable ADAT output"), true); break;
             }
         });
}

void HardwareController::setAdatOutPin(int pin) {
    live(QStringLiteral("adat output pin"), [pin](FfiCore *c) { return dspi_set_adat_out_pin(c, uint8_t(pin)); },
         [this, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("ADAT data pin set to GPIO %1").arg(pin), false); break;
             case PIN_CONFIG_PIN_IN_USE: setStatus(pinInUseText(pin, AdatOut, 0), true); break;
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 isn't available on this device").arg(pin), true); break;
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("ADAT output isn't supported on this device"), true); break;
             default: setStatus(tr("Failed to set ADAT pin"), true); break;
             }
         });
}

void HardwareController::setBckPin(int pin) {
    live(QStringLiteral("i2s bck pin"), [pin](FfiCore *c) { return dspi_set_i2s_bck_pin(c, 0, uint8_t(pin)); },
         [this, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("BCK pin set to GPIO %1, LRCLK = GPIO %2").arg(pin).arg(pin + 1), false); break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("All outputs must be S/PDIF before changing BCK pin"), true); break;
             case PIN_CONFIG_PIN_IN_USE: setStatus(tr("GPIO %1 or %2 is already in use").arg(pin).arg(pin + 1), true); break;
             default: setStatus(tr("Failed to set BCK pin"), true); break;
             }
         });
}

void HardwareController::setClockPinMode(int mode) {
    live(QStringLiteral("i2s clock pin mode"), [mode](FfiCore *c) { return dspi_set_i2s_clock_pin_mode(c, uint8_t(mode)); },
         [this, mode](uint8_t st) {
             const int slave = m_hw.value(QStringLiteral("i2sBckPinSlave")).toInt();
             switch (st) {
             case PIN_CONFIG_SUCCESS:
                 setStatus(mode == 1 ? tr("Separate clock pins - slave uses GPIO %1/%2").arg(slave).arg(slave + 1)
                                     : tr("Shared clock pins for master and slave"),
                           false);
                 break;
             case PIN_CONFIG_PIN_IN_USE:
                 setStatus(tr("Slave clock pair GPIO %1/%2 conflicts with another function - move it first").arg(slave).arg(slave + 1), true);
                 break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("Switch I2S output slots to S/PDIF before changing clock pins"), true); break;
             default: setStatus(tr("Failed to change clock pins"), true); break;
             }
         });
}

void HardwareController::setSlaveBckPin(int pin) {
    live(QStringLiteral("i2s slave bck pin"), [pin](FfiCore *c) { return dspi_set_i2s_bck_pin(c, 1, uint8_t(pin)); },
         [this, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("Slave BCK pin set to GPIO %1, LRCLK = GPIO %2").arg(pin).arg(pin + 1), false); break;
             case PIN_CONFIG_PIN_IN_USE: setStatus(tr("GPIO %1 or %2 is already in use").arg(pin).arg(pin + 1), true); break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("Can't move the slave clock pins while an I2S output is active"), true); break;
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 isn't available on this device").arg(pin), true); break;
             default: setStatus(tr("Failed to set slave BCK pin"), true); break;
             }
         });
}

void HardwareController::setMckEnabled(bool on) {
    live(QStringLiteral("mck enabled"), [on](FfiCore *c) { return dspi_set_mck_enabled(c, on); },
         [this, on](uint8_t st) {
             if (st == PIN_CONFIG_SUCCESS) setStatus(on ? tr("Master clock enabled") : tr("Master clock disabled"), false);
             else setStatus(on ? tr("Failed to enable master clock") : tr("Failed to disable master clock"), true);
         });
}

void HardwareController::setMckPin(int pin) {
    live(QStringLiteral("mck pin"), [pin](FfiCore *c) { return dspi_set_mck_pin(c, uint8_t(pin)); },
         [this, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("MCK pin set to GPIO %1").arg(pin), false); break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("Disable MCK before changing its pin"), true); break;
             case PIN_CONFIG_PIN_IN_USE: setStatus(tr("GPIO %1 is already in use").arg(pin), true); break;
             default: setStatus(tr("Failed to set MCK pin"), true); break;
             }
         });
}

void HardwareController::setMckMultiplier(int times) {
    const uint8_t code = times >= 256 ? 1 : 0;
    live(QStringLiteral("mck multiplier"), [code](FfiCore *c) { return dspi_set_mck_multiplier(c, code); },
         [this, times](uint8_t st) {
             if (st == PIN_CONFIG_SUCCESS) setStatus(tr("MCK multiplier set to %1x").arg(times), false);
             else setStatus(tr("Failed to set MCK multiplier"), true);
         });
}

void HardwareController::setInputRate(int hz) {
    const auto it = std::find(kInputRates.begin(), kInputRates.end(), hz);
    if (it == kInputRates.end()) return;
    const uint8_t index = uint8_t(it - kInputRates.begin());
    // A SET with no status byte: the transfer going through is the answer
    live(QStringLiteral("input rate"),
         [index](FfiCore *c) { return uint8_t(dspi_set_input_rate(c, index) ? PIN_CONFIG_SUCCESS : 0xFF); },
         [](uint8_t) {});
}

void HardwareController::setSpdifPin(int index, int pin) {
    live(QStringLiteral("spdif rx pin %1").arg(index),
         [index, pin](FfiCore *c) { return dspi_set_spdif_rx_pin(c, uint8_t(index), uint8_t(pin)); },
         [this, index, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("S/PDIF %1 RX pin set to GPIO %2").arg(index + 1).arg(pin), false); break;
             case PIN_CONFIG_PIN_IN_USE: setStatus(pinInUseText(pin, SpdifRx, index), true); break;
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 is not available on this platform").arg(pin), true); break;
             default: setStatus(tr("Failed to set S/PDIF %1 RX pin").arg(index + 1), true); break;
             }
         });
}

void HardwareController::setSpdifCount(int target) {
    const int count = m_hw.value(QStringLiteral("spdifCount")).toInt();
    if (!writable() || target < 1 || target > count || target == m_hw.value(QStringLiteral("spdifEnabledCount")).toInt())
        return;
    const QVariantList enabled = m_hw.value(QStringLiteral("spdifEnabled")).toList();
    std::array<bool, 4> on{};
    for (int i = 0; i < 4; ++i) on[size_t(i)] = enabled.value(i).toBool();
    // Enables run low to high, disables high to low; the first refusal stops
    // it: {index, enabling} of the input that was refused, index -1 if none
    auto refused = std::make_shared<QPair<int, bool>>(-1, false);
    beginOutputEdit();
    m_ctx.device->write(
        QStringLiteral("spdif input count"),
        [target, count, on, refused](FfiCore *c) {
            for (int i = 1; i < target; ++i) {
                if (!on[size_t(i)] && dspi_set_spdif_input_enabled(c, uint8_t(i), true) != PIN_CONFIG_SUCCESS) {
                    *refused = { i, true };
                    return false;
                }
            }
            for (int i = count - 1; i >= std::max(1, target); --i) {
                if (on[size_t(i)] && dspi_set_spdif_input_enabled(c, uint8_t(i), false) != PIN_CONFIG_SUCCESS) {
                    *refused = { i, false };
                    return false;
                }
            }
            return true;
        },
        [this, target, refused](bool) {
            const int i = refused->first;
            if (i < 0) {
                setStatus(tr("S/PDIF inputs set to %1").arg(target), false);
            } else if (refused->second) {
                const int pin = m_hw.value(QStringLiteral("spdifPins")).toList().value(i).toInt();
                const QString owner = ownerText(pin, SpdifRx, i);
                setStatus(owner.isEmpty() ? tr("Can't enable S/PDIF %1: GPIO %2 is unavailable").arg(i + 1).arg(pin)
                                          : tr("Can't enable S/PDIF %1: GPIO %2 is assigned to %3").arg(i + 1).arg(pin).arg(owner),
                          true);
            } else {
                setStatus(tr("Switch the input source away from S/PDIF %1 before reducing the input count").arg(i + 1), true);
            }
            pendingMaybeChanged();
        });
}

void HardwareController::setLgSoundSync(bool on) {
    if (!writable()) return;
    // Per preset, saved with the active preset: not part of the bar
    m_ctx.device->write(QStringLiteral("lg sound sync"), [on](FfiCore *c) { return dspi_set_lg_sound_sync(c, on); });
}

void HardwareController::setI2sClockMode(int clockMode) {
    live(QStringLiteral("i2s clock mode"),
         [clockMode](FfiCore *c) { return uint8_t(dspi_set_i2s_clock_mode(c, uint8_t(clockMode)) ? PIN_CONFIG_SUCCESS : 0xFF); },
         [](uint8_t) {});
}

void HardwareController::setI2sChannels(int channels) {
    const int oldChannels = m_hw.value(QStringLiteral("i2sChannels")).toInt();
    const QVariantList pins = m_hw.value(QStringLiteral("i2sRxPins")).toList();
    live(QStringLiteral("i2s input channels"),
         [channels](FfiCore *c) { return dspi_set_i2s_input_channels(c, uint8_t(channels)); },
         [this, channels, oldChannels, pins](uint8_t st) {
             const int pairs = channels / 2;
             if (st == PIN_CONFIG_SUCCESS) {
                 setStatus(pairs == 1 ? tr("I2S input set to %1 channels (1 pair)").arg(channels)
                                      : tr("I2S input set to %1 channels (%2 pairs)").arg(channels).arg(pairs),
                           false);
                 return;
             }
             if (st == PIN_CONFIG_PIN_IN_USE && channels > oldChannels) {
                 // The firmware answers a byte only: name the clashes as it
                 // checks them, the newly active pairs against each other too
                 const int from = std::max(1, oldChannels / 2);
                 QStringList conflicts;
                 for (int pair = from; pair < pairs && pair < pins.size(); ++pair) {
                     const int pin = pins.value(pair).toInt();
                     const QString owner = ownerText(pin, I2sRx, pair);
                     if (!owner.isEmpty()) {
                         conflicts << tr("GPIO %1 (pair %2) is used by %3").arg(pin).arg(pair + 1).arg(owner);
                         continue;
                     }
                     for (int other = from; other < pair; ++other) {
                         if (pins.value(other).toInt() == pin) {
                             conflicts << tr("GPIO %1 is set on both Serial Data %2 and %3").arg(pin).arg(other + 1).arg(pair + 1);
                             break;
                         }
                     }
                 }
                 if (conflicts.isEmpty())
                     setStatus(tr("Can't switch to %1 channels: a required data pin is already in use. Reassign the new pairs' pins, then try again.").arg(channels), true);
                 else
                     setStatus(tr("Can't switch to %1 channels - %2. %3").arg(channels).arg(conflicts.join(QStringLiteral("; ")),
                                   conflicts.size() == 1 ? tr("Reassign it, then try again.") : tr("Reassign them, then try again.")),
                               true);
                 return;
             }
             switch (st) {
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("Multichannel I2S isn't supported on this device"), true); break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("Can't change the bit clock while an I2S output is active"), true); break;
             default: setStatus(tr("Failed to set channel count"), true); break;
             }
         });
}

void HardwareController::setI2sRxPin(int pair, int pin) {
    live(QStringLiteral("i2s rx pin %1").arg(pair),
         [pair, pin](FfiCore *c) { return dspi_set_i2s_rx_pin(c, uint8_t(pair), uint8_t(pin)); },
         [this, pair, pin](uint8_t st) {
             const QString label = tr("Serial Data %1").arg(pair + 1);
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(tr("%1 set to GPIO %2").arg(label).arg(pin), false); break;
             case PIN_CONFIG_PIN_IN_USE: {
                 const QString owner = ownerText(pin, I2sRx, pair);
                 setStatus(owner.isEmpty() ? tr("That pin is already in use") : tr("GPIO %1 is already assigned to %2").arg(pin).arg(owner), true);
                 break;
             }
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 isn't available on this device").arg(pin), true); break;
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("Multichannel I2S isn't supported on this device"), true); break;
             case PIN_CONFIG_OUTPUT_ACTIVE: setStatus(tr("Can't change the bit clock while an I2S output is active"), true); break;
             default: setStatus(tr("Failed to set %1").arg(label), true); break;
             }
         });
}

void HardwareController::setAdatInEnabled(bool on) {
    const int pin = m_hw.value(QStringLiteral("adatInPin")).toInt();
    live(QStringLiteral("adat input"), [on](FfiCore *c) { return dspi_set_adat_input_enabled(c, on); },
         [this, on, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS: setStatus(on ? tr("ADAT input enabled") : tr("ADAT input disabled"), false); break;
             case PIN_CONFIG_PIN_IN_USE:
                 if (!on) setStatus(tr("Switch to another input source before disabling ADAT input"), true);
                 else if (pin != kNoPin && !ownerText(pin, AdatIn, 0).isEmpty()) setStatus(pinInUseText(pin, AdatIn, 0), true);
                 else setStatus(tr("That pin is already in use"), true);
                 break;
             case PIN_CONFIG_INVALID_PIN:
                 setStatus(on ? tr("Assign a valid data pin before enabling ADAT input") : tr("Invalid ADAT input pin"), true);
                 break;
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("ADAT input isn't supported on this device"), true); break;
             default: setStatus(tr("Failed to set ADAT input %1").arg(on ? tr("enabled") : tr("disabled")), true); break;
             }
         });
}

void HardwareController::setAdatInPin(int pin) {
    live(QStringLiteral("adat input pin"), [pin](FfiCore *c) { return dspi_set_adat_input_pin(c, uint8_t(pin)); },
         [this, pin](uint8_t st) {
             switch (st) {
             case PIN_CONFIG_SUCCESS:
                 setStatus(pin == kNoPin ? tr("ADAT input pin cleared") : tr("ADAT input pin set to GPIO %1").arg(pin), false);
                 break;
             case PIN_CONFIG_PIN_IN_USE: {
                 const QString owner = ownerText(pin, AdatIn, 0);
                 setStatus(owner.isEmpty() ? tr("That pin is already in use") : tr("GPIO %1 is already assigned to %2").arg(pin).arg(owner), true);
                 break;
             }
             case PIN_CONFIG_INVALID_PIN: setStatus(tr("GPIO %1 isn't available on this device").arg(pin), true); break;
             case PIN_CONFIG_INVALID_OUTPUT: setStatus(tr("ADAT input isn't supported on this device"), true); break;
             default: setStatus(tr("Failed to set ADAT input pin"), true); break;
             }
         });
}

void HardwareController::setAdatInClockMode(int mode) {
    live(QStringLiteral("adat input clock mode"),
         [mode](FfiCore *c) { return dspi_set_adat_input_clock_mode(c, uint8_t(mode)); },
         [this](uint8_t st) {
             if (st == PIN_CONFIG_SUCCESS) setStatus(tr("ADAT input clock mode updated"), false);
             else if (st == PIN_CONFIG_INVALID_OUTPUT) setStatus(tr("ADAT input isn't supported on this device"), true);
             else setStatus(tr("Failed to set ADAT input clock mode"), true);
         });
}

// ---- Global Parameters -----------------------------------------------------------------

HardwareController::Draft HardwareController::deviceDraft() const {
    Draft d;
    const DspState *s = m_ctx.device->state();
    if (!s) return d;
    d.startupMode = s->preset_startup_mode;
    d.defaultSlot = s->preset_default_slot;
    d.masterVolumeMode = s->master_volume_mode;
    d.outputConfigMode = s->output_config_mode;
    d.muteEnabled = s->dac_mute_enabled;
    d.muteActiveLow = s->dac_mute_active_low;
    d.mutePin = s->dac_mute_pin;
    d.muteHoldMs = s->dac_mute_hold_ms;
    d.muteReleaseMs = s->dac_mute_release_ms;
    return d;
}

QVariantMap HardwareController::draft() const {
    const Draft d = m_globalEdited ? m_draft : deviceDraft();
    return { { QStringLiteral("startupMode"), d.startupMode },
             { QStringLiteral("defaultSlot"), d.defaultSlot },
             { QStringLiteral("masterVolumeMode"), d.masterVolumeMode },
             { QStringLiteral("outputConfigMode"), d.outputConfigMode },
             { QStringLiteral("muteEnabled"), d.muteEnabled },
             { QStringLiteral("muteActiveLow"), d.muteActiveLow },
             { QStringLiteral("mutePin"), d.mutePin },
             { QStringLiteral("muteHoldMs"), d.muteHoldMs },
             { QStringLiteral("muteReleaseMs"), d.muteReleaseMs },
             // The mute test runs the device's saved settings
             { QStringLiteral("dirty"), globalDirty() } };
}

QStringList HardwareController::presetSlotLabels() const {
    QStringList out;
    const DspState *s = m_ctx.device->state();
    for (int slot = 0; slot < MAX_PRESETS; ++slot) {
        QString name;
        if (s && (s->preset_occupied & (1u << slot))) {
            name = QString::fromUtf8(reinterpret_cast<const char *>(s->preset_names[slot]),
                                     int(strnlen(reinterpret_cast<const char *>(s->preset_names[slot]), CHANNEL_NAME_LEN)))
                       .trimmed();
            if (name.isEmpty()) name = tr("Preset %1").arg(slot + 1);
        } else {
            name = tr("Empty");
        }
        out << tr("%1: %2").arg(slot + 1).arg(name);
    }
    return out;
}

void HardwareController::setDraft(const QString &key, const QVariant &value) {
    if (!connected()) return;
    if (!m_globalEdited) m_draft = deviceDraft();
    const int v = value.toInt();
    if (key == QLatin1String("startupMode")) m_draft.startupMode = v;
    else if (key == QLatin1String("defaultSlot")) m_draft.defaultSlot = v;
    else if (key == QLatin1String("masterVolumeMode")) m_draft.masterVolumeMode = v;
    else if (key == QLatin1String("outputConfigMode")) m_draft.outputConfigMode = v;
    else if (key == QLatin1String("muteActiveLow")) m_draft.muteActiveLow = value.toBool();
    else if (key == QLatin1String("mutePin")) m_draft.mutePin = v;
    else if (key == QLatin1String("muteHoldMs")) m_draft.muteHoldMs = v;
    else if (key == QLatin1String("muteReleaseMs")) m_draft.muteReleaseMs = v;
    else if (key == QLatin1String("muteEnabled")) {
        m_draft.muteEnabled = value.toBool();
        // Switched on with no pin: the first free GPIO, so the picker is never empty
        if (m_draft.muteEnabled && m_draft.mutePin == kNoPin) {
            const QVariantList free = pinChoices(DacMute, 0, -1);
            if (!free.isEmpty()) m_draft.mutePin = free.first().toMap().value(QStringLiteral("pin")).toInt();
        }
    } else return;
    m_globalEdited = true;
    m_saveFailure.clear();
    emit draftChanged();
    pendingMaybeChanged();
}

bool HardwareController::globalDirty() const {
    if (!m_globalEdited || !connected()) return false;
    Draft want = m_draft;
    const Draft have = deviceDraft();
    if (!m_has.value(QStringLiteral("dacMute")).toBool()) {
        want.muteEnabled = have.muteEnabled;
        want.muteActiveLow = have.muteActiveLow;
        want.mutePin = have.mutePin;
        want.muteHoldMs = have.muteHoldMs;
        want.muteReleaseMs = have.muteReleaseMs;
    }
    // The default slot only matters for the mode that loads it
    if (want.startupMode == 1 && have.startupMode == 1) want.defaultSlot = have.defaultSlot;
    return !(want == have);
}

void HardwareController::testDacMute() {
    if (!writable()) return;
    m_ctx.device->write(QStringLiteral("test external mute"), [](FfiCore *c) { return dspi_test_dac_mute(c) == 0; });
}

// ---- Control Interfaces ------------------------------------------------------------------

QString HardwareController::baudLabel(int baud) const {
    return baud >= 1000 && baud % 1000 == 0 ? QStringLiteral("%1k").arg(baud / 1000) : QString::number(baud);
}

void HardwareController::applyUart(const QVariantMap &config) {
    if (!writable()) return;
    UartConfig u{};
    u.enabled = config.value(QStringLiteral("enabled")).toBool();
    u.tx_pin = uint8_t(config.value(QStringLiteral("txPin")).toInt());
    u.rx_pin = uint8_t(config.value(QStringLiteral("rxPin")).toInt());
    u.baud = uint32_t(config.value(QStringLiteral("baud")).toUInt());
    u.notify = config.value(QStringLiteral("notify")).toBool();
    m_uartOutcome = { { QStringLiteral("applying"), true } };
    emit interfacesChanged();
    m_ctx.device->write(QStringLiteral("uart control"), [u](FfiCore *c) { return dspi_set_uart(c, u); },
                        [this](bool ok) {
                            if (!ok) {
                                m_uartOutcome = { { QStringLiteral("text"), tr("Failed to apply %1 configuration").arg(tr("UART")) },
                                                  { QStringLiteral("error"), true } };
                                emit interfacesChanged();
                                return;
                            }
                            readInterfacesLater(true);
                        });
}

void HardwareController::applyI2c(const QVariantMap &config) {
    if (!writable()) return;
    I2cConfig i{};
    i.enabled = config.value(QStringLiteral("enabled")).toBool();
    i.sda_pin = uint8_t(config.value(QStringLiteral("sdaPin")).toInt());
    i.scl_pin = uint8_t(config.value(QStringLiteral("sclPin")).toInt());
    i.address = uint8_t(config.value(QStringLiteral("address")).toInt());
    m_i2cOutcome = { { QStringLiteral("applying"), true } };
    emit interfacesChanged();
    m_ctx.device->write(QStringLiteral("i2c control"), [i](FfiCore *c) { return dspi_set_i2c(c, i); },
                        [this](bool ok) {
                            if (!ok) {
                                m_i2cOutcome = { { QStringLiteral("text"), tr("Failed to apply %1 configuration").arg(tr("I2C")) },
                                                 { QStringLiteral("error"), true } };
                                emit interfacesChanged();
                                return;
                            }
                            readInterfacesLater(false);
                        });
}

// The device applies and stores an interface later: its outcome is the last
// status it reports for it (the Mac's statusMessage)
void HardwareController::readInterfacesLater(bool uart) {
    QTimer::singleShot(kInterfaceDelay, this, [this, uart]() {
        m_ctx.device->write(
            uart ? QStringLiteral("uart control read back") : QStringLiteral("i2c control read back"),
            [](FfiCore *c) { return dspi_fetch_ctrl_ifaces(c); },
            [this, uart](bool) {
                const DspState *s = m_ctx.device->state();
                const QString iface = uart ? tr("UART") : tr("I2C");
                const int st = s ? (uart ? s->ctrl_status.uart_last_status : s->ctrl_status.i2c_last_status) : 0xFF;
                QString text;
                switch (st) {
                case PIN_CONFIG_SUCCESS: text = tr("%1 configuration applied and saved").arg(iface); break;
                case PIN_CONFIG_INVALID_PIN: text = tr("A pin is out of range or lacks the required %1 mux function").arg(iface); break;
                case PIN_CONFIG_PIN_IN_USE: text = tr("A pin is already claimed by another output or interface"); break;
                case PIN_CONFIG_INVALID_PARAM:
                    text = uart ? tr("Baud rate is out of range (9600 - 1000000)") : tr("Address is out of range (0x08 - 0x77)");
                    break;
                default: text = tr("Failed to apply %1 configuration").arg(iface); break;
                }
                const QVariantMap outcome{ { QStringLiteral("text"), text },
                                           { QStringLiteral("error"), st != PIN_CONFIG_SUCCESS } };
                if (uart) m_uartOutcome = outcome;
                else m_i2cOutcome = outcome;
                emit interfacesChanged();
            });
    });
}

// ---- the save bar ---------------------------------------------------------------------

bool HardwareController::outputDirty() const {
    const DspState *s = m_ctx.device->state();
    return m_outputEdited && s && s->output_config_mode == 0;
}

void HardwareController::beginOutputEdit() {
    const DspState *s = m_ctx.device->state();
    // With the presets keeping the configuration, a preset save keeps it
    if (!s || s->output_config_mode != 0) return;
    if (!m_outputEdited) {
        m_outputBaseline = m_ctx.device->snapshot().state;
        m_outputEdited = true;
    }
    m_saveFailure.clear();
    pendingMaybeChanged();
}

bool HardwareController::pending() const {
    if (!connected()) return false;
    for (const Part &p : m_parts)
        if (p.participant->hasPending()) return true;
    return !m_heldNote.isEmpty();
}

bool HardwareController::liveOnly() const {
    bool any = false;
    for (const Part &p : m_parts) {
        if (!p.participant->hasPending()) continue;
        if (!p.participant->liveOnly()) return false;
        any = true;
    }
    return any;
}

bool HardwareController::busy() const {
    if (m_saving) return true;
    for (const Part &p : m_parts)
        if (p.participant->busy()) return true;
    return false;
}

void HardwareController::addSaveParticipant(SaveParticipant *participant, int order) {
    auto at = std::find_if(m_parts.begin(), m_parts.end(), [order](const Part &p) { return p.order > order; });
    m_parts.insert(at, { participant, order });
    pendingMaybeChanged();
}

void HardwareController::removeSaveParticipant(SaveParticipant *participant) {
    m_parts.erase(std::remove_if(m_parts.begin(), m_parts.end(),
                                 [participant](const Part &p) { return p.participant == participant; }),
                  m_parts.end());
    pendingMaybeChanged();
}

void HardwareController::participantChanged() { pendingMaybeChanged(); }

void HardwareController::setHeldNote(const QString &note) {
    if (note == m_heldNote) return;
    m_heldNote = note;
    emit pendingChanged();
}

void HardwareController::pendingMaybeChanged() {
    emit pendingChanged();
    emit draftChanged();
}

void HardwareController::save() {
    if (m_saving || !writable() || !pending()) return;
    m_saving = true;
    m_saveFailure.clear();
    emit pendingChanged();
    runSave(0, std::make_shared<QStringList>(), m_ctx.device->snapshot().generation);
}

// One part after the other, in order; a device change midway stops it, so
// nothing of the old device's is written to the new one
void HardwareController::runSave(size_t step, std::shared_ptr<QStringList> failures, uint64_t generation) {
    if (m_ctx.device->snapshot().generation != generation || !connected()) {
        m_saving = false;
        pendingMaybeChanged();
        return;
    }
    while (step < m_parts.size() && !m_parts[step].participant->hasPending()) ++step;
    if (step >= m_parts.size()) {
        finishSave(*failures);
        return;
    }
    SaveParticipant *part = m_parts[step].participant;
    part->save([this, step, failures, generation](const QStringList &refused) {
        failures->append(refused);
        runSave(step + 1, failures, generation);
    });
}

void HardwareController::finishSave(const QStringList &failures) {
    m_saving = false;
    //: The save bar's orange line: what the device did not take, e.g.
    //: "Not saved: the startup preset, the latency."
    m_saveFailure = failures.isEmpty() ? QString() : tr("Not saved: %1.").arg(failures.join(QStringLiteral(", ")));
    if (failures.isEmpty()) m_outputBaseline.reset();
    pendingMaybeChanged();
}

void HardwareController::revert() {
    if (m_saving) return;
    m_saveFailure.clear();
    for (const Part &p : m_parts)
        if (p.participant->hasPending() || p.participant == m_globalPart) p.participant->revert();
    pendingMaybeChanged();
}

// Puts the configuration recorded at the first edit back, in the Mac's
// order: a lowered I2S channel count before the pins, a raised one after;
// S/PDIF pins before their enables; ADAT pins before the enable
void HardwareController::restoreOutputs() {
    if (!outputDirty() || !m_outputBaseline || !writable()) {
        m_outputEdited = false;
        return;
    }
    const std::shared_ptr<const DspState> base = m_outputBaseline;
    const bool rp40 = rp2040();
    const bool clockPins = m_has.value(QStringLiteral("i2sClockPinMode")).toBool();
    const bool adatOut = m_has.value(QStringLiteral("adatOut")).toBool();
    const bool adatIn = m_has.value(QStringLiteral("adatIn")).toBool();
    const bool multi = m_has.value(QStringLiteral("multiSpdif")).toBool();
    m_ctx.device->write(
        QStringLiteral("revert output configuration"),
        [base, rp40, clockPins, adatOut, adatIn, multi](FfiCore *c) {
            const DspState *now = dspi_get_state(c);
            if (!now) return false;
            const DspState n = *now;   // the record is rewritten by each call
            const DspState &b = *base;
            const int slotCount = rp40 ? 2 : 4;
            for (int s = 0; s < slotCount; ++s)
                if (n.output_types[s] != b.output_types[s]) dspi_set_output_type(c, uint8_t(s), b.output_types[s]);
            for (int i = 0; i < MAX_PHYSICAL_OUTPUTS; ++i)
                if (n.output_pins[i] != b.output_pins[i]) dspi_set_output_pin(c, uint8_t(i), b.output_pins[i]);
            if (n.i2s_bck_pin != b.i2s_bck_pin) dspi_set_i2s_bck_pin(c, 0, b.i2s_bck_pin);
            if (n.mck_enabled != b.mck_enabled) dspi_set_mck_enabled(c, b.mck_enabled);
            if (n.mck_pin != b.mck_pin) dspi_set_mck_pin(c, b.mck_pin);
            if (n.mck_multiplier != b.mck_multiplier) dspi_set_mck_multiplier(c, b.mck_multiplier);
            if (n.spdif_rx_pins[0] != b.spdif_rx_pins[0]) dspi_set_spdif_rx_pin(c, 0, b.spdif_rx_pins[0]);
            if (multi) {
                for (int i = 1; i < 4; ++i) {
                    const bool wasOn = b.spdif_inputs_enabled & (1u << i);
                    const bool isOn = n.spdif_inputs_enabled & (1u << i);
                    if (isOn && !wasOn) dspi_set_spdif_input_enabled(c, uint8_t(i), false);
                    if (n.spdif_rx_pins[i] != b.spdif_rx_pins[i]) dspi_set_spdif_rx_pin(c, uint8_t(i), b.spdif_rx_pins[i]);
                    if (!isOn && wasOn) dspi_set_spdif_input_enabled(c, uint8_t(i), true);
                }
            }
            const bool raising = b.i2s_input_channels > n.i2s_input_channels;
            if (!raising && n.i2s_input_channels != b.i2s_input_channels) dspi_set_i2s_input_channels(c, b.i2s_input_channels);
            for (int p = 0; p < 4; ++p)
                if (n.i2s_rx_pins[p] != b.i2s_rx_pins[p]) dspi_set_i2s_rx_pin(c, uint8_t(p), b.i2s_rx_pins[p]);
            if (raising) dspi_set_i2s_input_channels(c, b.i2s_input_channels);
            if (n.i2s_input_rate != b.i2s_input_rate) dspi_set_input_rate(c, b.i2s_input_rate);
            if (n.i2s_clock_mode != b.i2s_clock_mode) dspi_set_i2s_clock_mode(c, b.i2s_clock_mode);
            if (clockPins) {
                if (n.i2s_bck_pin_slave != b.i2s_bck_pin_slave) dspi_set_i2s_bck_pin(c, 1, b.i2s_bck_pin_slave);
                if (n.i2s_clock_pin_mode != b.i2s_clock_pin_mode) dspi_set_i2s_clock_pin_mode(c, b.i2s_clock_pin_mode);
            }
            if (adatOut) {
                if (n.adat_out_pin != b.adat_out_pin) dspi_set_adat_out_pin(c, b.adat_out_pin);
                if (n.adat_out_enabled != b.adat_out_enabled) dspi_set_adat_out_enabled(c, b.adat_out_enabled);
            }
            if (adatIn) {
                if (n.adat_input_enabled && !b.adat_input_enabled) dspi_set_adat_input_enabled(c, false);
                if (n.adat_input_pin != b.adat_input_pin) dspi_set_adat_input_pin(c, b.adat_input_pin);
                if (n.adat_input_clock_mode != b.adat_input_clock_mode)
                    dspi_set_adat_input_clock_mode(c, b.adat_input_clock_mode);
                if (!n.adat_input_enabled && b.adat_input_enabled) dspi_set_adat_input_enabled(c, true);
            }
            // -- WP QP8 -- The output limiters last, in the Mac's order
            // (applyLimiterSettings): a write to a linked output moves its whole
            // group, so a field-by-field pass would spread one output's value
            // over its old group. Every output leaves its group, takes its own
            // values unlinked, then rejoins in ascending order so each group's
            // lowest output leads; enable goes last, onto the new threshold.
            const int outs = std::clamp(int(n.num_output_channels), 0, int(MAX_OUTPUTS));
            bool limitersMoved = false;
            for (int o = 0; o < outs; ++o)
                limitersMoved = limitersMoved || n.limiter_threshold_db[o] != b.limiter_threshold_db[o] ||
                                n.limiter_release_ms[o] != b.limiter_release_ms[o] ||
                                n.limiter_link_group[o] != b.limiter_link_group[o] ||
                                n.limiter_enabled[o] != b.limiter_enabled[o];
            if (limitersMoved) {
                for (int o = 0; o < outs; ++o)
                    if (n.limiter_link_group[o] != 0) dspi_set_limiter_param(c, uint8_t(o), LIMITER_PARAM_LINK_GROUP, 0.0f);
                for (int o = 0; o < outs; ++o) {
                    const auto out = uint8_t(o);
                    dspi_set_limiter_param(c, out, LIMITER_PARAM_THRESHOLD_DB, b.limiter_threshold_db[o]);
                    dspi_set_limiter_param(c, out, LIMITER_PARAM_RELEASE_MS, b.limiter_release_ms[o]);
                    dspi_set_limiter_param(c, out, LIMITER_PARAM_ENABLED, b.limiter_enabled[o] ? 1.0f : 0.0f);
                }
                for (int o = 0; o < outs; ++o)
                    if (b.limiter_link_group[o] != 0)
                        dspi_set_limiter_param(c, uint8_t(o), LIMITER_PARAM_LINK_GROUP, float(b.limiter_link_group[o]));
            }
            return true;
        },
        [this](bool) {
            m_outputEdited = false;
            m_outputBaseline.reset();
            setStatus(QString(), false);
            pendingMaybeChanged();
        });
    m_outputEdited = false;
}

QString HardwareController::latencyMs(int us) {
    if (us % 1000 == 0) return tr("%1 ms").arg(us / 1000);
    return tr("%1 ms").arg(double(us) / 1000.0, 0, 'f', 2);
}

QString HardwareController::sanitizeName(const QString &name) {
    QString out;
    for (const QChar ch : name) {
        if (ch.unicode() >= 32 && ch.unicode() < 127) out.append(ch);
        if (out.size() >= 31) break;
    }
    return out;
}
