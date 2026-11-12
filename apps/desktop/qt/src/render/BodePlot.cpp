// -- WP QP3 -- The response graph's drawing; BodePlot.h says what it draws.
#include "BodePlot.h"

#include "GraphController.h"

#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <QtMath>
#include <algorithm>
#include <cmath>
#include <map>

namespace {

// The core's curve grid: MAGNITUDE_POINTS points from 10 Hz to 20 kHz, evenly
// spaced in log frequency (dspi_compute_magnitude_curve)
constexpr double kGridLowHz = 10.0;
constexpr double kGridHighHz = 20000.0;
constexpr int kPhaseKey = -1;
constexpr int kMoveMs = 200;

// Grid lines and labels, as the Mac's graph places them
const double kMajorHz[] = { 100, 1000, 10000 };
const double kMinorHz[] = { 20,   30,   40,   50,   60,   70,   80,   90,   200,  300,  400,  500,  600,
                            700,  800,  900,  2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 20000 };
const struct {
    double hz;
    const char *text;
} kFreqLabels[] = { { 20, "20" },   { 50, "50" },   { 100, "100" }, { 200, "200" }, { 500, "500" },
                    { 1000, "1k" }, { 2000, "2k" }, { 5000, "5k" }, { 10000, "10k" }, { 20000, "20k" } };

// A pen of `width` px dashed in px (QPen counts dashes in pen widths)
QPen dashed(const QBrush &brush, double width, std::initializer_list<double> px, Qt::PenCapStyle cap = Qt::FlatCap) {
    QPen pen(brush, width, Qt::CustomDashLine, cap, Qt::RoundJoin);
    QList<qreal> pattern;
    for (double v : px) pattern.append(v / width);
    pen.setDashPattern(pattern);
    return pen;
}

// The gradient of a line several channels share: a solid stretch per channel,
// blended where they meet (the Mac's gradientStops)
QGradientStops shareStops(const QVector<QColor> &colors) {
    QGradientStops stops;
    const int n = int(colors.size());
    if (n == 0) return stops;
    if (n == 1) return { { 0.0, colors[0] }, { 1.0, colors[0] } };
    const double blend = n == 2 ? 0.4 : n == 3 ? 0.25 : 0.15;
    const double segment = 1.0 / n;
    for (int i = 0; i < n; ++i) {
        const double centre = (i + 0.5) * segment;
        const double half = (segment - blend) / 2;
        if (i == 0) stops.append({ 0.0, colors[i] });
        stops.append({ qMax(0.0, centre - half), colors[i] });
        stops.append({ qMin(1.0, centre + half), colors[i] });
        if (i == n - 1) stops.append({ 1.0, colors[i] });
    }
    std::stable_sort(stops.begin(), stops.end(), [](const QGradientStop &a, const QGradientStop &b) { return a.first < b.first; });
    return stops;
}

QColor withAlpha(QColor c, double alpha) {
    c.setAlphaF(float(qBound(0.0, c.alphaF() * alpha, 1.0)));
    return c;
}

} // namespace

BodePlot::BodePlot(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAntialiasing(true);
    setOpaquePainting(false);
    m_timer.setInterval(16);
    connect(&m_timer, &QTimer::timeout, this, &BodePlot::tick);
    connect(this, &QQuickItem::widthChanged, this, [this]() { update(); });
    connect(this, &QQuickItem::heightChanged, this, [this]() { update(); });
}

void BodePlot::setGraph(GraphController *graph) {
    if (graph == m_graph) return;
    if (m_graph) disconnect(m_graph, nullptr, this, nullptr);
    m_graph = graph;
    if (m_graph) {
        connect(m_graph, &GraphController::curvesChanged, this, &BodePlot::retarget);
        connect(m_graph, &GraphController::overlayChannelChanged, this, &BodePlot::retarget);
        connect(m_graph, &GraphController::preferencesChanged, this, &BodePlot::retarget);
    }
    emit graphChanged();
    retarget();
}

void BodePlot::setLayers(int layers) {
    if (layers == m_layers) return;
    m_layers = layers;
    emit layersChanged();
    retarget();
}

void BodePlot::setChannels(const QList<int> &channels) {
    if (channels == m_channels) return;
    m_channels = channels;
    emit channelsChanged();
    retarget();
}

void BodePlot::setActiveChannel(int channel) {
    if (channel == m_active) return;
    m_active = channel;
    emit channelsChanged();
    retarget();
}

void BodePlot::setFollowsSelection(bool on) {
    if (on == m_follows) return;
    m_follows = on;
    emit channelsChanged();
    retarget();
}

void BodePlot::setExcludeOverlay(bool on) {
    if (on == m_excludeOverlay) return;
    m_excludeOverlay = on;
    emit channelsChanged();
    retarget();
}

void BodePlot::setChannelColors(const QVariantList &colors) {
    m_colorList = colors;
    m_colors.clear();
    for (const QVariant &v : colors) m_colors.append(v.value<QColor>());
    emit styleChanged();
    update();
}

void BodePlot::setGridColor(const QColor &c) {
    if (c == m_gridColor) return;
    m_gridColor = c;
    emit styleChanged();
    update();
}

void BodePlot::setLabelColor(const QColor &c) {
    if (c == m_labelColor) return;
    m_labelColor = c;
    emit styleChanged();
    update();
}

void BodePlot::setPhaseColor(const QColor &c) {
    if (c == m_phaseColor) return;
    m_phaseColor = c;
    emit styleChanged();
    update();
}

void BodePlot::setLabelFont(const QFont &f) {
    if (f == m_font) return;
    m_font = f;
    emit styleChanged();
    update();
}

void BodePlot::setAnimated(bool on) {
    if (on == m_animated) return;
    m_animated = on;
    emit styleChanged();
}

// ---- Mapping -------------------------------------------------------------------

double BodePlot::xForFrequency(double hz) const {
    if (!m_graph || hz <= 0) return 0;
    const double lo = std::log10(m_graph->minFreq()), hi = std::log10(m_graph->maxFreq());
    return (std::log10(hz) - lo) / (hi - lo) * width();
}

double BodePlot::frequencyForX(double x) const {
    if (!m_graph || width() <= 0) return 0;
    const double lo = std::log10(m_graph->minFreq()), hi = std::log10(m_graph->maxFreq());
    return std::pow(10.0, lo + x / width() * (hi - lo));
}

double BodePlot::yForDb(double db) const {
    if (!m_graph) return 0;
    const double top = m_graph->dbTop(), bottom = m_graph->dbBottom();
    return height() - (db - bottom) / (top - bottom) * height();
}

double BodePlot::dbForY(double y) const {
    if (!m_graph || height() <= 0) return 0;
    const double top = m_graph->dbTop(), bottom = m_graph->dbBottom();
    return bottom + (height() - y) / height() * (top - bottom);
}

double BodePlot::dbStep() const {
    const double span = m_graph ? m_graph->dbRange() : 50;
    return span <= 12 ? 1 : span <= 30 ? 3 : span <= 60 ? 5 : 10;
}

QPolygonF BodePlot::polyline(const QVector<double> &values, double top, double bottom) const {
    QPolygonF line;
    const int n = int(values.size());
    if (n < 2 || !m_graph) return line;
    line.reserve(n);
    const double dataLo = std::log10(kGridLowHz), dataHi = std::log10(kGridHighHz);
    const double viewLo = std::log10(m_graph->minFreq()), viewHi = std::log10(m_graph->maxFreq());
    const double w = width(), h = height();
    for (int i = 0; i < n; ++i) {
        const double logHz = dataLo + double(i) / (n - 1) * (dataHi - dataLo);
        const double x = (logHz - viewLo) / (viewHi - viewLo) * w;
        const double y = h - (values[i] - bottom) / (top - bottom) * h;
        line.append(QPointF(x, qIsFinite(y) ? y : h));
    }
    return line;
}

QColor BodePlot::colorOf(int channel) const {
    return channel >= 0 && channel < m_colors.size() && m_colors[channel].isValid() ? m_colors[channel] : m_labelColor;
}

// ---- Curves and their movement ---------------------------------------------------

void BodePlot::retarget() {
    QHash<int, QVector<double>> targets;
    m_drawnChannels.clear();
    if ((m_layers & Curves) && m_graph && m_graph->connected()) {
        const int skip = m_excludeOverlay ? m_graph->overlayChannel() : -1;
        for (int ch : m_channels) {
            if (ch == skip || m_drawnChannels.contains(ch)) continue;
            const QVector<double> &mag = m_graph->magnitudes(ch);
            if (mag.isEmpty()) continue;
            QVector<double> curve = mag;
            if (const double gain = m_graph->outputGain(ch); gain != 0)
                for (double &v : curve) v += gain;
            targets.insert(ch, curve);
            m_drawnChannels.append(ch);
        }
        std::sort(m_drawnChannels.begin(), m_drawnChannels.end());
        // The page's phase, while its curve shows
        if (m_graph->showPhase() && m_active >= 0 && m_channels.contains(m_active)) {
            const QVector<double> &ph = m_graph->phases(m_active, m_graph->phaseUnwrapped());
            if (!ph.isEmpty()) targets.insert(kPhaseKey, ph);
        }
    }

    bool moving = false;
    for (auto it = m_lines.begin(); it != m_lines.end();) {
        if (!targets.contains(it.key())) it = m_lines.erase(it);
        else ++it;
    }
    for (auto it = targets.cbegin(); it != targets.cend(); ++it) {
        Line &line = m_lines[it.key()];
        if (line.shown.size() != it.value().size() || !m_animated) {
            // New curves appear in place
            line.from = line.to = line.shown = it.value();
        } else if (line.to != it.value()) {
            line.from = line.shown;
            line.to = it.value();
            moving = true;
        }
    }
    if (moving) {
        m_clock.start();
        if (!m_timer.isActive()) m_timer.start();
    }

    // Channels with the same curve are one line; grouped by the curve they
    // move to, so a line that is moving stays one line
    std::map<QVector<double>, QList<int>> groups;
    for (int ch : m_drawnChannels) groups[m_lines.value(ch).to].append(ch);
    m_groups.clear();
    for (const auto &g : groups) m_groups.append(g.second);
    std::sort(m_groups.begin(), m_groups.end(), [](const QList<int> &a, const QList<int> &b) { return a.first() < b.first(); });
    m_drawnLines = int(m_groups.size());
    m_drawnPhase = m_lines.contains(kPhaseKey);
    emit drawn();
    update();
}

void BodePlot::tick() {
    const double t = qMin(1.0, double(m_clock.elapsed()) / kMoveMs);
    const double eased = 1.0 - std::pow(1.0 - t, 3.0);
    for (Line &line : m_lines) {
        if (line.from.size() != line.to.size()) {
            line.shown = line.to;
            continue;
        }
        for (int i = 0; i < line.to.size(); ++i) line.shown[i] = line.from[i] + (line.to[i] - line.from[i]) * eased;
    }
    if (t >= 1.0) {
        m_timer.stop();
        emit drawn();
    }
    update();
}

// ---- Painting --------------------------------------------------------------------

void BodePlot::paint(QPainter *p) {
    if (!m_graph || width() <= 0 || height() <= 0) return;
    p->setRenderHint(QPainter::Antialiasing, true);
    if (m_layers & Grid) paintGrid(p);
    if (m_layers & Curves) paintCurves(p);
    if (m_layers & Labels) paintLabels(p);
}

void BodePlot::paintGrid(QPainter *p) {
    const double w = width(), h = height();
    const double lo = m_graph->minFreq(), hi = m_graph->maxFreq();
    // The user's opacity scales each line's own strength
    const double strength = m_graph->gridOpacity();
    auto pen = [&](double alpha) { return QPen(withAlpha(m_gridColor, qMin(alpha * strength, 1.0)), 1.0); };

    if (m_graph->showFreqGrid()) {
        QPainterPath major, minor;
        for (double f : kMajorHz) {
            if (f < lo || f > hi) continue;
            const double x = xForFrequency(f);
            major.moveTo(x, 0);
            major.lineTo(x, h);
        }
        for (double f : kMinorHz) {
            if (f < lo || f > hi) continue;
            const double x = xForFrequency(f);
            minor.moveTo(x, 0);
            minor.lineTo(x, h);
        }
        p->strokePath(major, pen(0.15));
        p->strokePath(minor, pen(0.06));
    }

    if (m_graph->showDbGrid()) {
        const double top = m_graph->dbTop(), bottom = m_graph->dbBottom(), step = dbStep();
        QPainterPath lines;
        for (double db = std::ceil(bottom / step) * step; db <= top; db += step) {
            if (qAbs(db) <= 0.01) continue;   // the 0 dB line is drawn on its own
            const double y = yForDb(db);
            lines.moveTo(0, y);
            lines.lineTo(w, y);
        }
        p->strokePath(lines, pen(0.1));
        if (bottom <= 0 && top >= 0) {
            QPainterPath zero;
            zero.moveTo(0, yForDb(0));
            zero.lineTo(w, yForDb(0));
            p->strokePath(zero, pen(0.3));
        }
    }
}

void BodePlot::paintCurves(QPainter *p) {
    const double top = m_graph->dbTop(), bottom = m_graph->dbBottom();
    const double lw = m_graph->lineWidth();
    const bool glow = m_graph->showGlow();

    for (const QList<int> &group : m_groups) {
        const QPolygonF line = polyline(m_lines.value(group.first()).shown, top, bottom);
        bool anyActive = false;
        for (int ch : group) anyActive = anyActive || !m_follows || m_active < 0 || ch == m_active;
        if (group.size() == 1) {
            const QColor c = colorOf(group.first());
            if (glow && anyActive) {
                p->setPen(QPen(withAlpha(c, 0.12), lw * 4, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
                p->drawPolyline(line);
                p->setPen(QPen(withAlpha(c, 0.3), lw * 2, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
                p->drawPolyline(line);
            }
            p->setPen(anyActive ? QPen(c, lw, Qt::SolidLine, Qt::FlatCap, Qt::RoundJoin) : dashed(c, lw, { 6, 4 }));
            p->drawPolyline(line);
        } else {
            QVector<QColor> colors;
            for (int ch : group) colors.append(colorOf(ch));
            QLinearGradient gradient(0, 0, width(), 0);
            gradient.setStops(shareStops(colors));
            if (glow && anyActive) {
                p->setOpacity(0.07);
                p->setPen(QPen(QBrush(gradient), lw * 5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
                p->drawPolyline(line);
                p->setPen(QPen(QBrush(gradient), lw * 2.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
                p->drawPolyline(line);
                p->setOpacity(1.0);
            }
            const double width = lw * 1.25;
            p->setPen(anyActive ? QPen(QBrush(gradient), width, Qt::SolidLine, Qt::FlatCap, Qt::RoundJoin)
                                : dashed(QBrush(gradient), width, { 6, 4 }));
            p->drawPolyline(line);
        }
    }

    // The page's phase: dotted, on its own ±phaseTop axis
    if (m_lines.contains(kPhaseKey)) {
        const double pTop = m_graph->phaseTop();
        const double width = lw * 0.9;
        p->setPen(dashed(m_phaseColor, width, { 0.1, lw * 3 }, Qt::RoundCap));
        p->drawPolyline(polyline(m_lines.value(kPhaseKey).shown, pTop, -pTop));
    }
}

void BodePlot::paintLabels(QPainter *p) {
    const double w = width(), h = height();
    p->setFont(m_font);
    p->setPen(m_labelColor);
    const QFontMetricsF fm(m_font);

    if (m_graph->showFreqLabels()) {
        for (const auto &l : kFreqLabels) {
            if (l.hz < m_graph->minFreq() || l.hz > m_graph->maxFreq()) continue;
            const QString text = QString::fromLatin1(l.text);
            const double x = xForFrequency(l.hz) - fm.horizontalAdvance(text) / 2;
            p->drawText(QPointF(x, h - 2 - fm.descent()), text);
        }
    }

    if (m_graph->showDbLabels()) {
        const double top = m_graph->dbTop(), bottom = m_graph->dbBottom(), step = dbStep();
        for (double db = std::ceil(bottom / step) * step; db <= top; db += step) {
            const double v = qAbs(db) < 1e-9 ? 0.0 : db;
            const QString text = v >= 0 ? QStringLiteral("+%1").arg(v, 0, 'g') : QString::number(v, 'g');
            p->drawText(QPointF(4, yForDb(db) + (fm.ascent() - fm.descent()) / 2), text);
        }
    }

    // The phase axis on the right, in degrees, following the dB zoom
    if (m_graph->showPhase()) {
        p->setPen(withAlpha(m_phaseColor, 0.55));
        const double pTop = m_graph->phaseTop();
        for (double ph : { pTop, pTop / 2, 0.0, -pTop / 2, -pTop }) {
            const double y = h - (ph + pTop) / (2 * pTop) * h;
            const int deg = int(std::lround(ph));
            const QString text = deg == 0 ? QStringLiteral("0°") : QStringLiteral("%1%2°").arg(deg > 0 ? "+" : "").arg(deg);
            p->drawText(QPointF(w - 4 - fm.horizontalAdvance(text), y + (fm.ascent() - fm.descent()) / 2), text);
        }
    }
}
