// -- WP QP3 -- The response graph's model; GraphController.h says what it holds.
#include "GraphController.h"

#include "DeviceModel.h"
#include "Poller.h"

#include <QtMath>
#include <cmath>
#include <cstring>
#include <iterator>
#include <vector>

static const Registrar::Controller<GraphController> graphController;
static const Registrar::Window graphWindow("window.popOutGraph", "GraphWindow");

namespace {

// The slots' QML types, filled by static initialisers in no set order
std::vector<const char *> &slotTypes() {
    static std::vector<const char *> types(4, nullptr);
    return types;
}

const QVector<double> &noCurve() {
    static const QVector<double> empty;
    return empty;
}

// The Mac's AppSettings defaults; the keys are the previous desktop app's
constexpr double kDefaultMinFreq = 15;
constexpr double kDefaultMaxFreq = 20000;
constexpr double kDefaultRange = 50;
constexpr double kDefaultCenter = 0;
constexpr double kDefaultGridOpacity = 0.5;
constexpr double kDefaultLineWidth = 1.5;
constexpr double kDefaultHeight = 250;

// The setup page's choices
const double kMinFreqs[] = { 10, 15, 20, 50, 100 };
const double kMaxFreqs[] = { 5000, 10000, 20000 };

double nearestOf(const double *choices, int n, double v) {
    double best = choices[0];
    for (int i = 1; i < n; ++i)
        if (qAbs(choices[i] - v) < qAbs(best - v)) best = choices[i];
    return best;
}

} // namespace

void GraphController::registerSlot(SlotKind kind, const char *qmlType) {
    const auto i = size_t(kind);
    if (i < slotTypes().size()) slotTypes()[i] = qmlType;
}

bool GraphController::Chain::operator==(const Chain &o) const {
    return count == o.count && flat == o.flat &&
           std::memcmp(bands.data(), o.bands.data(), sizeof(FilterParams) * count) == 0;
}

GraphController::GraphController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_device(ctx.device), m_poller(ctx.poller) {
    m_settings.beginGroup(QStringLiteral("graph"));
    auto number = [this](const char *key, double fallback) {
        bool ok = false;
        const double v = m_settings.value(QLatin1String(key), fallback).toDouble(&ok);
        return ok && qIsFinite(v) ? v : fallback;
    };
    m_minFreq = nearestOf(kMinFreqs, int(std::size(kMinFreqs)), number("minFreq", kDefaultMinFreq));
    m_maxFreq = nearestOf(kMaxFreqs, int(std::size(kMaxFreqs)), number("maxFreq", kDefaultMaxFreq));
    m_dbRange = qBound(kRangeMin, number("dbRange", kDefaultRange), kRangeMax);
    m_dbCenter = qBound(kCenterMin, number("dbCenter", kDefaultCenter), kCenterMax);
    m_gridOpacity = qBound(0.0, number("gridOpacity", kDefaultGridOpacity), kGridOpacityMax);
    m_lineWidth = qBound(kLineMin, number("lineWidth", kDefaultLineWidth), kLineMax);
    m_height = qBound(kHeightMin, number("height", kDefaultHeight), kHeightMax);
    const struct {
        const char *key;
        bool fallback;
    } flags[] = {
        { "showFreqGrid", true }, { "showFreqLabels", true }, { "showDbGrid", true },   { "showDbLabels", true },
        { "freqReadout", true },  { "levelReadout", true },   { "showGlow", false },    { "showPhase", false },
        { "phaseUnwrapped", false }, { "popOutFollows", true },
    };
    for (const auto &f : flags)
        m_flags.insert(QLatin1String(f.key), m_settings.value(QLatin1String(f.key), f.fallback).toBool());

    connect(m_device, &DeviceModel::snapshotApplied, this, &GraphController::onSnapshot);
    if (m_poller) connect(m_poller, &Poller::statusChanged, this, &GraphController::onStatus);
}

// ---- Curves ------------------------------------------------------------------

GraphController::Chain GraphController::chainOf(const DspState &s, int channel) const {
    Chain c;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) c.bands[c.count++] = s.filters[channel][b];
    // Crossovers are an output's only
    if (channel >= m_firstOutput)
        for (int b = 0; b < MAX_XOVER_BANDS; ++b) c.bands[c.count++] = s.xover[channel][b];
    // The global bypass flattens the stereo input pair, as the Mac draws it
    c.flat = s.bypass && channel < 2;
    return c;
}

void GraphController::onSnapshot() {
    const DspState *s = m_device->state();
    const uint64_t generation = m_device->snapshot().generation;
    if (!s) {
        if (m_channelCount == 0) return;
        m_channelCount = m_firstOutput = m_outputCount = 0;
        for (auto &v : m_mag) v.clear();
        for (auto &v : m_phase) v.clear();
        for (auto &v : m_phaseUnwrapped) v.clear();
        m_chains.fill(Chain{});
        m_gainPreview.clear();
        ++m_revision;
        emit curvesChanged();
        return;
    }

    const int firstOutput = qBound(0, int(s->num_input_channels), MAX_INPUTS);
    const int outputs = qBound(0, int(s->num_output_channels), MAX_OUTPUTS);
    const int count = qMin(firstOutput + outputs, int(MAX_CHANNELS));
    const bool channelSetChanged = count != m_channelCount || firstOutput != m_firstOutput || generation != m_generation;
    m_firstOutput = firstOutput;
    m_outputCount = outputs;
    m_channelCount = count;
    m_generation = generation;

    bool changed = channelSetChanged;
    for (int ch = 0; ch < MAX_CHANNELS; ++ch) {
        if (ch >= count) {
            if (!m_mag[ch].isEmpty()) {
                m_mag[ch].clear();
                m_phase[ch].clear();
                m_phaseUnwrapped[ch].clear();
                changed = true;
            }
            m_chains[ch] = Chain{};
            continue;
        }
        const Chain chain = chainOf(*s, ch);
        if (!m_mag[ch].isEmpty() && chain == m_chains[ch]) continue;
        m_chains[ch] = chain;
        QVector<double> mag(MAGNITUDE_POINTS, 0.0), phase(MAGNITUDE_POINTS, 0.0), unwrapped(MAGNITUDE_POINTS, 0.0);
        if (!chain.flat) {
            dspi_compute_magnitude_curve(chain.bands.data(), chain.count, mag.data());
            dspi_compute_phase_curve(chain.bands.data(), chain.count, false, phase.data());
            dspi_compute_phase_curve(chain.bands.data(), chain.count, true, unwrapped.data());
        }
        m_mag[ch] = mag;
        m_phase[ch] = phase;
        m_phaseUnwrapped[ch] = unwrapped;
        changed = true;
    }
    for (int o = 0; o < MAX_OUTPUTS; ++o) {
        const double gain = o < outputs && qIsFinite(s->output_gain_db[o]) ? s->output_gain_db[o] : 0.0;
        if (gain != m_gain[o]) {
            m_gain[o] = gain;
            changed = true;
        }
    }

    // Visibility follows the channel set: new device, other channel counts, or
    // an output switched on or off while the overview shows
    std::array<bool, MAX_OUTPUTS> enabled{};
    for (int o = 0; o < outputs; ++o) enabled[o] = s->output_enabled[o];
    if (channelSetChanged) {
        m_enabled = enabled;
        m_popOutSet = false;
        resetDefaultVisibility();
    } else if (enabled != m_enabled) {
        bool vis = false;
        for (int o = 0; o < outputs; ++o) {
            if (enabled[o] == m_enabled[o]) continue;
            if (m_selected < 0) {
                m_visible[size_t(firstOutput + o)] = enabled[o];
                vis = true;
            }
        }
        m_enabled = enabled;
        if (vis) emit visibilityChanged();
    }

    if (changed) {
        ++m_revision;
        emit curvesChanged();
    }
}

const QVector<double> &GraphController::magnitudes(int channel) const {
    return channel >= 0 && channel < m_channelCount ? m_mag[size_t(channel)] : noCurve();
}

const QVector<double> &GraphController::phases(int channel, bool unwrapped) const {
    if (channel < 0 || channel >= m_channelCount) return noCurve();
    return unwrapped ? m_phaseUnwrapped[size_t(channel)] : m_phase[size_t(channel)];
}

double GraphController::outputGain(int channel) const {
    const int output = channel - m_firstOutput;
    if (channel < m_firstOutput || output >= m_outputCount || output < 0) return 0;
    return m_gainPreview.value(output, m_gain[size_t(output)]);
}

QVariantList GraphController::magnitudeCurve(int channel) const {
    QVariantList out;
    const double gain = outputGain(channel);
    for (double v : magnitudes(channel)) out.append(v + gain);
    return out;
}

QVariantList GraphController::phaseCurve(int channel) const {
    QVariantList out;
    for (double v : phases(channel, phaseUnwrapped())) out.append(v);
    return out;
}

void GraphController::previewOutputGain(int output, double db) {
    if (output < 0 || output >= MAX_OUTPUTS || !qIsFinite(db)) return;
    if (m_gainPreview.contains(output) && m_gainPreview.value(output) == db) return;
    m_gainPreview.insert(output, db);
    ++m_revision;
    emit curvesChanged();
}

void GraphController::endOutputGainPreview(int output) {
    if (!m_gainPreview.remove(output)) return;
    ++m_revision;
    emit curvesChanged();
}

void GraphController::setOverlayChannel(int channel) {
    if (channel < -1 || channel >= MAX_CHANNELS) channel = -1;
    if (channel == m_overlayChannel) return;
    m_overlayChannel = channel;
    emit overlayChannelChanged();
}

// ---- Visibility --------------------------------------------------------------

int GraphController::effectiveInputs() const {
    // The inputs the host plays into, as the device reports them; all of the
    // device's inputs before the first status read
    const int active = m_poller ? m_poller->activeInputs() : 0;
    return active > 0 ? qMin(active, m_firstOutput) : m_firstOutput;
}

void GraphController::applyDefaults(Visibility &v) const {
    v.fill(false);
    const int inputs = effectiveInputs();
    for (int i = 0; i < inputs; ++i) v[size_t(i)] = true;
    for (int o = 0; o < m_outputCount && m_firstOutput + o < MAX_CHANNELS; ++o)
        v[size_t(m_firstOutput + o)] = m_enabled[size_t(o)];
}

void GraphController::resetDefaultVisibility() {
    m_haveSaved = false;
    m_inputsShown = effectiveInputs();
    // A channel page keeps its soloed curve until the user leaves it
    if (m_selected >= 0) return;
    applyDefaults(m_visible);
    emit visibilityChanged();
}

void GraphController::onStatus() {
    // The host's channel count changed: a new default set of input curves
    if (connected() && effectiveInputs() != m_inputsShown) resetDefaultVisibility();
}

void GraphController::select(int channel, int partner) {
    if (channel >= MAX_CHANNELS) channel = -1;
    if (channel < 0) {
        if (m_selected < 0) return;
        m_selected = -1;
        // Back to the overview: the pills as the user left them there
        if (m_haveSaved) m_visible = m_savedOverview;
        else applyDefaults(m_visible);
        m_haveSaved = false;
    } else {
        // Only a step in from the overview is remembered, never a soloed page
        if (m_selected < 0) {
            m_savedOverview = m_visible;
            m_haveSaved = true;
        }
        m_selected = channel;
        m_visible.fill(false);
        m_visible[size_t(channel)] = true;
        if (partner >= 0 && partner < MAX_CHANNELS) m_visible[size_t(partner)] = true;
    }
    emit selectionChanged();
    emit visibilityChanged();
}

bool GraphController::isCurveVisible(int channel) const {
    return channel >= 0 && channel < MAX_CHANNELS && m_visible[size_t(channel)];
}

void GraphController::setCurveVisible(int channel, bool visible) {
    if (channel < 0 || channel >= MAX_CHANNELS || m_visible[size_t(channel)] == visible) return;
    m_visible[size_t(channel)] = visible;
    emit visibilityChanged();
}

void GraphController::toggleCurve(int channel) { setCurveVisible(channel, !isCurveVisible(channel)); }

QList<int> GraphController::popOutVisibleChannels() const {
    if (popOutFollows()) return visibleChannels();
    if (m_popOutSet) return channelsOf(m_popOut);
    // Until its first change, the pop-out's own set is the defaults
    Visibility v{};
    applyDefaults(v);
    return channelsOf(v);
}

bool GraphController::isPopOutCurveVisible(int channel) const {
    return popOutVisibleChannels().contains(channel);
}

void GraphController::togglePopOutCurve(int channel) {
    if (popOutFollows()) {
        toggleCurve(channel);
        return;
    }
    if (channel < 0 || channel >= MAX_CHANNELS) return;
    if (!m_popOutSet) {
        // The pop-out's own set starts with every active input and enabled output
        applyDefaults(m_popOut);
        m_popOutSet = true;
    }
    m_popOut[size_t(channel)] = !m_popOut[size_t(channel)];
    emit visibilityChanged();
}

QList<int> GraphController::channelsOf(const Visibility &v) {
    QList<int> out;
    for (int ch = 0; ch < MAX_CHANNELS; ++ch)
        if (v[size_t(ch)]) out.append(ch);
    return out;
}

// ---- Pop-out and slots ---------------------------------------------------------

void GraphController::setPoppedOut(bool shown) {
    if (shown == m_poppedOut) return;
    m_poppedOut = shown;
    emit poppedOutChanged();
}

void GraphController::closePopOut() {
    if (m_poppedOut) emit popOutCloseRequested();
}

QString GraphController::slotType(int kind) const {
    if (kind < 0 || size_t(kind) >= slotTypes().size() || !slotTypes()[size_t(kind)]) return QString();
    return QString::fromLatin1(slotTypes()[size_t(kind)]);
}

// ---- Preferences ---------------------------------------------------------------

void GraphController::store(const QString &key, const QVariant &value) {
    m_settings.setValue(key, value);
    emit preferencesChanged();
}

void GraphController::setFlag(const QString &key, bool on) {
    if (m_flags.value(key) == on) return;
    m_flags.insert(key, on);
    store(key, on);
}

void GraphController::setPopOutFollows(bool on) {
    if (popOutFollows() == on) return;
    m_popOutSet = false;
    setFlag(QStringLiteral("popOutFollows"), on);
    emit visibilityChanged();
}

void GraphController::setMinFreq(double hz) {
    hz = nearestOf(kMinFreqs, int(std::size(kMinFreqs)), hz);
    if (hz == m_minFreq) return;
    m_minFreq = hz;
    store(QStringLiteral("minFreq"), hz);
}

void GraphController::setMaxFreq(double hz) {
    hz = nearestOf(kMaxFreqs, int(std::size(kMaxFreqs)), hz);
    if (hz == m_maxFreq) return;
    m_maxFreq = hz;
    store(QStringLiteral("maxFreq"), hz);
}

void GraphController::setDbRange(double db) {
    if (!qIsFinite(db)) return;
    db = qBound(kRangeMin, std::round(db), kRangeMax);
    if (db == m_dbRange) return;
    m_dbRange = db;
    store(QStringLiteral("dbRange"), db);
}

void GraphController::setDbCenter(double db) {
    if (!qIsFinite(db)) return;
    db = qBound(kCenterMin, std::round(db), kCenterMax);
    if (db == m_dbCenter) return;
    m_dbCenter = db;
    store(QStringLiteral("dbCenter"), db);
}

void GraphController::setGridOpacity(double v) {
    if (!qIsFinite(v)) return;
    v = qBound(0.0, v, kGridOpacityMax);
    if (v == m_gridOpacity) return;
    m_gridOpacity = v;
    store(QStringLiteral("gridOpacity"), v);
}

void GraphController::setLineWidth(double v) {
    if (!qIsFinite(v)) return;
    v = qBound(kLineMin, std::round(v * 2) / 2, kLineMax);
    if (v == m_lineWidth) return;
    m_lineWidth = v;
    store(QStringLiteral("lineWidth"), v);
}

void GraphController::setGraphHeight(double v) {
    if (!qIsFinite(v)) return;
    v = qBound(kHeightMin, v, kHeightMax);
    if (v == m_height) return;
    m_height = v;
    store(QStringLiteral("height"), v);
}

void GraphController::resetScale() {
    setMinFreq(kDefaultMinFreq);
    setMaxFreq(kDefaultMaxFreq);
    setDbRange(kDefaultRange);
    setDbCenter(kDefaultCenter);
}
