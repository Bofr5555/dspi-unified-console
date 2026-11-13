// -- WP QP3 -- The response graph's drawing; BodePlot.h says what it draws.
#include "BodePlot.h"

#include "GraphController.h"

#include <QLinearGradient>
#include <QPainter>
#include <QtMath>
#include <algorithm>
#include <cmath>
#include <map>

namespace {

// The core's curve grid: point i of n sits at 10 Hz × 2000^(i/(n-1)), i.e.
// 10 Hz to 20 kHz evenly in log frequency (dspi_compute_magnitude_curve)
double gridHz(qsizetype i, qsizetype n) {
    return 10.0 * std::pow(2000.0, double(i) / double(n - 1));
}

constexpr int kPhaseKey = -1;
constexpr int kMoveMs = 200;

// The Mac's grid: a strong line at each decade from 100 Hz, a faint one at
// every other whole multiple of 10, 100 and 1000 Hz, and at 20 kHz
bool isDecade(int hz) { return hz == 100 || hz == 1000 || hz == 10000; }
QList<int> minorLines() {
    QList<int> out;
    for (int decade : { 10, 100, 1000 })
        for (int k = 2; k <= 9; ++k) out.append(k * decade);
    out.append(20000);
    return out;
}

// The frequency labels: 1, 2 and 5 of each decade from 20 Hz to 20 kHz,
// thousands with a "k"
QList<int> labelledHz() {
    QList<int> out;
    for (int decade : { 10, 100, 1000, 10000 })
        for (int k : { 1, 2, 5 })
            if (const int hz = k * decade; hz >= 20 && hz <= 20000) out.append(hz);
    return out;
}
QString hzText(int hz) { return hz < 1000 ? QString::number(hz) : QString::number(hz / 1000) + QLatin1Char('k'); }

// A pen with round joins; `px` dashes it, in pixels (QPen counts them in
// pen widths)
QPen stroke(const QBrush &brush, double width, Qt::PenCapStyle cap = Qt::FlatCap, std::initializer_list<double> px = {}) {
    QPen pen(brush, width);
    pen.setCapStyle(cap);
    pen.setJoinStyle(Qt::RoundJoin);
    if (px.size()) {
        QList<qreal> dashes;
        for (double v : px) dashes << v / width;
        pen.setDashPattern(dashes);
    }
    return pen;
}

// The gradient of a line several channels share: a solid stretch per channel,
// blended where two meet (the Mac's gradientStops)
QGradientStops shareStops(const QVector<QColor> &colors) {
    if (colors.isEmpty()) return {};
    if (colors.size() == 1) return { { 0.0, colors.first() }, { 1.0, colors.first() } };
    const double count = double(colors.size());
    const double blend = colors.size() == 2 ? 0.4 : colors.size() == 3 ? 0.25 : 0.15;
    const double solid = (1.0 / count - blend) / 2;
    QGradientStops stops{ { 0.0, colors.first() } };
    for (qsizetype k = 0; k < colors.size(); ++k) {
        const double mid = (double(k) + 0.5) / count;
        stops << QGradientStop(qMax(0.0, mid - solid), colors[k]) << QGradientStop(qMin(1.0, mid + solid), colors[k]);
    }
    stops << QGradientStop(1.0, colors.last());
    return stops;
}

QColor faded(QColor c, double alpha) {
    c.setAlphaF(float(qBound(0.0, c.alphaF() * alpha, 1.0)));
    return c;
}

} // namespace

BodePlot::BodePlot(QQuickItem *parent) : QQuickPaintedItem(parent) {
    m_timer.setTimerType(Qt::PreciseTimer);
    m_timer.callOnTimeout(this, &BodePlot::tick);
    setOpaquePainting(false);
    setAntialiasing(true);
}

void BodePlot::geometryChange(const QRectF &now, const QRectF &before) {
    QQuickPaintedItem::geometryChange(now, before);
    if (now.size() != before.size()) update();
}

void BodePlot::restyle() {
    emit styleChanged();
    update();
}

void BodePlot::setGraph(GraphController *graph) {
    if (graph == m_graph) return;
    if (m_graph) m_graph->disconnect(this);
    m_graph = graph;
    if (graph) {
        for (auto signal : { &GraphController::curvesChanged, &GraphController::overlayChannelChanged,
                             &GraphController::preferencesChanged })
            connect(graph, signal, this, &BodePlot::retarget);
    }
    emit graphChanged();
    retarget();
}

void BodePlot::setLayers(int layers) {
    if (!change(m_layers, layers)) return;
    emit layersChanged();
    retarget();
}

void BodePlot::setChannels(const QList<int> &channels) {
    if (!change(m_channels, channels)) return;
    emit channelsChanged();
    retarget();
}

void BodePlot::setActiveChannel(int channel) {
    if (!change(m_active, channel)) return;
    emit channelsChanged();
    retarget();
}

void BodePlot::setFollowsSelection(bool on) {
    if (!change(m_follows, on)) return;
    emit channelsChanged();
    retarget();
}

void BodePlot::setExcludeOverlay(bool on) {
    if (!change(m_excludeOverlay, on)) return;
    emit channelsChanged();
    retarget();
}

void BodePlot::setChannelColors(const QVariantList &colors) {
    m_colorList = colors;
    m_colors.resize(colors.size());
    std::transform(colors.cbegin(), colors.cend(), m_colors.begin(), [](const QVariant &c) { return c.value<QColor>(); });
    restyle();
}

void BodePlot::setGridColor(const QColor &c) { if (change(m_gridColor, c)) restyle(); }
void BodePlot::setLabelColor(const QColor &c) { if (change(m_labelColor, c)) restyle(); }
void BodePlot::setPhaseColor(const QColor &c) { if (change(m_phaseColor, c)) restyle(); }
void BodePlot::setLabelFont(const QFont &f) { if (change(m_font, f)) restyle(); }
void BodePlot::setAnimated(bool on) { if (change(m_animated, on)) emit styleChanged(); }

// ---- Mapping -------------------------------------------------------------------

double BodePlot::freqFraction(double hz) const {
    return std::log10(hz / m_graph->minFreq()) / std::log10(m_graph->maxFreq() / m_graph->minFreq());
}

double BodePlot::levelFraction(double db, double top, double bottom) {
    return (top - db) / (top - bottom);
}

double BodePlot::xForFrequency(double hz) const {
    return m_graph && hz > 0 ? freqFraction(hz) * width() : 0.0;
}

double BodePlot::frequencyForX(double x) const {
    if (!m_graph || width() <= 0) return 0.0;
    return m_graph->minFreq() * std::pow(m_graph->maxFreq() / m_graph->minFreq(), x / width());
}

double BodePlot::yForDb(double db) const {
    return m_graph ? levelFraction(db, m_graph->dbTop(), m_graph->dbBottom()) * height() : 0.0;
}

double BodePlot::dbForY(double y) const {
    if (!m_graph || height() <= 0) return 0.0;
    return m_graph->dbTop() - y / height() * m_graph->dbRange();
}

double BodePlot::dbStep() const {
    const double span = m_graph ? m_graph->dbRange() : 50;
    if (span <= 12) return 1;
    if (span <= 30) return 3;
    return span <= 60 ? 5 : 10;
}

QPolygonF BodePlot::polyline(const QVector<double> &values, double top, double bottom) const {
    QPolygonF line;
    if (!m_graph || values.size() < 2) return line;
    line.reserve(values.size());
    for (qsizetype i = 0; i < values.size(); ++i) {
        const double y = levelFraction(values[i], top, bottom) * height();
        line << QPointF(freqFraction(gridHz(i, values.size())) * width(), qIsFinite(y) ? y : height());
    }
    return line;
}

QColor BodePlot::colorOf(int channel) const {
    const QColor c = m_colors.value(channel);
    return c.isValid() ? c : m_labelColor;
}

// ---- Curves and their movement ---------------------------------------------------

void BodePlot::retarget() {
    QHash<int, QVector<double>> targets;
    m_drawnChannels.clear();
    if ((m_layers & Curves) && m_graph && m_graph->connected()) {
        const int skip = m_excludeOverlay ? m_graph->overlayChannel() : -1;
        for (int ch : std::as_const(m_channels)) {
            if (ch == skip || targets.contains(ch) || m_graph->magnitudes(ch).isEmpty()) continue;
            QVector<double> curve = m_graph->magnitudes(ch);
            const double gain = m_graph->outputGain(ch);
            std::for_each(curve.begin(), curve.end(), [gain](double &v) { v += gain; });
            targets.insert(ch, curve);
            m_drawnChannels << ch;
        }
        std::sort(m_drawnChannels.begin(), m_drawnChannels.end());
        // The page's phase, while its curve shows
        if (m_graph->showPhase() && m_active >= 0 && m_channels.contains(m_active)) {
            if (const QVector<double> &ph = m_graph->phases(m_active, m_graph->phaseUnwrapped()); !ph.isEmpty())
                targets.insert(kPhaseKey, ph);
        }
    }

    m_lines.removeIf([&targets](QHash<int, Line>::iterator it) { return !targets.contains(it.key()); });
    bool moving = false;
    for (auto t = targets.cbegin(); t != targets.cend(); ++t) {
        Line &line = m_lines[t.key()];
        const bool fresh = line.shown.size() != t.value().size();
        if (fresh || !m_animated) {
            // A curve that was not drawn appears where it is
            line.from = line.to = line.shown = t.value();
        } else if (line.to != t.value()) {
            line.from = line.shown;
            line.to = t.value();
            moving = true;
        }
    }
    if (moving) {
        m_clock.restart();
        m_timer.start(16);
    }

    // Channels whose curves are the same are one line; grouped by where the
    // curves are going, so a moving line stays one line
    // (in channel order, as m_drawnChannels is)
    std::map<QVector<double>, qsizetype> lineOf;
    m_groups.clear();
    for (int ch : std::as_const(m_drawnChannels)) {
        const auto found = lineOf.try_emplace(m_lines.value(ch).to, m_groups.size());
        if (found.second) m_groups.append(QList<int>());
        m_groups[found.first->second] << ch;
    }
    m_drawnLines = int(m_groups.size());
    m_drawnPhase = m_lines.contains(kPhaseKey);
    emit drawn();
    update();
}

void BodePlot::tick() {
    const double t = qMin(1.0, double(m_clock.elapsed()) / kMoveMs);
    const double k = 1.0 - std::pow(1.0 - t, 3.0);   // ease out
    for (Line &line : m_lines) {
        if (line.from.size() != line.to.size()) {
            line.shown = line.to;
            continue;
        }
        for (qsizetype j = 0, n = line.to.size(); j < n; ++j) line.shown[j] = line.from[j] + (line.to[j] - line.from[j]) * k;
    }
    if (t >= 1.0) {
        m_timer.stop();
        emit drawn();
    }
    update();
}

// ---- Painting --------------------------------------------------------------------

void BodePlot::paint(QPainter *painter) {
    if (!m_graph || boundingRect().isEmpty()) return;
    painter->setRenderHint(QPainter::Antialiasing);
    if (m_layers & Grid) paintGrid(painter);
    if (m_layers & Curves) paintCurves(painter);
    if (m_layers & Labels) paintLabels(painter);
}

void BodePlot::paintGrid(QPainter *painter) {
    const QRectF r = boundingRect();
    // The user's opacity scales each line's own strength
    const double strength = m_graph->graphGridOpacity();
    auto ink = [&](double alpha) { return QPen(faded(m_gridColor, qMin(alpha * strength, 1.0)), 1.0); };
    auto inView = [this](int hz) { return hz >= m_graph->minFreq() && hz <= m_graph->maxFreq(); };

    if (m_graph->showFrequencyGrid()) {
        QVector<QLineF> strong, faint;
        for (int hz : { 100, 1000, 10000 })
            if (inView(hz)) strong << QLineF(xForFrequency(hz), r.top(), xForFrequency(hz), r.bottom());
        for (int hz : minorLines())
            if (inView(hz) && !isDecade(hz)) faint << QLineF(xForFrequency(hz), r.top(), xForFrequency(hz), r.bottom());
        painter->setPen(ink(0.15));
        painter->drawLines(strong);
        painter->setPen(ink(0.06));
        painter->drawLines(faint);
    }

    if (m_graph->showDBGrid()) {
        const double step = dbStep();
        QVector<QLineF> rows;
        for (int k = int(std::ceil(m_graph->dbBottom() / step)); k * step <= m_graph->dbTop(); ++k)
            if (k != 0) rows << QLineF(r.left(), yForDb(k * step), r.right(), yForDb(k * step));
        painter->setPen(ink(0.1));
        painter->drawLines(rows);
        // 0 dB, stronger, when in view
        if (m_graph->dbBottom() <= 0 && m_graph->dbTop() >= 0) {
            painter->setPen(ink(0.3));
            painter->drawLine(QLineF(r.left(), yForDb(0), r.right(), yForDb(0)));
        }
    }
}

void BodePlot::paintCurves(QPainter *painter) {
    const double top = m_graph->dbTop(), bottom = m_graph->dbBottom();
    const double lw = m_graph->graphLineWidth();
    const bool glow = m_graph->showGraphGlow();

    for (const QList<int> &group : std::as_const(m_groups)) {
        const QPolygonF line = polyline(m_lines.value(group.first()).shown, top, bottom);
        const bool active = !m_follows || m_active < 0 || group.contains(m_active);
        const bool shared = group.size() > 1;
        QBrush brush(colorOf(group.first()));
        if (shared) {
            QVector<QColor> colors;
            for (int ch : group) colors << colorOf(ch);
            QLinearGradient across(QPointF(0, 0), QPointF(width(), 0));
            across.setStops(shareStops(colors));
            brush = QBrush(across);
        }
        if (glow && active) {
            // Wide faint strokes under the line (the Mac blurs them)
            painter->save();
            if (shared) {
                painter->setOpacity(0.07);
                painter->setPen(stroke(brush, lw * 5, Qt::RoundCap));
                painter->drawPolyline(line);
                painter->setPen(stroke(brush, lw * 2.5, Qt::RoundCap));
                painter->drawPolyline(line);
            } else {
                painter->setPen(stroke(faded(brush.color(), 0.12), lw * 4, Qt::RoundCap));
                painter->drawPolyline(line);
                painter->setPen(stroke(faded(brush.color(), 0.3), lw * 2, Qt::RoundCap));
                painter->drawPolyline(line);
            }
            painter->restore();
        }
        const double width = shared ? lw * 1.25 : lw;
        painter->setPen(active ? stroke(brush, width) : stroke(brush, width, Qt::FlatCap, { 6, 4 }));
        painter->drawPolyline(line);
    }

    // The page's phase: dotted, on its own ±phaseTop axis
    if (m_lines.contains(kPhaseKey)) {
        const double extent = m_graph->phaseTop();
        painter->setPen(stroke(m_phaseColor, lw * 0.9, Qt::RoundCap, { 0.1, lw * 3 }));
        painter->drawPolyline(polyline(m_lines.value(kPhaseKey).shown, extent, -extent));
    }
}

void BodePlot::paintLabels(QPainter *painter) {
    const QRectF r = boundingRect();
    const QFontMetricsF metrics(m_font);
    // Text centred on y: its baseline below by half the cap height
    const double middle = (metrics.ascent() - metrics.descent()) / 2;
    painter->setFont(m_font);
    painter->setPen(m_labelColor);

    if (m_graph->showFrequencyLabels()) {
        for (int hz : labelledHz()) {
            if (hz < m_graph->minFreq() || hz > m_graph->maxFreq()) continue;
            const QString text = hzText(hz);
            painter->drawText(QPointF(xForFrequency(hz) - metrics.horizontalAdvance(text) / 2, r.bottom() - 2 - metrics.descent()), text);
        }
    }

    if (m_graph->showDBLabels()) {
        const double step = dbStep();
        for (int k = int(std::ceil(m_graph->dbBottom() / step)); k * step <= m_graph->dbTop(); ++k) {
            const double db = k * step;
            painter->drawText(QPointF(r.left() + 4, yForDb(db) + middle), (k >= 0 ? QStringLiteral("+") : QString()) + QString::number(db, 'g'));
        }
    }

    // The phase axis on the right, in degrees, following the dB zoom
    if (m_graph->showPhase()) {
        painter->setPen(faded(m_phaseColor, 0.55));
        const double extent = m_graph->phaseTop();
        for (double share : { 1.0, 0.5, 0.0, -0.5, -1.0 }) {
            const long deg = std::lround(share * extent);
            const QString text = deg == 0 ? QStringLiteral("0°") : (deg > 0 ? QStringLiteral("+%1°") : QStringLiteral("%1°")).arg(deg);
            const double y = levelFraction(share, 1.0, -1.0) * r.height();
            painter->drawText(QPointF(r.right() - 4 - metrics.horizontalAdvance(text), y + middle), text);
        }
    }
}
