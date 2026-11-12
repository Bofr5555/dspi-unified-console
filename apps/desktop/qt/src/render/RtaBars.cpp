// -- WP QP9 -- One channel's spectrum bars (RtaBars.h).
#include "RtaBars.h"

#include "CoreApi.h"
#include "RtaController.h"

#include <QLinearGradient>
#include <QPainter>
#include <algorithm>
#include <cmath>
#include <vector>

namespace {

// A label's width per character, as the Mac estimates its 8 pt digits
constexpr double kGlyph = 4.9;
// Labels keep this much room between them
constexpr double kLabelGap = 4;

bool isDecade(double hz) {
    const double e = std::log10(hz);
    return std::fabs(e - std::round(e)) < 1e-9;
}

} // namespace

RtaBars::RtaBars(QQuickItem *parent) : RtaPaintedItem(parent) {}

void RtaBars::paintFrequencies(QPainter &g, const QRectF &plot, const std::vector<int> &bands, double slot) const {
    const RtaCapsInfo &probe = m_rta->capabilities();
    std::vector<std::pair<double, double>> taken;   // left and right of each label placed
    g.setPen(m_labelColor);
    g.setFont(m_labelFont);
    const auto place = [&](bool decades) {
        for (size_t pos = 0; pos < bands.size(); ++pos) {
            const double centre = probe.centres[bands[pos]];
            const auto mark = std::find_if(labelledHz().begin(), labelledHz().end(),
                                           [centre](double hz) { return std::fabs(centre - hz) <= 0.03 * hz; });
            if (mark == labelledHz().end() || isDecade(*mark) != decades) continue;
            const QString text = RtaController::shortHz(centre);
            const double mid = plot.left() + slot * (double(pos) + 0.5);
            const double half = text.size() * kGlyph / 2;
            if (mid - half < plot.left() || mid + half > plot.right()) continue;
            const bool crowded = std::any_of(taken.begin(), taken.end(), [&](const auto &t) {
                return mid - half < t.second + kLabelGap && mid + half + kLabelGap > t.first;
            });
            if (crowded) continue;
            taken.emplace_back(mid - half, mid + half);
            g.drawText(QRectF(mid - 20, height() - kLabelRow + 1, 40, kLabelRow - 1), int(Qt::AlignHCenter | Qt::AlignTop), text);
        }
    };
    place(true);
    place(false);
}

void RtaBars::paint(QPainter *target) {
    QPainter &g = *target;
    const double dt = tick();
    if (!m_rta || std::min(width(), height()) < 4) return;
    const QRectF plot(QPointF(0, 0), QSizeF(width(), height() - (m_frequencyLabels ? kLabelRow : 0)));
    g.setRenderHint(QPainter::Antialiasing);
    if (m_dbLabels) paintLevels(g, plot, true);

    std::vector<int> bands;
    for (int b = 0; b < m_rta->bandCount(); ++b)
        if (m_rta->bandVisible(b)) bands.push_back(b);
    if (bands.empty()) return;

    // This channel's frame, if the picture is of this tap and has one
    const RtaSnapshot &snap = m_rta->snapshot();
    const RtaChannelFrame *frame = nullptr;
    for (int f = 0; snap.tap == m_tap && f < std::min<int>(snap.count, RTA_MAX_CHANNELS); ++f)
        if (snap.frames[f].channel == m_channel) frame = &snap.frames[f];
    std::vector<double> levels, holds;
    levels.reserve(bands.size());
    holds.reserve(bands.size());
    for (int b : bands) {
        const bool known = frame && b < frame->n_bands;
        levels.push_back(known ? frame->avg_db[b] : RTA_FLOOR_DB);
        holds.push_back(known ? frame->peak_db[b] : RTA_FLOOR_DB);
    }
    // A new channel, side or band count starts without a glide
    const int series = ((m_tap << 6) | m_channel) << 6 | int(bands.size());
    if (series != m_series) {
        m_level.forget();
        m_hold.forget();
        m_series = series;
    }
    const double tau = m_active ? m_rta->smoothingTau() : 0;
    bool moving = m_level.advance(levels, dt, tau, false);
    moving = m_hold.advance(holds, dt, tau, true) || moving;

    const double slot = plot.width() / double(bands.size());
    const double gap = std::clamp(0.18 * slot, 0.5, 2.0);
    const double barWidth = std::max(1.0, slot - gap);
    const double corner = std::min(1.5, barWidth / 3);
    QColor top = m_tint, foot = m_tint, cap = m_tint;
    top.setAlphaF(0.95);
    foot.setAlphaF(0.45);
    cap.setAlphaF(0.9);
    const bool withCaps = m_rta->showPeakHold();
    int drawn = 0;
    g.setPen(Qt::NoPen);
    for (size_t pos = 0; pos < bands.size(); ++pos) {
        const double left = plot.left() + slot * double(pos) + gap / 2;
        const double rise = plot.height() * heightOf(m_level.values()[pos]);
        if (rise > 0.001 * plot.height()) {
            ++drawn;
            const QRectF bar(left, plot.bottom() - rise, barWidth, rise);
            QLinearGradient shade(bar.topLeft(), QPointF(left, plot.bottom()));
            shade.setColorAt(0, top);
            shade.setColorAt(1, foot);
            g.setBrush(shade);
            g.drawRoundedRect(bar, corner, corner);
        }
        const double peak = plot.height() * heightOf(m_hold.values()[pos]);
        if (withCaps && peak > 0.001 * plot.height())
            g.fillRect(QRectF(left, std::max(plot.top(), plot.bottom() - peak - 1), barWidth, 1.5), cap);
    }
    if (m_frequencyLabels) paintFrequencies(g, plot, bands, slot);
    const bool changed = drawn != m_drawn;
    m_drawn = drawn;
    afterPaint(changed, moving);
}
