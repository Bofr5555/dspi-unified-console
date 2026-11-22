// -- WP QP8 -- The subharmonic, tube, upmixer and limiter controller (ToolsController.h).
#include "ToolsController.h"

#include "CoreWorker.h"   // -- WP P1QT2 --

#include "ChannelListModel.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "SessionState.h"
#include "HardwareController.h"
#include "Shell.h"

#include <QPointer>
#include <QtGlobal>
#include <algorithm>
#include <array>
#include <cmath>
#include <vector>

static const Registrar::Controller<ToolsController> toolsController;
static const Registrar::Window subharmonicWindow("tools.subharmonic", "SubharmonicWindow");
static const Registrar::Window tubeWindow("tools.tube", "TubeWindow");
static const Registrar::Window upmixerWindow("tools.upmixer", "UpmixerWindow");

namespace {

// One parameter of an effect: its key, the core's id, the Mac's range and default
struct Param {
    const char *key;
    uint8_t id;
    float lo, hi, def;
};

const std::array<Param, 11> kSubharm{ {
    { "enabled", SUBHARM_PARAM_ENABLED, 0, 1, 0 },
    { "mask", SUBHARM_PARAM_MASK, 0, 65535, 65535 },
    { "low", SUBHARM_PARAM_LOW, -30, 12, 0 },
    { "high", SUBHARM_PARAM_HIGH, -30, 12, 0 },
    { "top", SUBHARM_PARAM_TOP, -30, 12, -30 },
    { "boost", SUBHARM_PARAM_BOOST, 0, 6, 0 },
    { "select", SUBHARM_PARAM_SELECT, 0, 2, 0 },
    { "depth", SUBHARM_PARAM_DEPTH, 0, 100, 100 },
    { "hold", SUBHARM_PARAM_HOLD, 50, 400, 150 },
    { "ceiling", SUBHARM_PARAM_CEILING, -40, 0, 0 },
    { "link", SUBHARM_PARAM_LINK, 0, 1, 1 },
} };

const std::array<Param, 14> kTube{ {
    { "enabled", TUBE_PARAM_ENABLED, 0, 1, 0 },
    { "mask", TUBE_PARAM_MASK, 0, 65535, 65535 },
    { "type", TUBE_PARAM_TYPE, 0, 16, 1 },
    { "drive", TUBE_PARAM_DRIVE, -30, 24, -12 },
    { "bias", TUBE_PARAM_BIAS, -100, 100, 10 },
    { "asym", TUBE_PARAM_ASYM, -12, 12, 3 },
    { "hardness", TUBE_PARAM_HARDNESS, 0, 100, 40 },
    { "sag", TUBE_PARAM_SAG, 0, 100, 15 },
    { "rectifier", TUBE_PARAM_RECTIFIER, 0, 3, 1 },
    { "xfmr", TUBE_PARAM_XFMR, 0, 1, 1 },
    { "damping", TUBE_PARAM_DAMPING, 1, 20, 2 },
    { "resonance", TUBE_PARAM_RESONANCE, 30, 150, 95 },
    { "mix", TUBE_PARAM_MIX, 0, 100, 100 },
    { "trim", TUBE_PARAM_TRIM, -12, 12, 0 },
} };

const std::array<Param, 14> kUpmix{ {
    { "enabled", UPMIX_PARAM_ENABLED, 0, 1, 0 },
    { "center", UPMIX_PARAM_CENTER_MODE, 0, 2, 1 },
    { "surround", UPMIX_PARAM_SURROUND_MODE, 0, 2, 2 },
    { "strength", UPMIX_PARAM_STRENGTH, 0, 100, 100 },
    { "width", UPMIX_PARAM_WIDTH, 0, 100, 25 },
    { "threshold", UPMIX_PARAM_THRESHOLD, 0, 95, 30 },
    { "attack", UPMIX_PARAM_ATTACK, 1, 500, 10 },
    { "release", UPMIX_PARAM_RELEASE, 5, 2000, 100 },
    { "detector", UPMIX_PARAM_DETECTOR_HPF, 20, 1000, 200 },
    { "delay", UPMIX_PARAM_SURROUND_DELAY, 0, 20, 12 },
    { "hpf", UPMIX_PARAM_SURROUND_HPF, 20, 2000, 300 },
    { "lpf", UPMIX_PARAM_SURROUND_LPF, 1000, 20000, 7000 },
    { "decorrelation", UPMIX_PARAM_DECORRELATION, 0, 100, 90 },
    { "presence", UPMIX_PARAM_PRESENCE, -12, 12, 0 },
} };

template <size_t N>
const Param *find(const std::array<Param, N> &table, const QString &key) {
    for (const Param &p : table)
        if (key == QLatin1String(p.key)) return &p;
    return nullptr;
}

float clampTo(double v, float lo, float hi) { return std::isfinite(v) ? std::clamp(float(v), lo, hi) : lo; }

// The subharmonic starting points (the Mac's table): low, high, boost; each sets the top band to its floor
struct SubharmPoint {
    const char *name;
    const char *detail;
    float low, high, boost;
};
const std::array<SubharmPoint, 4> kSubharmPoints{ {
    { QT_TRANSLATE_NOOP("ToolsController", "Subwoofer feed"), QT_TRANSLATE_NOOP("ToolsController", "Subtle added weight"),
      -6, -6, 0 },
    { QT_TRANSLATE_NOOP("ToolsController", "Club / large PA"), QT_TRANSLATE_NOOP("ToolsController", "Lower two bands, full"),
      0, 0, 3 },
    { QT_TRANSLATE_NOOP("ToolsController", "Thin recordings"), QT_TRANSLATE_NOOP("ToolsController", "Add a missing bottom"),
      0, -6, 3 },
    { QT_TRANSLATE_NOOP("ToolsController", "Cinema LFE"), QT_TRANSLATE_NOOP("ToolsController", "Lowest octave only"), 3,
      -12, 0 },
} };

// The tube types (the Mac's TUBE_TYPE_ROWS); index 0 is Custom. The character
// values themselves are the core's: choosing a type loads them there.
struct TubeRow {
    const char *name;
    const char *style;
    bool pushPull;
};
const std::array<TubeRow, 17> kTubeRows{ {
    { QT_TRANSLATE_NOOP("ToolsController", "Custom"), "", false },
    { "12AX7 / ECC83", QT_TRANSLATE_NOOP("ToolsController", "High-gain preamp triode"), false },
    { "5751", QT_TRANSLATE_NOOP("ToolsController", "Cooler 12AX7"), false },
    { "12AT7 / ECC81", QT_TRANSLATE_NOOP("ToolsController", "Medium-gain driver, more odd-order"), false },
    { "12AY7", QT_TRANSLATE_NOOP("ToolsController", "Tweed front end, gentle"), false },
    { "12AU7 / ECC82", QT_TRANSLATE_NOOP("ToolsController", "Clean line stage"), false },
    { "6SN7", QT_TRANSLATE_NOOP("ToolsController", "Octal hi-fi line stage, sweet"), false },
    { "6SL7", QT_TRANSLATE_NOOP("ToolsController", "Octal high-mu, rounder knee"), false },
    { "6DJ8 / ECC88 / 6922", QT_TRANSLATE_NOOP("ToolsController", "Clean, hard when pushed"), false },
    { "EF86 / 6267", QT_TRANSLATE_NOOP("ToolsController", "Pentode preamp, symmetric bite"), false },
    { "6SJ7", QT_TRANSLATE_NOOP("ToolsController", "Octal pentode, softer than EF86"), false },
    { "EL84 / 6BQ5", QT_TRANSLATE_NOOP("ToolsController", "Push-pull power, chimey"), true },
    { "EL34", QT_TRANSLATE_NOOP("ToolsController", "Push-pull power, mid crunch, deep sag"), true },
    { "6L6 / 5881", QT_TRANSLATE_NOOP("ToolsController", "Push-pull power, tight"), true },
    { "6V6", QT_TRANSLATE_NOOP("ToolsController", "Push-pull power, early breakup, heavy sag"), true },
    { "KT88 / 6550", QT_TRANSLATE_NOOP("ToolsController", "Push-pull hi-fi power, near linear"), true },
    { "300B / 2A3", QT_TRANSLATE_NOOP("ToolsController", "Single-ended DHT, pure even harmonics"), false },
} };

// The rectifiers (the Mac's TUBE_RECTIFIER_ROWS): sag depth scale, attack and release ms
struct RectifierRow {
    const char *name;
    float depth, attack, release;
};
const std::array<RectifierRow, 4> kRectifiers{ {
    { QT_TRANSLATE_NOOP("ToolsController", "Solid state"), 0.0f, 0, 0 },
    { "GZ34", 0.6f, 5, 120 },
    { "5U4", 1.0f, 8, 200 },
    { "5Y3", 1.3f, 10, 300 },
} };

// The tube starting points (the Mac's table): type, drive, rectifier, output stage, damping, resonance
struct TubePoint {
    const char *name;
    const char *detail;
    int type;
    float drive;
    int rectifier;
    bool xfmr;
    float damping, resonance;
};
const std::array<TubePoint, 5> kTubePoints{ {
    { QT_TRANSLATE_NOOP("ToolsController", "Clean default"),
      QT_TRANSLATE_NOOP("ToolsController", "12AX7, level-neutral, output stage on"), 1, -12, 1, true, 2, 95 },
    { QT_TRANSLATE_NOOP("ToolsController", "Warm hi-fi"), QT_TRANSLATE_NOOP("ToolsController", "12AU7 line stage, tightly damped"),
      5, -3, 1, true, 10, 95 },
    { QT_TRANSLATE_NOOP("ToolsController", "Single-ended sweetness"), QT_TRANSLATE_NOOP("ToolsController", "300B, loose damping"),
      16, 3, 1, true, 2, 95 },
    { QT_TRANSLATE_NOOP("ToolsController", "Guitar-amp style"),
      QT_TRANSLATE_NOOP("ToolsController", "12AX7 pushed, 5U4, loose damping"), 1, 15, 2, true, 2, 100 },
    { QT_TRANSLATE_NOOP("ToolsController", "Push-pull power"), QT_TRANSLATE_NOOP("ToolsController", "EL34 with the output stage on"),
      12, 0, 1, true, 6, 95 },
} };

// The subharmonic graph's axes (the Mac's SubharmBandView)
constexpr double kSubMinHz = 16, kSubMaxHz = 250;
constexpr double kSubDbMin = -42, kSubDbMax = 18;
constexpr double kBoostHz = 70, kBoostQ = 0.9;
constexpr double kDividerGain = 0.849;
constexpr double kSubFloor = -30, kCeilingOff = 0;
// The transfer curve's output range: a little above full scale, where a hot setting can reach
constexpr double kTubeYMax = 1.4;

// The tube's static waveshaper (the Mac's TubeShaper)
struct Shaper {
    double m, b, ratioN, c1, c3, c5, sP, sN, dryW, wetW, v0;
    Shaper(double drive, double bias, double asym, double hardness, double mix, double trim) {
        m = std::pow(10.0, drive / 20);
        b = bias / 200;
        ratioN = std::pow(10.0, -asym / 20);
        const double h = hardness / 100;
        c1 = 1.5 + 0.375 * h;
        c3 = -0.5 - 0.75 * h;
        c5 = 0.375 * h;
        // 1/m makeup in the half scales: drive moves the knee, not the level
        sP = 1 / (c1 * m);
        sN = std::pow(10.0, asym / 20) / (c1 * m);
        dryW = 1 - mix / 100;
        wetW = mix / 100 * std::pow(10.0, trim / 20);
        v0 = 0;
        v0 = shape(0);
    }
    double shape(double x) const {
        double t = m * x + b;
        if (t < 0) t *= ratioN;
        t = std::clamp(t, -1.0, 1.0);
        const double t2 = t * t;
        return t * (c1 + t2 * (c3 + t2 * c5)) * (t >= 0 ? sP : sN);
    }
    double output(double x) const { return dryW * x + wetW * (shape(x) - v0); }
};

} // namespace

ToolsController::ToolsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, [this]() {
        ++m_reads;
        switched();   // -- WP MDQT2c --
        emit toolsChanged();
        updatePolls();
        readUnprotected();   // -- WP P1QT2 --
    });
    connect(m_ctx.device->channels(), &ChannelListModel::countChanged, this, [this]() {
        ++m_reads;
        emit toolsChanged();
    });
    connect(m_ctx.poller, &Poller::metersChanged, this, &ToolsController::updateGlow);
    // -- WP MDQT2c -- Solo never stays on past the window that shows it: on the
    // device that is left too
    connect(ctx.registry, &DeviceRegistry::activeLeaving, this, [this] {
        if (m_subharmShown && subharmSolo()) setSubharmSolo(false);
    });

    m_subMeterPoll = ctx.poller->add(
        QStringLiteral("subharmonic meter"), 100, this,
        [](FfiCore *c) {
            std::vector<float> v(MAX_OUTPUTS, 0.0f);
            if (!dspi_fetch_subharm_meter(c, v.data())) v.clear();
            return v;
        },
        [this](const std::vector<float> &v) {
            if (v.empty()) return;
            QVariantList levels;
            const int n = std::min(numOutputs(), int(v.size()));
            for (int i = 0; i < n; ++i) levels << std::clamp(double(v[size_t(i)]), 0.0, 1.0);
            const StatePtr st = state();
            if (levels == st->subMeter) return;
            st->subMeter = levels;
            emit subharmMeterChanged();
        });
    ctx.poller->setEnabled(m_subMeterPoll, false);
    m_upmixPoll = ctx.poller->add(
        QStringLiteral("upmixer status"), 60, this,
        [](FfiCore *c) {
            UpmixStatus st{};
            const bool ok = dspi_fetch_upmix_status(c, &st);
            return std::make_pair(ok, st);
        },
        [this](const std::pair<bool, UpmixStatus> &r) {
            if (!r.first) return;
            const UpmixStatus &st = r.second;
            state()->upmixStatus = QVariantMap{ { QStringLiteral("active"), st.active },
                                         { QStringLiteral("parked"), int(st.parked_reason) },
                                         { QStringLiteral("correlation"), double(st.correlation) },
                                         { QStringLiteral("balance"), double(st.balance) },
                                         { QStringLiteral("center"), double(st.center_gain) },
                                         { QStringLiteral("ls"), double(st.ls_gain) },
                                         { QStringLiteral("rs"), double(st.rs_gain) } };
            emit upmixStatusChanged();
        });
    ctx.poller->setEnabled(m_upmixPoll, false);
}

// -- WP MDQT2c --
ToolsController::StatePtr ToolsController::state() const {
    return sessionState<State>(m_ctx.activeSession(), QStringLiteral("ToolsController"));
}

void ToolsController::switched() {
    const StatePtr st = state();
    if (st == m_announced) return;
    const bool first = !m_announced;
    m_announced = st;
    if (first) return;
    emit headroomChanged();
    emit subharmMeterChanged();
    emit upmixStatusChanged();
    emit unprotectedChanged();
    updateGlow();
    if (m_subharmShown) fetchHeadroom();
}

void ToolsController::apply(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done) {
    if (!writable()) return;
    m_ctx.device->write(what, std::move(setter), std::move(done));
}

bool ToolsController::connected() const { return m_ctx.device->connected() && m_ctx.device->state(); }
bool ToolsController::writable() const { return connected() && !m_ctx.device->readOnly(); }
int ToolsController::numOutputs() const {
    const DspState *s = m_ctx.device->state();
    return s ? std::clamp(int(s->num_output_channels), 0, int(MAX_OUTPUTS)) : 0;
}

int ToolsController::allOutputsMask() const {
    const int n = numOutputs();
    return n >= 16 ? 0xFFFF : (1 << n) - 1;
}
int ToolsController::subOnlyMask() const { return pdmOutput() >= 0 ? 1 << pdmOutput() : 0; }
int ToolsController::withoutSubMask() const { return allOutputsMask() & ~subOnlyMask(); }

QString ToolsController::outputName(int output) const {
    const ChannelListModel *list = m_ctx.device->channels();
    for (int row = 0; row < list->rowCount(); ++row) {
        if (list->get(row, QStringLiteral("kind")).toString() != QLatin1String("output") ||
            list->get(row, QStringLiteral("number")).toInt() != output)
            continue;
        const QString label = list->get(row, QStringLiteral("label")).toString();
        if (!label.isEmpty()) return label;
    }
    return tr("Out %1").arg(output + 1);
}

// ---- Windows and the live readings ---------------------------------------------

void ToolsController::windowShown(const QString &window, bool shown) {
    // -- WP MDQT2c -- one more place shows it, or one less; a report of "not
    // shown" by one that never was (a page unloaded) takes none away
    const auto counted = [shown](int &places) {
        places = std::max(0, places + (shown ? 1 : -1));
        return places;
    };
    if (window == QLatin1String("subharmonic")) {
        counted(m_subharmShown);
        if (shown) {
            fetchHeadroom();
        } else if (m_subharmShown == 0 && subharmSolo()) {
            // Solo takes the program off the masked outputs: never past this window
            setSubharmSolo(false);
        }
    } else if (window == QLatin1String("tube")) {
        counted(m_tubeShown);
        updateGlow();
    } else if (window == QLatin1String("upmixer")) {
        counted(m_upmixShown);
    }
    updatePolls();
}

void ToolsController::updatePolls() {
    const bool subOn = connected() && subharm(QStringLiteral("enabled")) != 0;
    m_ctx.poller->setEnabled(m_subMeterPoll, m_subharmShown && subharmExtended() && subOn);
    m_ctx.poller->setEnabled(m_upmixPoll, m_upmixShown && upmixSupported());
    const StatePtr st = state();
    if (!(m_subharmShown && subOn) && !st->subMeter.isEmpty()) {
        st->subMeter.clear();
        emit subharmMeterChanged();
    }
}

void ToolsController::fetchHeadroom() {
    if (subharmSupported()) fetchHeadroom(m_ctx.activeSession(), state());
}

void ToolsController::fetchHeadroom(DeviceSession *session, const StatePtr &st) {
    if (!session) return;
    session->worker()->call(
        this,
        [](FfiCore *c) {
            float db = 0;
            return dspi_fetch_subharm_headroom(c, &db) && std::isfinite(db) ? double(db) : NAN;
        },
        [this, st](double db) {
            if (std::isnan(db) || db == st->headroom) return;
            st->headroom = db;
            if (st == state()) emit headroomChanged();
        });
}

void ToolsController::updateGlow() {
    double glow = 0;
    const DspState *s = m_ctx.device->state();
    if (m_tubeShown && s && tube(QStringLiteral("enabled")) != 0) {
        const QVariantList peaks = m_ctx.poller->peaks();
        const int mask = int(tube(QStringLiteral("mask")));
        const int inputs = int(s->num_input_channels);
        double peak = 0;
        for (int o = 0; o < std::min(numOutputs(), 16); ++o) {
            if (!(mask & (1 << o))) continue;
            const int ch = inputs + o;
            if (ch < 0 || ch >= peaks.size()) continue;
            const double p = peaks.at(ch).toDouble();
            if (std::isfinite(p)) peak = std::max(peak, p);
        }
        // A square-root envelope keeps listening levels visible, in 1/255 steps
        glow = std::round(std::sqrt(std::min(peak, 1.0)) * 255) / 255;
    }
    if (glow == m_glow) return;
    m_glow = glow;
    emit tubeGlowChanged();
}

// ---- Subharmonic synthesizer ---------------------------------------------------

bool ToolsController::subharmSupported() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_SUBHARM) != 0;
}
bool ToolsController::subharmExtended() const {
    return subharmSupported() && (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_SUBHARM_EXT) != 0;
}
bool ToolsController::subharmSolo() const { return connected() && m_ctx.device->state()->subharm_solo; }

double ToolsController::subharm(const QString &key) const {
    const Param *p = find(kSubharm, key);
    if (!p) return 0;
    return connected() ? double(m_ctx.device->state()->subharm[p->id]) : double(p->def);
}

void ToolsController::setSubharm(const QString &key, double value) {
    const Param *p = find(kSubharm, key);
    if (!p || !subharmSupported()) return;
    // The V30 controls stall on V29 firmware
    if (!subharmExtended() && (p->id == SUBHARM_PARAM_TOP || p->id >= SUBHARM_PARAM_SELECT)) return;
    const float v = clampTo(value, p->lo, p->hi);
    const uint8_t id = p->id;
    // The headroom is derived from the switch, the levels, the boost and the ceiling
    const bool headroom = id != SUBHARM_PARAM_MASK && id != SUBHARM_PARAM_LINK && id != SUBHARM_PARAM_SELECT &&
                          id != SUBHARM_PARAM_DEPTH && id != SUBHARM_PARAM_HOLD;
    const StatePtr st = state();
    QPointer<DeviceSession> session(m_ctx.activeSession());
    apply(QStringLiteral("subharm %1").arg(key), [id, v](FfiCore *c) { return dspi_set_subharm_param(c, id, v); },
          [this, headroom, st, session](bool ok) {
              if (ok && headroom) fetchHeadroom(session, st);
          });
}

void ToolsController::setSubharmSolo(bool soloed) {
    if (!subharmExtended()) return;
    apply(QStringLiteral("subharm solo"), [soloed](FfiCore *c) { return dspi_set_subharm_solo(c, soloed); });
}

QVariantList ToolsController::subharmStartingPoints() {
    QVariantList rows;
    for (const SubharmPoint &p : kSubharmPoints)
        rows << QVariantMap{ { QStringLiteral("name"), tr(p.name) }, { QStringLiteral("detail"), tr(p.detail) } };
    return rows;
}

void ToolsController::applySubharmStartingPoint(int index) {
    if (index < 0 || index >= int(kSubharmPoints.size())) return;
    const SubharmPoint &p = kSubharmPoints[size_t(index)];
    setSubharm(QStringLiteral("low"), p.low);
    setSubharm(QStringLiteral("high"), p.high);
    setSubharm(QStringLiteral("boost"), p.boost);
    // None of them uses the top band: half a starting point is worse than none
    if (subharmExtended()) setSubharm(QStringLiteral("top"), kSubFloor);
}

void ToolsController::toggleSubharmOutput(int output) {
    if (output < 0 || output >= 16) return;
    setSubharm(QStringLiteral("mask"), int(subharm(QStringLiteral("mask"))) ^ (1 << output));
}

double ToolsController::subharmX(double hz) {
    const double f = std::clamp(hz, kSubMinHz, kSubMaxHz);
    return (std::log10(f) - std::log10(kSubMinHz)) / (std::log10(kSubMaxHz) - std::log10(kSubMinHz));
}

// The 70 Hz LF boost bell: the analog peaking prototype, peak gain A^2 with A = 10^(boost/40)
double ToolsController::subharmBellDb(double boostDb, double hz) {
    if (boostDb <= 0) return 0;
    const double a = std::pow(10.0, boostDb / 40);
    const double w = hz / kBoostHz;
    const double base = (1 - w * w) * (1 - w * w);
    const double num = base + std::pow(a * w / kBoostQ, 2);
    const double den = base + std::pow(w / (a * kBoostQ), 2);
    return 10 * std::log10(num / den);
}

// A synthesized sub's steady level: the band, the divider's own gain, the bell there
double ToolsController::subharmSubDb(double bandDb, double boostDb, double hz) {
    return bandDb + 20 * std::log10(kDividerGain) + subharmBellDb(boostDb, hz);
}

QVariantMap ToolsController::subharmGraph(double low, double high, double top, double boost, double ceiling) const {
    const auto yOf = [](double db) { return (kSubDbMax - std::clamp(db, kSubDbMin, kSubDbMax)) / (kSubDbMax - kSubDbMin); };
    QVariantMap out;
    QVariantList bell;
    if (boost > 0) {
        for (int i = 0; i <= 96; ++i) {
            const double t = i / 96.0;
            const double f = std::pow(10.0, std::log10(kSubMinHz) + t * (std::log10(kSubMaxHz) - std::log10(kSubMinHz)));
            bell << t << yOf(subharmBellDb(boost, f));
        }
    }
    out.insert(QStringLiteral("bell"), bell);
    out.insert(QStringLiteral("bellLabelY"), yOf(subharmBellDb(boost, kBoostHz)));
    out.insert(QStringLiteral("bellX"), subharmX(kBoostHz));

    // The three subs an octave below the program bands they come from
    const struct {
        double lo, hi, srcLo, srcHi, level, from, to, row;
    } bands[3] = { { 24, 36, 48, 72, low, 60, 30, 15 },
                   { 36, 56, 72, 112, high, 92, 46, 23 },
                   { 56, 80, 112, 160, top, 136, 68, 31 } };
    QVariantList blocks;
    bool allOff = true;
    for (const auto &b : bands) {
        const bool off = b.level <= kSubFloor;
        allOff = allOff && off;
        QVariantList top;
        if (!off) {
            for (int i = 0; i <= 16; ++i) {
                const double t = i / 16.0;
                const double f = b.lo * std::pow(b.hi / b.lo, t);
                top << subharmX(f) << yOf(subharmSubDb(b.level, boost, f));
            }
        }
        blocks << QVariantMap{ { QStringLiteral("off"), off },
                               { QStringLiteral("x0"), subharmX(b.lo) },
                               { QStringLiteral("x1"), subharmX(b.hi) },
                               { QStringLiteral("srcX0"), subharmX(b.srcLo) },
                               { QStringLiteral("srcX1"), subharmX(b.srcHi) },
                               { QStringLiteral("midY"), yOf(subharmSubDb(b.level, boost, std::sqrt(b.lo * b.hi))) },
                               { QStringLiteral("label"), QStringLiteral("%1-%2").arg(int(b.lo)).arg(int(b.hi)) },
                               { QStringLiteral("source"), QStringLiteral("%1-%2").arg(int(b.srcLo)).arg(int(b.srcHi)) },
                               // The octave arrow: from the program band to its sub, on its own row
                               { QStringLiteral("fromX"), subharmX(b.from) },
                               { QStringLiteral("toX"), subharmX(b.to) },
                               { QStringLiteral("arrowRow"), b.row },
                               { QStringLiteral("top"), top } };
    }
    out.insert(QStringLiteral("blocks"), blocks);
    out.insert(QStringLiteral("allOff"), allOff);
    out.insert(QStringLiteral("ceilingOn"), ceiling < kCeilingOff && !allOff);
    out.insert(QStringLiteral("ceilingY"), yOf(ceiling));
    out.insert(QStringLiteral("ceilingX"), subharmX(80));
    out.insert(QStringLiteral("zeroY"), yOf(0));
    QVariantList gridX, freqs, gridY, gridDb;
    for (int f : { 20, 50, 100, 200 }) {
        gridX << subharmX(f);
        freqs << f;
    }
    for (int db : { 12, 0, -12, -24, -36 }) {
        gridY << yOf(db);
        gridDb << db;
    }
    out.insert(QStringLiteral("gridX"), gridX);
    out.insert(QStringLiteral("freqs"), freqs);
    out.insert(QStringLiteral("gridY"), gridY);
    out.insert(QStringLiteral("gridDb"), gridDb);
    return out;
}

// ---- Tube modeller ----------------------------------------------------------------

bool ToolsController::tubeSupported() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_TUBE) != 0;
}

double ToolsController::tube(const QString &key) const {
    const Param *p = find(kTube, key);
    if (!p) return 0;
    return connected() ? double(m_ctx.device->state()->tube[p->id]) : double(p->def);
}

void ToolsController::setTube(const QString &key, double value) {
    const Param *p = find(kTube, key);
    if (!p || !tubeSupported()) return;
    const bool whole = p->id == TUBE_PARAM_TYPE || p->id == TUBE_PARAM_RECTIFIER || p->id == TUBE_PARAM_MASK ||
                       p->id == TUBE_PARAM_ENABLED || p->id == TUBE_PARAM_XFMR;
    float v = clampTo(value, p->lo, p->hi);
    if (whole) v = std::round(v);
    const uint8_t id = p->id;
    apply(QStringLiteral("tube %1").arg(key), [id, v](FfiCore *c) { return dspi_set_tube_param(c, id, v); });
}

QVariantList ToolsController::tubeTypes() {
    QVariantList rows;
    for (const TubeRow &r : kTubeRows) {
        const QString name = r.style[0] ? QString::fromLatin1(r.name) : tr(r.name);
        rows << QVariantMap{ { QStringLiteral("name"), name },
                             { QStringLiteral("shortName"), name.section(QStringLiteral(" / "), 0, 0) },
                             { QStringLiteral("style"), r.style[0] ? tr(r.style) : QString() },
                             { QStringLiteral("pushPull"), r.pushPull } };
    }
    return rows;
}

QVariantList ToolsController::tubeRectifiers() {
    QVariantList rows;
    for (const RectifierRow &r : kRectifiers)
        rows << QVariantMap{ { QStringLiteral("name"), r.depth > 0 ? QString::fromLatin1(r.name) : tr(r.name) },
                             { QStringLiteral("depth"), double(r.depth) },
                             { QStringLiteral("attack"), double(r.attack) },
                             { QStringLiteral("release"), double(r.release) } };
    return rows;
}

QVariantList ToolsController::tubeStartingPoints() {
    QVariantList rows;
    for (const TubePoint &p : kTubePoints)
        rows << QVariantMap{ { QStringLiteral("name"), tr(p.name) }, { QStringLiteral("detail"), tr(p.detail) } };
    return rows;
}

void ToolsController::applyTubeStartingPoint(int index) {
    if (index < 0 || index >= int(kTubePoints.size())) return;
    const TubePoint &p = kTubePoints[size_t(index)];
    // Every control but the character, which the type loads: the same sound whatever came before
    setTube(QStringLiteral("type"), p.type);
    setTube(QStringLiteral("drive"), p.drive);
    setTube(QStringLiteral("rectifier"), p.rectifier);
    setTube(QStringLiteral("damping"), p.damping);
    setTube(QStringLiteral("resonance"), p.resonance);
    setTube(QStringLiteral("xfmr"), p.xfmr ? 1 : 0);
    setTube(QStringLiteral("mix"), 100);
    setTube(QStringLiteral("trim"), 0);
}

void ToolsController::toggleTubeOutput(int output) {
    if (output < 0 || output >= 16) return;
    setTube(QStringLiteral("mask"), int(tube(QStringLiteral("mask"))) ^ (1 << output));
}

// 0 noval triode, 1 noval pentode, 2 noval power, 3 octal glass, 4 octal metal,
// 5 large octal power, 6 small octal power, 7 shouldered power, 8 beam bottle, 9 directly heated
int ToolsController::tubeFamily(int type) {
    switch (type) {
    case 6:
    case 7: return 3;
    case 9: return 1;
    case 10: return 4;
    case 11: return 2;
    case 12: return 5;
    case 13: return 7;
    case 14: return 6;
    case 15: return 8;
    case 16: return 9;
    default: return 0;
    }
}

double ToolsController::tubeOutput(double drive, double bias, double asym, double hardness, double mix, double trim,
                                   double x) {
    return Shaper(drive, bias, asym, hardness, mix, trim).output(x);
}

// The second and third harmonic of a full-scale sine against the fundamental:
// a 256-point DFT at three bins, exact for a curve without memory
void ToolsController::tubeHarmonics(double drive, double bias, double asym, double hardness, double mix, double trim,
                                    double &second, double &third) {
    const Shaper s(drive, bias, asym, hardness, mix, trim);
    constexpr int n = 256;
    constexpr double pi = 3.14159265358979323846;
    double re[3] = {}, im[3] = {};
    for (int i = 0; i < n; ++i) {
        const double phase = 2 * pi * i / n;
        const double y = s.output(std::sin(phase));
        for (int k = 0; k < 3; ++k) {
            re[k] += y * std::cos((k + 1) * phase);
            im[k] += y * std::sin((k + 1) * phase);
        }
    }
    double mag[3];
    for (int k = 0; k < 3; ++k) mag[k] = std::hypot(re[k], im[k]);
    const auto rel = [&](double a) { return mag[0] > 1e-12 && a > mag[0] * 1e-6 ? 20 * std::log10(a / mag[0]) : -120.0; };
    second = rel(mag[1]);
    third = rel(mag[2]);
}

QVariantMap ToolsController::tubeGraph(double drive, double bias, double asym, double hardness, double mix,
                                       double trim) const {
    const auto xOf = [](double x) { return (x + 1) / 2; };
    const auto yOf = [](double y) { return (kTubeYMax - std::clamp(y, -kTubeYMax, kTubeYMax)) / (2 * kTubeYMax); };
    const Shaper s(drive, bias, asym, hardness, mix, trim);
    QVariantList curve;
    for (int i = 0; i <= 240; ++i) {
        const double x = -1 + 2.0 * i / 240;
        curve << xOf(x) << yOf(s.output(x));
    }
    // The inputs the stage clips: m x + b = 1, and (m x + b) ratio = -1
    const double xPos = (1 - s.b) / s.m;
    const double xNeg = (-1 / s.ratioN - s.b) / s.m;
    double second = -120, third = -120;
    tubeHarmonics(drive, bias, asym, hardness, mix, trim, second, third);
    return QVariantMap{ { QStringLiteral("curve"), curve },
                        { QStringLiteral("clipPos"), xPos < 1 ? xOf(std::max(xPos, -1.0)) : NAN },
                        { QStringLiteral("clipNeg"), xNeg > -1 ? xOf(std::min(xNeg, 1.0)) : NAN },
                        { QStringLiteral("fullScaleTop"), yOf(1) },
                        { QStringLiteral("fullScaleBottom"), yOf(-1) },
                        { QStringLiteral("half"), QVariantList{ yOf(0.5), yOf(-0.5) } },
                        { QStringLiteral("zeroY"), yOf(0) },
                        { QStringLiteral("lineFrom"), yOf(-1) },
                        { QStringLiteral("lineTo"), yOf(1) },
                        { QStringLiteral("second"), second },
                        { QStringLiteral("third"), third } };
}

// The source impedance Zn/df against a speaker rising to 4x at resonance and 2x at the top
QVariantMap ToolsController::transformerLift(double damping) {
    const double df = std::max(damping, 1.0);
    const double bump = 4 * (df + 1) / (4 * df + 1);
    const double top = 2 * (df + 1) / (2 * df + 1);
    return QVariantMap{ { QStringLiteral("bell"), 20 * std::log10(bump) }, { QStringLiteral("top"), 20 * std::log10(top) } };
}

// ---- Stereo upmixer ------------------------------------------------------------

bool ToolsController::upmixSupported() const {
    if (!connected()) return false;
    const uint32_t f = m_ctx.device->snapshot().wireFeatures;
    return (f & WIRE_FEAT_UPMIX) && (f & WIRE_FEAT_UPMIX_PRESENCE) && m_ctx.device->state()->platform_id == 1;
}

double ToolsController::upmix(const QString &key) const {
    const Param *p = find(kUpmix, key);
    if (!p) return 0;
    return connected() ? double(m_ctx.device->state()->upmix[p->id]) : double(p->def);
}

void ToolsController::setUpmix(const QString &key, double value) {
    const Param *p = find(kUpmix, key);
    if (!p || !upmixSupported()) return;
    float v = clampTo(value, p->lo, p->hi);
    if (p->id <= UPMIX_PARAM_SURROUND_MODE) v = std::round(v);
    const uint8_t id = p->id;
    apply(QStringLiteral("upmix %1").arg(key), [id, v](FfiCore *c) { return dspi_set_upmix_param(c, id, v); });
}

// ---- Output limiters -----------------------------------------------------------

bool ToolsController::limiterSupported() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_LIMITER) != 0;
}

bool ToolsController::limiterOutput(int output) const {
    return limiterSupported() && output >= 0 && output < numOutputs();
}

QVariantMap ToolsController::limiter(int output) const {
    if (!connected() || output < 0 || output >= int(MAX_OUTPUTS))
        return QVariantMap{ { QStringLiteral("enabled"), false },
                            { QStringLiteral("threshold"), -1.0 },
                            { QStringLiteral("release"), 100.0 },
                            { QStringLiteral("group"), 0 } };
    const DspState *s = m_ctx.device->state();
    const int group = s->limiter_link_group[output];
    return QVariantMap{ { QStringLiteral("enabled"), s->limiter_enabled[output] },
                        { QStringLiteral("threshold"), double(s->limiter_threshold_db[output]) },
                        { QStringLiteral("release"), double(s->limiter_release_ms[output]) },
                        // A stored group above the four falls back to unlinked
                        { QStringLiteral("group"), group > 4 ? 0 : group } };
}

// An edit of the output configuration: the save bar keeps or reverts it
void ToolsController::limiterWrite(const QString &what, std::function<bool(FfiCore *)> setter) {
    if (!writable() || !limiterSupported()) return;
    if (auto *hw = Registrar::instance<HardwareController>()) hw->beginOutputEdit();
    apply(what, std::move(setter));
}

// The output card's enable key, so the cell and the popover coalesce
void ToolsController::setLimiterEnabled(int output, bool on) {
    if (!limiterOutput(output)) return;
    const auto o = uint8_t(output);
    limiterWrite(QStringLiteral("output limiter %1").arg(output),
                 [o, on](FfiCore *c) { return dspi_set_limiter_param(c, o, LIMITER_PARAM_ENABLED, on ? 1.0f : 0.0f); });
}

void ToolsController::setLimiterThreshold(int output, double db) {
    if (!limiterOutput(output) || std::isnan(db)) return;
    const auto o = uint8_t(output);
    const float v = clampTo(db, -30, 0);
    limiterWrite(QStringLiteral("output limiter threshold %1").arg(output),
                 [o, v](FfiCore *c) { return dspi_set_limiter_param(c, o, LIMITER_PARAM_THRESHOLD_DB, v); });
}

void ToolsController::setLimiterRelease(int output, double ms) {
    if (!limiterOutput(output) || std::isnan(ms)) return;
    const auto o = uint8_t(output);
    const float v = clampTo(ms, 10, 1000);
    limiterWrite(QStringLiteral("output limiter release %1").arg(output),
                 [o, v](FfiCore *c) { return dspi_set_limiter_param(c, o, LIMITER_PARAM_RELEASE_MS, v); });
}

void ToolsController::setLimiterGroup(int output, int group) {
    if (!limiterOutput(output)) return;
    const auto o = uint8_t(output);
    const float g = float(std::clamp(group, 0, 4));
    limiterWrite(QStringLiteral("output limiter group %1").arg(output),
                 [o, g](FfiCore *c) { return dspi_set_limiter_param(c, o, LIMITER_PARAM_LINK_GROUP, g); });
}

// One all-outputs write per value; the link groups stay as they are
void ToolsController::copyLimiterToAll(int output) {
    if (!limiterOutput(output)) return;
    const QVariantMap src = limiter(output);
    const float threshold = float(src.value(QStringLiteral("threshold")).toDouble());
    const float release = float(src.value(QStringLiteral("release")).toDouble());
    const float on = src.value(QStringLiteral("enabled")).toBool() ? 1.0f : 0.0f;
    limiterWrite(QStringLiteral("output limiters copy"), [threshold, release, on](FfiCore *c) {
        const uint8_t all = LIMITER_ALL_OUTPUTS;
        return dspi_set_limiter_param(c, all, LIMITER_PARAM_THRESHOLD_DB, threshold) &&
               dspi_set_limiter_param(c, all, LIMITER_PARAM_RELEASE_MS, release) &&
               dspi_set_limiter_param(c, all, LIMITER_PARAM_ENABLED, on);
    });
}

QVariantList ToolsController::stereoPairGroups() const {
    QVariantList groups;
    const int n = numOutputs();
    for (int o = 0; o < n; ++o) {
        const int pair = o / 2 + 1;
        groups << (o < pdmOutput() && pair <= 4 ? pair : 0);
    }
    return groups;
}

namespace {
// Each output's group in ascending order: each group's lowest output keeps its
// settings and the rest adopt them
bool writeGroups(FfiCore *c, const std::vector<float> &groups) {
    bool ok = true;
    for (size_t o = 0; o < groups.size(); ++o)
        ok = dspi_set_limiter_param(c, uint8_t(o), LIMITER_PARAM_LINK_GROUP, groups[o]) && ok;
    return ok;
}
} // namespace

void ToolsController::linkLimiterPairs() {
    std::vector<float> groups;
    for (const QVariant &g : stereoPairGroups()) groups.push_back(float(g.toInt()));
    limiterWrite(QStringLiteral("output limiters link"), [groups](FfiCore *c) { return writeGroups(c, groups); });
}

void ToolsController::unlinkLimiters() {
    const std::vector<float> groups(size_t(numOutputs()), 0.0f);
    limiterWrite(QStringLiteral("output limiters link"), [groups](FfiCore *c) { return writeGroups(c, groups); });
}

void ToolsController::switchLimitersOff() {
    limiterWrite(QStringLiteral("output limiters enable"),
                 [](FfiCore *c) { return dspi_set_limiter_param(c, LIMITER_ALL_OUTPUTS, LIMITER_PARAM_ENABLED, 0.0f); });
}

// Every limiter on at -1 dBFS, the stereo pairs linked. The groups go first, so
// the all-outputs writes after them land on every member alike.
// -- WP P1QT2 -- The core's helper (P1PR): every limiter on at -1 dBFS, the
// stereo pairs linked, the PDM sub left out, one undo step, held to the
// limits profile (an output's own ceiling, if lower, is kept and reported)
void ToolsController::protectAllOutputs() {
    limiterWrite(QStringLiteral("output limiters protect"), [](FfiCore *c) { return dspi_protect_all_outputs(c, kProtectCeilingDb); });
}

// -- WP P1QT2 -- PRO1 S2's warning, from the core's mirror, on the worker
void ToolsController::readUnprotected() {
    const StatePtr st = state();
    if (st->unprotectedAsked) return;
    st->unprotectedAsked = true;
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return dspi_outputs_without_limiter(c, kUnprotectedAboveDb); },
        [this, st](uint32_t mask) {
            st->unprotectedAsked = false;
            QVariantList list;
            for (int o = 0; o < MAX_OUTPUTS && o < 32; ++o)
                if (mask >> o & 1u) list << o;
            if (list == st->unprotected) return;
            st->unprotected = list;
            if (st == state()) emit unprotectedChanged();
        });
}

QString ToolsController::unprotectedText() const {
    const QVariantList unprotected = state()->unprotected;
    if (unprotected.isEmpty()) return QString();
    QStringList names;
    for (const QVariant &o : unprotected) names << outputName(o.toInt());
    //: PRO1 S2's warning row; %1 the outputs ("Out 1, Out 2"), %2 the gain (-10)
    return tr("No limiter on %1, above %2 dB of gain.")
        .arg(names.join(QStringLiteral(", ")))
        .arg(int(kUnprotectedAboveDb));
}

QString ToolsController::limiterLinkSummary(int output) const {
    const int group = limiter(output).value(QStringLiteral("group")).toInt();
    if (group == 0) return tr("Not linked.");
    QStringList others;
    for (int o = 0; o < numOutputs(); ++o)
        if (o != output && limiter(o).value(QStringLiteral("group")).toInt() == group) others << outputName(o);
    if (others.isEmpty()) return tr("No other outputs in group %1.").arg(group);
    const QString last = others.takeLast();
    //: "Linked with Out 1, Out 3 and Out 4."
    const QString list = others.isEmpty() ? last : tr("%1 and %2").arg(others.join(QStringLiteral(", ")), last);
    return tr("Linked with %1.").arg(list);
}
