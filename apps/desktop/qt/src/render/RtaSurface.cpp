// -- WP QP9 -- The spectrum's curves (RtaSurface.h).
#include "RtaSurface.h"

#include "CoreApi.h"
#include "Registrar.h"
#include "RtaController.h"

#include <QFontMetricsF>
#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <algorithm>
#include <cmath>

namespace {

// The labelled frequencies (the Mac's analyser grid)
constexpr double kLabelled[] = { 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000 };
// The label row under the window's plot
constexpr double kLabelRow = 12;
// Between the bass bank's last band and two above it the bins take over
constexpr int kBlendAbove = 2;

double norm(double db, double floor, double ceiling) {
    if (!std::isfinite(db) || ceiling <= floor) return 0;
    return std::clamp((db - floor) / (ceiling - floor), 0.0, 1.0);
}

// Cubic Hermite through (xs, ys) at x, slopes from the neighbours
double hermite(const std::vector<double> &xs, const std::vector<double> &ys, double x) {
    const size_t n = xs.size();
    if (n == 0) return NAN;
    if (n == 1 || x <= xs.front()) return ys.front();
    if (x >= xs.back()) return ys.back();
    size_t i = size_t(std::upper_bound(xs.begin(), xs.end(), x) - xs.begin()) - 1;
    i = std::min(i, n - 2);
    const auto slope = [&](size_t k) {
        const size_t a = k == 0 ? 0 : k - 1, b = std::min(k + 1, n - 1);
        const double dx = xs[b] - xs[a];
        return dx > 0 ? (ys[b] - ys[a]) / dx : 0.0;
    };
    const double h = xs[i + 1] - xs[i];
    if (h <= 0) return ys[i];
    const double t = (x - xs[i]) / h, t2 = t * t, t3 = t2 * t;
    return (2 * t3 - 3 * t2 + 1) * ys[i] + (t3 - 2 * t2 + t) * h * slope(i) + (-2 * t3 + 3 * t2) * ys[i + 1]
        + (t3 - t2) * h * slope(i + 1);
}

} // namespace

bool RtaGlide::step(const std::vector<double> &targets, double dt, double tau, bool peak) {
    if (m_values.size() != targets.size() || tau <= 0) {
        m_values = targets;
        return false;
    }
    bool moving = false;
    for (size_t i = 0; i < targets.size(); ++i) {
        double &v = m_values[i];
        const double t = targets[i];
        if (!std::isfinite(t) || !std::isfinite(v)) {
            v = t;
            continue;
        }
        const double T = t > v ? (peak ? 0.0 : 0.4 * tau) : tau;
        if (T <= 0) {
            v = t;
            continue;
        }
        v += (t - v) * (1.0 - std::exp(-dt / T));
        if (std::fabs(t - v) > 0.05) moving = true;
        else v = t;
    }
    return moving;
}

RtaSurface::RtaSurface(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAntialiasing(true);
    setAcceptedMouseButtons(Qt::NoButton);
    setAcceptHoverEvents(false);
    m_clock.start();
    connect(this, &RtaSurface::lookChanged, this, [this] { update(); });
    follow();
}

void RtaSurface::follow() {
    m_rta = Registrar::instance<RtaController>();
    if (!m_rta) return;
    connect(m_rta, &RtaController::pictureChanged, this, [this] { if (m_active) update(); });
    connect(m_rta, &RtaController::scaleChanged, this, [this] { update(); });
}

double RtaSurface::xFor(double hz, double plotX, double plotW) const {
    const double lo = std::log10(std::max(m_minFreq, 1.0)), hi = std::log10(std::max(m_maxFreq, m_minFreq + 1));
    return plotX + (std::log10(std::max(hz, 1.0)) - lo) / (hi - lo) * plotW;
}

void RtaSurface::paint(QPainter *p) {
    int drawn = 0;
    const double dt = std::clamp(m_clock.restart() / 1000.0, 0.0, 0.25);
    const QRectF plot(0, 0, width(), height() - (m_grid ? kLabelRow : 0));
    if (!m_rta || plot.width() < 2 || plot.height() < 2) return;
    const double floor = m_rta->floorDb(), ceiling = m_rta->ceilingDb();
    const auto yOf = [&](double db) { return plot.bottom() - plot.height() * norm(db, floor, ceiling); };
    p->setRenderHint(QPainter::Antialiasing, true);

    if (m_grid) {
        // dB lines every 12 dB from the top, the 0 dB line stronger; the labelled frequencies
        QPen line(m_gridColor);
        p->setFont(m_labelFont);
        for (double db = std::floor(ceiling / 12) * 12; db > floor; db -= 12) {
            const double y = yOf(db);
            QColor c = m_gridColor;
            c.setAlphaF(db == 0 ? 0.35 : 0.12);
            line.setColor(c);
            line.setWidthF(db == 0 ? 1.0 : 0.5);
            p->setPen(line);
            p->drawLine(QPointF(plot.left(), y), QPointF(plot.right(), y));
            QColor lc = m_labelColor;
            lc.setAlphaF(0.6);
            p->setPen(lc);
            p->drawText(QRectF(plot.right() - 42, y - 6, 40, 12), Qt::AlignRight | Qt::AlignTop, QString::number(int(db)));
        }
        for (double hz : kLabelled) {
            if (hz < m_minFreq || hz > m_maxFreq) continue;
            const double x = xFor(hz, plot.x(), plot.width());
            QColor c = m_gridColor;
            c.setAlphaF(0.10);
            p->setPen(QPen(c, 0.5));
            p->drawLine(QPointF(x, plot.top()), QPointF(x, plot.bottom()));
            p->setPen(m_labelColor);
            p->drawText(QRectF(x - 20, height() - kLabelRow + 1, 40, kLabelRow), Qt::AlignHCenter | Qt::AlignTop,
                        RtaController::shortHz(hz));
        }
    }

    const RtaSnapshot &snap = m_rta->snapshot();
    const RtaCapsInfo &caps = m_rta->caps();
    if (!m_active || !m_rta->supported() || snap.tap != m_tap) return;
    const double tau = m_rta->smoothingTau();
    const double opacity = m_rta->graphOpacity();
    const bool peaks = m_rta->showPeakHold();
    const int bands = m_rta->bandCount();
    const int columns = std::max(1, int(plot.width()));
    bool moving = false;

    // The bands drawn: resolved, with a centre; below the range only the last
    // one (the curve runs off the left edge), stop after the first above it
    std::vector<int> use;
    for (int b = 0; b < bands && b < RTA_MAX_BANDS; ++b) {
        if (!m_rta->bandVisible(b) || caps.centres[b] == 0) continue;
        const double hz = caps.centres[b];
        if (hz < m_minFreq) {
            if (!use.empty() && caps.centres[use.back()] < m_minFreq) use.back() = b;
            else use.push_back(b);
            continue;
        }
        use.push_back(b);
        if (hz > m_maxFreq) break;
    }

    const bool one = m_channels.size() == 1;
    QLinearGradient fade(0, plot.top(), 0, plot.bottom());
    for (int i = 0; i < m_channels.size(); ++i) {
        const int ch = m_channels.at(i);
        const RtaChannelFrame *frame = nullptr;
        for (int f = 0; f < snap.count && f < RTA_MAX_CHANNELS; ++f)
            if (snap.frames[f].channel == ch) frame = &snap.frames[f];
        if (!frame || use.empty()) continue;
        const QColor color = i < m_colors.size() ? m_colors.at(i).value<QColor>() : QColor(Qt::white);

        std::vector<double> avg, peak;
        for (int b : use) {
            avg.push_back(b < frame->n_bands ? frame->avg_db[b] : RTA_FLOOR_DB);
            peak.push_back(b < frame->n_bands ? frame->peak_db[b] : RTA_FLOOR_DB);
        }
        moving = m_avg[ch].step(avg, dt, tau, false) || moving;
        moving = m_peak[ch].step(peak, dt, tau, true) || moving;
        std::vector<double> xs, ya, yp;
        for (size_t k = 0; k < use.size(); ++k) {
            xs.push_back(xFor(caps.centres[use[k]], plot.x(), plot.width()));
            ya.push_back(yOf(m_avg[ch].values()[k]));
            yp.push_back(yOf(m_peak[ch].values()[k]));
        }

        // One channel with bins: the loudest bin per pixel column, gaps filled
        std::vector<double> binY(size_t(columns), NAN);
        const bool bins = one && snap.bins_count > 1 && snap.bins_channel == ch && snap.bins_sample_rate > 0;
        if (bins) {
            std::vector<double> col(size_t(columns), NAN);
            for (int k = 1; k < snap.bins_count && k < RTA_MAX_BINS; ++k) {
                const double hz = double(k) * snap.bins_sample_rate / (2.0 * snap.bins_count);
                if (hz < m_minFreq || hz > m_maxFreq) continue;
                const int c = std::clamp(int(std::lround(xFor(hz, plot.x(), plot.width()) - plot.x())), 0, columns - 1);
                const double db = snap.bins_db[k];
                if (std::isnan(col[size_t(c)]) || db > col[size_t(c)]) col[size_t(c)] = db;
            }
            int last = -1;
            for (int c = 0; c < columns; ++c) {
                if (std::isnan(col[size_t(c)])) continue;
                if (last >= 0)
                    for (int g = last + 1; g < c; ++g)
                        col[size_t(g)] = col[size_t(last)] + (col[size_t(c)] - col[size_t(last)]) * (g - last) / double(c - last);
                last = c;
            }
            if (m_binsKey != ch * 10000 + columns) {
                m_bins.reset();
                m_binsKey = ch * 10000 + columns;
            }
            std::vector<double> filled = col;
            for (double &v : filled)
                if (std::isnan(v)) v = RTA_FLOOR_DB;
            moving = m_bins.step(filled, dt, tau, false) || moving;
            for (int c = 0; c < columns; ++c)
                if (!std::isnan(col[size_t(c)])) binY[size_t(c)] = yOf(m_bins.values()[size_t(c)]);
        }
        const int bass = caps.bass_bands > 0 ? caps.bass_bands : 14;
        const int nc = caps.max_bands > 0 ? caps.max_bands : bands;
        const double loX = xFor(caps.centres[std::clamp(bass - 1, 0, RTA_MAX_BANDS - 1)], plot.x(), plot.width());
        const double hiX = xFor(caps.centres[std::clamp(std::min(bass + kBlendAbove, nc - 1), 0, RTA_MAX_BANDS - 1)],
                                plot.x(), plot.width());

        // One point per column: the band curve, crossfaded into the bins
        QPolygonF line, contour;
        const double x0 = std::max(plot.x(), xs.front()), x1 = std::min(plot.right(), xs.back());
        for (int c = 0; c < columns; ++c) {
            const double x = plot.x() + c;
            const bool inBands = x >= x0 && x <= x1;
            const double band = inBands ? std::clamp(hermite(xs, ya, x), plot.top(), plot.bottom()) : NAN;
            const double bin = binY[size_t(c)];
            double y = NAN;
            if (!std::isnan(band) && !std::isnan(bin)) {
                const double w = hiX > loX ? std::clamp((x - loX) / (hiX - loX), 0.0, 1.0) : 1.0;
                y = band + (bin - band) * w;
            } else if (!std::isnan(band)) {
                y = band;
            } else if (!std::isnan(bin)) {
                y = bin;
            }
            if (!std::isnan(y)) line << QPointF(x, y);
            if (peaks && inBands) contour << QPointF(x, std::clamp(hermite(xs, yp, x), plot.top(), plot.bottom()));
        }
        if (line.size() < 2) continue;
        ++drawn;

        // Fill to the bottom, its gradient across the whole plot
        QPainterPath area;
        area.moveTo(line.first().x(), plot.bottom());
        for (const QPointF &pt : line) area.lineTo(pt);
        area.lineTo(line.last().x(), plot.bottom());
        area.closeSubpath();
        QColor top = color, bottom = color;
        top.setAlphaF(0.34 * opacity);
        bottom.setAlphaF(0.02 * opacity);
        fade.setStops({ { 0.0, top }, { 1.0, bottom } });
        p->fillPath(area, fade);
        if (m_glow) {
            QColor g = color;
            g.setAlphaF(0.35 * opacity * 0.5);
            p->setPen(QPen(g, 4, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            p->drawPolyline(line);
        }
        QColor edge = color;
        edge.setAlphaF(0.55 * opacity);
        p->setPen(QPen(edge, 1, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        p->drawPolyline(line);
        if (contour.size() > 1) {
            QColor pc = color;
            pc.setAlphaF(0.35 * opacity);
            p->setPen(QPen(pc, 1, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            p->drawPolyline(contour);
        }
    }
    // Afterwards, not from inside paint: the count and the next frame of a glide
    const bool changed = drawn != m_drawn;
    m_drawn = drawn;
    QMetaObject::invokeMethod(
        this,
        [this, changed, moving] {
            if (changed) emit painted();
            if (moving && m_active) update();
        },
        Qt::QueuedConnection);
}
