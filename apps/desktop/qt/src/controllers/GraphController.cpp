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

// The switches' keys and defaults, in GraphController::Flag's order
const struct {
    const char *key;
    bool initially;
} kFlagKeys[] = {
    { "showFreqGrid", true }, { "showFreqLabels", true }, { "showDbGrid", true }, { "showDbLabels", true },
    { "freqReadout", true },  { "levelReadout", true },   { "showGlow", false },  { "showPhase", false },
    { "phaseUnwrapped", false }, { "popOutFollows", true },
};

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
    // A stored number, or `fallback` when it is missing or not a number
    auto stored = [this](const char *key, double fallback) {
        bool ok = false;
        const double v = m_settings.value(QLatin1String(key)).toDouble(&ok);
        return ok && qIsFinite(v) ? v : fallback;
    };
    m_scale.lowHz = nearestOf(kMinFreqs, int(std::size(kMinFreqs)), stored("minFreq", kDefaultMinFreq));
    m_scale.highHz = nearestOf(kMaxFreqs, int(std::size(kMaxFreqs)), stored("maxFreq", kDefaultMaxFreq));
    m_scale.span = qBound(kRangeMin, stored("dbRange", kDefaultRange), kRangeMax);
    m_scale.centre = qBound(kCenterMin, stored("dbCenter", kDefaultCenter), kCenterMax);
    m_scale.gridOpacity = qBound(0.0, stored("gridOpacity", kDefaultGridOpacity), kGridOpacityMax);
    m_scale.lineWidth = qBound(kLineMin, stored("lineWidth", kDefaultLineWidth), kLineMax);
    m_scale.height = qBound(kHeightMin, stored("height", kDefaultHeight), kHeightMax);
    for (int f = 0; f < FlagCount; ++f)
        m_on[size_t(f)] = m_settings.value(QLatin1String(kFlagKeys[f].key), kFlagKeys[f].initially).toBool();

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
        ++m_curveRev;
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
        ++m_curveRev;
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
    ++m_curveRev;
    emit curvesChanged();
}

void GraphController::endOutputGainPreview(int output) {
    if (!m_gainPreview.remove(output)) return;
    ++m_curveRev;
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
    if (popoutFollowsSelection()) return visibleChannels();
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
    if (popoutFollowsSelection()) {
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

void GraphController::setFlag(Flag f, bool on) {
    if (f < 0 || f >= FlagCount || m_on[size_t(f)] == on) return;
    m_on[size_t(f)] = on;
    store(QLatin1String(kFlagKeys[f].key), on);
}

void GraphController::setPopoutFollowsSelection(bool on) {
    if (on == popoutFollowsSelection()) return;
    // Switched off, the pop-out starts again from the default curves
    m_popOutSet = false;
    setFlag(PopoutFollows, on);
    emit visibilityChanged();
}

// Each setter keeps its value within the setup page's limits and stores it
void GraphController::setMinFreq(double hz) {
    const double v = nearestOf(kMinFreqs, int(std::size(kMinFreqs)), hz);
    if (v != m_scale.lowHz) store(QStringLiteral("minFreq"), m_scale.lowHz = v);
}

void GraphController::setMaxFreq(double hz) {
    const double v = nearestOf(kMaxFreqs, int(std::size(kMaxFreqs)), hz);
    if (v != m_scale.highHz) store(QStringLiteral("maxFreq"), m_scale.highHz = v);
}

void GraphController::setDbRange(double db) {
    const double v = qIsFinite(db) ? qBound(kRangeMin, std::round(db), kRangeMax) : m_scale.span;
    if (v != m_scale.span) store(QStringLiteral("dbRange"), m_scale.span = v);
}

void GraphController::setDbCenter(double db) {
    const double v = qIsFinite(db) ? qBound(kCenterMin, std::round(db), kCenterMax) : m_scale.centre;
    if (v != m_scale.centre) store(QStringLiteral("dbCenter"), m_scale.centre = v);
}

void GraphController::setGraphGridOpacity(double v) {
    const double c = qIsFinite(v) ? qBound(0.0, v, kGridOpacityMax) : m_scale.gridOpacity;
    if (c != m_scale.gridOpacity) store(QStringLiteral("gridOpacity"), m_scale.gridOpacity = c);
}

void GraphController::setGraphLineWidth(double v) {
    // Half points, as the Mac's slider steps
    const double c = qIsFinite(v) ? qBound(kLineMin, std::round(v * 2) / 2, kLineMax) : m_scale.lineWidth;
    if (c != m_scale.lineWidth) store(QStringLiteral("lineWidth"), m_scale.lineWidth = c);
}

void GraphController::setGraphHeight(double v) {
    const double c = qIsFinite(v) ? qBound(kHeightMin, v, kHeightMax) : m_scale.height;
    if (c != m_scale.height) store(QStringLiteral("height"), m_scale.height = c);
}

void GraphController::resetScale() {
    setMinFreq(kDefaultMinFreq);
    setMaxFreq(kDefaultMaxFreq);
    setDbRange(kDefaultRange);
    setDbCenter(kDefaultCenter);
}
