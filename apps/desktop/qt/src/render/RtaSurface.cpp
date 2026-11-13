// -- WP QP9 -- The spectrum's curves and what both renderers share (RtaSurface.h).
#include "RtaSurface.h"

#include "CoreApi.h"
#include "Registrar.h"
#include "RtaController.h"

#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <algorithm>
#include <cmath>
#include <optional>

namespace {

// Two bands above the bass bank's last the bins have taken over
constexpr int kBlendBands = 2;
// A glide this close to its target has arrived (dB)
constexpr double kArrived = 0.05;

QColor faded(QColor c, double alpha) {
    c.setAlphaF(std::clamp(alpha, 0.0, 1.0));
    return c;
}

// A trace's pen: round ends and joins
QPen tracePen(const QColor &c, double width) {
    QPen pen(c, width);
    pen.setCapStyle(Qt::RoundCap);
    pen.setJoinStyle(Qt::RoundJoin);
    return pen;
}

// A smooth curve through (xs, ys) evaluated at x: cubic Hermite with each
// knot's slope taken across its neighbours
std::optional<double> smoothAt(const std::vector<double> &xs, const std::vector<double> &ys, double x) {
    const size_t n = xs.size();
    if (n == 0 || x < xs.front() || x > xs.back()) return std::nullopt;
    if (n == 1) return ys.front();
    const size_t k = std::min<size_t>(size_t(std::upper_bound(xs.begin(), xs.end(), x) - xs.begin()), n - 1);
    const size_t a = k - 1;
    const auto tangent = [&](size_t i) {
        const size_t lo = i > 0 ? i - 1 : i, hi = std::min(i + 1, n - 1);
        const double run = xs[hi] - xs[lo];
        return run > 0 ? (ys[hi] - ys[lo]) / run : 0.0;
    };
    const double span = xs[k] - xs[a];
    if (span <= 0) return ys[a];
    const double u = (x - xs[a]) / span;
    const double h00 = (1 + 2 * u) * (1 - u) * (1 - u), h10 = u * (1 - u) * (1 - u);
    const double h01 = u * u * (3 - 2 * u), h11 = u * u * (u - 1);
    return h00 * ys[a] + h10 * span * tangent(a) + h01 * ys[k] + h11 * span * tangent(k);
}

} // namespace

// ---- RtaGlide ----

bool RtaGlide::advance(const std::vector<double> &targets, double dt, double tau, bool peak) {
    if (tau <= 0 || m_now.size() != targets.size()) {
        m_now = targets;
        return false;
    }
    bool travelling = false;
    std::transform(targets.begin(), targets.end(), m_now.begin(), m_now.begin(), [&](double goal, double now) {
        if (!std::isfinite(goal) || !std::isfinite(now)) return goal;
        const double constant = goal > now ? (peak ? 0.0 : 0.4 * tau) : tau;
        if (constant <= 0) return goal;
        // What is left of the distance after dt
        const double next = goal + (now - goal) * std::exp(-dt / constant);
        if (std::fabs(goal - next) <= kArrived) return goal;
        travelling = true;
        return next;
    });
    return travelling;
}

// ---- RtaPaintedItem ----

RtaPaintedItem::RtaPaintedItem(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAntialiasing(true);
    setAcceptedMouseButtons(Qt::NoButton);
    setAcceptHoverEvents(false);
    m_clock.start();
    m_rta = Registrar::instance<RtaController>();
    const auto repaint = [this] { update(); };
    connect(this, &RtaPaintedItem::lookChanged, this, repaint);
    if (!m_rta) return;
    connect(m_rta, &RtaController::scaleChanged, this, repaint);
    connect(m_rta, &RtaController::pictureChanged, this, [this] {
        if (m_active) update();
    });
}

const std::vector<double> &RtaPaintedItem::labelledHz() {
    static const std::vector<double> marks = [] {
        std::vector<double> out;
        for (double decade = 10; decade <= 10000; decade *= 10)
            for (double step : { 1.0, 2.0, 5.0 }) out.push_back(decade * step);
        out.push_back(20000);
        return out;
    }();
    return marks;
}

double RtaPaintedItem::heightOf(double db) const {
    const double bottom = m_rta ? m_rta->floorDb() : -90, top = m_rta ? m_rta->ceilingDb() : 6;
    if (!(top > bottom) || !std::isfinite(db)) return 0;
    return std::clamp((db - bottom) / (top - bottom), 0.0, 1.0);
}

void RtaPaintedItem::paintLevels(QPainter &g, const QRectF &plot, bool numbers) const {
    if (!m_rta) return;
    const double bottom = m_rta->floorDb(), top = m_rta->ceilingDb();
    g.setFont(m_labelFont);
    for (double db = 12 * std::floor(top / 12); db > bottom; db -= 12) {
        const bool zero = db == 0;
        const double y = plot.bottom() - plot.height() * heightOf(db);
        g.setPen(QPen(faded(m_gridColor, zero ? 0.35 : 0.12), zero ? 1.0 : 0.5));
        g.drawLine(QLineF(plot.left(), y, plot.right(), y));
        if (!numbers) continue;
        g.setPen(faded(m_labelColor, 0.6));
        const QRectF box(plot.right() - 44, y - 6, 42, 12);
        g.drawText(box, int(Qt::AlignRight | Qt::AlignTop), QString::number(int(db)));
    }
}

double RtaPaintedItem::tick() { return std::clamp(double(m_clock.restart()) / 1000.0, 0.0, 0.25); }

void RtaPaintedItem::afterPaint(bool countChanged, bool moving) {
    QMetaObject::invokeMethod(
        this,
        [this, countChanged, moving] {
            if (countChanged) emit painted();
            if (moving && m_active) update();
        },
        Qt::QueuedConnection);
}

// ---- RtaSurface ----

RtaSurface::RtaSurface(QQuickItem *parent) : RtaPaintedItem(parent) {}

double RtaSurface::xAt(double hz, const QRectF &plot) const {
    const double from = std::log10(std::max(1.0, m_minFreq));
    const double to = std::log10(std::max(m_minFreq + 1, m_maxFreq));
    return plot.left() + plot.width() * (std::log10(std::max(1.0, hz)) - from) / (to - from);
}

void RtaSurface::paint(QPainter *target) {
    QPainter &g = *target;
    const double dt = tick();
    const QRectF plot(QPointF(0, 0), QSizeF(width(), height() - (m_grid ? kLabelRow : 0)));
    if (!m_rta || plot.width() < 2 || plot.height() < 2) return;
    g.setRenderHint(QPainter::Antialiasing);
    const auto yAt = [&](double db) { return plot.bottom() - plot.height() * heightOf(db); };

    if (m_grid) {
        paintLevels(g, plot, true);
        for (double hz : labelledHz()) {
            if (hz < m_minFreq || hz > m_maxFreq) continue;
            const double x = xAt(hz, plot);
            g.setPen(QPen(faded(m_gridColor, 0.10), 0.5));
            g.drawLine(QLineF(x, plot.top(), x, plot.bottom()));
            g.setPen(m_labelColor);
            g.drawText(QRectF(x - 20, plot.bottom() + 1, 40, kLabelRow), int(Qt::AlignHCenter | Qt::AlignTop),
                       RtaController::shortHz(hz));
        }
    }

    const RtaSnapshot &snap = m_rta->snapshot();
    const RtaCapsInfo &probe = m_rta->capabilities();
    if (!m_active || !m_rta->available() || snap.tap != m_tap) return;
    const double tau = m_rta->smoothingTau();
    const double strength = m_rta->graphOpacity();
    const bool withPeaks = m_rta->showPeakHold();

    // The bands on this axis: resolved, with a centre; of those below it
    // only the last (the curve runs in from the left edge), and nothing past
    // the first above it
    std::vector<int> bands;
    for (int b = 0, n = std::min(m_rta->bandCount(), int(RTA_MAX_BANDS)); b < n; ++b) {
        const double hz = probe.centres[b];
        if (hz <= 0 || !m_rta->bandVisible(b)) continue;
        const bool below = hz < m_minFreq;
        if (below && !bands.empty() && probe.centres[bands.back()] < m_minFreq) bands.back() = b;
        else bands.push_back(b);
        if (hz > m_maxFreq) break;
    }
    if (bands.empty()) return;

    const int columns = std::max(1, int(plot.width()));
    const bool single = m_channels.size() == 1;
    bool moving = false;
    int drawn = 0;
    QLinearGradient shade(plot.topLeft(), plot.bottomLeft());

    for (int i = 0; i < m_channels.size(); ++i) {
        const int ch = m_channels.at(i);
        const RtaChannelFrame *frame = nullptr;
        for (int f = 0; f < std::min<int>(snap.count, RTA_MAX_CHANNELS); ++f)
            if (snap.frames[f].channel == ch) frame = &snap.frames[f];
        if (!frame) continue;
        const QColor tint = i < m_colors.size() ? m_colors.at(i).value<QColor>() : QColor(Qt::white);

        std::vector<double> levels(bands.size()), holds(bands.size());
        for (size_t k = 0; k < bands.size(); ++k) {
            const bool known = bands[k] < frame->n_bands;
            levels[k] = known ? frame->avg_db[bands[k]] : RTA_FLOOR_DB;
            holds[k] = known ? frame->peak_db[bands[k]] : RTA_FLOOR_DB;
        }
        moving = m_level[ch].advance(levels, dt, tau, false) || moving;
        moving = m_hold[ch].advance(holds, dt, tau, true) || moving;
        std::vector<double> knotX(bands.size()), knotY(bands.size()), holdY(bands.size());
        for (size_t k = 0; k < bands.size(); ++k) {
            knotX[k] = xAt(probe.centres[bands[k]], plot);
            knotY[k] = yAt(m_level[ch].values()[k]);
            holdY[k] = yAt(m_hold[ch].values()[k]);
        }

        // One channel: the transform's bins, the loudest per pixel column,
        // empty columns bridged in a straight line
        std::vector<double> fine(size_t(columns), NAN);
        if (single && snap.bins_channel == ch && snap.bins_count > 1 && snap.bins_sample_rate > 0) {
            std::vector<double> loudest(size_t(columns), -INFINITY);
            const double binHz = snap.bins_sample_rate / (2.0 * snap.bins_count);
            for (int k = 1; k < std::min<int>(snap.bins_count, RTA_MAX_BINS); ++k) {
                const double hz = k * binHz;
                if (hz < m_minFreq || hz > m_maxFreq) continue;
                const size_t col = size_t(std::clamp(int(std::lround(xAt(hz, plot) - plot.left())), 0, columns - 1));
                loudest[col] = std::max(loudest[col], double(snap.bins_db[k]));
            }
            std::vector<int> filled;
            for (int c = 0; c < columns; ++c)
                if (std::isfinite(loudest[size_t(c)])) filled.push_back(c);
            for (size_t j = 1; j < filled.size(); ++j)
                for (int c = filled[j - 1] + 1; c < filled[j]; ++c) {
                    const double w = double(c - filled[j - 1]) / (filled[j] - filled[j - 1]);
                    loudest[size_t(c)] = loudest[size_t(filled[j - 1])] * (1 - w) + loudest[size_t(filled[j])] * w;
                }
            if (m_fineFor != ch * 4096 + columns) {
                m_fine.forget();
                m_fineFor = ch * 4096 + columns;
            }
            std::vector<double> goals(loudest);
            for (double &v : goals) v = std::isfinite(v) ? v : RTA_FLOOR_DB;
            moving = m_fine.advance(goals, dt, tau, false) || moving;
            for (size_t c = 0; c < fine.size(); ++c)
                if (std::isfinite(loudest[c])) fine[c] = yAt(m_fine.values()[c]);
        }
        const int bass = probe.bass_bands ? probe.bass_bands : 14;
        const int total = probe.max_bands ? probe.max_bands : int(RTA_MAX_BANDS);
        const double blendFrom = xAt(probe.centres[std::clamp(bass - 1, 0, int(RTA_MAX_BANDS) - 1)], plot);
        const double blendTo = xAt(probe.centres[std::clamp(std::min(bass + kBlendBands, total - 1), 0, int(RTA_MAX_BANDS) - 1)], plot);

        // A point per column: the band curve, giving way to the bins
        QPainterPath edge, contour;
        double firstX = NAN, lastX = NAN;
        for (int c = 0; c < columns; ++c) {
            const double x = plot.left() + c;
            const std::optional<double> band = smoothAt(knotX, knotY, x);
            const double bin = fine[size_t(c)];
            std::optional<double> y;
            if (band && !std::isnan(bin)) {
                const double w = blendTo > blendFrom ? std::clamp((x - blendFrom) / (blendTo - blendFrom), 0.0, 1.0) : 1.0;
                y = std::clamp(*band, plot.top(), plot.bottom()) * (1 - w) + bin * w;
            } else if (band) {
                y = std::clamp(*band, plot.top(), plot.bottom());
            } else if (!std::isnan(bin)) {
                y = bin;
            }
            if (y) {
                if (std::isnan(firstX)) {
                    edge.moveTo(x, *y);
                    firstX = x;
                } else {
                    edge.lineTo(x, *y);
                }
                lastX = x;
            }
            if (withPeaks) {
                if (const std::optional<double> h = smoothAt(knotX, holdY, x)) {
                    const QPointF at(x, std::clamp(*h, plot.top(), plot.bottom()));
                    if (contour.elementCount() == 0) contour.moveTo(at);
                    else contour.lineTo(at);
                }
            }
        }
        if (edge.elementCount() < 2) continue;
        ++drawn;

        // The fill down to the bottom; its fade runs over the whole plot
        QPainterPath area(edge);
        area.lineTo(lastX, plot.bottom());
        area.lineTo(firstX, plot.bottom());
        area.closeSubpath();
        shade.setColorAt(0, faded(tint, 0.34 * strength));
        shade.setColorAt(1, faded(tint, 0.02 * strength));
        g.fillPath(area, shade);
        g.setBrush(Qt::NoBrush);
        if (m_glow) {
            g.setPen(tracePen(faded(tint, 0.175 * strength), 4));
            g.drawPath(edge);
        }
        g.setPen(tracePen(faded(tint, 0.55 * strength), 1));
        g.drawPath(edge);
        if (contour.elementCount() > 1) {
            g.setPen(tracePen(faded(tint, 0.35 * strength), 1));
            g.drawPath(contour);
        }
    }
    const bool changed = drawn != m_drawn;
    m_drawn = drawn;
    afterPaint(changed, moving);
}
