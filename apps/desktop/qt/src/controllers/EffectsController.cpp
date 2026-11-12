// -- WP QP7 -- The effect windows' controller (EffectsController.h).
#include "EffectsController.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "DeviceModel.h"

#include <QtGlobal>
#include <algorithm>
#include <array>
#include <cmath>

static const Registrar::Controller<EffectsController> effectsController;
static const Registrar::Window crossfeedWindow("tools.crossfeed", "CrossfeedWindow");
static const Registrar::Window loudnessWindow("tools.loudness", "LoudnessWindow");
static const Registrar::Window levellerWindow("tools.leveller", "LevellerWindow");
static const Registrar::Window psybassWindow("tools.psybass", "PsybassWindow");

namespace {

// dspi_proto::wire_compat::feat bits the header does not export yet
constexpr uint32_t kFeatLevellerMasks = 1u << 1;
constexpr uint32_t kFeatLoudnessMask = 1u << 2;
constexpr uint32_t kFeatCrossfeedMask = 1u << 3;

// The crossfeed presets' own values (Default, Chu Moy, Jan Meier); Custom uses the device's
struct CrossfeedPresetRow {
    const char *name;
    const char *description;
    float freq;
    float feed;
};
const std::array<CrossfeedPresetRow, 4> kCrossfeedPresets{ {
    { QT_TRANSLATE_NOOP("EffectsController", "Default"),
      QT_TRANSLATE_NOOP("EffectsController", "700 Hz / 4.5 dB - Balanced, most popular"), 700, 4.5f },
    { QT_TRANSLATE_NOOP("EffectsController", "Chu Moy"),
      QT_TRANSLATE_NOOP("EffectsController", "700 Hz / 6.0 dB - Stronger spatial effect"), 700, 6.0f },
    { QT_TRANSLATE_NOOP("EffectsController", "Jan Meier"),
      QT_TRANSLATE_NOOP("EffectsController", "650 Hz / 9.5 dB - Natural speaker-like"), 650, 9.5f },
    { QT_TRANSLATE_NOOP("EffectsController", "Custom"),
      QT_TRANSLATE_NOOP("EffectsController", "User-defined parameters"), 700, 4.5f },
} };

// Psychoacoustic bass starting points (cutoff Hz, harmonics dB, drive dB, character %, original dB)
struct StartingPoint {
    const char *name;
    const char *detail;
    float cutoff, harmonics, drive, character, original;
};
const std::array<StartingPoint, 4> kStartingPoints{ {
    { QT_TRANSLATE_NOOP("EffectsController", "Bookshelf speakers"),
      QT_TRANSLATE_NOOP("EffectsController", "Gentle low-end help"), 60, 0, 6, 50, 0 },
    { QT_TRANSLATE_NOOP("EffectsController", "Small Bluetooth"),
      QT_TRANSLATE_NOOP("EffectsController", "Portable speaker"), 100, 3, 9, 40, -12 },
    { QT_TRANSLATE_NOOP("EffectsController", "Laptop / tablet"),
      QT_TRANSLATE_NOOP("EffectsController", "Tiny drivers, protect them"), 180, 6, 12, 50, -24 },
    { QT_TRANSLATE_NOOP("EffectsController", "Headphone bass feel"),
      QT_TRANSLATE_NOOP("EffectsController", "Extra sub sensation"), 45, -3, 6, 30, 0 },
} };

// ISO 226:2003 per third-octave: frequency, alpha_f, L_U, T_f (the Mac's IsoData table)
struct IsoRow {
    float f, af, lu, tf;
};
const std::array<IsoRow, 30> kIso{ {
    { 20, 0.532f, -31.6f, 78.5f },   { 25, 0.506f, -27.2f, 68.7f },   { 31.5f, 0.480f, -23.0f, 59.5f },
    { 40, 0.455f, -19.1f, 51.1f },   { 50, 0.432f, -15.9f, 44.0f },   { 63, 0.409f, -13.0f, 37.5f },
    { 80, 0.387f, -10.3f, 31.5f },   { 100, 0.367f, -8.1f, 26.5f },   { 125, 0.349f, -6.2f, 22.1f },
    { 160, 0.330f, -4.5f, 17.9f },   { 200, 0.315f, -3.1f, 14.4f },   { 250, 0.301f, -2.0f, 11.4f },
    { 315, 0.288f, -1.1f, 8.6f },    { 400, 0.276f, -0.4f, 6.2f },    { 500, 0.267f, 0.0f, 4.4f },
    { 630, 0.259f, 0.3f, 3.0f },     { 800, 0.253f, 0.5f, 2.2f },     { 1000, 0.250f, 0.0f, 2.4f },
    { 1250, 0.246f, -2.7f, 3.5f },   { 1600, 0.244f, -4.1f, 1.7f },   { 2000, 0.243f, -1.0f, -1.3f },
    { 2500, 0.243f, 1.7f, -4.2f },   { 3150, 0.243f, 2.5f, -6.0f },   { 4000, 0.242f, 1.2f, -5.4f },
    { 5000, 0.242f, -2.1f, -1.5f },  { 6300, 0.245f, -7.1f, 6.0f },   { 8000, 0.254f, -11.2f, 12.6f },
    { 10000, 0.271f, -10.7f, 13.9f }, { 12500, 0.301f, -3.1f, 12.3f }, { 16000, 0.310f, -2.0f, 17.0f },
} };

// The loudness drawing's volume: the curve as it is 40 dB below the reference
constexpr double kLoudnessShownVolumeDb = -40.0;
// The crossfeed drawing's sample rate
constexpr double kCrossfeedRate = 48000.0;
constexpr double kPi = 3.14159265358979323846;

double clampTo(double v, double lo, double hi) { return std::isfinite(v) ? std::clamp(v, lo, hi) : lo; }

// The SPL of `phon` at one ISO row
double isoSpl(const IsoRow &r, double phon) {
    const double b = 0.4 * std::pow(10.0, (r.tf + r.lu) / 10.0 - 9.0);
    double af = 4.47e-3 * (std::pow(10.0, 0.025 * phon) - 1.15) + std::pow(b, double(r.af));
    if (af < 1e-10) af = 1e-10;
    return (10.0 / r.af) * std::log10(af) - r.lu + 94.0;
}

// The opposite path's one-pole low pass at `f` (real, imaginary)
std::pair<double, double> crossfeedLowpass(double fc, double feedDb, double f) {
    const double g = 1.0 / (1.0 + std::pow(10.0, feedDb / 20.0));
    const double x = std::exp(-2.0 * kPi * fc / kCrossfeedRate);
    const double a0 = g * (1.0 - x);
    const double w = 2.0 * kPi * f / kCrossfeedRate;
    const double re = 1.0 - x * std::cos(w);
    const double im = x * std::sin(w);
    const double mag2 = re * re + im * im;
    return { a0 * re / mag2, -a0 * im / mag2 };
}

double toDb(double mag) { return 20.0 * std::log10(std::max(mag, 1e-10)); }

// The y range around `gains`: at least 10 dB, with a fifth of it above and below
// (the Mac's domain), its grid lines every 5 dB (10 dB above 40), and every
// point turned to 0..1.
QVariantMap graphOf(const std::vector<std::vector<std::pair<double, double>>> &curves, const QStringList &names) {
    double lo = INFINITY, hi = -INFINITY;
    for (const auto &c : curves)
        for (const auto &p : c) {
            lo = std::min(lo, p.second);
            hi = std::max(hi, p.second);
        }
    if (!std::isfinite(lo) || !std::isfinite(hi)) lo = hi = 0;
    const double visual = std::max(hi - lo, 10.0);
    const double pad = visual * 0.2;
    const double dMin = lo - pad, dMax = hi + pad;
    const double span = dMax - dMin;
    const auto yOf = [&](double db) { return (db - dMin) / span; };

    QVariantMap out;
    for (size_t i = 0; i < curves.size() && i < size_t(names.size()); ++i) {
        QVariantList pts;
        pts.reserve(int(curves[i].size()) * 2);
        for (const auto &p : curves[i]) {
            pts << EffectsController::freqX(p.first) << yOf(p.second);
        }
        out.insert(names.at(int(i)), pts);
    }
    const double step = span > 40 ? 10 : 5;
    QVariantList gridDb, gridY;
    for (double v = std::ceil(dMin / step) * step; v <= dMax; v += step) {
        gridDb << int(v);
        gridY << yOf(v);
    }
    out.insert(QStringLiteral("gridDb"), gridDb);
    out.insert(QStringLiteral("gridY"), gridY);
    return out;
}

} // namespace

EffectsController::EffectsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, [this]() {
        ++m_reads;
        emit effectsChanged();
    });
    // The input count the leveller's chips follow
    connect(m_ctx.device->channels(), &ChannelListModel::countChanged, this, [this]() {
        ++m_reads;
        emit effectsChanged();
    });
}

void EffectsController::apply(const QString &what, std::function<bool(FfiCore *)> setter) {
    if (!writable()) return;
    m_ctx.device->write(what, std::move(setter));
}

bool EffectsController::connected() const { return m_ctx.device->connected() && m_ctx.device->state(); }
bool EffectsController::writable() const { return connected() && !m_ctx.device->readOnly(); }
int EffectsController::numOutputs() const {
    const DspState *s = m_ctx.device->state();
    return s ? std::clamp(int(s->num_output_channels), 0, int(MAX_OUTPUTS)) : 0;
}

// ---- Crossfeed -----------------------------------------------------------------

bool EffectsController::crossfeedOn() const { return connected() && m_ctx.device->state()->crossfeed_enabled; }
int EffectsController::crossfeedPreset() const { return connected() ? m_ctx.device->state()->crossfeed_preset : 0; }
double EffectsController::crossfeedFreq() const {
    return connected() ? m_ctx.device->state()->crossfeed_freq : kCrossfeedPresets[0].freq;
}
double EffectsController::crossfeedFeed() const {
    return connected() ? m_ctx.device->state()->crossfeed_feed : kCrossfeedPresets[0].feed;
}
double EffectsController::crossfeedShownFreq() const {
    const int p = crossfeedPreset();
    return p >= 0 && p < crossfeedCustomPreset() ? double(kCrossfeedPresets[size_t(p)].freq) : crossfeedFreq();
}
double EffectsController::crossfeedShownFeed() const {
    const int p = crossfeedPreset();
    return p >= 0 && p < crossfeedCustomPreset() ? double(kCrossfeedPresets[size_t(p)].feed) : crossfeedFeed();
}
bool EffectsController::crossfeedItd() const { return connected() ? m_ctx.device->state()->crossfeed_itd : true; }
int EffectsController::crossfeedPairMask() const {
    return connected() ? m_ctx.device->state()->crossfeed_output_pair_mask : 0x01;
}
bool EffectsController::crossfeedMaskSupported() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & kFeatCrossfeedMask) != 0;
}
// The stereo slots: 2 on RP2040, 4 on RP2350 (the mono PDM sub is never in a pair)
int EffectsController::crossfeedPairCount() const {
    if (!connected()) return 0;
    return m_ctx.device->state()->platform_id == 0 ? 2 : 4;
}

QVariantList EffectsController::crossfeedPresets() {
    QVariantList rows;
    for (const CrossfeedPresetRow &r : kCrossfeedPresets)
        rows << QVariantMap{ { QStringLiteral("name"), tr(r.name) },
                             { QStringLiteral("description"), tr(r.description) },
                             { QStringLiteral("freq"), r.freq },
                             { QStringLiteral("feed"), r.feed } };
    return rows;
}

void EffectsController::setEnabled(const QString &effect, bool on) {
    // SidebarController's keys, so the quick strip and the window coalesce
    if (effect == QLatin1String("crossfeed"))
        apply(QStringLiteral("crossfeed enabled"), [on](FfiCore *c) { return dspi_set_crossfeed(c, on); });
    else if (effect == QLatin1String("loudness"))
        apply(QStringLiteral("loudness enabled"), [on](FfiCore *c) { return dspi_set_loudness(c, on); });
    else if (effect == QLatin1String("leveller"))
        apply(QStringLiteral("leveller enabled"), [on](FfiCore *c) { return dspi_set_leveller_enabled(c, on); });
    else if (effect == QLatin1String("psybass") && psybassSupported())
        apply(QStringLiteral("psybass enabled"), [on](FfiCore *c) { return dspi_set_psybass_enabled(c, on); });
}

void EffectsController::setCrossfeedPreset(int preset) {
    if (preset < 0 || preset > crossfeedCustomPreset()) return;
    const uint8_t p = uint8_t(preset);
    apply(QStringLiteral("crossfeed preset"), [p](FfiCore *c) { return dspi_set_crossfeed_preset(c, p); });
}

void EffectsController::setCrossfeedFreq(double hz, bool live) {
    const float v = float(clampTo(hz, 500, 2000));
    apply(QStringLiteral("crossfeed freq"), [v](FfiCore *c) { return dspi_set_crossfeed_freq(c, v); });
    if (!live && crossfeedPreset() != crossfeedCustomPreset()) setCrossfeedPreset(crossfeedCustomPreset());
}

void EffectsController::setCrossfeedFeed(double db, bool live) {
    const float v = float(clampTo(db, 0, 15));
    apply(QStringLiteral("crossfeed feed"), [v](FfiCore *c) { return dspi_set_crossfeed_feed(c, v); });
    if (!live && crossfeedPreset() != crossfeedCustomPreset()) setCrossfeedPreset(crossfeedCustomPreset());
}

void EffectsController::setCrossfeedItd(bool on) {
    apply(QStringLiteral("crossfeed itd"), [on](FfiCore *c) { return dspi_set_crossfeed_itd(c, on); });
}

int EffectsController::allPairsMask() const {
    const int n = crossfeedPairCount();
    return n >= 8 ? 0xFF : (1 << n) - 1;
}

void EffectsController::setCrossfeedPairMask(int mask) {
    if (!crossfeedMaskSupported()) return;
    const uint8_t m = uint8_t(mask & 0xFF);
    apply(QStringLiteral("crossfeed pairs"), [m](FfiCore *c) { return dspi_set_crossfeed_outputs(c, m); });
}

void EffectsController::toggleCrossfeedPair(int pair) {
    if (pair < 0 || pair >= 8) return;
    setCrossfeedPairMask(crossfeedPairMask() ^ (1 << pair));
}

// ---- Loudness ------------------------------------------------------------------

bool EffectsController::loudnessOn() const { return connected() && m_ctx.device->state()->loudness_enabled; }
double EffectsController::loudnessRef() const { return connected() ? m_ctx.device->state()->loudness_ref_spl : 83.0; }
double EffectsController::loudnessIntensity() const {
    return connected() ? m_ctx.device->state()->loudness_intensity : 100.0;
}
int EffectsController::loudnessMask() const { return connected() ? m_ctx.device->state()->loudness_output_mask : 0xFFFF; }
bool EffectsController::loudnessMaskSupported() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & kFeatLoudnessMask) != 0;
}

void EffectsController::setLoudnessRef(double spl) {
    const float v = float(clampTo(spl, 40, 100));
    apply(QStringLiteral("loudness ref"), [v](FfiCore *c) { return dspi_set_loudness_ref(c, v); });
}

void EffectsController::setLoudnessIntensity(double pct) {
    const float v = float(clampTo(pct, 0, 200));
    apply(QStringLiteral("loudness intensity"), [v](FfiCore *c) { return dspi_set_loudness_intensity(c, v); });
}

void EffectsController::setMask16(const QString &what, int mask, bool (*setter)(FfiCore *, uint16_t)) {
    const uint16_t m = uint16_t(mask & 0xFFFF);
    apply(what, [m, setter](FfiCore *c) { return setter(c, m); });
}

void EffectsController::setLoudnessMask(int mask) {
    if (loudnessMaskSupported()) setMask16(QStringLiteral("loudness outputs"), mask, &dspi_set_loudness_mask);
}

void EffectsController::toggleLoudnessOutput(int output) {
    if (output < 0 || output >= 16) return;
    setLoudnessMask(loudnessMask() ^ (1 << output));
}

// ---- Leveller ------------------------------------------------------------------

bool EffectsController::levellerOn() const { return connected() && m_ctx.device->state()->leveller_enabled; }
double EffectsController::levellerAmount() const { return connected() ? m_ctx.device->state()->leveller_amount : 50.0; }
int EffectsController::levellerSpeed() const {
    return connected() ? std::clamp(int(m_ctx.device->state()->leveller_speed), 0, 2) : 0;
}
double EffectsController::levellerMaxGain() const {
    return connected() ? m_ctx.device->state()->leveller_max_gain_db : 15.0;
}
bool EffectsController::levellerLookahead() const {
    return connected() ? m_ctx.device->state()->leveller_lookahead : true;
}
double EffectsController::levellerGate() const { return connected() ? m_ctx.device->state()->leveller_gate_db : -96.0; }
int EffectsController::levellerDetectorMask() const {
    return connected() ? m_ctx.device->state()->leveller_detector_mask : 0xFF;
}
int EffectsController::levellerApplyMask() const { return connected() ? m_ctx.device->state()->leveller_apply_mask : 0xFF; }
// The inputs the app shows (2..8), as the sidebar lists them
int EffectsController::levellerChannels() const { return std::clamp(m_ctx.device->channels()->inputCount(), 2, 8); }
bool EffectsController::levellerMasksShown() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & kFeatLevellerMasks) != 0 &&
           m_ctx.device->state()->num_input_channels > 2 && levellerChannels() > 2;
}

void EffectsController::setLevellerAmount(double pct) {
    const float v = float(clampTo(pct, 0, 100));
    apply(QStringLiteral("leveller amount"), [v](FfiCore *c) { return dspi_set_leveller_amount(c, v); });
}

void EffectsController::setLevellerSpeed(int speedIndex) {
    if (speedIndex < 0 || speedIndex > 2) return;
    const uint8_t s = uint8_t(speedIndex);
    apply(QStringLiteral("leveller speed"), [s](FfiCore *c) { return dspi_set_leveller_speed(c, s); });
}

void EffectsController::setLevellerMaxGain(double db) {
    const float v = float(clampTo(db, 0, 35));
    apply(QStringLiteral("leveller max gain"), [v](FfiCore *c) { return dspi_set_leveller_max_gain(c, v); });
}

void EffectsController::setLevellerLookahead(bool on) {
    apply(QStringLiteral("leveller lookahead"), [on](FfiCore *c) { return dspi_set_leveller_lookahead(c, on); });
}

void EffectsController::setLevellerGate(double db) {
    const float v = float(clampTo(db, -96, 0));
    apply(QStringLiteral("leveller gate"), [v](FfiCore *c) { return dspi_set_leveller_gate(c, v); });
}

void EffectsController::setLevellerMasks(int detectorMask, int applyMask) {
    const uint8_t d = uint8_t(detectorMask & 0xFF), a = uint8_t(applyMask & 0xFF);
    apply(QStringLiteral("leveller masks"), [d, a](FfiCore *c) { return dspi_set_leveller_masks(c, d, a); });
}

void EffectsController::toggleLevellerChannel(bool detector, int channel) {
    if (channel < 0 || channel >= 8) return;
    const int bit = 1 << channel;
    if (detector)
        setLevellerMasks(levellerDetectorMask() ^ bit, levellerApplyMask());
    else
        setLevellerMasks(levellerDetectorMask(), levellerApplyMask() ^ bit);
}

// ---- Psychoacoustic bass -----------------------------------------------------------

bool EffectsController::psybassSupported() const {
    return connected() && (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_PSYBASS) != 0;
}
bool EffectsController::psybassOn() const { return connected() && m_ctx.device->state()->psybass_enabled; }
double EffectsController::psybassCutoff() const { return connected() ? m_ctx.device->state()->psybass_cutoff_hz : 80.0; }
double EffectsController::psybassHarmonics() const {
    return connected() ? m_ctx.device->state()->psybass_harmonics_db : 0.0;
}
double EffectsController::psybassDrive() const { return connected() ? m_ctx.device->state()->psybass_drive_db : 6.0; }
double EffectsController::psybassCharacter() const {
    return connected() ? m_ctx.device->state()->psybass_character_pct : 50.0;
}
double EffectsController::psybassOriginal() const {
    return connected() ? m_ctx.device->state()->psybass_original_db : 0.0;
}
int EffectsController::psybassMask() const { return connected() ? m_ctx.device->state()->psybass_output_mask : 0xFFFF; }

QVariantList EffectsController::psybassStartingPoints() {
    QVariantList rows;
    for (const StartingPoint &p : kStartingPoints)
        rows << QVariantMap{ { QStringLiteral("name"), tr(p.name) }, { QStringLiteral("detail"), tr(p.detail) } };
    return rows;
}

void EffectsController::setPsybassParam(const QString &key, uint8_t param, double value) {
    if (!psybassSupported()) return;
    const float v = float(value);
    apply(QStringLiteral("psybass ") + key, [param, v](FfiCore *c) { return dspi_set_psybass_param(c, param, v); });
}

void EffectsController::setPsybass(const QString &param, double value) {
    if (param == QLatin1String("cutoff"))
        setPsybassParam(param, PSYBASS_PARAM_CUTOFF, clampTo(value, 30, 300));
    else if (param == QLatin1String("harmonics"))
        setPsybassParam(param, PSYBASS_PARAM_HARMONICS, clampTo(value, -24, 12));
    else if (param == QLatin1String("drive"))
        setPsybassParam(param, PSYBASS_PARAM_DRIVE, clampTo(value, 0, 18));
    else if (param == QLatin1String("character"))
        setPsybassParam(param, PSYBASS_PARAM_CHARACTER, clampTo(value, 0, 100));
    else if (param == QLatin1String("original"))
        setPsybassParam(param, PSYBASS_PARAM_ORIGINAL, clampTo(value, -60, 0));
}

void EffectsController::applyPsybassStartingPoint(int index) {
    if (index < 0 || index >= int(kStartingPoints.size())) return;
    const StartingPoint &p = kStartingPoints[size_t(index)];
    setPsybass(QStringLiteral("cutoff"), p.cutoff);
    setPsybass(QStringLiteral("harmonics"), p.harmonics);
    setPsybass(QStringLiteral("drive"), p.drive);
    setPsybass(QStringLiteral("character"), p.character);
    setPsybass(QStringLiteral("original"), p.original);
}

int EffectsController::allOutputsMask() const {
    const int n = numOutputs();
    return n >= 16 ? 0xFFFF : (1 << n) - 1;
}

int EffectsController::outputsWithoutSubMask() const {
    const int pdm = pdmOutput();
    return pdm >= 0 ? allOutputsMask() & ~(1 << pdm) : allOutputsMask();
}

void EffectsController::setPsybassMask(int mask) {
    if (psybassSupported()) setMask16(QStringLiteral("psybass outputs"), mask, &dspi_set_psybass_mask);
}

void EffectsController::togglePsybassOutput(int output) {
    if (output < 0 || output >= 16) return;
    setPsybassMask(psybassMask() ^ (1 << output));
}

// ---- Names ---------------------------------------------------------------------

QString EffectsController::outputName(int output) const {
    const ChannelListModel *list = m_ctx.device->channels();
    for (int row = 0; row < list->rowCount(); ++row) {
        if (list->get(row, QStringLiteral("kind")).toString() == QLatin1String("output") &&
            list->get(row, QStringLiteral("number")).toInt() == output) {
            const QString label = list->get(row, QStringLiteral("label")).toString();
            if (!label.isEmpty()) return label;
        }
    }
    return tr("Out %1").arg(output + 1);
}

QString EffectsController::inputName(int input) const {
    const ChannelListModel *list = m_ctx.device->channels();
    const int row = list->rowOfChannel(input);
    const QString label = row >= 0 ? list->get(row, QStringLiteral("label")).toString() : QString();
    return label.isEmpty() ? tr("Ch %1").arg(input + 1) : label;
}

// ---- Curves --------------------------------------------------------------------

double EffectsController::freqX(double hz) {
    const double f = std::clamp(std::isfinite(hz) ? hz : 20.0, 20.0, 20000.0);
    return (std::log10(f) - std::log10(20.0)) / (std::log10(20000.0) - std::log10(20.0));
}

double EffectsController::crossfeedOppositeDb(double fc, double feedDb, double f) {
    const auto [re, im] = crossfeedLowpass(fc, feedDb, f);
    return toDb(std::hypot(re, im));
}

double EffectsController::crossfeedDirectDb(double fc, double feedDb, double f) {
    // The direct path is one minus the opposite one
    const auto [re, im] = crossfeedLowpass(fc, feedDb, f);
    return toDb(std::hypot(1.0 - re, -im));
}

QVariantMap EffectsController::crossfeedGraph(double freqHz, double feedDb) const {
    std::vector<std::pair<double, double>> direct, opposite;
    constexpr int points = 100;
    for (int i = 0; i < points; ++i) {
        const double f = 20.0 * std::pow(1000.0, double(i) / (points - 1));
        direct.emplace_back(f, crossfeedDirectDb(freqHz, feedDb, f));
        opposite.emplace_back(f, crossfeedOppositeDb(freqHz, feedDb, f));
    }
    return graphOf({ direct, opposite }, { QStringLiteral("direct"), QStringLiteral("crossfeed") });
}

int EffectsController::isoRows() { return int(kIso.size()); }
double EffectsController::isoFreq(int row) { return row >= 0 && row < isoRows() ? kIso[size_t(row)].f : 0; }

double EffectsController::loudnessCompensationDb(double refSpl, double intensityPct, int isoRow) {
    if (isoRow < 0 || isoRow >= isoRows()) return 0;
    const double phon = std::clamp(refSpl + kLoudnessShownVolumeDb, 20.0, std::max(refSpl, 20.0));
    if (phon >= refSpl) return 0;
    const IsoRow &r = kIso[size_t(isoRow)];
    const double shaped = isoSpl(r, phon) - isoSpl(r, refSpl);
    const double flat = phon - refSpl;
    return (shaped - flat) * (intensityPct / 100.0);
}

QVariantMap EffectsController::loudnessGraph(double refSpl, double intensityPct) const {
    std::vector<std::pair<double, double>> curve;
    for (int i = 0; i < isoRows(); ++i) curve.emplace_back(isoFreq(i), loudnessCompensationDb(refSpl, intensityPct, i));
    return graphOf({ curve }, { QStringLiteral("curve") });
}

QVariantMap EffectsController::psybassSketch(double cutoffHz, double harmonicsDb, double originalDb) const {
    return { { QStringLiteral("fc"), freqX(cutoffHz) },
             { QStringLiteral("fc4"), freqX(cutoffHz * 4) },
             { QStringLiteral("original"), clampTo((originalDb + 60) / 60, 0, 1) },
             { QStringLiteral("harmonics"), clampTo((harmonicsDb + 24) / 36, 0, 1) } };
}
