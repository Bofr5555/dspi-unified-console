// -- WP QP9 -- One channel's spectrum bars (RtaBars.h).
#include "RtaBars.h"

#include "CoreApi.h"
#include "Registrar.h"
#include "RtaController.h"

#include <QLinearGradient>
#include <QPainter>
#include <algorithm>
#include <cmath>
#include <vector>

namespace {

constexpr double kLabelRow = 12;
constexpr double kLabelled[] = { 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000 };
// Width of one label character (the Mac's estimate for its 8 pt digits)
constexpr double kCharWidth = 4.9;

double norm(double db, double floor, double ceiling) {
    if (!std::isfinite(db) || ceiling <= floor) return 0;
    return std::clamp((db - floor) / (ceiling - floor), 0.0, 1.0);
}

bool decade(double hz) { return hz == 10 || hz == 100 || hz == 1000 || hz == 10000; }

} // namespace

RtaBars::RtaBars(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAntialiasing(true);
    setAcceptedMouseButtons(Qt::NoButton);
    m_clock.start();
    connect(this, &RtaBars::lookChanged, this, [this] { update(); });
    m_rta = Registrar::instance<RtaController>();
    if (m_rta) {
        connect(m_rta, &RtaController::pictureChanged, this, [this] { if (m_active) update(); });
        connect(m_rta, &RtaController::scaleChanged, this, [this] { update(); });
    }
}

void RtaBars::paint(QPainter *p) {
    const double dt = std::clamp(m_clock.restart() / 1000.0, 0.0, 0.25);
    if (!m_rta || width() < 4 || height() < 4) return;
    const QRectF plot(0, 0, width(), height() - (m_freqLabels ? kLabelRow : 0));
    const double floor = m_rta->floorDb(), ceiling = m_rta->ceilingDb();
    const auto yOf = [&](double db) { return plot.bottom() - plot.height() * norm(db, floor, ceiling); };
    p->setRenderHint(QPainter::Antialiasing, true);
    p->setFont(m_labelFont);

    if (m_dbLabels) {
        for (double db = std::floor(ceiling / 12) * 12; db > floor; db -= 12) {
            const double y = yOf(db);
            QColor c = m_gridColor;
            c.setAlphaF(db == 0 ? 0.35 : 0.12);
            p->setPen(QPen(c, db == 0 ? 1.0 : 0.5));
            p->drawLine(QPointF(plot.left(), y), QPointF(plot.right(), y));
            QColor lc = m_labelColor;
            lc.setAlphaF(0.6);
            p->setPen(lc);
            p->drawText(QRectF(plot.right() - 42, y - 6, 40, 12), Qt::AlignRight | Qt::AlignTop, QString::number(int(db)));
        }
    }

    // The resolved bands; the rest are left out and the others share the width
    const RtaSnapshot &snap = m_rta->snapshot();
    const RtaCapsInfo &caps = m_rta->caps();
    std::vector<int> bands;
    for (int b = 0; b < m_rta->bandCount(); ++b)
        if (m_rta->bandVisible(b)) bands.push_back(b);
    if (bands.empty()) return;
    const RtaChannelFrame *frame = nullptr;
    if (snap.tap == m_tap)
        for (int f = 0; f < snap.count && f < RTA_MAX_CHANNELS; ++f)
            if (snap.frames[f].channel == m_channel) frame = &snap.frames[f];
    std::vector<double> avg, peak;
    for (int b : bands) {
        const bool have = frame && b < frame->n_bands;
        avg.push_back(have ? frame->avg_db[b] : RTA_FLOOR_DB);
        peak.push_back(have ? frame->peak_db[b] : RTA_FLOOR_DB);
    }
    // Another channel, tap or band count: no glide from the old picture
    const int key = (m_tap * 64 + m_channel) * 64 + int(bands.size());
    if (key != m_seriesKey) {
        m_avg.reset();
        m_peak.reset();
        m_seriesKey = key;
    }
    const double tau = m_active ? m_rta->smoothingTau() : 0;
    bool moving = m_avg.step(avg, dt, tau, false);
    moving = m_peak.step(peak, dt, tau, true) || moving;

    const double slot = plot.width() / double(bands.size());
    const double gap = std::clamp(slot * 0.18, 0.5, 2.0);
    const double barW = std::max(1.0, slot - gap);
    const bool peaks = m_rta->showPeakHold();
    int drawn = 0;
    QLinearGradient fill(0, plot.top(), 0, plot.bottom());
    for (size_t i = 0; i < bands.size(); ++i) {
        const double x = plot.x() + double(i) * slot + gap / 2;
        const double n = norm(m_avg.values()[i], floor, ceiling);
        if (n > 0.001) {
            ++drawn;
            const double h = plot.height() * n;
            QColor top = m_color, bottom = m_color;
            top.setAlphaF(0.95);
            bottom.setAlphaF(0.45);
            fill.setStart(0, plot.bottom() - h);
            fill.setStops({ { 0.0, top }, { 1.0, bottom } });
            const double r = std::min(1.5, barW / 3);
            p->setPen(Qt::NoPen);
            p->setBrush(fill);
            p->drawRoundedRect(QRectF(x, plot.bottom() - h, barW, h), r, r);
        }
        const double np = norm(m_peak.values()[i], floor, ceiling);
        if (peaks && np > 0.001) {
            QColor cap = m_color;
            cap.setAlphaF(0.9);
            p->fillRect(QRectF(x, std::max(plot.top(), plot.bottom() - plot.height() * np - 1), barW, 1.5), cap);
        }
    }

    if (m_freqLabels) {
        // Decades first, then the others where they fit with a 4 px gap
        struct Placed { double l, r; };
        std::vector<Placed> placed;
        p->setPen(m_labelColor);
        for (int pass = 0; pass < 2; ++pass) {
            for (size_t i = 0; i < bands.size(); ++i) {
                const double hz = caps.centres[bands[i]];
                if (hz <= 0) continue;
                for (double l : kLabelled) {
                    if (std::fabs(hz - l) > l * 0.03 || decade(l) != (pass == 0)) continue;
                    const QString text = RtaController::shortHz(hz);
                    const double w = text.size() * kCharWidth;
                    const double cx = plot.x() + (double(i) + 0.5) * slot;
                    const Placed me{ cx - w / 2, cx + w / 2 };
                    if (me.l < plot.left() || me.r > plot.right()) continue;
                    const bool clash = std::any_of(placed.begin(), placed.end(),
                                                   [&](const Placed &o) { return me.l < o.r + 4 && me.r + 4 > o.l; });
                    if (clash) continue;
                    placed.push_back(me);
                    p->drawText(QRectF(cx - 20, height() - 11, 40, 11), Qt::AlignHCenter | Qt::AlignTop, text);
                }
            }
        }
    }
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
