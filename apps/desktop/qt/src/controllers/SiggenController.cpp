// -- WP QP9 -- The signal generator's controller (SiggenController.h).
#include "SiggenController.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "SessionModel.h"
#include "SessionState.h"

#include <QCoreApplication>
#include <QPointer>
#include <QtGlobal>
#include <algorithm>
#include <cmath>

static const Registrar::Controller<SiggenController> siggenController;
static const Registrar::Window siggenWindow("tools.signalGenerator", "SiggenWindow");

namespace {

const char *const kContext = "SiggenController";

struct ParamRow {
    const char *label;
    int semantic;
    float min, max, def;
};
// The Mac's catalogue (TestSignalsView): tile label, the status line's name,
// the blurb, the timing model and the parameters with the firmware's ranges
struct TypeRow {
    const char *tile;
    const char *name;
    const char *blurb;
    int timing;
    std::array<ParamRow, 4> params;
};
using S = SiggenController;
constexpr ParamRow kNone{ nullptr, S::Unused, 0, 0, 0 };
const std::array<TypeRow, SIGGEN_TYPE_COUNT> kTypes{ {
    { QT_TRANSLATE_NOOP("SiggenController", "Sine"), QT_TRANSLATE_NOOP("SiggenController", "Sine"),
      QT_TRANSLATE_NOOP("SiggenController", "Pure tone, THD approx -139 dB"), S::Continuous,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Frequency"), S::FreqHz, 1, 30000, 1000 }, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Square"), QT_TRANSLATE_NOOP("SiggenController", "Square wave"),
      QT_TRANSLATE_NOOP("SiggenController", "Band-limited (polyBLEP) square"), S::Continuous,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Frequency"), S::FreqHz, 1, 30000, 100 }, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "White"), QT_TRANSLATE_NOOP("SiggenController", "White noise"),
      QT_TRANSLATE_NOOP("SiggenController", "Uniform white noise"), S::Continuous, { { kNone, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Pink"), QT_TRANSLATE_NOOP("SiggenController", "Pink noise"),
      QT_TRANSLATE_NOOP("SiggenController", "-3 dB/oct, level-safe normalized"), S::Continuous,
      { { kNone, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Log Swp"), QT_TRANSLATE_NOOP("SiggenController", "Log sweep"),
      QT_TRANSLATE_NOOP("SiggenController", "Exponential sweep for room measurement"), S::Sweep,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Start"), S::FreqHz, 1, 30000, 20 },
          { QT_TRANSLATE_NOOP("SiggenController", "End"), S::FreqHz, 1, 30000, 20000 }, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Lin Swp"), QT_TRANSLATE_NOOP("SiggenController", "Linear sweep"),
      QT_TRANSLATE_NOOP("SiggenController", "Linear frequency sweep"), S::Sweep,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Start"), S::FreqHz, 1, 30000, 20 },
          { QT_TRANSLATE_NOOP("SiggenController", "End"), S::FreqHz, 1, 30000, 20000 }, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Step Swp"), QT_TRANSLATE_NOOP("SiggenController", "Stepped sweep"),
      QT_TRANSLATE_NOOP("SiggenController", "Discrete tones stepping up the band"), S::Sweep,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Start"), S::FreqHz, 1, 30000, 20 },
          { QT_TRANSLATE_NOOP("SiggenController", "End"), S::FreqHz, 1, 30000, 20000 },
          { QT_TRANSLATE_NOOP("SiggenController", "Steps/octave"), S::Count, 1, 24, 3 },
          { QT_TRANSLATE_NOOP("SiggenController", "Dwell"), S::Ms, 20, 10000, 250 } } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Impulse"), QT_TRANSLATE_NOOP("SiggenController", "Impulse"),
      QT_TRANSLATE_NOOP("SiggenController", "Single-sample unit impulses"), S::Pattern,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Period"), S::Ms, 10, 60000, 500 }, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Clicks"), QT_TRANSLATE_NOOP("SiggenController", "Alternating clicks"),
      QT_TRANSLATE_NOOP("SiggenController", "Clicks with alternating polarity"), S::Pattern,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Period"), S::Ms, 10, 60000, 500 }, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Polarity"), QT_TRANSLATE_NOOP("SiggenController", "Polarity pulse"),
      QT_TRANSLATE_NOOP("SiggenController", "Positive half-sine lobe per period"), S::Pattern,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Pulse width"), S::Ms, 1, 100, 5 },
          { QT_TRANSLATE_NOOP("SiggenController", "Period"), S::Ms, 10, 60000, 500 }, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Burst"), QT_TRANSLATE_NOOP("SiggenController", "Tone burst"),
      QT_TRANSLATE_NOOP("SiggenController", "Sine bursts with raised-cosine edges"), S::Pattern,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Frequency"), S::FreqHz, 1, 30000, 1000 },
          { QT_TRANSLATE_NOOP("SiggenController", "On cycles"), S::Cycles, 1, 1000, 8 },
          { QT_TRANSLATE_NOOP("SiggenController", "Off cycles"), S::Cycles, 0, 1000, 8 },
          { QT_TRANSLATE_NOOP("SiggenController", "Edge cycles"), S::Cycles, 0, 100, 2 } } } },
    { QT_TRANSLATE_NOOP("SiggenController", "2-Tone"), QT_TRANSLATE_NOOP("SiggenController", "Tone pair"),
      QT_TRANSLATE_NOOP("SiggenController", "IMD test pair (SMPTE / CCIF)"), S::Continuous,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Tone 1"), S::FreqHz, 1, 30000, 60 },
          { QT_TRANSLATE_NOOP("SiggenController", "Tone 2"), S::FreqHz, 1, 30000, 7000 },
          { QT_TRANSLATE_NOOP("SiggenController", "Ratio A1/A2"), S::Ratio, 0.1f, 10, 4 }, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Multi"), QT_TRANSLATE_NOOP("SiggenController", "Multitone"),
      QT_TRANSLATE_NOOP("SiggenController", "Log-spaced tones, Schroeder phases"), S::Continuous,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Tones"), S::Count, 2, 16, 10 },
          { QT_TRANSLATE_NOOP("SiggenController", "Low"), S::FreqHz, 1, 30000, 20 },
          { QT_TRANSLATE_NOOP("SiggenController", "High"), S::FreqHz, 1, 30000, 20000 }, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "ISP"), QT_TRANSLATE_NOOP("SiggenController", "ISP test"),
      QT_TRANSLATE_NOOP("SiggenController", "Inter-sample-peak over patterns"), S::Continuous,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Pattern"), S::PatternChoice, 0, 1, 0 }, kNone, kNone, kNone } } },
    { QT_TRANSLATE_NOOP("SiggenController", "Chan ID"), QT_TRANSLATE_NOOP("SiggenController", "Channel ID"),
      QT_TRANSLATE_NOOP("SiggenController", "Counted pentatonic blips per channel"), S::Pattern,
      { { { QT_TRANSLATE_NOOP("SiggenController", "Blip length"), S::Ms, 30, 1000, 120 }, kNone, kNone, kNone } } },
} };

QString tr(const char *text) { return QCoreApplication::translate(kContext, text); }

bool validType(int id) { return id >= 0 && id < int(kTypes.size()); }

// The draft before any device: Sine 1 kHz on outputs 1 and 2 at -20 dBFS
SiggenConfig initialDraft() {
    SiggenConfig c{};
    c.signal_type = SiggenController::Sine;
    c.channel_mask = 0x0003;
    c.level_db = float(SiggenController::kDefaultLevel);
    c.p[0] = 1000;
    return c;
}

bool sameConfig(const SiggenConfig &a, const SiggenConfig &b) {
    return a.signal_type == b.signal_type && a.channel_mask == b.channel_mask && a.invert_mask == b.invert_mask
        && a.flags == b.flags && a.level_db == b.level_db && a.duration_ms == b.duration_ms && a.repeat == b.repeat
        && a.gap_ms == b.gap_ms && std::equal(std::begin(a.p), std::end(a.p), std::begin(b.p));
}

double finiteOr(double v, double fallback) { return std::isfinite(v) ? v : fallback; }

struct Fetched {
    bool capsOk = false;
    SiggenCaps caps{};
    bool configOk = false;
    SiggenConfig config{};
    bool statusOk = false;
    SiggenStatus status{};
};

} // namespace

SiggenController::Device::Device() : draft(initialDraft()) { status.active_channel = kNotWalking; }

SiggenController::StatePtr SiggenController::state() const {
    return sessionState<Device>(m_ctx.activeSession(), QStringLiteral("SiggenController"));
}

SiggenController::SiggenController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx) {
    m_live.setSingleShot(true);
    m_live.setInterval(kLiveApplyMs);
    connect(&m_live, &QTimer::timeout, this, &SiggenController::sendLive);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &SiggenController::onSnapshot);
    connect(ctx.device, &DeviceModel::stateChanged, this, &SiggenController::onState);
    // -- WP MDQT2c -- the safety rule: nothing plays on on a device that is left
    connect(ctx.registry, &DeviceRegistry::activeLeaving, this, [this] { stopOn(m_ctx.activeSession()); });
    connect(ctx.registry, &DeviceRegistry::sessionStopping, this, &SiggenController::stopOn);
    if (ctx.poller) {
        m_poll = ctx.poller->add(
            QStringLiteral("signal generator status"), kStatusPollMs, this,
            [](FfiCore *c) {
                std::pair<bool, SiggenStatus> r{ false, {} };
                r.first = dspi_siggen_get_status(c, &r.second);
                return r;
            },
            [this](const std::pair<bool, SiggenStatus> &r) {
                if (r.first) setStatus(state(), r.second);
            });
        updatePoll();
    }
}

bool SiggenController::connected() const { return m_ctx.device->connected(); }

bool SiggenController::controlsEnabled() const {
    return connected() && state()->supported && !m_ctx.device->readOnly();
}

int SiggenController::outputCount() const {
    if (state()->supported && state()->caps.output_channels > 0) return state()->caps.output_channels;
    return m_ctx.device->numOutputs();
}

int SiggenController::timingOf(int type) const {
    if (!validType(type)) return Continuous;
    if (state()->supported) {
        for (int i = 0; i < state()->caps.type_count && i < SIGGEN_TYPE_COUNT; ++i) {
            const SiggenTypeInfo &t = state()->caps.types[i];
            if (t.id == type && t.timing <= Pattern) return t.timing;
        }
    }
    return kTypes[size_t(type)].timing;
}

SiggenController::Param SiggenController::paramOf(int type, int index) const {
    Param p;
    if (!validType(type) || index < 0 || index > 3) return p;
    const ParamRow &row = kTypes[size_t(type)].params[size_t(index)];
    p = { row.semantic, row.min, row.max, row.def };
    if (state()->supported) {
        for (int i = 0; i < state()->caps.type_count && i < SIGGEN_TYPE_COUNT; ++i) {
            const SiggenTypeInfo &t = state()->caps.types[i];
            if (t.id != type) continue;
            const SiggenParam &d = t.params[index];
            if (d.semantic > PatternChoice || !std::isfinite(d.min) || !std::isfinite(d.max) || !std::isfinite(d.def))
                break;
            p = { d.semantic, d.min, d.max, d.def };
            break;
        }
    }
    // The multitone's count is held to what the device plays
    if (type == Multitone && index == 0 && state()->caps.multitone_max > 0)
        p.max = std::min(p.max > 0 ? p.max : double(state()->caps.multitone_max), double(state()->caps.multitone_max));
    return p;
}

QVariantList SiggenController::types() const {
    QVariantList out;
    for (int id = 0; id < int(kTypes.size()); ++id) {
        const TypeRow &row = kTypes[size_t(id)];
        QVariantList params;
        for (int i = 0; i < 4; ++i) {
            const Param p = paramOf(id, i);
            const char *label = row.params[size_t(i)].label;
            params.append(QVariantMap{
                { QStringLiteral("label"), label ? tr(label) : QStringLiteral("p%1").arg(i + 1) },
                { QStringLiteral("semantic"), p.semantic },
                { QStringLiteral("min"), p.min },
                { QStringLiteral("max"), p.max },
                { QStringLiteral("def"), p.def },
            });
        }
        out.append(QVariantMap{
            { QStringLiteral("id"), id },
            { QStringLiteral("label"), tr(row.tile) },
            { QStringLiteral("name"), tr(row.name) },
            { QStringLiteral("blurb"), tr(row.blurb) },
            { QStringLiteral("timing"), timingOf(id) },
            { QStringLiteral("params"), params },
        });
    }
    return out;
}

QVariantList SiggenController::params() const {
    QVariantList out;
    for (float v : state()->draft.p) out.append(double(v));
    return out;
}

bool SiggenController::walking() const {
    return (state()->draft.flags & SIGGEN_FLAG_WALK) != 0 || state()->draft.signal_type == ChannelId;
}

// ---- The state as QML reads it ----

bool SiggenController::available() const { return state()->supported; }
int SiggenController::mostTones() const { return state()->caps.multitone_max; }
int SiggenController::signalType() const { return state()->draft.signal_type; }
int SiggenController::channelMask() const { return state()->draft.channel_mask; }
int SiggenController::invertMask() const { return state()->draft.invert_mask; }
int SiggenController::flags() const { return state()->draft.flags; }
double SiggenController::levelDb() const { return state()->draft.level_db; }
double SiggenController::durationSeconds() const { return state()->draft.duration_ms / 1000.0; }
int SiggenController::repeat() const { return state()->draft.repeat; }
int SiggenController::gapMs() const { return state()->draft.gap_ms; }
int SiggenController::timing() const { return timingOf(state()->draft.signal_type); }
bool SiggenController::playing() const { return state()->status.state != Idle; }
int SiggenController::phase() const { return state()->status.state; }
int SiggenController::walkOutput() const { return state()->status.active_channel; }
bool SiggenController::windowShown() const { return m_shown; }
Poller::Id SiggenController::statusPoll() const { return m_poll; }

// ---- Edits ----

void SiggenController::edit(const std::function<void(SiggenConfig &)> &change) {
    SiggenConfig next = state()->draft;
    change(next);
    if (sameConfig(next, state()->draft)) return;
    state()->draft = next;
    emit draftEdited();
    emit captionsChanged();
    if (playing() && controlsEnabled()) m_live.start();
}

void SiggenController::selectType(int id) {
    if (!validType(id)) return;
    edit([&](SiggenConfig &c) {
        c.signal_type = uint8_t(id);
        for (int i = 0; i < 4; ++i) c.p[i] = float(paramOf(id, i).def);
        c.duration_ms = timingOf(id) == Sweep ? 5000 : 0;
        c.repeat = 0;
        c.gap_ms = 0;
    });
}

void SiggenController::stepOutput(int output) {
    if (output < 0 || output >= 16) return;
    const uint16_t bit = uint16_t(1u << output);
    edit([&](SiggenConfig &c) {
        if (!(c.channel_mask & bit)) c.channel_mask |= bit;
        else if (!(c.invert_mask & bit)) c.invert_mask |= bit;
        else {
            c.channel_mask &= uint16_t(~bit);
            c.invert_mask &= uint16_t(~bit);
        }
    });
}

void SiggenController::allOutputs() {
    const int n = std::clamp(outputCount(), 0, 16);
    const uint16_t all = state()->supported && state()->caps.valid_mask ? state()->caps.valid_mask : uint16_t((1u << n) - 1u);
    edit([&](SiggenConfig &c) { c.channel_mask = all; });
}

void SiggenController::noOutputs() {
    edit([](SiggenConfig &c) {
        c.channel_mask = 0;
        c.invert_mask = 0;
    });
}

void SiggenController::setLevel(double db, bool fromSlider) {
    const double v = std::clamp(finiteOr(db, state()->draft.level_db), fromSlider ? kSliderMin : kLevelMin, kLevelMax);
    edit([&](SiggenConfig &c) { c.level_db = float(v); });
}

void SiggenController::setParameter(int index, double value) {
    if (index < 0 || index > 3 || !std::isfinite(value)) return;
    const Param p = paramOf(state()->draft.signal_type, index);
    double v = value;
    if (p.semantic == Count || p.semantic == Cycles || p.semantic == PatternChoice) v = std::round(v);
    v = std::max(v, p.min);
    if (p.max > 0) v = std::min(v, p.max);
    edit([&](SiggenConfig &c) { c.p[index] = float(v); });
}

void SiggenController::setDurationSeconds(double seconds, double minimum) {
    if (!std::isfinite(seconds)) return;
    const double s = std::min(std::max(seconds, std::max(0.0, minimum)), 4294967.0);
    edit([&](SiggenConfig &c) { c.duration_ms = uint32_t(s * 1000.0); });
}

void SiggenController::setRepeat(double count) {
    if (!std::isfinite(count)) return;
    edit([&](SiggenConfig &c) { c.repeat = uint16_t(std::clamp(std::round(count), 0.0, 65535.0)); });
}

void SiggenController::setGapMs(double ms) {
    if (!std::isfinite(ms)) return;
    edit([&](SiggenConfig &c) { c.gap_ms = uint16_t(std::clamp(std::round(ms), 0.0, 65535.0)); });
}

void SiggenController::setOption(int flag, bool on) {
    if (flag != SIGGEN_FLAG_RAW && flag != SIGGEN_FLAG_DECORR && flag != SIGGEN_FLAG_WALK) return;
    edit([&](SiggenConfig &c) { c.flags = on ? uint8_t(c.flags | flag) : uint8_t(c.flags & ~flag); });
}

void SiggenController::applyPreset(const QString &which) {
    if (state()->draft.signal_type != TonePair) return;
    const bool smpte = which == QLatin1String("smpte");
    if (!smpte && which != QLatin1String("ccif")) return;
    edit([&](SiggenConfig &c) {
        c.p[0] = smpte ? 60 : 19000;
        c.p[1] = smpte ? 7000 : 20000;
        c.p[2] = smpte ? 4 : 1;
    });
}

// ---- Transport ----

void SiggenController::play() {
    if (!controlsEnabled() || !startBlocker().isEmpty()) return;
    m_live.stop();
    sendStart(state()->draft);
}

// -- WP MDQT2c -- The configuration and a Start, to the active device
void SiggenController::sendStart(const SiggenConfig &cfg) {
    const StatePtr st = state();
    QPointer<DeviceSession> session(m_ctx.activeSession());
    started(st);
    ++st->starts;
    m_ctx.device->write(
        QStringLiteral("siggen"),
        [cfg](FfiCore *c) { return dspi_siggen_set_config(c, &cfg) && dspi_siggen_control(c, SIGGEN_CTL_START); },
        [this, st, session](bool) {
            --st->starts;
            fetchStatus(session, st);
        });
}

void SiggenController::started(const StatePtr &st) {
    st->ours = true;
    st->seen = false;
    st->serial = m_ctx.device->serial();
}

// -- WP MDQT2c -- What was started here on `session`'s device stops (a Start that
// still waits there is replaced by it)
void SiggenController::stopOn(DeviceSession *session) {
    if (!session) return;
    const StatePtr st = sessionState<Device>(session, QStringLiteral("SiggenController"));
    if (shown(st)) m_live.stop();
    if (!st->ours) return;
    st->ours = false;
    // What plays now is a measurement's: it stops its own (or runs on)
    if (session->hasJob(QStringLiteral("MeasureController"))) return;
    QPointer<DeviceSession> held(session);
    session->model()->write(
        QStringLiteral("siggen"), [](FfiCore *c) { return dspi_siggen_control(c, SIGGEN_CTL_STOP); },
        [this, st, held](bool) { fetchStatus(held, st); });
    session->drainFirst();
}

// -- WP QPI3 -- play() for one output's channel-ID blips: -12 dB, two passes,
// then the generator goes idle by itself (Commands.swift's identifyOutput).
// The draft stays; the write is the generator's, so it replaces a queued Start.
void SiggenController::playIdent(int output) {
    if (output < 0 || output >= 16 || !controlsEnabled()) return;
    const uint16_t bit = uint16_t(1u << output);
    const uint16_t mask = bit & (state()->caps.valid_mask ? state()->caps.valid_mask : bit);
    if (!mask) return;
    m_live.stop();
    SiggenConfig cfg{};
    cfg.signal_type = ChannelId;
    cfg.channel_mask = mask;
    cfg.level_db = -12.0f;
    cfg.repeat = 2;
    cfg.p[0] = 120.0f;   // ms per blip
    sendStart(cfg);
}

void SiggenController::stopPlaying(bool immediate) {
    m_live.stop();
    const uint16_t action = immediate ? SIGGEN_CTL_STOP_NOW : SIGGEN_CTL_STOP;
    const StatePtr st = state();
    QPointer<DeviceSession> session(m_ctx.activeSession());
    m_ctx.device->write(
        QStringLiteral("siggen"), [action](FfiCore *c) { return dspi_siggen_control(c, action); },
        [this, st, session](bool) { fetchStatus(session, st); });
}

void SiggenController::toggle() {
    if (playing()) {
        if (controlsEnabled()) stopPlaying(false);
    } else {
        play();
    }
}

void SiggenController::sendLive() {
    // The device changed, the generator stopped, or the draft cannot play: nothing
    if (!playing() || !startBlocker().isEmpty() || m_ctx.device->snapshot().generation != state()->generation) return;
    const StatePtr st = state();
    QPointer<DeviceSession> session(m_ctx.activeSession());
    const SiggenConfig cfg = st->draft;
    st->ours = true;   // -- WP MDQT2c -- what plays is this page's from now on
    st->seen = true;
    st->serial = m_ctx.device->serial();
    m_ctx.device->write(
        QStringLiteral("siggen"), [cfg](FfiCore *c) { return dspi_siggen_set_config(c, &cfg); },
        [this, st, session](bool) { fetchStatus(session, st); });
}

// ---- Device ----

void SiggenController::onSnapshot() {
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    const StatePtr st = state();
    // -- WP MDQT2c -- another device is active: its draft and its status
    if (st != m_announced) {
        const bool first = !m_announced;
        m_announced = st;
        m_live.stop();
        updatePoll();
        if (!first) {
            emit draftEdited();
            emit playbackChanged();
            fetchStatus();
        }
    }
    if (!snap.connected) {
        if (st->generation != 0) {
            // Disconnected: the status goes back to idle; the capabilities stay
            // (the Mac keeps them until the next device says otherwise)
            st->generation = 0;
            m_live.stop();
            SiggenStatus idle{};
            idle.active_channel = kNotWalking;
            // A lost connection does not end what plays on the device
            const bool ours = st->ours;
            setStatus(st, idle);
            st->ours = ours;
        }
        emit generatorChanged();
        emit captionsChanged();
        return;
    }
    if (snap.generation != st->generation) {
        st->generation = snap.generation;
        if (st->serial != m_ctx.device->serial()) st->ours = false;
        m_live.stop();
        fetchAll();
    }
    // The outputs' names and the read-only state follow every snapshot
    emit generatorChanged();
    emit captionsChanged();
}

void SiggenController::onState(uint flags) {
    if (flags & NOTIFY_SIGGEN) fetchStatus();
}

void SiggenController::fetchAll() {
    const StatePtr st = state();
    const uint64_t generation = st->generation;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            Fetched f;
            f.capsOk = dspi_siggen_fetch_caps(c, &f.caps);
            if (f.capsOk && f.caps.supported) {
                f.configOk = dspi_siggen_get_config(c, &f.config);
                f.statusOk = dspi_siggen_get_status(c, &f.status);
            }
            return f;
        },
        [this, st, generation](const Fetched &f) {
            if (generation != st->generation) return;
            st->supported = f.capsOk && f.caps.supported;
            st->caps = st->supported ? f.caps : SiggenCaps{};
            if (st->supported) {
                if (f.configOk && f.config.channel_mask != 0 && validType(f.config.signal_type)) {
                    st->draft = f.config;
                } else {
                    st->draft.channel_mask &= st->caps.valid_mask ? st->caps.valid_mask : 0xFFFF;
                    if (st->draft.channel_mask == 0) st->draft.channel_mask = 1;
                }
                if (f.statusOk) setStatus(st, f.status);
            }
            if (!shown(st)) return;
            emit generatorChanged();
            emit draftEdited();
            emit captionsChanged();
        });
}

void SiggenController::fetchStatus() { fetchStatus(m_ctx.activeSession(), state()); }

void SiggenController::fetchStatus(DeviceSession *session, const StatePtr &st) {
    if (!session || !session->model()->connected()) return;
    QPointer<DeviceSession> held(session);
    session->worker()->call(
        this,
        [](FfiCore *c) {
            std::pair<bool, SiggenStatus> r{ false, {} };
            r.first = dspi_siggen_get_status(c, &r.second);
            return r;
        },
        [this, held, st](const std::pair<bool, SiggenStatus> &r) {
            if (r.first && held && held->model()->connected()) setStatus(st, r.second);
        });
}

void SiggenController::setStatus(const StatePtr &st, const SiggenStatus &s) {
    if (s.state != Idle) st->seen = true;
    else if (st->seen && st->starts == 0) st->ours = false;
    const SiggenStatus &was = st->status;
    const bool same = s.state == was.state && s.signal_type == was.signal_type
        && s.active_channel == was.active_channel && s.elapsed_ms == was.elapsed_ms
        && s.cycles_done == was.cycles_done && s.stop_reason == was.stop_reason
        && s.current_freq == was.current_freq;
    if (same) return;
    st->status = s;
    if (!shown(st)) return;
    updatePoll();
    emit playbackChanged();
    emit captionsChanged();
}

void SiggenController::setWindowShown(bool shown) {
    if (m_shown == shown) return;
    m_shown = shown;
    // Opening the window reads the status once (an Identify may have started it)
    if (shown) fetchStatus();
    emit windowShownChanged();
}

void SiggenController::updatePoll() {
    if (m_ctx.poller) m_ctx.poller->setEnabled(m_poll, playing());
}

// ---- Texts ----

QString SiggenController::phaseText() const {
    switch (state()->status.state) {
    case FadeIn: return QCoreApplication::translate("SiggenController", "Fading in");
    case Run: return QCoreApplication::translate("SiggenController", "Running");
    case Gap: return QCoreApplication::translate("SiggenController", "Gap");
    case FadeOut: return QCoreApplication::translate("SiggenController", "Fading out");
    default: return QCoreApplication::translate("SiggenController", "Idle");
    }
}

QString SiggenController::startBlocker() const {
    if (!connected()) return QCoreApplication::translate("SiggenController", "No device connected");
    if (!state()->supported) return QCoreApplication::translate("SiggenController", "Firmware has no signal generator");
    if (state()->draft.channel_mask == 0) return QCoreApplication::translate("SiggenController", "Select at least one output");
    if (timing() == Sweep && state()->draft.duration_ms == 0)
        return QCoreApplication::translate("SiggenController", "Sweep length must be greater than 0");
    return QString();
}

QString SiggenController::transportTitle() const {
    const auto nameOf = [](int type) { return validType(type) ? tr(kTypes[size_t(type)].name) : QString(); };
    if (playing()) return phaseText() + QStringLiteral(" · ") + nameOf(state()->status.signal_type);
    const QString blocker = startBlocker();
    if (!blocker.isEmpty() && connected() && state()->supported) return blocker;
    return QCoreApplication::translate("SiggenController", "Ready · %1").arg(nameOf(state()->draft.signal_type));
}

QString SiggenController::transportDetail() const {
    if (playing()) {
        QStringList parts{ formatElapsed(state()->status.elapsed_ms) };
        if (state()->status.current_freq > 0) parts.append(formatFrequency(state()->status.current_freq));
        if (state()->status.cycles_done > 0) parts.append(QCoreApplication::translate("SiggenController", "cycle %1").arg(state()->status.cycles_done));
        if (state()->status.active_channel != kNotWalking) parts.append(outputName(state()->status.active_channel));
        parts.append(QCoreApplication::translate("SiggenController", "edits apply live"));
        return parts.join(QStringLiteral(" · "));
    }
    switch (state()->status.stop_reason) {
    case StopCompleted:
        return QCoreApplication::translate("SiggenController", "Finished after %1").arg(formatElapsed(state()->status.elapsed_ms));
    case StopHost: return QCoreApplication::translate("SiggenController", "Stopped");
    case StopPreset: return QCoreApplication::translate("SiggenController", "Stopped by preset load");
    default: break;
    }
    const int n = int(qPopulationCount(quint32(state()->draft.channel_mask)));
    const QString level = QString::number(double(state()->draft.level_db), 'f', 1);
    return n == 1 ? QCoreApplication::translate("SiggenController", "1 output · peak %1 dBFS").arg(level)
                  : QCoreApplication::translate("SiggenController", "%1 outputs · peak %2 dBFS").arg(n).arg(level);
}

QString SiggenController::outputName(int output) const {
    const ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows ? rows->rowOfChannel(m_ctx.device->numInputs() + output) : -1;
    if (row >= 0) {
        const QString label = rows->get(row, QStringLiteral("label")).toString();
        if (!label.isEmpty()) return label;
    }
    return QCoreApplication::translate("SiggenController", "Out %1").arg(output + 1);
}

bool SiggenController::outputEnabled(int output) const {
    const ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows ? rows->rowOfChannel(m_ctx.device->numInputs() + output) : -1;
    return row < 0 || rows->get(row, QStringLiteral("outputEnabled")).toBool();
}

QString SiggenController::formatElapsed(double ms) {
    const double s = std::max(0.0, ms / 1000.0);
    if (s < 60) return QStringLiteral("%1 s").arg(s, 0, 'f', 1);
    const int minutes = int(std::floor(s / 60));
    return QStringLiteral("%1:%2").arg(minutes).arg(std::fmod(s, 60.0), 4, 'f', 1, QLatin1Char('0'));
}

QString SiggenController::formatFrequency(double hz) {
    if (hz >= 1000) return QStringLiteral("%1 kHz").arg(hz / 1000.0, 0, 'f', 2);
    return QStringLiteral("%1 Hz").arg(hz, 0, 'f', 0);
}

QString SiggenController::unitFor(int semantic) {
    switch (semantic) {
    case FreqHz: return QStringLiteral("Hz");
    case Ms: return QStringLiteral("ms");
    case Cycles: return QCoreApplication::translate("SiggenController", "cyc");
    case Ratio: return QStringLiteral("×");
    default: return QString();
    }
}
