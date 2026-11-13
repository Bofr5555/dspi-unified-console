// -- WP QP8 -- The Tube Modeller's tube (TubeIllustration.h).
#include "TubeIllustration.h"

#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <QRadialGradient>
#include <algorithm>
#include <vector>

namespace {

// One tube on the 120 x 200 design grid (the Mac's TubeGeometry): x runs
// -60..60 about the axis, y from 0 at the top to 200 at the pin tips.
struct Plate {
    QRectF rect;
    double fins = 0;            // the side flanges of a power plate
    std::vector<double> ribs;   // pressed stiffening ribs
    bool mesh = false;          // the 300B's woven plate
};
struct Glow {
    QPointF at;
    double radius;
};
struct Rod {
    double x, from, to;
};
struct Mica {
    double y, half;
};
struct Geometry {
    // The envelope's left edge as (half-width, y), bottom to top; mirrored, closed by a dome at `top`
    std::vector<QPointF> profile;
    double top = 0;
    bool metal = false;
    bool bakelite = false;      // an octal base under the glass
    double baseTop = 0, baseBottom = 0, baseTopHalf = 0, baseBottomHalf = 0;
    bool key = false;
    std::vector<double> pins;
    double pinTop = 0, pinBottom = 0, pinWidth = 1.6;
    bool getter = false;
    double getterFrom = 0, getterTo = 0;
    std::vector<Mica> micas;
    std::vector<Rod> rods;
    std::vector<Plate> plates;
    std::vector<QPointF> filament;
    std::vector<Glow> glows;
    bool ring = false;          // the pressed ring of a steel envelope
    double ringFrom = 0, ringTo = 0;

    double glassBottom() const { return profile.front().y(); }
    double widestHalf() const {
        double w = 0;
        for (const QPointF &p : profile) w = std::max(w, p.x());
        return w;
    }
};

void base(Geometry &g, double top, double bottom, double topHalf, double bottomHalf, bool key) {
    g.bakelite = true;
    g.baseTop = top;
    g.baseBottom = bottom;
    g.baseTopHalf = topHalf;
    g.baseBottomHalf = bottomHalf;
    g.key = key;
}
void pins(Geometry &g, std::vector<double> xs, double top, double bottom, double width) {
    g.pins = std::move(xs);
    g.pinTop = top;
    g.pinBottom = bottom;
    g.pinWidth = width;
}
void getter(Geometry &g, double from, double to) {
    g.getter = true;
    g.getterFrom = from;
    g.getterTo = to;
}

// The Mac's geometry table, one entry per family
Geometry geometryOf(int family) {
    Geometry g;
    switch (family) {
    case 1: // noval pentode (EF86)
        g.profile = { { 28, 166 }, { 28, 72 } };
        g.top = 52;
        pins(g, { -20, -10, 0, 10, 20 }, 166, 190, 1.6);
        getter(g, 52, 67);
        g.micas = { { 80, 25 }, { 142, 25 } };
        g.rods = { { -22, 76, 164 }, { 22, 76, 164 } };
        g.plates = { { QRectF(-17, 86, 34, 52), 0, { -6, 6 } } };
        g.glows = { { { 0, 81 }, 14 }, { { 0, 143 }, 12 } };
        break;
    case 2: // noval power (EL84)
        g.profile = { { 32, 168 }, { 32, 58 } };
        g.top = 34;
        pins(g, { -22, -11, 0, 11, 22 }, 168, 192, 1.6);
        getter(g, 34, 50);
        g.micas = { { 60, 29 }, { 144, 29 } };
        g.rods = { { -29, 56, 166 }, { 29, 56, 166 } };
        g.plates = { { QRectF(-20, 66, 40, 72), 5, { -7, 7 } } };
        g.glows = { { { 0, 61 }, 16 }, { { 0, 144 }, 14 } };
        break;
    case 3: // octal glass (6SN7, 6SL7)
        g.profile = { { 30, 148 }, { 30, 50 } };
        g.top = 26;
        base(g, 146, 176, 33, 31, true);
        pins(g, { -21, -7, 7, 21 }, 176, 196, 2.6);
        getter(g, 26, 41);
        g.micas = { { 58, 27 }, { 128, 27 } };
        g.rods = { { -26, 54, 146 }, { 0, 54, 146 }, { 26, 54, 146 } };
        g.plates = { { QRectF(-22, 62, 19, 62), 0, { -12.5 } }, { QRectF(3, 62, 19, 62), 0, { 12.5 } } };
        g.glows = { { { -12.5, 58 }, 12 }, { { 12.5, 58 }, 12 }, { { -12.5, 128 }, 11 }, { { 12.5, 128 }, 11 } };
        break;
    case 4: // octal metal (6SJ7): no glow but a faint warmth where the can meets the base
        g.profile = { { 27, 150 }, { 27, 60 } };
        g.top = 52;
        g.metal = true;
        base(g, 148, 176, 30, 28, true);
        pins(g, { -19, -6.5, 6.5, 19 }, 176, 196, 2.6);
        g.glows = { { { 0, 149 }, 9 } };
        g.ring = true;
        g.ringFrom = 126;
        g.ringTo = 132;
        break;
    case 5: // large octal power (EL34)
        g.profile = { { 34, 152 }, { 34, 44 } };
        g.top = 14;
        base(g, 150, 180, 37, 35, true);
        pins(g, { -24, -8, 8, 24 }, 180, 199, 2.8);
        getter(g, 14, 31);
        g.micas = { { 46, 31 }, { 134, 31 } };
        g.rods = { { -30, 42, 150 }, { 30, 42, 150 } };
        g.plates = { { QRectF(-23, 52, 46, 78), 6, { 0 } } };
        g.glows = { { { 0, 47 }, 18 }, { { 0, 135 }, 16 } };
        break;
    case 6: // small octal power (6V6)
        g.profile = { { 27, 152 }, { 27, 66 } };
        g.top = 44;
        base(g, 150, 178, 31, 29, true);
        pins(g, { -20, -7, 7, 20 }, 178, 197, 2.6);
        getter(g, 44, 59);
        g.micas = { { 68, 24 }, { 136, 24 } };
        g.rods = { { -24, 64, 150 }, { 24, 64, 150 } };
        g.plates = { { QRectF(-17, 74, 34, 58), 5, { 0 } } };
        g.glows = { { { 0, 69 }, 14 }, { { 0, 137 }, 12 } };
        break;
    case 7: // shouldered power (6L6, the "coke bottle")
        g.profile = { { 26, 152 }, { 30, 140 }, { 36, 116 }, { 36, 104 }, { 27, 76 }, { 26, 60 } };
        g.top = 24;
        base(g, 150, 180, 34, 32, true);
        pins(g, { -22, -7, 7, 22 }, 180, 199, 2.8);
        getter(g, 24, 40);
        g.micas = { { 66, 22 }, { 136, 30 } };
        g.rods = { { -24, 62, 150 }, { 24, 62, 150 } };
        g.plates = { { QRectF(-20, 72, 40, 60), 7, { 0 } } };
        g.glows = { { { 0, 68 }, 16 }, { { 0, 137 }, 14 } };
        break;
    case 8: // beam bottle (KT88)
        g.profile = { { 30, 156 }, { 32, 144 }, { 42, 108 }, { 42, 40 } };
        g.top = 8;
        base(g, 154, 184, 36, 34, true);
        pins(g, { -24, -8, 8, 24 }, 184, 200, 2.8);
        getter(g, 8, 25);
        g.micas = { { 46, 39 }, { 138, 36 } };
        g.rods = { { -34, 42, 154 }, { 34, 42, 154 } };
        g.plates = { { QRectF(-26, 54, 52, 80), 7, { -9, 9 } } };
        g.glows = { { { 0, 49 }, 20 }, { { 0, 139 }, 16 } };
        break;
    case 9: // directly heated (300B): the filament glows, strung above the plate
        g.profile = { { 22, 162 }, { 25, 152 }, { 42, 118 }, { 43, 98 }, { 34, 56 }, { 23, 36 } };
        g.top = 10;
        base(g, 160, 188, 36, 34, false);
        pins(g, { -13, 13 }, 188, 200, 4.5);
        getter(g, 10, 22);
        g.micas = { { 50, 24 }, { 140, 27 } };
        g.rods = { { -24, 48, 160 }, { 24, 48, 160 }, { -7, 50, 57 }, { 7, 50, 57 } };
        g.plates = { { QRectF(-19, 74, 38, 62), 0, {}, true } };
        g.filament = { { -15, 74 }, { -7, 57 }, { 0, 74 }, { 7, 57 }, { 15, 74 } };
        g.glows = { { { 0, 66 }, 34 } };
        break;
    default: // noval dual triode (12AX7 and kin), also Custom's
        g.profile = { { 28, 166 }, { 28, 72 } };
        g.top = 52;
        pins(g, { -20, -10, 0, 10, 20 }, 166, 190, 1.6);
        getter(g, 52, 67);
        g.micas = { { 80, 25 }, { 142, 25 } };
        g.rods = { { -25, 76, 164 }, { 0, 76, 164 }, { 25, 76, 164 } };
        g.plates = { { QRectF(-23, 84, 20, 54), 0, { -13 } }, { QRectF(3, 84, 20, 54), 0, { 13 } } };
        g.glows = { { { -13, 80 }, 12 }, { { 13, 80 }, 12 }, { { -13, 142 }, 11 }, { { 13, 142 }, 11 } };
        break;
    }
    return g;
}

// The artwork's own colours
QColor grey(double white, double alpha = 1) { return QColor::fromRgbF(float(white), float(white), float(white), float(alpha)); }
QColor whiteA(double alpha) { return grey(1, alpha); }
QColor blackA(double alpha) { return grey(0, alpha); }
const QColor kHeater = QColor::fromRgbF(1.0f, 0.52f, 0.16f);
const QColor kCore = QColor::fromRgbF(1.0f, 0.78f, 0.45f);
QColor withAlpha(QColor c, double a) {
    c.setAlphaF(float(a));
    return c;
}

// Catmull-Rom through `pts`, from the path's current point
void smooth(QPainterPath &path, const std::vector<QPointF> &pts) {
    if (pts.size() < 2) return;
    for (size_t i = 0; i + 1 < pts.size(); ++i) {
        const QPointF p0 = pts[i == 0 ? 0 : i - 1], p1 = pts[i], p2 = pts[i + 1];
        const QPointF p3 = pts[std::min(i + 2, pts.size() - 1)];
        path.cubicTo(p1 + (p2 - p0) / 6, p2 - (p3 - p1) / 6, p2);
    }
}

QPainterPath envelope(const Geometry &g) {
    std::vector<QPointF> left, right;
    for (const QPointF &p : g.profile) left.emplace_back(-p.x(), p.y());
    for (auto it = g.profile.rbegin(); it != g.profile.rend(); ++it) right.push_back(*it);
    const QPointF last = g.profile.back();
    constexpr double k = 0.5523;
    const double rise = last.y() - g.top;
    QPainterPath p;
    p.moveTo(left.front());
    smooth(p, left);
    // The crown: two quarter ellipses meeting at the top
    p.cubicTo(QPointF(-last.x(), last.y() - rise * k), QPointF(-last.x() * k, g.top), QPointF(0, g.top));
    p.cubicTo(QPointF(last.x() * k, g.top), QPointF(last.x(), last.y() - rise * k), last);
    smooth(p, right);
    // A noval tube's glass ends in a pressed button that bulges slightly
    p.quadTo(QPointF(0, g.glassBottom() + 5), left.front());
    p.closeSubpath();
    return p;
}

double halfWidth(const Geometry &g, double y) {
    const auto &pts = g.profile;
    for (size_t i = 0; i + 1 < pts.size(); ++i) {
        if (y <= pts[i].y() && y >= pts[i + 1].y()) {
            const double t = (pts[i].y() - y) / std::max(pts[i].y() - pts[i + 1].y(), 0.001);
            return pts[i].x() + (pts[i + 1].x() - pts[i].x()) * t;
        }
    }
    return pts.back().x();
}

QLinearGradient across(double x0, double x1, std::initializer_list<std::pair<double, QColor>> stops) {
    QLinearGradient grad(x0, 0, x1, 0);
    for (const auto &s : stops) grad.setColorAt(s.first, s.second);
    return grad;
}

void drawPins(QPainter &p, const Geometry &g) {
    for (double x : g.pins) {
        const QRectF pin(x - g.pinWidth / 2, g.pinTop - 2, g.pinWidth, g.pinBottom - g.pinTop + 2);
        p.setPen(Qt::NoPen);
        p.setBrush(across(pin.left(), pin.right(), { { 0, grey(0.45) }, { 0.5, grey(0.85) }, { 1, grey(0.5) } }));
        p.drawRoundedRect(pin, g.pinWidth / 2, g.pinWidth / 2);
    }
}

void drawBase(QPainter &p, const Geometry &g) {
    if (!g.bakelite) return;
    const QColor dark = QColor::fromRgbF(0.09f, 0.07f, 0.06f), mid = QColor::fromRgbF(0.25f, 0.19f, 0.15f),
                 edge = QColor::fromRgbF(0.11f, 0.08f, 0.07f);
    p.setPen(Qt::NoPen);
    if (g.key) {
        // The octal locating key between the pins
        p.setBrush(QColor::fromRgbF(0.16f, 0.12f, 0.10f));
        p.drawRoundedRect(QRectF(-2.8, g.baseBottom - 2, 5.6, 9), 1.5, 1.5);
    }
    constexpr double r = 3.5;
    QPainterPath b;
    b.moveTo(-g.baseTopHalf, g.baseTop);
    b.lineTo(g.baseTopHalf, g.baseTop);
    b.lineTo(g.baseBottomHalf, g.baseBottom - r);
    b.quadTo(g.baseBottomHalf, g.baseBottom, g.baseBottomHalf - r, g.baseBottom);
    b.lineTo(-g.baseBottomHalf + r, g.baseBottom);
    b.quadTo(-g.baseBottomHalf, g.baseBottom, -g.baseBottomHalf, g.baseBottom - r);
    b.closeSubpath();
    p.setBrush(across(-g.baseTopHalf, g.baseTopHalf, { { 0, dark }, { 0.5, mid }, { 1, edge } }));
    p.setPen(QPen(whiteA(0.14), 0.7));
    p.drawPath(b);
    p.setPen(QPen(whiteA(0.18), 0.6));
    p.drawLine(QPointF(-g.baseTopHalf + 1, g.baseTop + 1.2), QPointF(g.baseTopHalf - 1, g.baseTop + 1.2));
}

void drawPlate(QPainter &p, const Plate &plate) {
    const QRectF r = plate.rect;
    if (plate.fins > 0) {
        for (double side : { r.left() - plate.fins, r.right() }) {
            p.setBrush(grey(0.3));
            p.setPen(QPen(whiteA(0.18), 0.5));
            p.drawRect(QRectF(side, r.top() + 4, plate.fins, r.height() - 8));
        }
    }
    QPainterPath body;
    body.addRoundedRect(r, 1.2, 1.2);
    p.setPen(Qt::NoPen);
    p.setBrush(across(r.left(), r.right(), { { 0, grey(0.2) }, { 0.33, grey(0.36) }, { 0.66, grey(0.24) }, { 1, grey(0.16) } }));
    p.drawPath(body);
    if (plate.mesh) {
        p.save();
        p.setClipPath(body, Qt::IntersectClip);
        p.setPen(QPen(whiteA(0.13), 0.45));
        for (double x = r.left() - r.height(); x < r.right(); x += 3.2) {
            p.drawLine(QPointF(x, r.bottom()), QPointF(x + r.height(), r.top()));
            p.drawLine(QPointF(x, r.top()), QPointF(x + r.height(), r.bottom()));
        }
        p.restore();
    }
    for (double rib : plate.ribs) {
        p.setPen(QPen(blackA(0.45), 1.0));
        p.drawLine(QPointF(rib, r.top() + 2), QPointF(rib, r.bottom() - 2));
        p.setPen(QPen(whiteA(0.14), 0.6));
        p.drawLine(QPointF(rib + 0.9, r.top() + 2), QPointF(rib + 0.9, r.bottom() - 2));
    }
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(whiteA(0.22), 0.6));
    p.drawPath(body);
}

void drawCan(QPainter &p, const Geometry &g, const QPainterPath &env) {
    const double w = g.widestHalf();
    const auto steel = [](double x0, double x1) {
        return across(x0, x1, { { 0, grey(0.3) }, { 0.28, grey(0.66) }, { 0.6, grey(0.48) }, { 1, grey(0.26) } });
    };
    const QRectF band(-w - 3, g.ringFrom, (w + 3) * 2, g.ringTo - g.ringFrom);
    p.setPen(Qt::NoPen);
    if (g.ring) {
        p.setBrush(steel(-w - 3, w + 3));
        p.drawRoundedRect(band, 2, 2);
    }
    p.setBrush(steel(-w, w));
    p.drawPath(env);
    if (g.ring) {
        p.setBrush(steel(-w - 3, w + 3));
        p.setPen(QPen(blackA(0.35), 0.6));
        p.drawRoundedRect(band, 2, 2);
    }
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(whiteA(0.25), 0.8));
    p.drawPath(env);
    // The sealing cap on the crown
    p.setBrush(grey(0.55));
    p.setPen(QPen(blackA(0.3), 0.5));
    p.drawRoundedRect(QRectF(-7, g.top - 3, 14, 4), 1.5, 1.5);
}

void drawTube(QPainter &p, const Geometry &g) {
    const QPainterPath env = envelope(g);
    const double w = g.widestHalf();

    // The contact shadow on the shelf
    QRadialGradient shadow(QPointF(0, g.pinBottom + 0.5), w * 0.9);
    shadow.setColorAt(0, blackA(0.45));
    shadow.setColorAt(1, blackA(0));
    p.save();
    p.translate(0, g.pinBottom + 0.5);
    p.scale(1, 4.0 / (w * 0.9));
    p.translate(0, -(g.pinBottom + 0.5));
    p.setPen(Qt::NoPen);
    p.setBrush(shadow);
    p.drawEllipse(QPointF(0, g.pinBottom + 0.5), w * 0.9, w * 0.9);
    p.restore();

    drawPins(p, g);
    drawBase(p, g);
    if (g.metal) {
        drawCan(p, g, env);
        return;
    }

    // The glass: a faint cylinder shade, lighter at the rims
    p.setPen(Qt::NoPen);
    p.setBrush(blackA(0.22));
    p.drawPath(env);
    p.setBrush(across(-w, w, { { 0, whiteA(0.16) }, { 0.2, whiteA(0.04) }, { 0.65, whiteA(0.02) }, { 0.92, whiteA(0.10) },
                              { 1, whiteA(0.14) } }));
    p.drawPath(env);

    p.save();
    p.setClipPath(env, Qt::IntersectClip);
    p.setPen(QPen(whiteA(0.32), 0.8));
    for (const Rod &rod : g.rods) p.drawLine(QPointF(rod.x, rod.from), QPointF(rod.x, rod.to));
    for (const Plate &plate : g.plates) drawPlate(p, plate);
    p.setPen(Qt::NoPen);
    p.setBrush(whiteA(0.38));
    for (const Mica &m : g.micas) p.drawRoundedRect(QRectF(-m.half, m.y - 0.9, m.half * 2, 1.8), 0.9, 0.9);
    if (!g.filament.empty()) {
        // The unlit filament, a dull wire; the heater layer lights it
        QPen wire(whiteA(0.45), 0.9);
        wire.setJoinStyle(Qt::RoundJoin);
        p.setPen(wire);
        p.setBrush(Qt::NoBrush);
        p.drawPolyline(g.filament.data(), int(g.filament.size()));
    }
    if (g.getter) {
        // The silvered flash the getter leaves inside the crown
        QLinearGradient flash(0, g.getterFrom, 0, g.getterTo);
        flash.setColorAt(0, grey(0.88));
        flash.setColorAt(0.45, grey(0.62, 0.95));
        flash.setColorAt(1, grey(0.4, 0));
        p.setPen(Qt::NoPen);
        p.setBrush(flash);
        p.drawRect(QRectF(-60, g.getterFrom - 2, 120, g.getterTo - g.getterFrom + 2));
    }
    // A reflection down the left flank
    const double reflTop = g.profile.back().y() - 2, reflBottom = g.glassBottom() - 10;
    QPainterPath refl;
    for (int i = 0; i <= 12; ++i) {
        const double y = reflTop + (reflBottom - reflTop) * i / 12;
        const QPointF pt(-halfWidth(g, y) + 5, y);
        if (i == 0) refl.moveTo(pt);
        else refl.lineTo(pt);
    }
    QLinearGradient sheen(0, reflTop, 0, reflBottom);
    sheen.setColorAt(0, whiteA(0.2));
    sheen.setColorAt(1, whiteA(0.02));
    QPen reflPen(QBrush(sheen), 2.5);
    reflPen.setCapStyle(Qt::RoundCap);
    p.setPen(reflPen);
    p.setBrush(Qt::NoBrush);
    p.drawPath(refl);
    p.restore();

    p.setPen(QPen(whiteA(0.5), 1.0));
    p.setBrush(Qt::NoBrush);
    p.drawPath(env);
    // The exhaust tip, where the glass was sealed
    p.setBrush(whiteA(0.12));
    p.setPen(QPen(whiteA(0.5), 0.9));
    p.drawRoundedRect(QRectF(-2.6, g.top - 6, 5.2, 7.5), 2.6, 2.6);
}

// The heater's glow, or with `bloom` the flare over it while the stage is driven
void drawGlow(QPainter &p, const Geometry &g, bool bloom) {
    const QPainterPath env = envelope(g);
    const bool dht = !g.filament.empty();
    const QColor color = dht ? kCore : kHeater;
    p.setPen(Qt::NoPen);
    if (!g.metal && !bloom) {
        // A faint warm cast through the whole envelope
        p.setBrush(withAlpha(kHeater, 0.05));
        p.drawPath(env);
    }
    for (const Glow &glow : g.glows) {
        const double r = bloom ? glow.radius * 1.7 : glow.radius;
        QRadialGradient halo(glow.at, r);
        halo.setColorAt(0, withAlpha(color, (bloom ? 0.35 : 0.55) * (g.metal ? 0.45 : 1)));
        halo.setColorAt(0.45, withAlpha(kHeater, bloom ? 0.12 : 0.2));
        halo.setColorAt(1, withAlpha(kHeater, 0));
        p.setBrush(halo);
        p.drawEllipse(glow.at, r, r);
        if (bloom || dht || g.metal) continue;
        // Where the heater shows: a short bright bar inside a softer one
        const QRectF streak(glow.at.x() - 1.1, glow.at.y() - 2.6, 2.2, 5.2);
        p.setBrush(withAlpha(kHeater, 0.45));
        p.drawRoundedRect(streak.adjusted(-1.2, -1.2, 1.2, 1.2), 2.3, 2.3);
        p.setBrush(kCore);
        p.drawRoundedRect(streak, 1.1, 1.1);
    }
    if (dht) {
        // The soft edge as two wider, fainter strokes under the wire
        p.setBrush(Qt::NoBrush);
        for (double extra : { 4.0, 2.0 }) {
            QPen soft(withAlpha(kHeater, bloom ? 0.25 : 0.35), (bloom ? 3 : 2.2) + extra);
            soft.setJoinStyle(Qt::RoundJoin);
            soft.setCapStyle(Qt::RoundCap);
            p.setPen(soft);
            p.drawPolyline(g.filament.data(), int(g.filament.size()));
        }
        if (!bloom) {
            QPen hot(QColor::fromRgbF(1.0f, 0.9f, 0.7f), 1.0);
            hot.setJoinStyle(Qt::RoundJoin);
            p.setPen(hot);
            p.drawPolyline(g.filament.data(), int(g.filament.size()));
        }
    }
}

} // namespace

TubeIllustration::TubeIllustration(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAntialiasing(true);
    setAcceptedMouseButtons(Qt::NoButton);
}

void TubeIllustration::setFamily(int family) {
    family = std::clamp(family, 0, kFamilies - 1);
    if (family == m_family) return;
    m_family = family;
    emit familyChanged();
    update();
}

void TubeIllustration::setPart(Part part) {
    if (part == m_part) return;
    m_part = part;
    emit partChanged();
    update();
}

void TubeIllustration::paint(QPainter *painter) {
    if (width() <= 0 || height() <= 0) return;
    const Geometry g = geometryOf(m_family);
    painter->setRenderHint(QPainter::Antialiasing);
    // The design grid scaled to fit and centred, never stretched
    const double s = std::min(height() / 200, width() / 120);
    painter->translate(width() / 2, (height() - 200 * s) / 2);
    painter->scale(s, s);
    if (m_part == Tube) drawTube(*painter, g);
    else drawGlow(*painter, g, m_part == Bloom);
}
