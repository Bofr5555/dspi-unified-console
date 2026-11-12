// -- WP QP4 -- The on-graph PEQ editor; PeqOverlay.h lists its gestures.
#include "PeqOverlay.h"

#include "ChannelController.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "PeqEditorController.h"
#include "Registrar.h"

#include <QCursor>
#include <QGuiApplication>
#include <QKeyEvent>
#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <QQuickWindow>
#include <algorithm>
#include <cmath>

static const GraphController::Slot peqOverlaySlot(GraphController::PeqOverlay, "PeqGraphOverlay");
// -- WP QPI2 -- the band bar under the graph
static const GraphController::Slot peqDockSlot(GraphController::PeqDock, "PeqBandBar");

namespace {

// The Mac editor's tuning
constexpr double kDotHit = 10;
constexpr double kCurveHit = 6;
constexpr double kDragThreshold = 2;
constexpr double kZoomZone = 40;
constexpr double kQPointsPerOctave = 60;
constexpr double kFine = 0.12;
constexpr double kBandAnimation = 0.22;
constexpr double kEmphasisTau = 0.07;
constexpr int kCommitDelayMs = 450;
constexpr int kDeviceIntervalMs = 33;
constexpr double kWheelSession = 0.5;
constexpr int kRevealDwellMs = 250;
constexpr int kChipHideMs = 350;
constexpr double kShelfZone = 0.12;
constexpr double kPanelGap = 16;

constexpr Qt::KeyboardModifier kCommand = Qt::ControlModifier;
constexpr Qt::KeyboardModifier kOption = Qt::AltModifier;

// Every editor in the app (the main window's and the pop-out's): the first
// that edits owns the graph's overlay channel
QList<PeqOverlay *> &overlays() {
    static QList<PeqOverlay *> all;
    return all;
}

double distance(const QPointF &a, const QPointF &b) { return std::hypot(a.x() - b.x(), a.y() - b.y()); }

QColor faded(QColor c, double alpha) {
    c.setAlphaF(float(qBound(0.0, c.alphaF() * alpha, 1.0)));
    return c;
}

QPen pen(const QColor &c, double width, Qt::PenStyle style = Qt::SolidLine) {
    QPen p(c, width, style, Qt::RoundCap, Qt::RoundJoin);
    return p;
}

} // namespace

PeqOverlay::PeqOverlay(QQuickItem *parentItem) : QQuickPaintedItem(parentItem) {
    setAcceptedMouseButtons(Qt::RightButton | Qt::LeftButton);
    // Hover comes from PeqGraphOverlay.qml's HoverHandler (hoverAt), so the
    // graph's own hover (its gear) still sees the pointer
    setActiveFocusOnTab(true);
    setAntialiasing(true);
    m_clock.start();
    overlays().append(this);
    m_emphasis.resize(BANDS_PER_CHANNEL);
    m_frame.setInterval(16);
    connect(&m_frame, &QTimer::timeout, this, &PeqOverlay::tick);
    for (QTimer *t : { &m_chipHide, &m_reveal, &m_device, &m_commit }) t->setSingleShot(true);
    m_chipHide.setInterval(kChipHideMs);
    m_reveal.setInterval(kRevealDwellMs);
    m_device.setInterval(kDeviceIntervalMs);
    m_commit.setInterval(kCommitDelayMs);
    connect(&m_chipHide, &QTimer::timeout, this, &PeqOverlay::hideChip);
    connect(&m_device, &QTimer::timeout, this, &PeqOverlay::sendToDevice);
    connect(&m_commit, &QTimer::timeout, this, &PeqOverlay::commitLive);
    connect(&m_reveal, &QTimer::timeout, this, [this] {
        if (m_hovered < 0 || !m_editor) return;
        if (m_gesture.kind != Gesture::Idle || graphWheelActive() || chipWheelActive()) {
            m_reveal.start();
            return;
        }
        m_editor->reveal(m_hovered);
    });
}

PeqOverlay::~PeqOverlay() {
    commitLive();
    overlays().removeAll(this);
    m_channel = -1;
    updateOwnership();
}

// ---- Inputs and configuration ----------------------------------------------

void PeqOverlay::setGraph(GraphController *g) {
    if (g == m_graph) return;
    if (m_graph) disconnect(m_graph, nullptr, this, nullptr);
    m_graph = g;
    if (!m_editor) {
        m_editor = Registrar::instance<PeqEditorController>();
        if (m_editor) {
            connect(m_editor, &PeqEditorController::selectionChanged, this, [this] {
                // -- WP QPI2 -- an overlay that edits nothing (a closed pop-out's)
                // has no bands to pin the chip to
                if (!editing()) return;
                const QSet<int> &s = selection();
                if (s.size() == 1) m_rangeAnchor = *s.cbegin();
                pinChipToSelection();
                invalidate();
            });
            connect(m_editor, &PeqEditorController::rowHoveredChanged, this, [this] {
                m_rowHovered = m_editor->rowHovered();
                invalidate();
            });
        }
        if (auto *cc = Registrar::instance<ChannelController>()) {
            connect(cc, &ChannelController::changed, this, &PeqOverlay::reconfigure);
            connect(cc, &ChannelController::crossoverTabShownChanged, this, &PeqOverlay::reconfigure);
        }
    }
    if (m_graph) {
        connect(m_graph, &GraphController::preferencesChanged, this, &PeqOverlay::reconfigure);
        connect(m_graph, &GraphController::visibilityChanged, this, &PeqOverlay::reconfigure);
        connect(m_graph, &GraphController::curvesChanged, this, &PeqOverlay::reconfigure);
        connect(m_graph, &GraphController::poppedOutChanged, this, &PeqOverlay::reconfigure);
    }
    emit graphChanged();
    reconfigure();
}

void PeqOverlay::setPageChannel(int ch) {
    if (ch == m_pageChannel) return;
    m_pageChannel = ch;
    emit pageChannelChanged();
    reconfigure();
}

void PeqOverlay::setCurveColor(const QColor &c) {
    if (c == m_curveColor) return;
    m_curveColor = c;
    emit curveColorChanged();
    rebuild();
}

void PeqOverlay::setBandColors(const QVariantList &colors) {
    m_bandColorList = colors;
    m_bandColors.clear();
    for (const QVariant &v : colors) m_bandColors << v.value<QColor>();
    emit bandColorsChanged();
    refreshChip();
    rebuild();
}

void PeqOverlay::setBackdrop(const QColor &c) {
    if (c == m_backdrop) return;
    m_backdrop = c;
    emit backdropChanged();
    update();
}

void PeqOverlay::setInk(const QColor &c) {
    if (c == m_ink) return;
    m_ink = c;
    emit inkChanged();
    rebuild();
}

void PeqOverlay::itemChange(ItemChange what, const ItemChangeData &data) {
    QQuickPaintedItem::itemChange(what, data);
    if (what == ItemVisibleHasChanged || what == ItemSceneChange) {
        // -- WP QPI2 -- follow only the window the item is in now
        if (what == ItemSceneChange) {
            if (m_window) disconnect(m_window, &QWindow::visibleChanged, this, &PeqOverlay::reconfigure);
            m_window = data.window;
            if (m_window)
                connect(m_window, &QWindow::visibleChanged, this, &PeqOverlay::reconfigure, Qt::UniqueConnection);
        }
        QMetaObject::invokeMethod(this, &PeqOverlay::reconfigure, Qt::QueuedConnection);
    }
}

void PeqOverlay::readBands(QVector<FilterParams> &bands, QVector<FilterParams> &statics) const {
    bands.clear();
    statics.clear();
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) bands << m_editor->band(m_channel, b, false);
    if (m_graph && m_channel >= m_graph->firstOutput())
        for (int b = 0; b < MAX_XOVER_BANDS; ++b) statics << m_editor->band(m_channel, b, true);
}

void PeqOverlay::reconfigure() {
    if (!m_editor || !m_graph) return;
    // Our own writes change the bands as they go out: read them once after
    if (m_inReconfigure || m_writing > 0) {
        m_reconfigurePending = true;
        return;
    }
    m_inReconfigure = true;
    do {
        m_reconfigurePending = false;
        configure();
    } while (m_reconfigurePending);
    m_inReconfigure = false;
}

void PeqOverlay::writing(const std::function<void()> &write) {
    ++m_writing;
    write();
    --m_writing;
    if (m_writing == 0 && m_reconfigurePending && !m_inReconfigure) reconfigure();
}

void PeqOverlay::configure() {
    auto *cc = Registrar::instance<ChannelController>();
    DeviceModel *device = DeviceModel::instance();
    const DspState *s = device ? device->state() : nullptr;
    const int ch = m_pageChannel;
    // The Mac's editedChannel: a device that takes writes, the page's channel
    // with its curve shown, and the PEQ tab (the XO tab lists other bands)
    const bool shown = isVisible() && window() && window()->isVisible();
    const bool edits = shown && s && cc && cc->writable() && ch >= 0 && ch < m_graph->channelCount()
        && m_graph->isCurveVisible(ch) && !(cc->crossoverTabShown() && ch >= m_graph->firstOutput());
    const int next = edits ? ch : -1;

    // A pending wheel or key edit belongs to the channel it was made on
    if (next != m_channel) commitLive();
    const int old = m_channel;
    m_channel = next;
    m_available.clear();
    for (int t : m_editor->availableTypes()) m_available.insert(t);
    m_bypassSupported = m_editor->bypassSupported();
    m_offsetDb = next >= 0 ? m_graph->outputGain(next) : 0.0;
    m_flat = s && next >= 0 && s->bypass && next < 2;

    QVector<FilterParams> bands, statics;
    if (next >= 0) readBands(bands, statics);
    m_statics = statics;

    if (old != next || next < 0) {
        resetInteraction();
        m_committed = m_shown = bands;
        m_animFrom.reset();
    } else {
        bool differs = bands.size() != m_committed.size();
        for (int b = 0; !differs && b < bands.size(); ++b) differs = !Peq::same(bands[b], m_committed[b]);
        if (differs) {
            m_committed = bands;
            if (m_gesture.kind == Gesture::Idle && m_live.isEmpty()) {
                startBandAnimation();
            } else {
                // Mid-edit, only the bands not being edited follow the device
                if (m_shown.size() != m_committed.size()) m_shown = m_committed;
                for (int b = 0; b < m_shown.size(); ++b)
                    m_shown[b] = m_live.contains(b) ? m_live.value(b) : m_committed[b];
            }
        }
    }
    updateOwnership();
    refreshChip();
    // -- WP QPI2 -- an editor taking over (the pop-out, a page) shows the
    // selection in its band bar at once
    if (old != next && next >= 0) pinChipToSelection();
    invalidate();
    if (old != next) emit viewChanged();
}

void PeqOverlay::updateOwnership() {
    // One editor draws the edited curve, and the curve layer leaves its
    // channel out; decided over all editors, so their order does not matter
    int owner = -1;
    for (const PeqOverlay *o : std::as_const(overlays()))
        if (o->m_channel >= 0) {
            owner = o->m_channel;
            break;
        }
    if (m_graph) m_graph->setOverlayChannel(owner);
    if (m_editor) m_editor->setEditedChannel(owner);
}

void PeqOverlay::resetInteraction() {
    closeCard();
    m_gesture = Gesture{};
    m_rangeAnchor = m_hovered = -1;
    m_ghost.reset();
    m_ghostShown.reset();
    m_ghostAmount = 0;
    m_chipShown = false;
    m_chipBand = -1;
    m_shapePage = false;
    m_chipEditing = false;
    m_chipHide.stop();
    m_reveal.stop();
    hideReadouts();
    m_emphasis.fill(Emphasis{});
    unsetCursor();
}

const QSet<int> &PeqOverlay::selection() const {
    static const QSet<int> none;
    return m_editor ? m_editor->selection() : none;
}

QList<int> PeqOverlay::selectedBands() const {
    QList<int> out(selection().cbegin(), selection().cend());
    std::sort(out.begin(), out.end());
    return out;
}

// ---- Mapping (BodePlot's: log frequency across, dB down) -------------------

double PeqOverlay::xOf(double hz) const {
    if (!m_graph || hz <= 0) return 0;
    return std::log10(hz / m_graph->minFreq()) / std::log10(m_graph->maxFreq() / m_graph->minFreq()) * width();
}

double PeqOverlay::freqAt(double x) const {
    if (!m_graph) return 1000;
    return m_graph->minFreq() * std::pow(m_graph->maxFreq() / m_graph->minFreq(), x / qMax(width(), 1.0));
}

double PeqOverlay::yOf(double db) const {
    if (!m_graph) return 0;
    return (m_graph->dbTop() - db) / (m_graph->dbTop() - m_graph->dbBottom()) * height();
}

double PeqOverlay::dbAt(double y) const {
    if (!m_graph) return 0;
    return m_graph->dbTop() - y / qMax(height(), 1.0) * (m_graph->dbTop() - m_graph->dbBottom());
}

double PeqOverlay::dbPerPoint() const {
    return m_graph ? (m_graph->dbTop() - m_graph->dbBottom()) / qMax(height(), 1.0) : 0.1;
}

double PeqOverlay::freqLo() const { return qMax(Peq::kFreqMin, m_graph ? m_graph->minFreq() : Peq::kFreqMin); }
double PeqOverlay::freqHi() const { return qMax(freqLo(), qMin(Peq::kFreqMax, m_graph ? m_graph->maxFreq() : Peq::kFreqMax)); }

// ---- Band queries ------------------------------------------------------------

bool PeqOverlay::isBand(int b) const { return b >= 0 && b < bandCount() && Peq::isGraphBand(m_shown[b]); }

QList<int> PeqOverlay::graphBands() const {
    QList<int> out;
    for (int b = 0; b < bandCount(); ++b)
        if (isBand(b)) out << b;
    return out;
}

QPointF PeqOverlay::dotOf(const FilterParams &p) const {
    const double y = yOf(Peq::dotDb(Peq::roleOf(p), p));
    return { xOf(p.freq), qBound(6.0, y, qMax(height() - 6, 6.0)) };
}

double PeqOverlay::combinedDb(double hz) const {
    double db = m_offsetDb;
    if (m_flat) return db;
    QVector<FilterParams> audible;
    for (int b : graphBands())
        if (!m_shown[b].bypass) audible << m_shown[b];
    audible += m_statics;
    return db + Peq::response(audible.constData(), int(audible.size()), hz);
}

// The dot under `p`, preferring selected and hovered dots, which draw on top
int PeqOverlay::dotUnder(const QPointF &p) const {
    int best = -1;
    double score = 0;
    for (int b : graphBands()) {
        const double d = distance(dotOf(m_shown[b]), p);
        if (d > kDotHit) continue;
        const double s = d - (selection().contains(b) ? 3 : 0) - (m_hovered == b ? 2 : 0);
        if (best < 0 || s < score) {
            best = b;
            score = s;
        }
    }
    return best;
}

// The band whose fill (between its curve and 0 dB) holds `p`: it lights and
// takes the wheel, but clicks go through it to the graph
int PeqOverlay::fillUnder(const QPointF &p) const {
    const double hz = freqAt(p.x());
    const double zero = yOf(0);
    int best = -1;
    double bestD = 0;
    for (int b : graphBands()) {
        if (Peq::roleOf(m_shown[b]).kind == Peq::Role::Locked) continue;
        const double y = yOf(Peq::bandDb(m_shown[b], hz));
        if (std::abs(y - zero) < 3 || p.y() < qMin(y, zero) || p.y() > qMax(y, zero)) continue;
        const double d = std::abs(p.y() - y);
        if (best < 0 || d < bestD) {
            best = b;
            bestD = d;
        }
    }
    return best;
}

int PeqOverlay::bandUnder(const QPointF &p) const {
    const int dot = dotUnder(p);
    return dot >= 0 ? dot : fillUnder(p);
}

bool PeqOverlay::onCurveAt(const QPointF &p) const { return std::abs(yOf(combinedDb(freqAt(p.x()))) - p.y()) <= kCurveHit; }

int PeqOverlay::freeSlot() const {
    for (int b = 0; b < qMin(int(m_committed.size()), int(BANDS_PER_CHANNEL)); ++b)
        if (m_committed[b].filter_type == FILTER_FLAT && !m_live.contains(b)) return b;
    return -1;
}

QList<int> PeqOverlay::frequencyOrder(QList<int> bands) const {
    // -- WP QPI2 -- through current(), which checks the index
    std::sort(bands.begin(), bands.end(), [this](int a, int b) {
        const float fa = current(a).freq, fb = current(b).freq;
        return fa != fb ? fa < fb : a < b;
    });
    return bands;
}

FilterParams PeqOverlay::current(int b) const {
    if (m_live.contains(b)) return m_live.value(b);
    return b >= 0 && b < m_shown.size() ? m_shown[b] : Peq::flatBand();
}

// A double-click makes a bell at the pointer's frequency and level; pulling
// the curve makes a flat bell, or a shelf near either end
FilterParams PeqOverlay::creation(const QPointF &p, bool fromCurve) const {
    const double fx = p.x() / qMax(width(), 1.0);
    FilterParams band = Peq::make(FILTER_PEAKING, Peq::clamp(freqAt(p.x()), Peq::kFreqMin, Peq::kFreqMax), 1, 0);
    if (fromCurve) {
        if (fx < kShelfZone && m_available.contains(FILTER_LOWSHELF)) {
            band.filter_type = FILTER_LOWSHELF;
            band.q = 0.707f;
        } else if (fx > 1 - kShelfZone && m_available.contains(FILTER_HIGHSHELF)) {
            band.filter_type = FILTER_HIGHSHELF;
            band.q = 0.707f;
        }
        return band;
    }
    band.gain = float(Peq::clamp(dbAt(p.y()), Peq::kGainMin, Peq::kGainMax));
    return band;
}

// A new band of `shape` at `order` at `p`: the pointer's frequency, and its
// level as the gain for shapes that have one
std::optional<FilterParams> PeqOverlay::shaped(int shape, int order, const QPointF &p) const {
    const int type = Peq::typeFor(shape, order);
    if (type < 0) return std::nullopt;
    const bool narrow = shape == Peq::Bell || shape == Peq::Notch;
    FilterParams band = Peq::make(type, Peq::clamp(freqAt(p.x()), freqLo(), freqHi()), narrow ? 1 : 0.707, 0);
    if (Peq::usesGain(type)) band.gain = float(Peq::clamp(dbAt(p.y()), Peq::kGainMin, Peq::kGainMax));
    return band;
}

// A band action covers the selection when the band is in it, else the band alone
QSet<int> PeqOverlay::targets(int b) const { return selection().contains(b) ? selection() : QSet<int>{ b }; }

// The selected band the chip stays on and the wheel adjusts: the one it
// shows if selected, else the anchor, else the lowest
int PeqOverlay::selectedChipBand() const {
    const QSet<int> &s = selection();
    if (s.isEmpty() || !editing()) return -1; // -- WP QPI2 --
    if (m_chipShown && s.contains(m_chipBand)) return m_chipBand;
    if (s.contains(m_rangeAnchor)) return m_rangeAnchor;
    return frequencyOrder(QList<int>(s.cbegin(), s.cend())).value(0, -1);
}

// ---- Drawing -------------------------------------------------------------------

double PeqOverlay::now() const { return double(m_clock.nsecsElapsed()) / 1e9; }

void PeqOverlay::invalidate() {
    if (!editing()) {
        m_frame.stop();
        rebuild();
        return;
    }
    updateEmphasisTargets();
    bool running = m_animFrom.has_value() || m_ghostAmount != (m_ghost ? 1.0f : 0.0f);
    for (const Emphasis &e : std::as_const(m_emphasis)) running = running || !e.settled();
    if (running) startFrames();
    rebuild();
}

void PeqOverlay::updateEmphasisTargets() {
    QSet<int> dragged;
    if (m_gesture.kind == Gesture::Drag)
        for (auto it = m_gesture.start.cbegin(); it != m_gesture.start.cend(); ++it) dragged.insert(it.key());
    for (int i = 0; i < m_emphasis.size(); ++i) {
        const bool shown = isBand(i);
        m_emphasis[i].selectTarget = shown && selection().contains(i) ? 1 : 0;
        m_emphasis[i].hoverTarget = shown
                && (m_hovered == i || m_rowHovered == i || dragged.contains(i) || (m_chipShown && m_chipBand == i))
            ? 1
            : 0;
    }
}

void PeqOverlay::startFrames() {
    if (m_frame.isActive()) return;
    m_lastFrame = 0;
    m_frame.start();
    emit viewChanged();
}

void PeqOverlay::tick() {
    if (!advance(now())) {
        m_frame.stop();
        emit viewChanged();
    }
    rebuild();
}

void PeqOverlay::settle() {
    updateEmphasisTargets();
    advance(INFINITY);
    m_frame.stop();
    rebuild();
    emit viewChanged();
}

// Advances the animations to `t`; true while any still runs. INFINITY
// finishes them all.
bool PeqOverlay::advance(double t) {
    const bool finite = std::isfinite(t);
    const double dt = finite ? (m_lastFrame == 0 ? 1.0 / 60 : qMin(t - m_lastFrame, 0.1)) : 1.0;
    m_lastFrame = finite ? t : 0;
    bool running = false;

    if (m_animFrom) {
        const double f = finite ? (t - m_animStart) / kBandAnimation : 1;
        QVector<FilterParams> target = m_committed;
        for (auto it = m_live.cbegin(); it != m_live.cend(); ++it)
            if (it.key() < target.size()) target[it.key()] = it.value();
        if (f >= 1 || m_animFrom->size() != target.size()) {
            m_shown = target;
            m_animFrom.reset();
            refreshChip();
        } else {
            const double eased = 1 - std::pow(1 - f, 3);
            for (int b = 0; b < target.size(); ++b) m_shown[b] = Peq::blend((*m_animFrom)[b], target[b], eased);
            running = true;
        }
    }

    const float k = float(1 - std::exp(-dt / kEmphasisTau));
    auto approach = [k](float &v, float target) {
        v += (target - v) * k;
        if (std::abs(target - v) < 0.004f) v = target;
    };
    for (Emphasis &e : m_emphasis) {
        approach(e.hover, e.hoverTarget);
        approach(e.select, e.selectTarget);
        running = running || !e.settled();
    }
    approach(m_ghostAmount, m_ghost ? 1.0f : 0.0f);
    if (m_ghost) m_ghostShown = m_ghost;
    if (m_ghostAmount == 0) m_ghostShown.reset();
    running = running || m_ghostAmount != (m_ghost ? 1.0f : 0.0f);
    if (!running) m_lastFrame = 0;
    return running;
}

void PeqOverlay::startBandAnimation() {
    const QVector<FilterParams> &target = m_committed;
    if (m_shown.size() != target.size()) {
        m_shown = target;
        return;
    }
    bool significant = false;
    for (int b = 0; b < target.size() && !significant; ++b) {
        const FilterParams &a = m_shown[b], &z = target[b];
        significant = a.filter_type != z.filter_type || a.bypass != z.bypass || std::abs(a.gain - z.gain) > 0.02
            || std::abs(std::log(qMax(double(a.freq), 1.0) / qMax(double(z.freq), 1.0))) > 0.002
            || std::abs(std::log(qMax(double(a.q), 0.01) / qMax(double(z.q), 0.01))) > 0.002;
    }
    if (significant) {
        m_animFrom = m_shown;
        m_animStart = now();
    } else {
        m_shown = target;
    }
}

void PeqOverlay::rebuild() {
    Picture pic;
    const double w = width(), h = height();
    if (!editing() || w < 2 || h < 2 || !m_graph) {
        m_picture = pic;
        update();
        return;
    }
    const int columns = qBound(48, int(w / 2), 600);
    const double zero = yOf(0);
    const double dim = m_flat ? 0.5 : 1.0;
    auto sampleX = [&](int i) { return w * i / (columns - 1); };
    auto clampY = [h](double y) { return qBound(-h, y, 2 * h); };
    // Lines are only worked out again when their band or the scale changed
    const Scale scale{ w, h, m_graph->minFreq(), m_graph->maxFreq(), m_graph->dbTop(), m_graph->dbBottom(), m_offsetDb, m_flat ? 1.0 : 0.0 };
    if (scale != m_scaleKey) {
        m_scaleKey = scale;
        m_lineCache.clear();
        m_curveKey.clear();
    }

    for (int b : graphBands()) {
        const FilterParams &band = m_shown[b];
        const Emphasis &e = m_emphasis[b];
        const double lift = qMax(e.hover, e.select);
        QColor color = m_bandColors.value(b % qMax(int(m_bandColors.size()), 1), m_ink);
        if (band.bypass) {
            const double grey = (color.redF() + color.greenF() + color.blueF()) / 3;
            auto mix = [grey](double c) { return grey + (c - grey) * 0.35; };
            color = QColor::fromRgbF(float(mix(color.redF())), float(mix(color.greenF())), float(mix(color.blueF())));
        }
        Picture::Lobe lobe;
        lobe.color = color;
        lobe.lineOpacity = (band.bypass ? 0.35 + 0.25 * lift : 0.9 * lift) * dim;
        lobe.fillOpacity = (band.bypass ? 0.06 + 0.06 * lift : 0.22 + 0.2 * lift) * dim;
        lobe.zeroY = zero;
        // The dB a lobe reaches, which bounds its fill: 0 dB to the gain for
        // bells and shelves, down off the plot for cuts and notches
        double lo = m_graph->dbBottom() - 1, hi = m_graph->dbTop() + 1;
        if (Peq::usesGain(band.filter_type)) {
            lo = qMin(double(band.gain), 0.0);
            hi = qMax(double(band.gain), 0.0);
        } else if (band.filter_type == FILTER_LOWPASS || band.filter_type == FILTER_HIGHPASS) {
            hi = qMax(20 * std::log10(qMax(double(band.q), 0.01)), 0.0);
        } else if (band.filter_type == FILTER_ALLPASS || band.filter_type == FILTER_ALLPASS1) {
            lo = hi = 0;
        }
        const double top = qMax(yOf(hi) - 2, 0.0), bottom = qMin(yOf(lo) + 2, h);
        lobe.edgeY = std::abs(yOf(hi) - zero) > std::abs(yOf(lo) - zero) ? yOf(hi) : yOf(lo);
        auto cached = m_lineCache.find(b);
        if (cached == m_lineCache.end() || !Peq::same(cached->first, band)) {
            QPolygonF line;
            for (int i = 0; i < columns; ++i) {
                const double x = sampleX(i);
                line << QPointF(x, clampY(yOf(Peq::bandDb(band, freqAt(x)))));
            }
            cached = m_lineCache.insert(b, qMakePair(band, line));
        }
        lobe.line = cached->second;
        if (bottom - top > 0.5) {
            lobe.fill = lobe.line;
            for (QPointF &pt : lobe.fill) pt.setY(qBound(top, pt.y(), bottom));
            lobe.fill << QPointF(w, zero) << QPointF(0, zero);
        }
        pic.lobes << lobe;

        const QPointF c = dotOf(band);
        Picture::Dot dot;
        dot.at = c;
        dot.radius = 5 + 1.5 * lift + 0.5 * e.select;
        dot.color = color;
        dot.alpha = (band.bypass ? 0.55 : 1.0) * dim;
        dot.select = e.select;
        dot.order = lift + e.select;
        pic.dots << dot;
    }
    if (m_ghostShown && m_ghostAmount > 0) {
        Picture::Dot ghost;
        ghost.at = dotOf(*m_ghostShown);
        ghost.color = m_ink;
        ghost.alpha = 0.3 * m_ghostAmount;
        ghost.order = -1;
        pic.dots << ghost;
    }
    // Emphasised dots last, so they draw on top
    std::stable_sort(pic.dots.begin(), pic.dots.end(), [](const Picture::Dot &a, const Picture::Dot &b) { return a.order < b.order; });

    QVector<FilterParams> key;
    for (int b : graphBands()) key << m_shown[b];
    key += m_statics;
    bool sameCurve = key.size() == m_curveKey.size() && !m_curve.isEmpty();
    for (int i = 0; sameCurve && i < key.size(); ++i) sameCurve = Peq::same(key[i], m_curveKey[i]);
    if (!sameCurve) {
        m_curveKey = key;
        m_curve.clear();
        for (int i = 0; i < columns; ++i) {
            const double x = sampleX(i);
            m_curve << QPointF(x, clampY(yOf(combinedDb(freqAt(x)))));
        }
    }
    pic.curve = m_curve;
    pic.curveColor = m_curveColor;
    pic.lineWidth = m_graph->graphLineWidth();
    pic.glow = m_graph->showGraphGlow();
    if (m_gesture.kind == Gesture::Marquee) pic.marquee = QRectF(m_gesture.at, m_gesture.to).normalized();
    m_picture = pic;
    update();
}

void PeqOverlay::paint(QPainter *canvas) {
    const Picture &pic = m_picture;
    if (pic.curve.isEmpty()) return;
    canvas->setRenderHint(QPainter::Antialiasing);

    // Every band's lobe, faint and fading towards 0 dB; hovering or
    // selecting a band brightens it and adds its outline
    for (const Picture::Lobe &lobe : pic.lobes) {
        if (lobe.fill.size() > 2 && lobe.fillOpacity > 0.001) {
            QLinearGradient fade(QPointF(0, lobe.edgeY), QPointF(0, lobe.zeroY));
            fade.setColorAt(0, faded(lobe.color, lobe.fillOpacity));
            fade.setColorAt(1, faded(lobe.color, lobe.fillOpacity * 0.15));
            canvas->setPen(Qt::NoPen);
            canvas->setBrush(fade);
            canvas->drawPolygon(lobe.fill);
        }
        if (lobe.lineOpacity > 0.001) {
            canvas->setBrush(Qt::NoBrush);
            canvas->setPen(pen(faded(lobe.color, lobe.lineOpacity), 1.25));
            canvas->drawPolyline(lobe.line);
        }
    }

    // The channel's combined curve, with BodePlot's glow when it is on
    canvas->setBrush(Qt::NoBrush);
    if (pic.glow) {
        canvas->setPen(pen(faded(pic.curveColor, 0.12), pic.lineWidth * 4));
        canvas->drawPolyline(pic.curve);
        canvas->setPen(pen(faded(pic.curveColor, 0.3), pic.lineWidth * 2));
        canvas->drawPolyline(pic.curve);
    }
    canvas->setPen(pen(pic.curveColor, pic.lineWidth));
    canvas->drawPolyline(pic.curve);

    // Flat discs; a selected one grows a centre in the plot's background
    canvas->setPen(Qt::NoPen);
    for (const Picture::Dot &dot : pic.dots) {
        canvas->setBrush(faded(dot.color, dot.alpha));
        canvas->drawEllipse(dot.at, dot.radius, dot.radius);
        if (dot.select > 0.01) {
            const double r = 2.2 * dot.select;
            canvas->setBrush(faded(m_backdrop, qMin(dot.select * 2, 1.0) * dot.alpha));
            canvas->drawEllipse(dot.at, r, r);
        }
    }

    if (!pic.marquee.isNull()) {
        canvas->setBrush(faded(m_ink, 0.06));
        canvas->setPen(QPen(faded(m_ink, 0.45), 1, Qt::DashLine));
        canvas->drawRect(pic.marquee);
    }
}

void PeqOverlay::geometryChange(const QRectF &now, const QRectF &before) {
    QQuickPaintedItem::geometryChange(now, before);
    if (now.size() == before.size()) return;
    rebuild();
}

// ---- The editing pipeline -------------------------------------------------------

// A change in progress: drawn, shown in the chip and the list, sent to the
// device at most 30 times a second, and committed later
void PeqOverlay::setLive(const QHash<int, FilterParams> &changes) {
    if (changes.isEmpty()) return;
    if (m_animFrom) {
        m_animFrom.reset();
        m_shown = m_committed;
        for (auto it = m_live.cbegin(); it != m_live.cend(); ++it)
            if (it.key() < m_shown.size()) m_shown[it.key()] = it.value();
    }
    for (auto it = changes.cbegin(); it != changes.cend(); ++it) {
        if (it.key() >= m_shown.size()) continue;
        m_live.insert(it.key(), it.value());
        m_shown[it.key()] = it.value();
        m_editor->showLive(it.key(), it.value());
        m_deviceDirty.insert(it.key());
    }
    if (!m_device.isActive()) sendToDevice();
    refreshChip();
    invalidate();
}

void PeqOverlay::sendToDevice() {
    if (m_channel < 0 || m_deviceDirty.isEmpty()) return;
    QList<int> dirty(m_deviceDirty.cbegin(), m_deviceDirty.cend());
    std::sort(dirty.begin(), dirty.end());
    m_deviceDirty.clear();
    QList<QPair<int, FilterParams>> out;
    for (int b : dirty)
        if (m_live.contains(b)) out << qMakePair(b, m_live.value(b));
    const int ch = m_channel;
    writing([&] { m_editor->writeBands(ch, out); });
    m_device.start();
}

// Writes live edits once, as they stand
void PeqOverlay::commitLive() {
    m_device.stop();
    m_deviceDirty.clear();
    m_commit.stop();
    if (m_editor) m_editor->endLive();
    QList<QPair<int, FilterParams>> changes;
    QList<int> keys = m_live.keys();
    std::sort(keys.begin(), keys.end());
    for (int b : keys) {
        const FilterParams p = m_live.value(b);
        if (b < m_committed.size() && !Peq::same(p, m_committed[b])) changes << qMakePair(b, p);
    }
    m_live.clear();
    if (changes.isEmpty() || m_channel < 0 || !m_editor) return;
    for (const auto &[b, p] : std::as_const(changes)) m_committed[b] = p;
    const int ch = m_channel;
    writing([&] { m_editor->writeBands(ch, changes); });
}

// Wheel and key edits: live now, committed once they pause
void PeqOverlay::setLiveThenCommit(const QHash<int, FilterParams> &changes) {
    setLive(changes);
    m_commit.start();
}

// Discrete edits (create, delete, shape, typed values) commit at once
void PeqOverlay::commitNow(const QHash<int, FilterParams> &changes) {
    commitLive();
    if (m_channel < 0 || changes.isEmpty()) return;
    m_animFrom.reset();
    QList<int> keys = changes.keys();
    std::sort(keys.begin(), keys.end());
    QList<QPair<int, FilterParams>> out;
    for (int b : keys) {
        if (b >= m_committed.size()) continue;
        m_committed[b] = m_shown[b] = changes.value(b);
        out << qMakePair(b, changes.value(b));
    }
    const int ch = m_channel;
    writing([&] { m_editor->writeBands(ch, out); });
    refreshChip();
    invalidate();
}

void PeqOverlay::setSelection(const QSet<int> &bands) {
    QSet<int> valid;
    for (int b : bands)
        if (isBand(b)) valid.insert(b);
    if (valid == selection() || !m_editor) return;
    m_editor->setSelection(valid);
    pinChipToSelection();
    invalidate();
    emit viewChanged();
}

void PeqOverlay::setHovered(int band) {
    if (band == m_hovered) return;
    m_hovered = band;
    if (m_editor) m_editor->setPointedBand(band);
    scheduleReveal();
    invalidate();
    emit viewChanged();
}

// Once the pointer rests on a band, its row scrolls into view; a sweep across
// the graph passes over bands too briefly to move the list
void PeqOverlay::scheduleReveal() {
    m_reveal.stop();
    if (m_hovered >= 0) m_reveal.start();
}

// ---- Band operations ---------------------------------------------------------------

int PeqOverlay::createBand(const FilterParams &p, bool select) {
    const int slot = freeSlot();
    if (slot < 0) {
        emit refused();
        return -1;
    }
    commitNow({ { slot, p } });
    if (select) {
        setSelection({ slot });
        m_rangeAnchor = slot;
    }
    m_ghost.reset();
    return slot;
}

void PeqOverlay::deleteBands(const QSet<int> &bands) {
    QHash<int, FilterParams> changes;
    for (int b : bands)
        if (isBand(b)) changes.insert(b, Peq::flatBand());
    if (changes.isEmpty()) return;
    QSet<int> rest = selection();
    for (auto it = changes.cbegin(); it != changes.cend(); ++it) rest.remove(it.key());
    setSelection(rest);
    if (m_chipShown && changes.contains(m_chipBand)) hideChip();
    commitNow(changes);
    pinChipToSelection();
}

void PeqOverlay::toggleBypass(const QSet<int> &bands) {
    if (!m_bypassSupported || m_channel < 0) return;
    QList<int> list;
    for (int b : bands)
        if (isBand(b)) list << b;
    std::sort(list.begin(), list.end());
    if (list.isEmpty()) return;
    commitLive();
    const bool bypass = !m_shown[list.first()].bypass;
    m_animFrom.reset();
    const int ch = m_channel;
    writing([&] {
        for (int b : list) {
            m_committed[b].bypass = bypass;
            m_shown[b].bypass = bypass;
            m_editor->setBandBypassed(ch, b, bypass);
        }
    });
    refreshChip();
    invalidate();
}

// Changes shape and order, keeping frequency and carrying gain and Q over
// where the new shape uses them
void PeqOverlay::setShape(const QSet<int> &bands, int shape, int order) {
    QHash<int, FilterParams> changes;
    for (int b : bands) {
        if (!isBand(b)) continue;
        int type = Peq::typeFor(shape, order);
        if (type < 0) type = Peq::typeFor(shape, 2);
        if (type < 0 || !m_available.contains(type)) continue;
        const FilterParams was = current(b);
        FilterParams p = was;
        p.filter_type = uint8_t(type);
        // Leaving the Linkwitz Transform drops its fp back to 0 dB
        if (was.filter_type == FILTER_LINKWITZ_TRANSFORM || !Peq::usesGain(type)) p.gain = 0;
        if (Peq::usesQ(type) && !Peq::usesQ(was.filter_type)) p.q = shape == Peq::Bell || shape == Peq::Notch ? 1.0f : 0.707f;
        changes.insert(b, p);
    }
    commitNow(changes);
}

void PeqOverlay::invertGain(const QSet<int> &bands) {
    QHash<int, FilterParams> changes;
    for (int b : bands) {
        if (!isBand(b) || !Peq::usesGain(current(b).filter_type)) continue;
        FilterParams p = current(b);
        p.gain = -p.gain;
        changes.insert(b, p);
    }
    commitNow(changes);
}

// ---- Pointer ------------------------------------------------------------------

void PeqOverlay::hoverAt(const QPointF &p) { pointerMoved(p); }

void PeqOverlay::hoverLeft() {
    m_pointer.reset();
    if (m_gesture.kind != Gesture::Idle) return;
    setHovered(-1);
    m_ghost.reset();
    hideReadouts();
    scheduleChipHide();
    unsetCursor();
    invalidate();
}

void PeqOverlay::pointerMoved(const QPointF &p) {
    if (!editing()) return;
    m_pointer = p;
    updateHover(p);
}

void PeqOverlay::updateHover(const QPointF &p) {
    if (m_gesture.kind != Gesture::Idle || !editing()) return;
    if (!boundingRect().contains(p)) {
        setHovered(-1);
        m_ghost.reset();
        hideReadouts();
        scheduleChipHide();
        invalidate();
        return;
    }
    // While the card is open the faint dot marks where its band will go, and
    // nothing else on the graph responds
    if (m_cardPoint) {
        setHovered(-1);
        m_ghost = creation(*m_cardPoint, false);
        hideReadouts();
        unsetCursor();
        invalidate();
        return;
    }
    const int dot = dotUnder(p);
    setHovered(dot >= 0 ? dot : fillUnder(p));
    // -- WP QPI2 -- The band bar under the graph (the owner's choice over the
    // Mac's chip at the dot) shows the selection: a hovered dot only lights.
    // A band a gesture showed without selecting it stays while the pointer is on it.
    const int pin = selectedChipBand();
    if (pin >= 0) {
        if (!m_chipShown || m_chipBand != pin) showChip(pin);
    } else if (dot >= 0 && m_chipShown && m_chipBand == dot) {
        m_chipHide.stop();
    } else if (m_chipShown && !m_chipEditing) {
        scheduleChipHide();
    }
    if (dot < 0) {
        if (freeSlot() >= 0) {
            m_ghost = creation(p, false);
            showReadouts(p);
        } else {
            m_ghost.reset();
            m_levelText.clear();
            m_axisText = tr("All %1 bands in use").arg(qMin(int(m_committed.size()), int(BANDS_PER_CHANNEL)));
            m_axisX = p.x();
            emit viewChanged();
        }
    } else {
        m_ghost.reset();
        hideReadouts();
    }
    if (dot >= 0) setCursor(Qt::OpenHandCursor);
    else unsetCursor();
    invalidate();
}

void PeqOverlay::showReadouts(const QPointF &p) {
    if (!m_graph) return;
    m_axisText = m_graph->showFrequencyReadout() ? Peq::frequencyText(freqAt(p.x())) : QString();
    m_axisX = p.x();
    // The pointer's level: the gain a double-clicked bell, or a bell or shelf
    // from the card, takes
    m_levelText = m_graph->showLevelReadout() ? Peq::gainText(Peq::clamp(dbAt(p.y()), Peq::kGainMin, Peq::kGainMax)) : QString();
    m_levelY = p.y();
    emit viewChanged();
}

void PeqOverlay::hideReadouts() {
    if (m_axisText.isEmpty() && m_levelText.isEmpty()) return;
    m_axisText.clear();
    m_levelText.clear();
    emit viewChanged();
}

void PeqOverlay::mousePressEvent(QMouseEvent *ev) {
    if (!editing()) {
        ev->ignore();
        return;
    }
    forceActiveFocus(Qt::MouseFocusReason);
    const QPointF p = ev->position();
    if (ev->button() == Qt::RightButton) {
        if (m_cardPoint) {
            closeCard();
            m_gesture = Gesture{};
            return;
        }
        closeShapePage();
        openMenu(p, dotUnder(p));
        return;
    }
    press(p, ev->modifiers(), false);
}

void PeqOverlay::mouseDoubleClickEvent(QMouseEvent *ev) {
    if (!editing()) {
        ev->ignore();
        return;
    }
    // Qt sends the second press, then this
    if (ev->button() == Qt::LeftButton) press(ev->position(), ev->modifiers(), true);
}

void PeqOverlay::press(const QPointF &p, Qt::KeyboardModifiers mods, bool doubleClick) {
    closeShapePage();
    // The card takes its own clicks; one anywhere else closes it and does
    // nothing to the graph
    if (m_cardPoint) {
        closeCard();
        m_gesture = Gesture{};
        updateHover(p);
        return;
    }
    if (doubleClick) {
        const int b = dotUnder(p);
        if (b >= 0) {
            showChip(b);
            emit editRequested(Peq::Freq);
        } else {
            createBand(creation(p, false));
        }
        m_gesture = Gesture{};
        return;
    }
    const int b = dotUnder(p);
    if (b >= 0) {
        // Modified presses wait for the release (a click) or movement (a
        // drag), so Alt and Shift can mean both
        if (!(mods & (kCommand | kOption | Qt::ShiftModifier)) && !selection().contains(b)) {
            setSelection({ b });
            m_rangeAnchor = b;
        }
        m_gesture = Gesture{};
        m_gesture.kind = Gesture::Press;
        m_gesture.band = b;
        m_gesture.at = p;
        m_gesture.mods = mods;
        // A modified press outside the selection leaves the chip on it
        const int pin = selectedChipBand();
        showChip(selection().isEmpty() || selection().contains(b) ? b : (pin >= 0 ? pin : b));
        return;
    }
    if (mods & kCommand) {
        openCard(p);
        m_gesture = Gesture{};
        return;
    }
    m_gesture = Gesture{};
    m_gesture.kind = Gesture::Background;
    m_gesture.at = p;
    m_gesture.mods = mods;
    m_gesture.onCurve = freeSlot() >= 0 && onCurveAt(p);
}

void PeqOverlay::mouseMoveEvent(QMouseEvent *ev) {
    if (!editing()) return;
    const QPointF p = ev->position();
    m_pointer = p;
    switch (m_gesture.kind) {
    case Gesture::Press: {
        if (distance(p, m_gesture.at) < kDragThreshold) return;
        const int b = m_gesture.band;
        const Qt::KeyboardModifiers mods = m_gesture.mods;
        QSet<int> bands = selection().contains(b) ? selection() : QSet<int>{ b };
        if (mods & kCommand) bands.insert(b);
        if (Peq::roleOf(current(b)).kind == Peq::Role::Locked) {
            m_gesture = Gesture{};
            return;
        }
        if ((mods & kCommand) && !selection().contains(b)) {
            setSelection(selection() | QSet<int>{ b });
        } else if (!selection().contains(b)) {
            setSelection({ b });
            m_rangeAnchor = b;
        }
        beginDrag(b, bands, m_gesture.at, mods & kCommand);
        continueDrag(p, ev->modifiers());
        break;
    }
    case Gesture::Background: {
        if (distance(p, m_gesture.at) < kDragThreshold) return;
        m_ghost.reset();
        hideReadouts();
        const int slot = freeSlot();
        if (m_gesture.onCurve && slot >= 0) {
            // Pull a new band out of the curve: flat at first, so the curve
            // does not jump, then it follows the pointer
            FilterParams band = creation(m_gesture.at, true);
            band.gain = 0;
            m_live.insert(slot, band);
            if (slot < m_shown.size()) m_shown[slot] = band;
            setSelection({ slot });
            m_rangeAnchor = slot;
            beginDrag(slot, { slot }, m_gesture.at, false);
            continueDrag(p, ev->modifiers());
        } else {
            // Ctrl opens the card, so only Shift adds to the selection here
            const QSet<int> base = (m_gesture.mods & Qt::ShiftModifier) ? selection() : QSet<int>{};
            const QPointF from = m_gesture.at;
            m_gesture = Gesture{};
            m_gesture.kind = Gesture::Marquee;
            m_gesture.at = from;
            m_gesture.to = p;
            m_gesture.base = base;
            updateMarquee();
            emit viewChanged();
        }
        break;
    }
    case Gesture::Drag: continueDrag(p, ev->modifiers()); break;
    case Gesture::Marquee:
        m_gesture.to = p;
        updateMarquee();
        break;
    case Gesture::Idle: break;
    }
}

void PeqOverlay::mouseReleaseEvent(QMouseEvent *ev) {
    if (!editing() || ev->button() != Qt::LeftButton) return;
    const QPointF p = ev->position();
    const Gesture::Kind kind = m_gesture.kind;
    switch (kind) {
    case Gesture::Press: {
        // Released without moving: a click
        const int b = m_gesture.band;
        const Qt::KeyboardModifiers mods = m_gesture.mods;
        if (mods & kOption) {
            toggleBypass(targets(b));
        } else if (mods & kCommand) {
            QSet<int> next = selection();
            if (!next.remove(b)) next.insert(b);
            setSelection(next);
            m_rangeAnchor = b;
        } else if (mods & Qt::ShiftModifier) {
            const QList<int> order = frequencyOrder(graphBands());
            const int i = order.indexOf(m_rangeAnchor), j = order.indexOf(b);
            if (i >= 0 && j >= 0) setSelection(QSet<int>(order.cbegin() + qMin(i, j), order.cbegin() + qMax(i, j) + 1));
            else {
                setSelection({ b });
                m_rangeAnchor = b;
            }
        } else {
            setSelection({ b });
            m_rangeAnchor = b;
        }
        break;
    }
    case Gesture::Background:
        // A click on empty graph only deselects, and dismisses the chip at once
        setSelection({});
        hideChip();
        break;
    case Gesture::Drag:
        m_gesture = Gesture{};
        commitLive();
        setCursor(Qt::OpenHandCursor);
        break;
    case Gesture::Marquee: break;
    case Gesture::Idle: break;
    }
    m_gesture = Gesture{};
    if (kind == Gesture::Marquee) emit viewChanged();
    invalidate();
    updateHover(p);
}

void PeqOverlay::beginDrag(int grabbed, const QSet<int> &bands, const QPointF &at, bool qMode) {
    m_commit.stop();
    Gesture g;
    g.kind = Gesture::Drag;
    g.grabbed = grabbed;
    for (int b : bands)
        if (isBand(b) || m_live.contains(b)) g.start.insert(b, current(b));
    g.start.insert(grabbed, current(grabbed));
    g.last = at;
    g.qMode = qMode;
    m_gesture = g;
    m_ghost.reset();
    hideReadouts();
    showChip(grabbed);
    setCursor(qMode ? Qt::SizeVerCursor : Qt::ClosedHandCursor);
}

// Alt locks the drag to one axis whenever it is held, never making the band
// jump: movement on the other axis is just not applied. Engaging mid-drag
// takes the axis moved along most; with too little movement to tell, the next
// few points decide.
void PeqOverlay::continueDrag(const QPointF &p, Qt::KeyboardModifiers mods) {
    if (m_gesture.kind != Gesture::Drag) return;
    Gesture &g = m_gesture;
    const double scale = (mods & Qt::ShiftModifier) ? kFine : 1.0;
    const QPointF move = (p - g.last) * scale;
    g.last = p;
    auto dominant = [](const QPointF &v) { return std::abs(v.x()) >= std::abs(v.y()) ? 1 : 2; };
    if (!(mods & kOption)) {
        g.axis = 0;
        g.pending = false;
        g.offset += move;
    } else {
        if (g.axis == 0 && !g.pending) {
            if (std::hypot(g.offset.x(), g.offset.y()) >= 4) g.axis = dominant(g.offset);
            else {
                g.pending = true;
                g.gathered = {};
            }
        }
        if (g.pending) {
            g.gathered += move;
            if (std::hypot(g.gathered.x(), g.gathered.y()) >= 4) {
                g.axis = dominant(g.gathered);
                g.pending = false;
                if (g.axis == 1) g.offset.rx() += g.gathered.x();
                else g.offset.ry() += g.gathered.y();
            }
        } else if (g.axis == 1) {
            g.offset.rx() += move.x();
        } else {
            g.offset.ry() += move.y();
        }
    }
    setLive(dragResult(g));
}

// The grabbed dot follows the pointer; the others move by the same frequency
// ratio and the same dB
QHash<int, FilterParams> PeqOverlay::dragResult(const Gesture &g) const {
    QHash<int, FilterParams> out;
    if (!g.start.contains(g.grabbed)) return out;
    const FilterParams g0 = g.start.value(g.grabbed);
    const double dx = g.offset.x(), dy = g.offset.y();
    if (g.qMode) {
        const double factor = std::pow(2.0, -dy / kQPointsPerOctave);
        for (auto it = g.start.cbegin(); it != g.start.cend(); ++it) {
            if (!Peq::usesQ(it->filter_type)) continue;
            FilterParams p = it.value();
            p.q = float(Peq::clamp(p.q * factor, Peq::kQMin, Peq::kQMax));
            out.insert(it.key(), p);
        }
        return out;
    }
    const double newFreq = Peq::clamp(freqAt(xOf(g0.freq) + dx), freqLo(), freqHi());
    const double ratio = newFreq / qMax(double(g0.freq), 1.0);
    const double dbDelta = -dy * dbPerPoint();
    const Peq::Role r0 = Peq::roleOf(g0);
    FilterParams grabbed = g0;
    grabbed.freq = float(newFreq);
    switch (r0.kind) {
    case Peq::Role::Gain:
        grabbed.gain = float(Peq::clamp(g0.gain + dbDelta / r0.value, Peq::kGainMin, Peq::kGainMax));
        break;
    case Peq::Role::Resonance:
        grabbed.q = float(Peq::clamp(std::pow(10.0, (Peq::dotDb(r0, g0) + dbDelta) / 20), Peq::kQMin, Peq::kQMax));
        break;
    case Peq::Role::Fixed:
        // A dot at a fixed level cannot follow vertically: frequency only
        break;
    case Peq::Role::Locked: return out;
    }
    out.insert(g.grabbed, grabbed);
    // The others take what the grabbed band's gain actually moved (after its
    // clamp); a grabbed band without gain passes on the pointer's travel
    const bool gainMoved = r0.kind == Peq::Role::Gain;
    const double gainOffset = double(grabbed.gain - g0.gain);
    for (auto it = g.start.cbegin(); it != g.start.cend(); ++it) {
        if (it.key() == g.grabbed) continue;
        FilterParams p = it.value();
        const Peq::Role r = Peq::roleOf(p);
        if (r.kind == Peq::Role::Locked) continue;
        p.freq = float(Peq::clamp(p.freq * ratio, freqLo(), freqHi()));
        if (Peq::usesGain(p.filter_type)) {
            const double scaleOf = r.kind == Peq::Role::Gain ? qMax(r.value, 0.05) : 1.0;
            p.gain = float(Peq::clamp(p.gain + (gainMoved ? gainOffset : dbDelta / scaleOf), Peq::kGainMin, Peq::kGainMax));
        }
        out.insert(it.key(), p);
    }
    return out;
}

void PeqOverlay::updateMarquee() {
    if (m_gesture.kind != Gesture::Marquee) return;
    const QRectF rect = QRectF(m_gesture.at, m_gesture.to).normalized();
    QList<int> inside;
    for (int b : graphBands())
        if (rect.contains(dotOf(m_shown[b]))) inside << b;
    setSelection(m_gesture.base | QSet<int>(inside.cbegin(), inside.cend()));
    if (!inside.isEmpty()) m_rangeAnchor = frequencyOrder(inside).first();
    rebuild();
}

void PeqOverlay::wheelEvent(QWheelEvent *ev) {
    const QPointF p = ev->position();
    const bool precise = !ev->pixelDelta().isNull();
    const QPoint raw = precise ? ev->pixelDelta() : ev->angleDelta();
    // Shift turns a wheel sideways on some systems: take whichever moved
    const double step = raw.y() != 0 ? raw.y() : raw.x();
    const double delta = precise ? step : step / 120.0 * 8.0;
    const bool begins = ev->phase() == Qt::ScrollBegin;
    if (begins) m_chipWheelField = -1;
    if (editing() && handleWheel(p, delta, ev->modifiers(), begins)) {
        ev->accept();
        return;
    }
    if (p.x() <= kZoomZone && m_graph && delta != 0) {
        // The dB zoom strip at the left edge
        const double notches = precise ? step * 0.3 : step / 120.0 * 3.0;
        m_graph->setDbRange(qBound(GraphController::kRangeMin, m_graph->dbRange() - notches, GraphController::kRangeMax));
        ev->accept();
        return;
    }
    ev->ignore();
}

// One wheel step; false when no band takes it. A gesture stays with the band
// it began on, and a selection owns the wheel: drifting over another band
// cannot switch to it.
bool PeqOverlay::handleWheel(const QPointF &p, double rawDelta, Qt::KeyboardModifiers mods, bool begins) {
    if (!editing()) return false;
    const double t = now();
    const int owner = selectedChipBand();
    int target = -1;
    if (m_gesture.kind == Gesture::Drag) {
        target = m_gesture.grabbed;
    } else if (!begins && isBand(m_wheelTarget) && t - m_wheelTime < kWheelSession && (owner < 0 || selection().contains(m_wheelTarget))) {
        target = m_wheelTarget;
    } else {
        const int under = bandUnder(p);
        if (under >= 0) target = owner >= 0 ? owner : under;
    }
    if (target < 0) {
        m_wheelTarget = -1;
        return false;
    }
    m_wheelTarget = target;
    m_wheelTime = t;
    m_chipWheelField = -1;
    const double delta = rawDelta * ((mods & Qt::ShiftModifier) ? kFine : 1.0);
    if (delta == 0) return true;

    // One transformation of a band, applied to the live band and, mid-drag,
    // to the drag's starting point too, which every movement rebuilds from
    const bool gain = mods & kCommand;
    auto transform = [gain, delta](const FilterParams &p) -> std::optional<FilterParams> {
        FilterParams q = p;
        if (gain) {
            if (!Peq::usesGain(p.filter_type)) return std::nullopt;
            q.gain = float(Peq::clamp(p.gain + delta * 0.05, Peq::kGainMin, Peq::kGainMax));
        } else {
            if (!Peq::usesQ(p.filter_type)) return std::nullopt;
            q.q = float(Peq::clamp(p.q * std::pow(2.0, delta / 100), Peq::kQMin, Peq::kQMax));
        }
        return q;
    };
    QHash<int, FilterParams> changes;
    for (int b : targets(target))
        if (const auto c = transform(current(b))) changes.insert(b, *c);
    if (m_gesture.kind == Gesture::Drag) {
        for (auto it = m_gesture.start.begin(); it != m_gesture.start.end(); ++it)
            if (changes.contains(it.key()))
                if (const auto c = transform(it.value())) it.value() = *c;
        setLive(changes);
    } else {
        setLiveThenCommit(changes);
    }
    if (m_hovered < 0) setHovered(target);
    showChip(target);
    return true;
}

void PeqOverlay::openMenu(const QPointF &p, int band) {
    if (band >= 0) {
        if (!selection().contains(band)) {
            setSelection({ band });
            m_rangeAnchor = band;
        }
        m_menuTargets = targets(band);
    } else {
        m_menuTargets = selection();
    }
    m_menuBand = band;
    emit menuRequested(p.x(), p.y());
}

QVariantMap PeqOverlay::menuModel() const {
    QVariantMap m;
    if (m_menuBand >= 0 && isBand(m_menuBand)) {
        const int n = int(m_menuTargets.size());
        const FilterParams p = current(m_menuBand);
        m[QStringLiteral("kind")] = QStringLiteral("band");
        m[QStringLiteral("title")] = n > 1 ? tr("%1 Bands").arg(n) : tr("Band %1").arg(m_menuBand + 1);
        // Shape and typed values are the chip's; the menu offers the slope
        int shape = -1, order = 0;
        QVariantList orders;
        if (Peq::shapeOf(p.filter_type, &shape, &order)) {
            const int t1 = Peq::typeFor(shape, 1), t2 = Peq::typeFor(shape, 2);
            if (t1 >= 0 && t2 >= 0 && m_available.contains(t1) && m_available.contains(t2))
                for (int o : { 1, 2 })
                    orders << QVariantMap{ { QStringLiteral("title"), Peq::orderTitle(shape, o) },
                                           { QStringLiteral("order"), o },
                                           { QStringLiteral("checked"), o == order } };
        }
        m[QStringLiteral("slopeTitle")] = shape == Peq::AllPass ? tr("Order") : tr("Slope");
        m[QStringLiteral("orders")] = orders;
        m[QStringLiteral("bypass")] = m_bypassSupported ? (p.bypass ? tr("Enable") : tr("Bypass")) : QString();
        m[QStringLiteral("invert")] = Peq::usesGain(p.filter_type);
        m[QStringLiteral("delete")] = n > 1 ? tr("Delete %1 Bands").arg(n) : tr("Delete Band");
    } else {
        const int n = int(selection().size());
        m[QStringLiteral("kind")] = QStringLiteral("graph");
        m[QStringLiteral("selectAll")] = !graphBands().isEmpty();
        m[QStringLiteral("deselect")] = n > 0;
        m[QStringLiteral("delete")] = n == 0 ? QString() : n > 1 ? tr("Delete %1 Selected Bands").arg(n) : tr("Delete Selected Band");
    }
    return m;
}

void PeqOverlay::menuAction(const QString &action, int argument) {
    if (!editing()) return;
    if (action == QLatin1String("order")) {
        int shape = -1;
        if (Peq::shapeOf(current(m_menuBand).filter_type, &shape, nullptr)) setShape(m_menuTargets, shape, argument);
    } else if (action == QLatin1String("bypass")) {
        toggleBypass(m_menuTargets);
    } else if (action == QLatin1String("invert")) {
        invertGain(m_menuTargets);
    } else if (action == QLatin1String("delete")) {
        deleteBands(m_menuTargets);
    } else if (action == QLatin1String("selectAll")) {
        const QList<int> all = graphBands();
        setSelection(QSet<int>(all.cbegin(), all.cend()));
    } else if (action == QLatin1String("deselect")) {
        setSelection({});
    } else if (action == QLatin1String("deleteSelected")) {
        deleteBands(selection());
    }
}

// ---- Keys ------------------------------------------------------------------------

bool PeqOverlay::takesKey(const QKeyEvent *ev) const {
    if (!editing()) return false;
    const Qt::KeyboardModifiers mods = ev->modifiers() & (kCommand | kOption | Qt::ShiftModifier | Qt::MetaModifier);
    switch (ev->key()) {
    case Qt::Key_Delete:
    case Qt::Key_Backspace:
    case Qt::Key_Escape:
    case Qt::Key_Left:
    case Qt::Key_Right:
    case Qt::Key_Up:
    case Qt::Key_Down:
    case Qt::Key_Return:
    case Qt::Key_Enter:
    case Qt::Key_Menu: return true;
    case Qt::Key_Tab:
    case Qt::Key_Backtab: return !graphBands().isEmpty();
    case Qt::Key_F10: return mods == Qt::ShiftModifier;
    case Qt::Key_A: return mods == kCommand;
    default: return false;
    }
}

bool PeqOverlay::event(QEvent *ev) {
    // The editor's keys win over the menus' shortcuts (Delete, Ctrl+A) while
    // the graph has the keyboard
    if (ev->type() == QEvent::ShortcutOverride && takesKey(static_cast<QKeyEvent *>(ev))) {
        ev->accept();
        return true;
    }
    return QQuickPaintedItem::event(ev);
}

void PeqOverlay::keyPressEvent(QKeyEvent *ev) {
    if (!takesKey(ev)) {
        ev->ignore();
        return;
    }
    const Qt::KeyboardModifiers mods = ev->modifiers();
    switch (ev->key()) {
    case Qt::Key_Delete:
    case Qt::Key_Backspace: deleteBands(selection()); break;
    case Qt::Key_Escape:
        if (m_cardPoint) {
            closeCard();
            break;
        }
        if (m_chipShown && m_shapePage) {
            closeShapePage();
            break;
        }
        setSelection({});
        hideChip();
        break;
    case Qt::Key_Tab: stepSelection(mods & Qt::ShiftModifier); break;
    case Qt::Key_Backtab: stepSelection(true); break;
    case Qt::Key_Left:
    case Qt::Key_Right:
    case Qt::Key_Up:
    case Qt::Key_Down: nudge(ev->key(), mods); break;
    case Qt::Key_Return:
    case Qt::Key_Enter: {
        // Type the values of the band the chip stays on, as a double-click does
        const int b = selectedChipBand();
        if (b < 0) break;
        showChip(b);
        emit editRequested(Peq::Freq);
        break;
    }
    case Qt::Key_F10:
    case Qt::Key_Menu: {
        const int b = selectedChipBand();
        const QPointF at = b >= 0 ? dotOf(current(b)) : m_pointer.value_or(QPointF(width() / 2, height() / 2));
        openMenu(at, b);
        break;
    }
    case Qt::Key_A: {
        const QList<int> all = graphBands();
        setSelection(QSet<int>(all.cbegin(), all.cend()));
        break;
    }
    default: break;
    }
    ev->accept();
}

void PeqOverlay::stepSelection(bool backwards) {
    const QList<int> order = frequencyOrder(graphBands());
    if (order.isEmpty()) return;
    const int from = order.indexOf(m_rangeAnchor);
    const int n = int(order.size());
    const int next = from >= 0 ? ((from + (backwards ? -1 : 1)) % n + n) % n : (backwards ? n - 1 : 0);
    setSelection({ order[next] });
    m_rangeAnchor = order[next];
    showChip(order[next]);
}

void PeqOverlay::nudge(int key, Qt::KeyboardModifiers mods) {
    const bool fine = mods & Qt::ShiftModifier;
    QHash<int, FilterParams> changes;
    for (int b : selection()) {
        if (!isBand(b)) continue;
        FilterParams p = current(b);
        if (key == Qt::Key_Left || key == Qt::Key_Right) {
            const double octaves = (fine ? 1.0 / 96 : 1.0 / 12) * (key == Qt::Key_Right ? 1 : -1);
            p.freq = float(Peq::clamp(p.freq * std::pow(2.0, octaves), freqLo(), freqHi()));
        } else {
            const double sign = key == Qt::Key_Up ? 1 : -1;
            if ((mods & kOption) || !Peq::usesGain(p.filter_type)) {
                if (!Peq::usesQ(p.filter_type)) continue;
                p.q = float(Peq::clamp(p.q * std::pow(2.0, (fine ? 0.02 : 0.1) * sign), Peq::kQMin, Peq::kQMax));
            } else {
                p.gain = float(Peq::clamp(p.gain + (fine ? 0.1 : 0.5) * sign, Peq::kGainMin, Peq::kGainMax));
            }
        }
        changes.insert(b, p);
    }
    setLiveThenCommit(changes);
    if (selection().contains(m_rangeAnchor)) showChip(m_rangeAnchor);
}

// ---- The chip ----------------------------------------------------------------------

void PeqOverlay::showChip(int b) {
    if (!isBand(b)) return;
    m_chipHide.stop();
    if (m_chipBand != b) closeShapePage();
    const bool changed = m_chipBand != b || !m_chipShown;
    m_chipBand = b;
    m_chipShown = true;
    refreshChip();
    invalidate();
    if (changed) emit viewChanged();
}

void PeqOverlay::refreshChip() {
    if (m_chipBand < 0) return;
    if (!isBand(m_chipBand)) {
        if (m_chipShown) hideChip();
        return;
    }
    const FilterParams p = current(m_chipBand);
    QVariantMap info;
    auto row = [](int field, const QString &label, const QString &number, const QString &unit, bool adjustable,
                  bool inlineUnit = false) {
        return QVariantMap{ { QStringLiteral("field"), field },        { QStringLiteral("label"), label },
                            { QStringLiteral("number"), number },      { QStringLiteral("unit"), unit },
                            { QStringLiteral("adjustable"), adjustable }, { QStringLiteral("inline"), inlineUnit } };
    };
    auto freqParts = [](double hz) {
        return hz >= 1000 ? qMakePair(Peq::truncated(hz / 1000), tr("kHz")) : qMakePair(Peq::truncated(hz), tr("Hz"));
    };
    QVariantList rows;
    int shape = -1, order = 0;
    const bool reshapable = Peq::shapeOf(p.filter_type, &shape, &order);
    info[QStringLiteral("band")] = m_chipBand;
    info[QStringLiteral("color")] = m_bandColors.value(m_chipBand % qMax(int(m_bandColors.size()), 1), m_ink);
    info[QStringLiteral("bypass")] = p.bypass;
    info[QStringLiteral("bypassSupported")] = m_bypassSupported;
    info[QStringLiteral("powerTip")] = p.bypass ? tr("Enable band (Alt-click the dot)") : tr("Bypass band (Alt-click the dot)");
    info[QStringLiteral("reshapable")] = reshapable;
    info[QStringLiteral("shape")] = shape;
    info[QStringLiteral("order")] = order;
    if (reshapable) {
        info[QStringLiteral("code")] = Peq::shapeCode(shape);
        info[QStringLiteral("shapeTip")] = tr("%1 - click for shapes and slopes").arg(Peq::shapeTitle(shape));
        const auto f = freqParts(p.freq);
        rows << row(Peq::Freq, tr("Freq"), f.first, f.second, true);
        if (Peq::usesGain(p.filter_type)) rows << row(Peq::Gain, tr("Gain"), Peq::truncated(p.gain, true), tr("dB"), true);
        if (Peq::usesQ(p.filter_type))
            // "Width", as the band list calls it, with Q as its unit
            rows << row(Peq::Width, tr("Width"), Peq::truncated(p.q), tr("Q"), true);
        else if (shape == Peq::AllPass)
            rows << row(Peq::Width, tr("Order"), tr("1st"), QString(), false);
        else
            rows << row(Peq::Width, tr("Slope"), QStringLiteral("6"), tr("dB/oct"), false, true);
    } else {
        // The Linkwitz Transform: shown here, edited in the band list
        info[QStringLiteral("code")] = QStringLiteral("LT");
        info[QStringLiteral("shapeTip")] = tr("Linkwitz Transform - edit it in the band list");
        const auto f0 = freqParts(p.freq), fp = freqParts(p.gain);
        rows << row(Peq::Freq, tr("f0"), f0.first, f0.second, false);
        rows << row(Peq::Gain, tr("fp"), fp.first, fp.second, false);
    }
    // -- WP QPI2 -- for the bar: the slope a cut or all-pass can step to,
    // invert while the band has a gain
    info[QStringLiteral("invertible")] = Peq::usesGain(p.filter_type);
    info[QStringLiteral("deleteTip")] = tr("Delete Band");
    info[QStringLiteral("invertTip")] = tr("Invert Gain");
    info[QStringLiteral("rows")] = rows;
    m_chipInfo = info;
    emit viewChanged();
}

// Where a panel of `size` sits beside `point`: away from 0 dB first, so a chip
// never sits on its own lobe, then beside it, the lobe side last
QRectF PeqOverlay::panelFrame(const QPointF &point, const QSizeF &size, bool boost) const {
    const QRectF area = boundingRect().adjusted(4, 4, -4, -4);
    auto cx = [&](double x) { return qBound(area.left(), x, qMax(area.right() - size.width(), area.left())); };
    auto cy = [&](double y) { return qBound(area.top(), y, qMax(area.bottom() - size.height(), area.top())); };
    const QRectF above(QPointF(cx(point.x() - size.width() / 2), point.y() - kPanelGap - size.height()), size);
    const QRectF below(QPointF(cx(point.x() - size.width() / 2), point.y() + kPanelGap), size);
    const QRectF right(QPointF(point.x() + kPanelGap, cy(point.y() - size.height() / 2)), size);
    const QRectF left(QPointF(point.x() - kPanelGap - size.width(), cy(point.y() - size.height() / 2)), size);
    const QList<QRectF> candidates = boost ? QList<QRectF>{ above, right, left, below } : QList<QRectF>{ below, right, left, above };
    QRectF frame = candidates.first();
    for (const QRectF &c : candidates)
        if (area.contains(c)) {
            frame = c;
            break;
        }
    frame.moveTopLeft(QPointF(cx(frame.left()), cy(frame.top())));
    return frame;
}

// While bands are selected the chip stays on the selection; with none it
// follows the hovered dot
void PeqOverlay::pinChipToSelection() {
    const int pin = selectedChipBand();
    if (pin >= 0) {
        if (m_gesture.kind == Gesture::Marquee) return;
        if (!m_chipShown || m_chipBand != pin) showChip(pin);
    } else if (m_chipShown && (!m_pointer || dotUnder(*m_pointer) != m_chipBand)) {
        scheduleChipHide();
    }
}

void PeqOverlay::scheduleChipHide() {
    if (!m_chipShown || m_chipEditing || m_overChip) return;
    if (selection().contains(m_chipBand)) return;
    if (m_gesture.kind == Gesture::Drag) return;
    m_chipHide.start();
}

void PeqOverlay::hideChip() {
    m_chipHide.stop();
    closeShapePage();
    m_chipEditing = false;
    if (!m_chipShown) return;
    m_chipShown = false;
    m_overChip = false;
    invalidate();
    emit viewChanged();
}

void PeqOverlay::closeShapePage() {
    if (!m_shapePage) return;
    m_shapePage = false;
    emit viewChanged();
}

void PeqOverlay::chipToggleShapes() {
    if (!m_chipShown) return;
    if (m_shapePage) {
        closeShapePage();
        return;
    }
    if (!m_chipInfo.value(QStringLiteral("reshapable")).toBool()) return;
    m_shapePage = true;
    emit viewChanged();
}

void PeqOverlay::chipBypass() {
    if (m_chipShown) toggleBypass(targets(m_chipBand));
}

// -- WP QPI2 -- the bar's invert and delete, on the selection as the band menu's
void PeqOverlay::chipInvert() {
    if (m_chipShown && Peq::usesGain(current(m_chipBand).filter_type)) invertGain(targets(m_chipBand));
}

void PeqOverlay::chipDelete() {
    if (!m_chipShown) return;
    deleteBands(targets(m_chipBand));
    hideChip();
}

void PeqOverlay::chipPick(int shape, int order) {
    if (!m_chipShown) return;
    setShape(targets(m_chipBand), shape, order);
    closeShapePage();
}

void PeqOverlay::chipEditing(bool editing) {
    m_chipEditing = editing;
    if (!editing) scheduleChipHide();
}

void PeqOverlay::chipHovered(bool inside) {
    m_overChip = inside;
    if (inside) {
        m_chipHide.stop();
        m_ghost.reset();
        hideReadouts();
        invalidate();
    } else if (!m_pointer || dotUnder(*m_pointer) != m_chipBand) {
        scheduleChipHide();
    }
}

QString PeqOverlay::chipEditText(int field) const {
    if (m_chipBand < 0) return {};
    // The bare number: the unit is implied, and "2k" or "A4" work
    const FilterParams p = current(m_chipBand);
    switch (field) {
    case Peq::Freq: return Peq::shortFrequency(p.freq);
    case Peq::Gain: return QString::number(p.gain, 'f', 2);
    default: return QString::number(p.q, 'f', 3);
    }
}

bool PeqOverlay::chipText(int field, const QString &text) {
    if (!m_chipShown || !isBand(m_chipBand)) return false;
    FilterParams p = current(m_chipBand);
    double v = 0;
    switch (field) {
    case Peq::Freq:
        if (!Peq::parseFrequency(text, &v)) return false;
        p.freq = float(Peq::clamp(v, Peq::kFreqMin, Peq::kFreqMax));
        break;
    case Peq::Gain:
        if (!Peq::usesGain(p.filter_type) || !Peq::parseNumber(text, QStringLiteral("db"), &v)) return false;
        p.gain = float(Peq::clamp(v, Peq::kGainMin, Peq::kGainMax));
        break;
    default:
        if (!Peq::usesQ(p.filter_type) || !Peq::parseNumber(text, QString(), &v)) return false;
        p.q = float(Peq::clamp(v, Peq::kQMin, Peq::kQMax));
        break;
    }
    if (!Peq::same(p, current(m_chipBand))) commitNow({ { m_chipBand, p } });
    return true;
}

std::optional<FilterParams> PeqOverlay::adjusted(const FilterParams &p, int field, double delta) const {
    FilterParams q = p;
    switch (field) {
    case Peq::Freq: q.freq = float(Peq::clamp(p.freq * std::pow(2.0, delta / 100), freqLo(), freqHi())); break;
    case Peq::Gain:
        if (!Peq::usesGain(p.filter_type)) return std::nullopt;
        q.gain = float(Peq::clamp(p.gain + delta * 0.1, Peq::kGainMin, Peq::kGainMax));
        break;
    default:
        if (!Peq::usesQ(p.filter_type)) return std::nullopt;
        q.q = float(Peq::clamp(p.q * std::pow(2.0, delta / 80), Peq::kQMin, Peq::kQMax));
        break;
    }
    return q;
}

void PeqOverlay::chipAdjust(int field, double delta, int phase) {
    if (!m_chipShown) return;
    if (phase == 0) {
        m_commit.stop();
        m_chipAdjustStart = current(m_chipBand);
    } else if (phase == 1) {
        if (m_chipAdjustStart)
            if (const auto p = adjusted(*m_chipAdjustStart, field, delta)) setLive({ { m_chipBand, *p } });
    } else {
        m_chipAdjustStart.reset();
        commitLive();
    }
}

// A scroll gesture on a chip field keeps that field, even when the dot (and
// the chip with it) moves another row under the pointer
void PeqOverlay::chipScroll(int field, double delta, bool fine) {
    if (!m_chipShown) return;
    const int locked = chipWheelActive() && m_chipWheelField >= 0 ? m_chipWheelField : field;
    m_chipWheelField = locked;
    m_chipWheelTime = now();
    m_wheelTarget = -1;
    const double d = delta * (fine ? kFine : 1.0);
    if (const auto p = adjusted(current(m_chipBand), locked, d * 0.5)) setLiveThenCommit({ { m_chipBand, *p } });
}

bool PeqOverlay::graphWheelActive() const { return m_wheelTarget >= 0 && now() - m_wheelTime < kWheelSession; }

bool PeqOverlay::chipWheelActive() const {
    return m_chipWheelField >= 0 && m_chipShown && now() - m_chipWheelTime < kWheelSession;
}

// ---- The card --------------------------------------------------------------------------

void PeqOverlay::openCard(const QPointF &p) {
    if (freeSlot() < 0) {
        // Every band is in use: nothing to create
        emit refused();
        return;
    }
    closeShapePage();
    m_cardPoint = p;
    m_cardRect = panelFrame(p, QSizeF(kChipWidth, kChipFirstRow + 3 * kChipRow + kChipBottom), dbAt(p.y()) >= 0);
    emit viewChanged();
    updateHover(p);
}

void PeqOverlay::closeCard() {
    if (!m_cardPoint) return;
    m_cardPoint.reset();
    m_ghost.reset();
    emit viewChanged();
    invalidate();
}

// The new band is selected and its chip opens as the card closes, so the
// choice turns straight into the band's values
void PeqOverlay::cardPick(int shape, int order) {
    if (!m_cardPoint) return;
    const QPointF at = *m_cardPoint;
    const auto band = shaped(shape, order, at);
    closeCard();
    if (!band) return;
    const int slot = createBand(*band);
    if (slot >= 0) showChip(slot);
}

QString PeqOverlay::orderLabel(int shape, int order) const { return Peq::orderLabel(shape, order); }
QString PeqOverlay::orderHelp(int shape, int order) const { return Peq::orderHelp(shape, order); }

QVariantList PeqOverlay::shapes() const {
    QVariantList out;
    for (int s = 0; s < Peq::ShapeCount; ++s) {
        QVariantList orders;
        for (int o : { 1, 2 }) {
            const int t = Peq::typeFor(s, o);
            if (t >= 0 && m_available.contains(t)) orders << o;
        }
        if (orders.isEmpty()) continue;
        out << QVariantMap{ { QStringLiteral("shape"), s }, { QStringLiteral("title"), Peq::shapeTitle(s) }, { QStringLiteral("orders"), orders } };
    }
    return out;
}

// ---- For tests ------------------------------------------------------------------------

QPointF PeqOverlay::dotPoint(int band) const { return isBand(band) ? dotOf(m_shown[band]) : QPointF(-1, -1); }

QVariantMap PeqOverlay::shownBand(int band) const {
    if (band < 0 || band >= m_shown.size()) return {};
    const FilterParams &p = m_shown[band];
    return { { QStringLiteral("type"), int(p.filter_type) }, { QStringLiteral("freq"), double(p.freq) },
             { QStringLiteral("q"), double(p.q) },          { QStringLiteral("gain"), double(p.gain) },
             { QStringLiteral("bypass"), p.bypass } };
}

double PeqOverlay::curveDbAt(double hz) const { return combinedDb(hz); }

// ---- Shape glyphs ------------------------------------------------------------------------

PeqShapeGlyph::PeqShapeGlyph(QQuickItem *parentItem) : QQuickPaintedItem(parentItem) {
    setImplicitSize(18, 12);
    setAntialiasing(true);
}

void PeqShapeGlyph::setShape(int s) {
    if (s == m_shape) return;
    m_shape = s;
    emit glyphChanged();
    update();
}

void PeqShapeGlyph::setInk(const QColor &value) {
    if (value == m_ink) return;
    m_ink = value;
    emit glyphChanged();
    update();
}

void PeqShapeGlyph::paint(QPainter *p) {
    // Drawn on the Mac's 18 x 12 grid, scaled to the item
    p->setRenderHint(QPainter::Antialiasing);
    p->scale(width() / 18.0, height() / 12.0);
    QPainterPath line;
    const double mid = 7;
    switch (m_shape) {
    case Peq::Bell:
        line.moveTo(1, 9);
        line.cubicTo(5, 9, 6.5, 2, 9, 2);
        line.cubicTo(11.5, 2, 13, 9, 17, 9);
        break;
    case Peq::LowShelf:
        line.moveTo(1, 3);
        line.lineTo(6, 3);
        line.cubicTo(9, 3, 9, 9, 12, 9);
        line.lineTo(17, 9);
        break;
    case Peq::HighShelf:
        line.moveTo(1, 9);
        line.lineTo(6, 9);
        line.cubicTo(9, 9, 9, 3, 12, 3);
        line.lineTo(17, 3);
        break;
    case Peq::LowCut:
        line.moveTo(3, 11);
        line.cubicTo(5, 5, 7, 4, 10, 4);
        line.lineTo(17, 4);
        break;
    case Peq::HighCut:
        line.moveTo(1, 4);
        line.lineTo(8, 4);
        line.cubicTo(11, 4, 13, 5, 15, 11);
        break;
    case Peq::Notch:
        line.moveTo(1, 3);
        line.lineTo(6.5, 3);
        line.cubicTo(8.2, 3, 8.6, 11, 9, 11);
        line.cubicTo(9.4, 11, 9.8, 3, 11.5, 3);
        line.lineTo(17, 3);
        break;
    default:
        line.moveTo(1, mid);
        line.lineTo(5, mid);
        line.cubicTo(8, 0, 10, 14, 13, mid);
        line.lineTo(17, mid);
        break;
    }
    p->setPen(QPen(m_ink, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    p->setBrush(Qt::NoBrush);
    p->drawPath(line);
}
